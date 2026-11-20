//! The needles of the fingerprint scan (`agent-bench fingerprints`).
//!
//! A needle is text that would reveal the landed answer if a mounted host path held it. The
//! needles file has one `<kind> <text>` line per needle, sorted and without repeats, and no
//! needle text holds whitespace:
//!
//! - `commit`: the full landed commit id;
//! - `canary`: the task's canary, the canary file's content trimmed;
//! - `identifier`: every token `[A-Za-z_][A-Za-z0-9_]*` on a line the landed diff adds that
//!   is at least [`MIN_IDENTIFIER`] characters long, compound (an underscore, or an ASCII
//!   capital after the first character), and absent from every text blob of the base tree.
//!   Names of crates the landed lockfiles add ([`LOCKFILES`]), in both their `-` and `_`
//!   forms, are left out: the offline Cargo home holds those crates on purpose.
//!
//! Git only reads the source repository.

use crate::{Result, fail, git};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const MIN_IDENTIFIER: usize = 10;
pub const LOCKFILES: &[&str] = &["Cargo.lock", "installer/Cargo.lock"];

/// What `agent-bench fingerprints` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub needles: usize,
    pub identifiers: usize,
    /// Crates the landed lockfiles add, whose names are not needles.
    pub excluded_crates: Vec<String>,
}

/// Writes the needles for `base..landed` in `source` and the canary in `canary` to `out`.
pub fn write(
    source: &Path,
    base: &str,
    landed: &str,
    canary: &Path,
    out: &Path,
) -> Result<Summary> {
    let canary_text = fs::read_to_string(canary)
        .map_err(|error| crate::Error(format!("{}: {error}", canary.display())))?;
    let canary_text = canary_text.trim();
    if canary_text.is_empty() {
        fail!("{}: the canary is empty", canary.display());
    }
    if canary_text.contains(char::is_whitespace) {
        fail!("{}: the canary holds whitespace", canary.display());
    }
    let base = git::commit(source, base)?;
    let landed = git::commit(source, landed)?;

    let diff = git::run(
        source,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=0",
            &base,
            &landed,
        ],
    )?;
    let mut identifiers: BTreeSet<String> = added_lines(&diff)
        .flat_map(tokens)
        .filter(|token| compound(token))
        .map(|token| String::from_utf8_lossy(token).into_owned())
        .collect();
    remove_base_tokens(source, &base, &mut identifiers)?;

    let mut excluded_crates = BTreeSet::new();
    for path in LOCKFILES {
        let before = crate_names(&blob_at(source, &base, path)?);
        for name in crate_names(&blob_at(source, &landed, path)?) {
            if !before.contains(&name) {
                excluded_crates.insert(name);
            }
        }
    }
    for name in &excluded_crates {
        identifiers.remove(name);
        identifiers.remove(&name.replace('-', "_"));
    }

    let mut needles: Vec<String> =
        vec![format!("commit {landed}"), format!("canary {canary_text}")];
    needles.extend(identifiers.iter().map(|name| format!("identifier {name}")));
    needles.sort();
    needles.dedup();
    let text: String = needles.iter().map(|needle| format!("{needle}\n")).collect();
    fs::write(out, text).map_err(|error| crate::Error(format!("{}: {error}", out.display())))?;
    Ok(Summary {
        needles: needles.len(),
        identifiers: identifiers.len(),
        excluded_crates: excluded_crates.into_iter().collect(),
    })
}

/// The content of the lines a diff adds (`+` lines inside hunks, never the `+++` header).
fn added_lines(diff: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut in_hunk = false;
    diff.split(|byte| *byte == b'\n').filter_map(move |line| {
        if line.starts_with(b"diff ") {
            in_hunk = false;
        } else if line.starts_with(b"@@") {
            in_hunk = true;
        } else if in_hunk && let Some(added) = line.strip_prefix(b"+") {
            return Some(added);
        }
        None
    })
}

/// The `[A-Za-z_][A-Za-z0-9_]*` tokens of `bytes`.
fn tokens(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
        .map(|run| {
            let start = run
                .iter()
                .position(|byte| !byte.is_ascii_digit())
                .unwrap_or(run.len());
            &run[start..]
        })
        .filter(|token| !token.is_empty())
}

fn compound(token: &[u8]) -> bool {
    token.len() >= MIN_IDENTIFIER
        && (token.contains(&b'_') || token[1..].iter().any(u8::is_ascii_uppercase))
}

