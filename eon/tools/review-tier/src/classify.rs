//! Pure classification over parsed git output: no processes, no repository.
use crate::Result;
use crate::rules::{self, Tier};
use std::collections::BTreeMap;

/// One `git diff --raw -z --no-renames` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub old_mode: String,
    pub new_mode: String,
    pub old_object: String,
    pub new_object: String,
    pub status: char,
    pub path: String,
}

/// One path's tier and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub tier: Tier,
    pub path: String,
    pub reason: String,
}

/// Parses `:<old mode> <new mode> <old sha> <new sha> <status>\0<path>\0`
/// records. Any record that does not fit, or a status other than A, D, M, T
/// or U, is an error.
pub fn parse_raw(raw: &[u8]) -> Result<Vec<Change>> {
    let mut fields = raw.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            if fields.next().is_some() {
                return Err("malformed raw diff: empty record".into());
            }
            break;
        }
        let header =
            std::str::from_utf8(header).map_err(|_| "malformed raw diff: header is not UTF-8")?;
        let path = fields
            .next()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| format!("malformed raw diff: no path after {header:?}"))?;
        let path = String::from_utf8(path.to_vec())
            .map_err(|_| format!("path is not UTF-8 after {header:?}"))?;
        changes.push(parse_header(header, path)?);
    }
    Ok(changes)
}

fn parse_header(header: &str, path: String) -> Result<Change> {
    let malformed = || format!("malformed raw diff record {header:?} for {path}");
    let parts: Vec<&str> = header
        .strip_prefix(':')
        .ok_or_else(malformed)?
        .split(' ')
        .collect();
    let [old_mode, new_mode, old_object, new_object, status] = parts[..] else {
        return Err(malformed().into());
    };
    let mode = |mode: &str| mode.len() == 6 && mode.bytes().all(|byte| matches!(byte, b'0'..=b'7'));
    let object = |object: &str| {
        matches!(object.len(), 40 | 64) && object.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    if !(mode(old_mode) && mode(new_mode) && object(old_object) && object(new_object)) {
        return Err(malformed().into());
    }
    let status = match status {
        "A" | "D" | "M" | "T" | "U" => status.chars().next().ok_or_else(malformed)?,
        _ => return Err(format!("unknown diff status {status:?} for {path}").into()),
    };
    Ok(Change {
        old_mode: old_mode.into(),
        new_mode: new_mode.into(),
        old_object: old_object.into(),
        new_object: new_object.into(),
        status,
        path,
    })
}

/// Whether the decision for `change` reads its added and removed lines: a
/// `.rs` path that neither a mode rule nor a High path rule decides, and that
/// the Medium list holds.
pub fn needs_lines(change: &Change) -> bool {
    change.path.ends_with(".rs")
        && mode_reason(change).is_none()
        && rules::path_rule(&change.path).is_some_and(|rule| rule.tier == Tier::Medium)
}

fn mode_reason(change: &Change) -> Option<&'static str> {
    rules::MODE_RULES
        .iter()
        .find(|rule| (rule.matches)(change))
        .map(|rule| rule.reason)
}

/// One path's `git diff -U0` hunks: each hunk's lines after its `@@` header,
/// byte for byte with their `+`, `-` or `\` marker.
pub type Hunks = Vec<Vec<Vec<u8>>>;

