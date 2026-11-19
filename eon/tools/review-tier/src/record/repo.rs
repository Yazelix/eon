//! What the record tool reads from git and from the tracker's JSONL export.
use super::model::{RULES_FILES, Record, SOURCE_PREFIX, sha256_hex};
use crate::Result;
use crate::classify;
use crate::git::Git;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const TRACKER: &str = ".beads/issues.jsonl";

/// One issue as `.beads/issues.jsonl` holds it; other fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub close_reason: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub text: String,
}

impl Issue {
    pub fn is_open(&self) -> bool {
        !matches!(self.status.as_str(), "closed" | "tombstone")
    }
}

/// The issues with the given IDs, from a JSONL export.
pub fn issues(jsonl: &str, ids: &BTreeSet<String>) -> Result<BTreeMap<String, Issue>> {
    let needles: Vec<(String, &String)> = ids
        .iter()
        .map(|id| (format!("\"id\":\"{id}\""), id))
        .collect();
    let mut found = BTreeMap::new();
    for line in jsonl.lines() {
        if !needles
            .iter()
            .any(|(needle, _)| line.contains(needle.as_str()))
        {
            continue;
        }
        let issue: Issue = serde_json::from_str(line)
            .map_err(|error| format!("{TRACKER}: cannot parse an issue: {error}"))?;
        if ids.contains(&issue.id) {
            found.insert(issue.id.clone(), issue);
        }
    }
    Ok(found)
}

/// The tracker export committed at `revision`; empty when it has none.
pub fn tracker_at(git: &Git, revision: &str) -> Result<String> {
    let path = format!("{revision}:{TRACKER}");
    if git.rev_parse(&path).is_err() {
        return Ok(String::new());
    }
    text(git.run(&["cat-file", "blob", &path])?)
}

pub fn blob_text(git: &Git, revision: &str, path: &str) -> Result<String> {
    text(git.run(&["cat-file", "blob", &format!("{revision}:{path}")])?)
}

fn text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| "git printed non-UTF-8 output".into())
}

/// `commit`, `tree`, `blob` or an error when git does not have the object.
pub fn object_type(git: &Git, id: &str) -> Result<String> {
    let kind = text(git.run(&["cat-file", "-t", id])?)?;
    Ok(kind.trim_end().to_string())
}

/// `src-1:` + SHA-256 of `git ls-tree -r -z --full-tree <tree>` without the
/// tracker export's entry, so a landing commit, whose tracker differs from
/// the proof tree's, has the same digest when its source is the same.
pub fn source_digest(git: &Git, tree: &str) -> Result<String> {
    let listing = git.run(&["ls-tree", "-r", "-z", "--full-tree", tree])?;
    let mut kept = Vec::with_capacity(listing.len());
    for entry in listing
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let path = entry
            .splitn(2, |byte| *byte == b'\t')
            .nth(1)
            .unwrap_or_default();
        if path != TRACKER.as_bytes() {
            kept.extend_from_slice(entry);
            kept.push(0);
        }
    }
    Ok(format!("{SOURCE_PREFIX}{}", sha256_hex(&kept)))
}

/// The rules files' blob IDs at `commit`.
pub fn rules_blobs(git: &Git, commit: &str) -> Result<[String; 4]> {
    let mut blobs: [String; 4] = Default::default();
    for (blob, path) in blobs.iter_mut().zip(RULES_FILES) {
        *blob = git
            .rev_parse(&format!("{commit}:{path}"))
            .map_err(|_| format!("{path} is missing at {commit}"))?;
    }
    Ok(blobs)
}

/// Package names of the workspace members at `commit`.
pub fn workspace_packages(git: &Git, commit: &str) -> Result<BTreeSet<String>> {
    let root: toml::Table = toml::from_str(&blob_text(git, commit, "Cargo.toml")?)?;
    let members = root
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(|members| members.as_array())
        .ok_or("Cargo.toml has no workspace members")?;
    let mut names = BTreeSet::new();
    for member in members {
        let member = member
            .as_str()
            .ok_or("a workspace member is not a string")?;
        let manifest: toml::Table =
            toml::from_str(&blob_text(git, commit, &format!("{member}/Cargo.toml"))?)?;
        let name = manifest
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(|name| name.as_str())
            .ok_or_else(|| format!("{member}/Cargo.toml has no package name"))?;
        names.insert(name.to_string());
    }
    Ok(names)
}

/// Why the proof tree is not exactly the reviewed change applied onto
/// `landing.base`: every path the reviewed change touches must be as the
/// reviewed tree has it, and the landing may change no other path outside
/// `.beads/`. Empty when it is.
pub fn identity_problems(git: &Git, record: &Record) -> Result<Vec<String>> {
    let (landing, candidate) = (&record.landing, &record.candidate);
    let reviewed = changed_paths(git, &candidate.base, &candidate.tree)?;
    let drift = changed_paths(git, &candidate.tree, &landing.proof_tree)?;
    let landed = changed_paths(git, &landing.base, &landing.proof_tree)?;
    let mut problems = Vec::new();
    for path in drift.intersection(&reviewed) {
        problems.push(format!(
            "the proof tree's {path} differs from the reviewed tree's"
        ));
    }
    for path in landed.difference(&reviewed) {
        problems.push(format!(
            "the landing changes {path}, which the reviewed change does not"
        ));
    }
    Ok(problems)
}

/// Paths outside `.beads/` that differ between two tree-ish objects.
fn changed_paths(git: &Git, from: &str, to: &str) -> Result<BTreeSet<String>> {
    let changes = classify::parse_raw(&git.raw_diff(from, to)?)?;
    Ok(changes
        .into_iter()
        .map(|change| change.path)
        .filter(|path| !path.starts_with(".beads/"))
        .collect())
}

pub fn is_ancestor(git: &Git, ancestor: &str, commit: &str) -> Result<bool> {
    Ok(git.merge_base(ancestor, commit).ok().as_deref() == Some(ancestor))
}

/// The commit on `tip`'s first-parent line whose parent is `base`, or why
/// there is none.
pub fn landing_commit(
    git: &Git,
    base: &str,
    tip: &str,
) -> Result<std::result::Result<String, String>> {
    let listing = text(git.run(&[
        "rev-list",
        "--first-parent",
        "--reverse",
        "--parents",
        &format!("{base}..{tip}"),
    ])?)?;
    let Some(first) = listing.lines().next() else {
        return Ok(Err(format!("{tip} has no commit after {base}")));
    };
    let mut ids = first.split(' ');
    let commit = ids.next().unwrap_or_default().to_string();
    if ids.next() != Some(base) {
        return Ok(Err(format!(
            "{base} is not on {tip}'s first-parent line: the first commit after it, {commit}, has another parent"
        )));
    }
    Ok(Ok(commit))
}
