//! The search engine and its two users. The post-run token scan: after
//! the harness exits, every regular file and symbolic link target under
//! the run directory is searched (by `lstat`, following no links), and any
//! other special file but an overlay whiteout is a hit, for the bound credentials'
//! tokens in plain, base64, hex and reversed form, for Claude token
//! prefixes, and for JWTs that carry a Codex account ID; a hit deletes the
//! file and quarantines the run. And `scan-tree`, which searches the trees
//! a jail mounts from the host for a task's fingerprints (and optionally the
//! credentials). The engine checks a two-byte prefix table before its
//! four-byte index, so multi-gigabyte trees stay minutes long.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Result;

/// Token prefixes that count as a hit whatever the token, when a
/// token-length body follows them: a bare prefix (in docs or this crate's
/// own source) is not a token.
pub const PREFIXES: &[&str] = &["sk-ant-oat", "sk-ant-ort", "sk-ant-api"];

/// Token characters a prefix hit needs after the prefix.
pub const PREFIX_BODY: usize = 40;

/// One secret to look for. `name` is a key path such as
/// `claude.claudeAiOauth.accessToken`; the value never leaves memory.
pub struct Secret {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub path: PathBuf,
    pub credential: String,
    pub form: String,
}

/// What the engine looks for: `bytes`, then `body` token characters.
pub struct Pattern {
    pub label: String,
    pub form: String,
    pub bytes: Vec<u8>,
    pub body: usize,
}

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_encode(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..=chunk.len() {
            out.push(char::from(
                alphabet[((word >> (18 - 6 * index)) & 63) as usize],
            ));
        }
    }
    out
}

/// Decodes base64 or base64url without padding checks; `None` on a bad
/// character.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes().filter(|&byte| byte != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The base64 text of `value` at each of the three byte alignments, cut to
/// the characters that depend only on `value`.
fn base64_alignments(value: &[u8], alphabet: &[u8; 64]) -> Vec<String> {
    (0..3)
        .filter_map(|pad| {
            let mut bytes = vec![0u8; pad];
            bytes.extend_from_slice(value);
            let encoded = base64_encode(&bytes, alphabet);
            let start = [0, 2, 3][pad];
            let end = encoded.len().checked_sub(3)?;
            (end > start + 8).then(|| encoded[start..end].to_owned())
        })
        .collect()
}

/// Every form of every secret, then the token prefixes.
pub fn patterns(secrets: &[Secret]) -> Vec<Pattern> {
    let mut out = Vec::new();
    for secret in secrets.iter().filter(|secret| secret.value.len() >= 12) {
        let value = secret.value.as_bytes();
        let mut forms: Vec<(String, Vec<u8>)> = vec![
            ("plain".into(), value.to_vec()),
            ("reversed".into(), value.iter().rev().copied().collect()),
            ("hex".into(), crate::hex(value).into_bytes()),
            (
                "hex-upper".into(),
                crate::hex(value).to_uppercase().into_bytes(),
            ),
        ];
        for (name, alphabet) in [("base64", STANDARD), ("base64url", URL_SAFE)] {
            for (index, text) in base64_alignments(value, alphabet).into_iter().enumerate() {
                forms.push((format!("{name}-{index}"), text.into_bytes()));
            }
        }
        out.extend(forms.into_iter().map(|(form, bytes)| Pattern {
            label: secret.name.clone(),
            form,
            bytes,
            body: 0,
        }));
    }
    out.extend(PREFIXES.iter().map(|prefix| Pattern {
        label: format!("prefix {prefix}"),
        form: "plain".into(),
        bytes: prefix.as_bytes().to_vec(),
        body: PREFIX_BODY,
    }));
    out
}

/// What `mode` and `rdev` (from `lstat`) make a file that is not a regular
/// file, a directory or a symbolic link: a hit, since it may carry or hide
/// anything, unless it is an overlay whiteout (a 0:0 character device).
pub fn special_file(mode: u32, rdev: u64) -> Option<&'static str> {
    use rustix::fs::FileType;
    match FileType::from_raw_mode(mode) {
        FileType::RegularFile | FileType::Directory | FileType::Symlink => None,
        FileType::CharacterDevice if rdev == 0 => None,
        FileType::CharacterDevice => Some("character device"),
        FileType::BlockDevice => Some("block device"),
        FileType::Fifo => Some("fifo"),
        FileType::Socket => Some("socket"),
        FileType::Unknown => Some("unknown file type"),
    }
}