/// Parses `git diff -U0` output for `paths` into each path's hunks, in the
/// order git printed them. A section for any other path is an error.
pub fn parse_hunks(diff: &[u8], paths: &[&str]) -> Result<BTreeMap<String, Hunks>> {
    let headers: BTreeMap<String, &str> = paths
        .iter()
        .map(|path| (section_header(path), *path))
        .collect();
    let mut sections: BTreeMap<String, Hunks> = BTreeMap::new();
    let mut current: Option<&str> = None;
    let text = diff.strip_suffix(b"\n").unwrap_or(diff);
    if text.is_empty() {
        return Ok(sections);
    }
    for line in text.split(|byte| *byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            let header = String::from_utf8_lossy(line);
            let path = *headers
                .get(header.as_ref())
                .ok_or_else(|| format!("unexpected diff section {header:?}"))?;
            if sections.insert(path.into(), Vec::new()).is_some() {
                return Err(format!("repeated diff section for {path}").into());
            }
            current = Some(path);
        } else {
            let path = current.ok_or_else(|| {
                format!(
                    "diff text before any section: {:?}",
                    String::from_utf8_lossy(line)
                )
            })?;
            let hunks = sections.entry(path.into()).or_default();
            if line.starts_with(b"@@ ") {
                hunks.push(Vec::new());
            } else if let Some(hunk) = hunks.last_mut() {
                hunk.push(line.to_vec());
            }
        }
    }
    Ok(sections)
}

/// Parses `git diff -U0` output for `paths` into each path's added and
/// removed lines. A section for any other path is an error.
pub fn parse_line_diff(diff: &[u8], paths: &[&str]) -> Result<BTreeMap<String, Vec<String>>> {
    Ok(parse_hunks(diff, paths)?
        .into_iter()
        .map(|(path, hunks)| {
            let lines = hunks
                .iter()
                .flatten()
                .filter(|line| matches!(line.first(), Some(b'+' | b'-')))
                .map(|line| String::from_utf8_lossy(&line[1..]).into_owned())
                .collect();
            (path, lines)
        })
        .collect())
}

/// The `diff --git` header git prints for `path` with `a/` and `b/` prefixes,
/// C-quoted the way git quotes it when `core.quotepath` is off.
fn section_header(path: &str) -> String {
    let needs_quote = path
        .chars()
        .any(|ch| ch == '"' || ch == '\\' || ch.is_ascii_control());
    if !needs_quote {
        return format!("diff --git a/{path} b/{path}");
    }
    let mut quoted = String::new();
    for ch in path.chars() {
        match ch {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{7}' => quoted.push_str("\\a"),
            '\u{8}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{b}' => quoted.push_str("\\v"),
            '\u{c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            ch if ch.is_ascii_control() => quoted.push_str(&format!("\\{:03o}", ch as u32)),
            ch => quoted.push(ch),
        }
    }
    format!("diff --git \"a/{quoted}\" \"b/{quoted}\"")
}

/// Decides every change. `lines` holds the added and removed lines of each
/// path for which [`needs_lines`] holds; `read_by_code` answers whether a
/// `.rs` or `.nix` file in the base or candidate tree names a Markdown path.
pub fn classify(
    changes: &[Change],
    lines: &BTreeMap<String, Vec<String>>,
    mut read_by_code: impl FnMut(&str) -> Result<bool>,
) -> Result<Vec<Verdict>> {
    if changes.is_empty() {
        return Err("empty diff: nothing to review".into());
    }
    let mut verdicts = Vec::new();
    for change in changes {
        let (tier, reason) = decide(change, lines, &mut read_by_code)?;
        verdicts.push(Verdict {
            tier,
            path: change.path.clone(),
            reason,
        });
    }
    verdicts.sort_by(|left, right| right.tier.cmp(&left.tier).then(left.path.cmp(&right.path)));
    Ok(verdicts)
}