/// Removes from `candidates` every token that occurs in a text blob of `commit`'s tree.
fn remove_base_tokens(
    source: &Path,
    commit: &str,
    candidates: &mut BTreeSet<String>,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let listing = git::run(source, &["ls-tree", "-r", "-z", "--full-tree", commit])?;
    let blobs: BTreeSet<String> = git::entries(&listing)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.oid)
        .collect();
    let input: String = blobs.iter().map(|oid| format!("{oid}\n")).collect();
    let batch = git::run_with_input(source, &["cat-file", "--batch"], input.as_bytes())?;
    let mut rest = batch.as_slice();
    while !rest.is_empty() {
        let Some(end) = rest.iter().position(|byte| *byte == b'\n') else {
            fail!("git cat-file --batch: truncated header");
        };
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        let size: usize = match header.rsplit_once(' ').map(|(_, size)| size.parse()) {
            Some(Ok(size)) => size,
            _ => fail!("git cat-file --batch: unexpected header {header:?}"),
        };
        let Some(content) = rest.get(end + 1..end + 1 + size) else {
            fail!("git cat-file --batch: truncated object {header:?}");
        };
        if !content.contains(&0) {
            for token in tokens(content) {
                if let Ok(token) = std::str::from_utf8(token) {
                    candidates.remove(token);
                }
            }
        }
        rest = rest.get(end + 2 + size..).unwrap_or_default();
    }
    Ok(())
}

/// The blob at `path` in `commit`, or nothing when the path is absent.
fn blob_at(source: &Path, commit: &str, path: &str) -> Result<Vec<u8>> {
    let listing = git::run(source, &["ls-tree", "-z", commit, "--", path])?;
    match git::entries(&listing)?
        .into_iter()
        .find(|entry| entry.path == path)
    {
        Some(entry) if entry.kind == "blob" => git::run(source, &["cat-file", "blob", &entry.oid]),
        Some(_) => fail!("{path} in {commit} is not a file"),
        None => Ok(Vec::new()),
    }
}

/// The `name = "…"` values of a Cargo lockfile.
fn crate_names(lockfile: &[u8]) -> BTreeSet<String> {
    String::from_utf8_lossy(lockfile)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("name = \"")?.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tests::git as fixture_git;

    fn lock(names: &[&str]) -> String {
        names
            .iter()
            .map(|name| format!("[[package]]\nname = \"{name}\"\nversion = \"1.0.0\"\n\n"))
            .collect()
    }

    /// Base: one source file, a binary blob and a lockfile. Landed: new lines with each kind
    /// of token, an added crate in both lockfiles.
    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fixture_git(root, &["init", "-q"]);
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn existing_identifier() {}\n").unwrap();
        fs::write(root.join("blob.bin"), b"\0binary_only_ident\n").unwrap();
        fs::write(root.join("Cargo.lock"), lock(&["serde"])).unwrap();
        fixture_git(root, &["add", "-A"]);
        fixture_git(root, &["commit", "-q", "-m", "base"]);
        fs::write(
            root.join("src/lib.rs"),
            "fn existing_identifier() {}\n\
             fn brand_new_function() -> CamelCaseThing { existing_identifier() }\n\
             // short_one plainlowercaseword Capitalized 123digit_prefixed binary_only_ident\n\
             use fresh_crate_name::Thing;\n\
             use installer_only::Other;\n\
             ++leading_plus_plus\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            lock(&["serde", "fresh-crate-name"]),
        )
        .unwrap();
        fs::create_dir(root.join("installer")).unwrap();
        fs::write(root.join("installer/Cargo.lock"), lock(&["installer_only"])).unwrap();
        fixture_git(root, &["add", "-A"]);
        fixture_git(root, &["commit", "-q", "-m", "landed"]);
        let landed = fixture_git(root, &["rev-parse", "HEAD"]);
        (dir, landed)
    }

    #[test]
    fn needles_from_a_two_commit_repo() {
        let (dir, landed) = repo();
        let canary = dir.path().join("canary");
        fs::write(&canary, "  canary-7f3a9  \n").unwrap();
        let out = dir.path().join("needles");
        let summary = write(dir.path(), "HEAD~1", "HEAD", &canary, &out).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        let expected = format!(
            "canary canary-7f3a9\ncommit {landed}\nidentifier CamelCaseThing\n\
             identifier binary_only_ident\nidentifier brand_new_function\n\
             identifier digit_prefixed\nidentifier leading_plus_plus\n"
        );
        assert_eq!(text, expected);
        assert_eq!(
            summary,
            Summary {
                needles: 7,
                identifiers: 5,
                excluded_crates: vec!["fresh-crate-name".into(), "installer_only".into()],
            }
        );
    }

    #[test]
    fn a_bad_canary_is_refused() {
        let (dir, _) = repo();
        let out = dir.path().join("needles");
        for bad in ["", "  \n", "two words"] {
            let canary = dir.path().join("canary");
            fs::write(&canary, bad).unwrap();
            assert!(
                write(dir.path(), "HEAD~1", "HEAD", &canary, &out).is_err(),
                "{bad:?}"
            );
        }
        assert!(!out.exists());
    }
}