/// One hit: the file, the pattern's label and form. A file or directory
/// the engine cannot read is a hit labelled `unreadable`: it may hold
/// anything. A special file is labelled `special-file`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub path: PathBuf,
    pub label: String,
    pub form: String,
}

/// The search engine: a 64 KiB two-byte prefix table ahead of a four-byte
/// index over the patterns, plus JWTs carrying any of `account_ids`.
pub struct Engine {
    patterns: Vec<Pattern>,
    first: Vec<bool>,
    index: HashMap<[u8; 4], Vec<usize>>,
    overlap: usize,
    account_ids: Vec<String>,
}

fn pair(bytes: &[u8]) -> usize {
    (usize::from(bytes[0]) << 8) | usize::from(bytes[1])
}

impl Engine {
    pub fn new(patterns: Vec<Pattern>, account_ids: Vec<String>) -> Result<Self> {
        let mut first = vec![false; 1 << 16];
        let mut index: HashMap<[u8; 4], Vec<usize>> = HashMap::new();
        for (number, pattern) in patterns.iter().enumerate() {
            let key: [u8; 4] = pattern
                .bytes
                .get(..4)
                .and_then(|key| key.try_into().ok())
                .ok_or(format!("pattern {} is shorter than 4 bytes", pattern.label))?;
            first[pair(&key)] = true;
            index.entry(key).or_default().push(number);
        }
        if !account_ids.is_empty() {
            first[pair(b"eyJ")] = true;
        }
        let overlap = patterns
            .iter()
            .map(|pattern| pattern.bytes.len() + pattern.body)
            .max()
            .unwrap_or(4)
            .max(4096);
        Ok(Self {
            patterns,
            first,
            index,
            overlap,
            account_ids,
        })
    }

    /// Searches `root` (a directory, or one regular file), following no
    /// link but searching its target text, and returns every hit, sorted.
    pub fn tree(&self, root: &Path) -> Vec<Hit> {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        let mut hits = Vec::new();
        let unreadable = |path: &Path, err: std::io::Error| Hit {
            path: path.to_path_buf(),
            label: "unreadable".into(),
            form: err.kind().to_string(),
        };
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) => {
                    hits.push(unreadable(&path, err));
                    continue;
                }
            };
            let found = |found: Vec<(String, String)>| {
                found.into_iter().map(|(label, form)| Hit {
                    path: path.clone(),
                    label,
                    form,
                })
            };
            if let Some(kind) = special_file(metadata.mode(), metadata.rdev()) {
                hits.push(Hit {
                    path: path.clone(),
                    label: "special-file".into(),
                    form: kind.into(),
                });
            } else if metadata.file_type().is_symlink() {
                match std::fs::read_link(&path) {
                    Ok(target) => {
                        let mut text = Vec::new();
                        self.search(target.as_os_str().as_bytes(), &mut text);
                        hits.extend(found(text));
                    }
                    Err(err) => hits.push(unreadable(&path, err)),
                }
            } else if metadata.is_file() {
                match self.file(&path) {
                    Ok(text) => hits.extend(found(text)),
                    Err(err) => hits.push(unreadable(&path, err)),
                }
            } else if metadata.is_dir() {
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry {
                                Ok(entry) => stack.push(entry.path()),
                                Err(err) => hits.push(unreadable(&path, err)),
                            }
                        }
                    }
                    Err(err) => hits.push(unreadable(&path, err)),
                }
            }
        }
        hits.sort();
        hits.dedup();
        hits
    }

    fn file(&self, path: &Path) -> std::io::Result<Vec<(String, String)>> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)?;
        let mut hits = Vec::new();
        let mut window: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 4 << 20];
        loop {
            let count = file.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            window.extend_from_slice(&chunk[..count]);
            self.search(&window, &mut hits);
            if window.len() > self.overlap {
                window.drain(..window.len() - self.overlap);
            }
        }
        hits.sort();
        hits.dedup();
        Ok(hits)
    }

    fn search(&self, bytes: &[u8], hits: &mut Vec<(String, String)>) {
        for start in 0..bytes.len().saturating_sub(3) {
            if !self.first[pair(&bytes[start..])] {
                continue;
            }
            let key: [u8; 4] = [
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
            ];
            if let Some(candidates) = self.index.get(&key) {
                for &number in candidates {
                    let pattern = &self.patterns[number];
                    let body = &bytes[start + pattern.bytes.len().min(bytes.len() - start)..];
                    let token_body = body.len() >= pattern.body
                        && body[..pattern.body].iter().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
                        });
                    if bytes[start..].starts_with(&pattern.bytes) && token_body {
                        hits.push((pattern.label.clone(), pattern.form.clone()));
                    }
                }
            }
            if !self.account_ids.is_empty()
                && key[..3] == *b"eyJ"
                && let Some(account) = jwt_account(&bytes[start..], &self.account_ids)
            {
                hits.push((format!("jwt for account {account}"), "jwt".into()));
            }
        }
    }
}

