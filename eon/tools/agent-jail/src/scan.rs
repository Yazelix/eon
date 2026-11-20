//! The post-run token scan: after the harness exits, every regular file
//! under the run directory is searched (by `lstat`, following no links) for
//! the bound credentials' tokens in plain, base64, hex and reversed form,
//! for Claude token prefixes, and for JWTs that carry a Codex account ID.
//! A hit deletes the file and quarantines the run.

use std::collections::HashMap;
use std::io::Read;
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

struct Pattern {
    credential: String,
    form: String,
    bytes: Vec<u8>,
    /// Token characters that must follow `bytes` (0: none).
    body: usize,
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

fn patterns(secrets: &[Secret]) -> Vec<Pattern> {
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
            credential: secret.name.clone(),
            form,
            bytes,
            body: 0,
        }));
    }
    out.extend(PREFIXES.iter().map(|prefix| Pattern {
        credential: format!("prefix {prefix}"),
        form: "plain".into(),
        bytes: prefix.as_bytes().to_vec(),
        body: PREFIX_BODY,
    }));
    out
}

/// Searches `root` and returns every finding; never follows a link.
pub fn scan(root: &Path, secrets: &[Secret], account_ids: &[String]) -> Result<Vec<Finding>> {
    let patterns = patterns(secrets);
    let mut index: HashMap<[u8; 4], Vec<usize>> = HashMap::new();
    for (number, pattern) in patterns.iter().enumerate() {
        let key: [u8; 4] = pattern.bytes[..4].try_into().map_err(|_| "short pattern")?;
        index.entry(key).or_default().push(number);
    }
    let longest = patterns
        .iter()
        .map(|pattern| pattern.bytes.len())
        .max()
        .unwrap_or(4)
        .max(4096);
    let mut findings = Vec::new();
    // Anything the scan cannot read (a jailed command can chmod 000 its own
    // files) quarantines the run: it may hold a token in any form.
    let unreadable = |path: &Path, err: std::io::Error| Finding {
        path: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
        credential: "unreadable".into(),
        form: err.kind().to_string(),
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(err) => {
                findings.push(unreadable(&directory, err));
                continue;
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(err) => {
                    findings.push(unreadable(&directory, err));
                    continue;
                }
            };
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) => {
                    findings.push(unreadable(&path, err));
                    continue;
                }
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                match scan_file(&path, &patterns, &index, longest, account_ids) {
                    Ok(hits) => {
                        findings.extend(hits.into_iter().map(|(credential, form)| Finding {
                            path: relative.clone(),
                            credential,
                            form,
                        }))
                    }
                    Err(err) => findings.push(unreadable(&path, err)),
                }
            }
        }
    }
    findings.sort_by(|left, right| {
        (&left.path, &left.credential).cmp(&(&right.path, &right.credential))
    });
    findings.dedup();
    Ok(findings)
}

fn scan_file(
    path: &Path,
    patterns: &[Pattern],
    index: &HashMap<[u8; 4], Vec<usize>>,
    overlap: usize,
    account_ids: &[String],
) -> std::io::Result<Vec<(String, String)>> {
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
        search(&window, patterns, index, account_ids, &mut hits);
        if window.len() > overlap {
            window.drain(..window.len() - overlap);
        }
    }
    hits.sort();
    hits.dedup();
    Ok(hits)
}

use std::os::unix::fs::OpenOptionsExt;

fn search(
    bytes: &[u8],
    patterns: &[Pattern],
    index: &HashMap<[u8; 4], Vec<usize>>,
    account_ids: &[String],
    hits: &mut Vec<(String, String)>,
) {
    for start in 0..bytes.len().saturating_sub(3) {
        let key: [u8; 4] = [
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ];
        if let Some(candidates) = index.get(&key) {
            for &number in candidates {
                let pattern = &patterns[number];
                let body = &bytes[start + pattern.bytes.len().min(bytes.len() - start)..];
                let token_body = body.len() >= pattern.body
                    && body[..pattern.body]
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
                if bytes[start..].starts_with(&pattern.bytes) && token_body {
                    hits.push((pattern.credential.clone(), pattern.form.clone()));
                }
            }
        }
        if !account_ids.is_empty()
            && key[..3] == *b"eyJ"
            && let Some(account) = jwt_account(&bytes[start..], account_ids)
        {
            hits.push((format!("jwt for account {account}"), "jwt".into()));
        }
    }
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

/// Deletes every file with a finding and records the paths and credential
/// names in `meta/token-scan.json`.
pub fn quarantine(root: &Path, findings: &[Finding], report: &Path) -> Result {
    let mut not_deleted = Vec::new();
    for finding in findings {
        let path = root.join(&finding.path);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file())
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
}