fn decide(
    change: &Change,
    lines: &BTreeMap<String, Vec<String>>,
    read_by_code: &mut impl FnMut(&str) -> Result<bool>,
) -> Result<(Tier, String)> {
    if let Some(reason) = mode_reason(change) {
        return Ok((Tier::High, reason.into()));
    }
    let Some(rule) = rules::path_rule(&change.path) else {
        return Ok((Tier::High, rules::UNLISTED.into()));
    };
    match rule.tier {
        Tier::High => Ok((Tier::High, rule.reason.into())),
        Tier::Medium if needs_lines(change) => {
            let changed = match lines.get(&change.path) {
                Some(changed) => changed.as_slice(),
                None if change.old_object == change.new_object => &[],
                None => return Err(format!("no line diff for {}", change.path).into()),
            };
            let token = rules::RISKY_TOKENS
                .iter()
                .find(|token| changed.iter().any(|line| line.contains(**token)));
            Ok(match token {
                Some(token) => (Tier::High, format!("risky token: {token}")),
                None => (Tier::Medium, rule.reason.into()),
            })
        }
        Tier::Medium => Ok((Tier::Medium, rule.reason.into())),
        Tier::Low => {
            let plain = |mode: &str| mode == rules::REGULAR || mode == rules::ABSENT;
            if !(plain(&change.old_mode) && plain(&change.new_mode)) {
                Ok((Tier::High, rules::LOW_MODE.into()))
            } else if change.path.ends_with(".md") && read_by_code(&change.path)? {
                Ok((Tier::Medium, rules::READ_BY_CODE.into()))
            } else {
                Ok((Tier::Low, rule.reason.into()))
            }
        }
    }
}

/// The slice tier: the highest path tier.
pub fn slice_tier(verdicts: &[Verdict]) -> Tier {
    verdicts
        .iter()
        .map(|verdict| verdict.tier)
        .max()
        .unwrap_or(Tier::High)
}