/// The post-run token scan of `root`; never follows a link.
pub fn scan(root: &Path, secrets: &[Secret], account_ids: &[String]) -> Result<Vec<Finding>> {
    let engine = Engine::new(patterns(secrets), account_ids.to_vec())?;
    let mut findings: Vec<Finding> = engine
        .tree(root)
        .into_iter()
        .map(|hit| Finding {
            path: hit
                .path
                .strip_prefix(root)
                .unwrap_or(&hit.path)
                .to_path_buf(),
            credential: hit.label,
            form: hit.form,
        })
        .collect();
    findings.sort_by(|left, right| {
        (&left.path, &left.credential).cmp(&(&right.path, &right.credential))
    });
    findings.dedup();
    Ok(findings)
}

/// The kinds a needles file line may name.
pub const NEEDLE_KINDS: &[&str] = &["commit", "canary", "identifier"];

/// Parses a needles file: one `<kind> <text>` per line.
pub fn needles(text: &str) -> Result<Vec<Pattern>> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (kind, needle) = line
                .split_once(' ')
                .ok_or(format!("needle line {line:?} is not <kind> <text>"))?;
            if !NEEDLE_KINDS.contains(&kind) {
                return Err(format!("needle kind {kind} is not one of {NEEDLE_KINDS:?}"));
            }
            if needle.len() < 4 {
                return Err(format!("needle {needle:?} is shorter than 4 bytes"));
            }
            Ok(Pattern {
                label: kind.to_owned(),
                form: needle.to_owned(),
                bytes: needle.as_bytes().to_vec(),
                body: 0,
            })
        })
        .collect()
}

/// One `scan-tree` hit as printed: never a secret's value.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TreeHit {
    pub path: PathBuf,
    pub kind: String,
    pub needle: String,
}

/// `agent-jail scan-tree`: searches every path for the needles and the
/// credentials' secrets (named, never shown).
pub fn scan_tree(
    mut needles: Vec<Pattern>,
    secrets: &[Secret],
    account_ids: &[String],
    paths: &[PathBuf],
) -> Result<Vec<TreeHit>> {
    // The needles are what the gate is for (the landed diff, the commit, the canary);
    // credentials only add to them, so an empty needles list fails whatever else is given.
    if needles.is_empty() {
        return Err("scan-tree has no needle to look for".into());
    }
    if !secrets.is_empty() || !account_ids.is_empty() {
        needles.extend(patterns(secrets).into_iter().map(|pattern| Pattern {
            form: format!("{} {}", pattern.label, pattern.form),
            label: "credential".into(),
            ..pattern
        }));
    }
    let engine = Engine::new(needles, account_ids.to_vec())?;
    Ok(paths
        .iter()
        .flat_map(|path| engine.tree(path))
        .map(|hit| {
            let (kind, needle) = match hit.label.as_str() {
                kind if NEEDLE_KINDS.contains(&kind)
                    || matches!(kind, "credential" | "unreadable" | "special-file") =>
                {
                    (hit.label, hit.form)
                }
                // A JWT carrying an account ID: its label names the account's digest.
                _ => ("credential".to_owned(), hit.label),
            };
            TreeHit {
                path: hit.path,
                kind,
                needle,
            }
        })
        .collect())
}

/// The account ID a JWT at the start of `bytes` carries, if it is one of
/// `account_ids`.
fn jwt_account(bytes: &[u8], account_ids: &[String]) -> Option<String> {
    let token_char = |byte: &u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.');
    let end = bytes
        .iter()
        .position(|byte| !token_char(byte))
        .unwrap_or(bytes.len());
    let token = std::str::from_utf8(&bytes[..end]).ok()?;
    let mut parts = token.split('.');
    let (_, payload) = (parts.next()?, parts.next()?);
    let decoded = base64_decode(payload)?;
    let text = String::from_utf8_lossy(&decoded);
    account_ids
        .iter()
        .find(|account| text.contains(account.as_str()))
        .map(|account| crate::sha256_hex(account.as_bytes())[..12].to_owned())
}

#[derive(Serialize)]
struct Report<'a> {
    quarantined: bool,
    findings: &'a [Finding],
    not_deleted: &'a [String],
}

/// Deletes every file, link or special file with a finding (never a
/// directory, never a link's target) and records the paths and credential
/// names in `meta/token-scan.json`.
pub fn quarantine(root: &Path, findings: &[Finding], report: &Path) -> Result {
    let mut not_deleted = Vec::new();
    for finding in findings {
        let path = root.join(&finding.path);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_dir())
            && let Err(err) = std::fs::remove_file(&path)
        {
            not_deleted.push(format!("{}: {err}", finding.path.display()));
        }
    }
    let text = serde_json::to_string_pretty(&Report {
        quarantined: !findings.is_empty(),
        findings,
        not_deleted: &not_deleted,
    })
    .map_err(|err| err.to_string())?;
    std::fs::write(report, text + "\n").map_err(|err| format!("{}: {err}", report.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sx-test-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH";

    fn secrets() -> Vec<Secret> {
        vec![Secret {
            name: "fixture.accessToken".into(),
            value: TOKEN.into(),
        }]
    }

    #[test]
    fn base64_round_trips() {
        for text in ["", "a", "ab", "abc", "abcd", TOKEN] {
            let encoded = base64_encode(text.as_bytes(), STANDARD);
            assert_eq!(base64_decode(&encoded).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn every_form_is_found_and_quarantined() {
        let root = crate::test_dir("scan");
        let plants: Vec<(String, String)> = vec![
            ("plain.txt".into(), format!("x {TOKEN} y")),
            ("reversed.txt".into(), TOKEN.chars().rev().collect()),
            ("hex.txt".into(), crate::hex(TOKEN.as_bytes())),
            (
                "b64-0.txt".into(),
                base64_encode(TOKEN.as_bytes(), STANDARD),
            ),
            (
                "b64-1.txt".into(),
                base64_encode(format!("k{TOKEN}").as_bytes(), STANDARD),
            ),
            (
                "b64-2.txt".into(),
                base64_encode(format!("kk{TOKEN}").as_bytes(), URL_SAFE),
            ),
            (
                "prefix.txt".into(),
                format!("sk-ant-oat01-{}", "Ab9_-".repeat(10)),
            ),
            ("clean.txt".into(), "nothing here".into()),
            // A bare prefix, as in docs or this crate's own source.
            (
                "bare-prefix.txt".into(),
                "PREFIXES = [\"sk-ant-oat\", \"sk-ant-ort\"]; sk-ant-api01-short".into(),
            ),
        ];
        std::fs::create_dir(root.join("deep")).unwrap();
        for (name, text) in &plants {
            std::fs::write(root.join("deep").join(name), text).unwrap();
        }
        // A link out of the run must be neither followed nor deleted.
        let outside = crate::test_dir("scan-outside");
        std::fs::write(outside.join("secret.txt"), TOKEN).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dirlink")).unwrap();

        let findings = scan(&root, &secrets(), &[]).unwrap();
        let paths: Vec<String> = findings
            .iter()
            .map(|f| f.path.display().to_string())
            .collect();
        for (name, _) in plants
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "clean.txt" | "bare-prefix.txt"))
        {
            assert!(
                paths.contains(&format!("deep/{name}")),
                "{name} missing from {paths:?}"
            );
        }
        assert!(
            !paths.iter().any(|path| path.contains("clean")
                || path.contains("link")
                || path.contains("bare"))
        );

        let report = root.join("token-scan.json");
        quarantine(&root, &findings, &report).unwrap();
        assert!(!root.join("deep/plain.txt").exists());
        assert!(root.join("deep/clean.txt").exists());
        assert!(outside.join("secret.txt").exists());
        let text = std::fs::read_to_string(&report).unwrap();
        assert!(text.contains("\"quarantined\": true"));
        assert!(
            !text.contains(TOKEN) && !text.contains(&TOKEN[10..30]),
            "{text}"
        );
    }

    #[test]
    fn unreadable_entries_quarantine_and_the_report_is_always_written() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_dir("scan-unreadable");
        std::fs::create_dir(root.join("locked")).unwrap();
        std::fs::write(root.join("locked/encoded.txt"), "uryyb").unwrap();
        std::fs::write(root.join("hidden.txt"), "uryyb").unwrap();
        let mode = |path: &str, bits| {
            std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(bits))
                .unwrap()
        };
        mode("hidden.txt", 0o000);
        mode("locked", 0o000);
        // The test may run with CAP_DAC_OVERRIDE (root in a sandbox).
        let enforced = std::fs::read(root.join("hidden.txt")).is_err();
        let findings = scan(&root, &secrets(), &[]).unwrap();
        let report = root.join("token-scan.json");
        quarantine(&root, &findings, &report).unwrap();
        mode("locked", 0o755);
        if enforced {
            let unreadable: Vec<String> = findings
                .iter()
                .filter(|finding| finding.credential == "unreadable")
                .map(|finding| finding.path.display().to_string())
                .collect();
            assert_eq!(unreadable, ["hidden.txt", "locked"], "{findings:?}");
            let text = std::fs::read_to_string(&report).unwrap();
            assert!(text.contains("\"quarantined\": true"), "{text}");
        }
    }

    #[test]
    fn special_files_are_hits_except_whiteouts() {
        use rustix::fs::{FileType, makedev};
        let mode = |kind: FileType| kind.as_raw_mode() | 0o644;
        for kind in [
            FileType::RegularFile,
            FileType::Directory,
            FileType::Symlink,
        ] {
            assert_eq!(special_file(mode(kind), 0), None, "{kind:?}");
        }
        // An overlay whiteout is a 0:0 character device; any other is a hit.
        assert_eq!(special_file(mode(FileType::CharacterDevice), 0), None);
        assert_eq!(
            special_file(mode(FileType::CharacterDevice), makedev(1, 3)),
            Some("character device")
        );
        assert_eq!(
            special_file(mode(FileType::CharacterDevice), makedev(0, 1)),
            Some("character device")
        );
        for (kind, name) in [
            (FileType::BlockDevice, "block device"),
            (FileType::Fifo, "fifo"),
            (FileType::Socket, "socket"),
        ] {
            assert_eq!(special_file(mode(kind), 0), Some(name));
        }
    }

    #[test]
    fn link_targets_are_searched_and_special_files_quarantine() {
        let root = crate::test_dir("scan-special");
        let upper = root.join("overlay/target/upper");
        std::fs::create_dir_all(&upper).unwrap();
        std::fs::create_dir(root.join("workspace")).unwrap();
        let prefixed = format!("sk-ant-oat01-{}", "Ab9_-".repeat(10));
        std::os::unix::fs::symlink(&prefixed, root.join("workspace/leak")).unwrap();
        std::os::unix::fs::symlink(format!("x/{TOKEN}"), upper.join("leak")).unwrap();
        std::os::unix::fs::symlink("../clean/target", root.join("workspace/clean")).unwrap();
        crate::mknod(&upper.join("fifo"), rustix::fs::FileType::Fifo, 0).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(root.join("socket")).unwrap();
        // Unlinking a lower-layer file leaves a whiteout; Linux lets anyone
        // make one, but a sandbox may not.
        let whiteout = crate::mknod(
            &upper.join("deleted"),
            rustix::fs::FileType::CharacterDevice,
            0,
        );
        if let Err(err) = &whiteout {
            eprintln!("no whiteout here ({err}); its classification is unit-tested");
        }

        let findings = scan(&root, &secrets(), &[]).unwrap();
        let found: Vec<(String, &str)> = findings
            .iter()
            .map(|f| (f.path.display().to_string(), f.credential.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("overlay/target/upper/fifo".to_owned(), "special-file"),
                (
                    "overlay/target/upper/leak".to_owned(),
                    "fixture.accessToken"
                ),
                ("socket".to_owned(), "special-file"),
                ("workspace/leak".to_owned(), "prefix sk-ant-oat"),
            ]
        );
        quarantine(&root, &findings, &root.join("token-scan.json")).unwrap();
        for gone in [
            "workspace/leak",
            "overlay/target/upper/leak",
            "overlay/target/upper/fifo",
            "socket",
        ] {
            assert!(
                std::fs::symlink_metadata(root.join(gone)).is_err(),
                "{gone}"
            );
        }
        assert!(std::fs::symlink_metadata(root.join("workspace/clean")).is_ok());
        if whiteout.is_ok() {
            assert!(std::fs::symlink_metadata(upper.join("deleted")).is_ok());
        }
    }

    #[test]
    fn jwts_carrying_an_account_id_are_found() {
        let root = crate::test_dir("scan-jwt");
        let payload = base64_encode(br#"{"chatgpt_account_id":"acct-1234-5678"}"#, URL_SAFE);
        std::fs::write(
            root.join("session.jsonl"),
            format!("{{\"t\":\"eyJhbGciOiJIUzI1NiJ9.{payload}.sig\"}}"),
        )
        .unwrap();
        let findings = scan(&root, &[], &["acct-1234-5678".into()]).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].form, "jwt");
        assert!(!findings[0].credential.contains("acct-1234"));
    }

    #[test]
    fn hits_across_chunk_boundaries_are_found() {
        let root = crate::test_dir("scan-chunk");
        let mut big = vec![b'.'; (4 << 20) - 10];
        big.extend_from_slice(TOKEN.as_bytes());
        std::fs::write(root.join("big"), big).unwrap();
        assert_eq!(scan(&root, &secrets(), &[]).unwrap().len(), 1);
    }

    #[test]
    fn scan_tree_finds_needles_and_names_credentials_without_values() {
        let root = crate::test_dir("scan-tree");
        let commit = "c9de7044aa11bb22cc33dd44ee55ff6677889900";
        let found = needles(&format!(
            "commit {commit}\ncanary AGENT-BENCH-CANARY-1234\nidentifier parse_rota_overrides\n"
        ))
        .unwrap();
        std::fs::create_dir_all(root.join("tree/deep")).unwrap();
        std::fs::write(root.join("tree/deep/log"), format!("built from {commit}")).unwrap();
        // A needle across the engine's 4 MiB chunk boundary.
        let mut big = vec![b'x'; (4 << 20) - 7];
        big.extend_from_slice(b"parse_rota_overrides");
        std::fs::write(root.join("tree/big"), big).unwrap();
        std::fs::write(root.join("tree/clean"), "nothing").unwrap();
        std::fs::write(
            root.join("single"),
            format!("{TOKEN} AGENT-BENCH-CANARY-1234"),
        )
        .unwrap();
        std::os::unix::fs::symlink(root.join("single"), root.join("tree/link")).unwrap();
        let hits = scan_tree(
            found,
            &secrets(),
            &[],
            &[root.join("tree"), root.join("single")],
        )
        .unwrap();
        let summary: Vec<(String, &str, &str)> = hits
            .iter()
            .map(|hit| {
                (
                    hit.path.strip_prefix(&root).unwrap().display().to_string(),
                    hit.kind.as_str(),
                    hit.needle.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("tree/big".to_owned(), "identifier", "parse_rota_overrides"),
                ("tree/deep/log".to_owned(), "commit", commit),
                ("single".to_owned(), "canary", "AGENT-BENCH-CANARY-1234"),
                (
                    "single".to_owned(),
                    "credential",
                    "fixture.accessToken plain"
                ),
            ]
        );
        let printed = serde_json::to_string(&hits).unwrap();
        assert!(!printed.contains(TOKEN), "{printed}");
        // A clean tree passes; bad needle files are refused.
        assert!(
            scan_tree(
                needles("canary ZZZZ-not-here").unwrap(),
                &[],
                &[],
                &[root.join("tree/deep")]
            )
            .unwrap()
            .is_empty()
        );
        // A search for nothing cannot pass, and credentials do not stand in for needles.
        assert!(scan_tree(Vec::new(), &[], &[], &[root.join("tree")]).is_err());
        assert!(scan_tree(Vec::new(), &secrets(), &[], &[root.join("tree")]).is_err());
        assert!(
            scan_tree(
                needles("\n\n").unwrap(),
                &secrets(),
                &[],
                &[root.join("tree")]
            )
            .is_err()
        );
        assert!(needles("secret abcdef").is_err());
        assert!(needles("commit abc").is_err());
        assert!(needles("commitabcdef").is_err());
    }
}