/// One aligned `<tier>  <path>  <reason>` line per verdict.
pub fn render(verdicts: &[Verdict]) -> String {
    let tier_width = verdicts
        .iter()
        .map(|v| v.tier.name().len())
        .max()
        .unwrap_or(0);
    let path_width = verdicts
        .iter()
        .map(|v| v.path.chars().count())
        .max()
        .unwrap_or(0);
    verdicts
        .iter()
        .map(|verdict| {
            format!(
                "{:<tier_width$}  {:<path_width$}  {}\n",
                verdict.tier.name(),
                verdict.path,
                verdict.reason
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: &str = "0000000000000000000000000000000000000000";
    const ONE: &str = "1111111111111111111111111111111111111111";
    const TWO: &str = "2222222222222222222222222222222222222222";

    fn change(old_mode: &str, new_mode: &str, status: char, path: &str) -> Change {
        let object = |mode: &str, sha: &'static str| if mode == "000000" { ZERO } else { sha };
        Change {
            old_mode: old_mode.into(),
            new_mode: new_mode.into(),
            old_object: object(old_mode, ONE).into(),
            new_object: object(new_mode, TWO).into(),
            status,
            path: path.into(),
        }
    }

    fn modified(path: &str) -> Change {
        change("100644", "100644", 'M', path)
    }

    fn one(change: Change, lines: &[&str], referenced: bool) -> (Tier, String) {
        let mut map = BTreeMap::new();
        if needs_lines(&change) {
            map.insert(
                change.path.clone(),
                lines.iter().map(|line| String::from(*line)).collect(),
            );
        }
        let verdicts = classify(&[change], &map, |_| Ok(referenced)).unwrap();
        (verdicts[0].tier, verdicts[0].reason.clone())
    }

    fn tier(path: &str) -> (Tier, String) {
        one(modified(path), &[], false)
    }

    fn high(reason: &str) -> (Tier, String) {
        (Tier::High, reason.into())
    }

    #[test]
    fn high_pattern_families() {
        assert_eq!(tier("orbit/CONTRACTS.md"), high("contract"));
        assert_eq!(tier("CONTRACTS.md"), high("contract"));
        assert_eq!(
            tier("orbit/crates/protocol/src/lib.rs"),
            high("wire and protocol")
        );
        assert_eq!(tier("venus/src/transport.rs"), high("wire and protocol"));
        assert_eq!(tier("orbit/PROOF.bend"), high("Bend policy and proofs"));
        assert_eq!(tier("orbit/proof/a.ts"), high("Bend policy and proofs"));
        assert_eq!(tier("orbit/src/pty.rs"), high("lifecycle"));
        assert_eq!(
            tier("runtime/crates/eon-runtime/src/generation/swap.rs"),
            high("lifecycle")
        );
        assert_eq!(tier("eon/crates/eon/src/update.rs"), high("lifecycle"));
        assert_eq!(tier("installer/install.sh"), high("trust boundary"));
        assert_eq!(
            tier("eon/crates/eon-agent/src/launch.rs"),
            high("trust boundary")
        );
        assert_eq!(
            tier("eon/crates/eon/Cargo.toml"),
            high("dependencies and composition")
        );
        assert_eq!(tier("Cargo.lock"), high("dependencies and composition"));
        assert_eq!(tier("nix/a.nix"), high("dependencies and composition"));
        assert_eq!(
            tier(".github/workflows/a.yml"),
            high("dependencies and composition")
        );
        assert_eq!(
            tier("venus/.gitattributes"),
            high("dependencies and composition")
        );
        assert_eq!(
            tier("eon/tools/review-tier/src/rules.rs"),
            high("gate or validator tool")
        );
        assert_eq!(
            tier("orbit/tools/governance/README.md"),
            high("gate or validator tool")
        );
        assert_eq!(tier("venus/AGENTS.md"), high("agent policy"));
        assert_eq!(tier(".agent-protocols.json"), high("agent policy"));
        assert_eq!(tier("docs/AGENT-RULES.md"), high("agent policy"));
        assert_eq!(tier("REVIEW.md"), high("agent policy"));
    }

    #[test]
    fn medium_list_and_default_high() {
        let medium = |reason: &str| (Tier::Medium, String::from(reason));
        assert_eq!(tier("venus/src/model.rs"), medium("ordinary code"));
        assert_eq!(tier("orbit/src/lib.rs"), medium("ordinary code"));
        assert_eq!(
            tier("runtime/crates/eon-runtime/src/paths.rs"),
            medium("ordinary code")
        );
        assert_eq!(tier("eon/crates/eon/src/main.rs"), medium("ordinary code"));
        assert_eq!(tier("eon/skills/eon/SKILL.md"), medium("ordinary code"));
        assert_eq!(tier("demo/script.txt"), medium("ordinary code"));
        assert_eq!(
            tier("eon/crates/eon/config.example.toml"),
            medium("ordinary code")
        );
        assert_eq!(tier("venus/tests/render.rs"), medium("test"));
        assert_eq!(tier("orbit/testdata/a.bin"), medium("test"));
        assert_eq!(tier("venus/assets/icon.png"), medium("asset"));
        assert_eq!(tier("assets/logo.svg"), medium("asset"));
        assert_eq!(tier("README"), high("not on the Medium or Low lists"));
        assert_eq!(
            tier("eon/crates/eon/build.rs"),
            high("not on the Medium or Low lists")
        );
    }

    #[test]
    fn low_list_and_referenced_markdown() {
        for beads in [
            ".beads/issues.jsonl",
            ".beads/config.yaml",
            ".beads/metadata.json",
        ] {
            assert_eq!(tier(beads), (Tier::Low, "planning".into()), "{beads}");
        }
        for other in [".beads/payload.rs", ".beads/.gitignore", ".beads/a.jsonl"] {
            assert_eq!(
                tier(other),
                high("not on the Medium or Low lists"),
                "{other}"
            );
        }
        assert_eq!(tier("docs/REMOTE.md"), (Tier::Low, "docs".into()));
        assert_eq!(tier("README.md"), (Tier::Low, "docs".into()));
        assert_eq!(
            one(modified("docs/CONFIGURATION.md"), &[], true),
            (Tier::Medium, "Markdown read by code".into())
        );
        assert_eq!(
            one(change("000000", "100644", 'A', "docs/NEW.md"), &[], false),
            (Tier::Low, "docs".into())
        );
        assert_eq!(
            one(change("100644", "000000", 'D', "docs/OLD.md"), &[], false),
            (Tier::Low, "docs".into())
        );
        assert_eq!(
            one(change("000000", "100755", 'A', "docs/run.md"), &[], false),
            high("mode")
        );
        let mut asked = Vec::new();
        let verdicts = classify(
            &[modified("docs/A.md"), modified(".beads/issues.jsonl")],
            &BTreeMap::new(),
            |path| {
                asked.push(String::from(path));
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!(asked, ["docs/A.md"]);
        assert_eq!(verdicts.len(), 2);
        let failed = classify(&[modified("docs/A.md")], &BTreeMap::new(), |_| {
            Err("grep failed".into())
        });
        assert!(failed.is_err());
    }

    #[test]
    fn mode_and_type_rules() {
        assert_eq!(
            one(
                change("100644", "100755", 'M', "venus/src/a.rs"),
                &[],
                false
            ),
            high("mode change")
        );
        assert_eq!(
            one(change("000000", "120000", 'A', "docs/link.md"), &[], false),
            high("symlink")
        );
        assert_eq!(
            one(change("160000", "160000", 'M', "components/x"), &[], false),
            high("submodule")
        );
        assert_eq!(
            one(
                change("100644", "120000", 'T', "venus/src/a.rs"),
                &[],
                false
            ),
            high("mode change")
        );
        assert_eq!(
            one(
                change("100644", "100644", 'T', "venus/src/a.rs"),
                &[],
                false
            ),
            high("type change")
        );
        assert_eq!(
            one(
                change("100644", "100644", 'U', "venus/src/a.rs"),
                &[],
                false
            ),
            high("unmerged")
        );
    }

    #[test]
    fn risky_tokens_on_medium_rust() {
        let path = "venus/src/window.rs";
        assert_eq!(
            one(
                modified(path),
                &["    let x = 1;", "    println!(\"kill\");"],
                false
            ),
            (Tier::Medium, "ordinary code".into())
        );
        assert_eq!(
            one(modified(path), &["    unsafe { libc::close(fd) };"], false),
            high("risky token: unsafe")
        );
        assert_eq!(
            one(
                modified(path),
                &["nix::sys::signal::kill(pid, SIGTERM)"],
                false
            ),
            high("risky token: kill(")
        );
        assert_eq!(
            one(
                modified("venus/tests/a.rs"),
                &["let l = UnixListener::bind(p);"],
                false
            ),
            high("risky token: UnixListener::bind")
        );
        assert!(!needs_lines(&modified("venus/src/transport.rs")));
        assert!(!needs_lines(&modified("venus/assets/a.txt")));
        assert!(!needs_lines(&modified("eon/crates/eon/build.rs")));
        let missing = classify(&[modified(path)], &BTreeMap::new(), |_| Ok(false));
        assert!(missing.is_err());
    }

    /// Literal on purpose: removing a token from the rule table turns this red.
    #[test]
    fn every_risky_token_makes_a_medium_rust_line_high() {
        for token in [
            "unsafe",
            "pre_exec",
            "kill(",
            "killpg",
            "SIGKILL",
            "SIGTERM",
            "set_permissions",
            "from_mode",
            "UnixListener::bind",
            "UnixDatagram::bind",
            "TcpListener::bind",
            "UdpSocket::bind",
        ] {
            assert_eq!(
                one(
                    modified("venus/src/window.rs"),
                    &[&format!("    let x = {token};")],
                    false
                ),
                high(&format!("risky token: {token}")),
                "{token}"
            );
        }
    }

    #[test]
    fn slice_tier_is_the_maximum_and_output_is_sorted() {
        let verdicts = classify(
            &[
                modified("venus/src/b.rs"),
                modified("docs/A.md"),
                modified("flake.nix"),
                modified("venus/src/a.rs"),
            ],
            &BTreeMap::from([
                ("venus/src/a.rs".into(), vec![]),
                ("venus/src/b.rs".into(), vec![]),
            ]),
            |_| Ok(false),
        )
        .unwrap();
        assert_eq!(slice_tier(&verdicts), Tier::High);
        assert_eq!(
            render(&verdicts),
            "high    flake.nix       dependencies and composition\n\
             medium  venus/src/a.rs  ordinary code\n\
             medium  venus/src/b.rs  ordinary code\n\
             low     docs/A.md       docs\n"
        );
        assert_eq!(slice_tier(&verdicts[1..]), Tier::Medium);
        assert_eq!(slice_tier(&verdicts[3..]), Tier::Low);
        assert!(classify(&[], &BTreeMap::new(), |_| Ok(false)).is_err());
    }

    #[test]
    fn raw_records_parse_and_fail_closed() {
        let raw = format!(
            ":100644 100644 {ONE} {TWO} M\0venus/src/a b.rs\0:000000 100644 {ZERO} {TWO} A\0docs/é.md\0"
        );
        let changes = parse_raw(raw.as_bytes()).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "venus/src/a b.rs");
        assert_eq!(changes[1].status, 'A');
        assert_eq!(changes[1].old_mode, "000000");
        assert!(parse_raw(b"").unwrap().is_empty());
        for bad in [
            format!(":100644 100644 {ONE} {TWO} R100\0a\0b\0"),
            format!(":100644 100644 {ONE} {TWO} C\0a\0"),
            format!(":100644 100644 {ONE} {TWO} X\0a\0"),
            format!(":100644 100644 {ONE} {TWO}\0a\0"),
            format!("100644 100644 {ONE} {TWO} M\0a\0"),
            format!(":100644 100644 {ONE} {TWO} M\0"),
            format!(":100644 100644 {ONE} {TWO} M\0\0"),
            format!(":10064 100644 {ONE} {TWO} M\0a\0"),
            format!(":100648 100644 {ONE} {TWO} M\0a\0"),
            format!(":100644 100644 1111 {TWO} M\0a\0"),
            format!(":100644 100644 {ONE} {TWO} M\0a\0\0x"),
        ] {
            assert!(parse_raw(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert!(parse_raw(b":100644 100644 x y M\0\xff\0").is_err());
    }

    #[test]
    fn line_diff_sections() {
        let diff = b"diff --git a/venus/src/a.rs b/venus/src/a.rs\n\
index 1111..2222 100644\n\
--- a/venus/src/a.rs\n\
+++ b/venus/src/a.rs\n\
@@ -1 +1 @@\n\
-old line\n\
+--- new line\n\
diff --git a/venus/src/empty.rs b/venus/src/empty.rs\n\
new file mode 100644\n\
index 0000..e69d\n\
diff --git \"a/venus/src/q\\\"\\tx.rs\" \"b/venus/src/q\\\"\\tx.rs\"\n\
--- \"a/venus/src/q\\\"\\tx.rs\"\n\
+++ \"b/venus/src/q\\\"\\tx.rs\"\n\
@@ -0,0 +1 @@\n\
+unsafe\n\
\\ No newline at end of file\n";
        let paths = [
            "venus/src/a.rs",
            "venus/src/empty.rs",
            "venus/src/q\"\tx.rs",
        ];
        let lines = parse_line_diff(diff, &paths).unwrap();
        assert_eq!(lines["venus/src/a.rs"], ["old line", "--- new line"]);
        assert!(lines["venus/src/empty.rs"].is_empty());
        assert_eq!(lines["venus/src/q\"\tx.rs"], ["unsafe"]);
        assert!(parse_line_diff(diff, &paths[..2]).is_err());
        assert!(parse_line_diff(b"@@ -1 +1 @@\n+x\n", &paths).is_err());
        assert!(parse_line_diff(b"", &paths).unwrap().is_empty());
    }
}
