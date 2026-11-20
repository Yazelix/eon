//! Synthetic workspaces and the launched-workspace check.
//!
//! [`create`] takes a base commit's tree without `.beads/` (and any other named top-level
//! entries) and makes it the only commit of a fresh repository: fixed author, committer,
//! date and message, one branch, no remote, no reflog, no hooks, and every working-tree mtime
//! set to the same instant. The source repository is only read: its objects are copied with
//! `pack-objects`, never cloned or fetched. The same tree always gives the same commit.
//!
//! [`check`] verifies a workspace before an agent starts in it: exactly the expected objects
//! (loose and packed, reachable or not), only HEAD and one branch, nothing else under `.git`
//! (no remotes, reflogs, `FETCH_HEAD`, `ORIG_HEAD`, stash, alternates or hooks), a clean
//! working tree, and normalized mtimes. Git commands that read the workspace's own config or
//! index run only after the filesystem checks pass, so a planted `core.fsmonitor` or hook
//! never runs.

use crate::git;
use crate::{Result, fail, sha256_hex};
use rustix::fs::{AtFlags, CWD, Timespec, Timestamps, utimensat};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// 2020-01-01T00:00:00Z: the commit date and every mtime. Constant across tasks, so it says
/// nothing about when the task's work happened.
pub const EPOCH: i64 = 1_577_836_800;
pub const BRANCH: &str = "main";
pub const AUTHOR_NAME: &str = "agent-bench";
pub const AUTHOR_EMAIL: &str = "agent-bench@invalid";
pub const MESSAGE: &str = "Synthetic workspace";
/// Always dropped: the tracker holds the work's later history.
pub const ALWAYS_DROPPED: &str = ".beads";

const GIT_ENTRIES: &[&str] = &["HEAD", "config", "index", "objects", "refs", "packed-refs"];
const CONFIG_KEYS: &[&str] = &[
    "core.repositoryformatversion",
    "core.filemode",
    "core.bare",
    "core.logallrefupdates",
    "extensions.objectformat",
];
/// Written by `git init` on macOS; booleans only.
const BOOLEAN_CONFIG_KEYS: &[&str] = &["core.ignorecase", "core.precomposeunicode"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub commit: String,
    pub branch: String,
    /// The task record's `snapshot_object_set_sha256`.
    pub object_set_sha256: String,
}

/// Makes a synthetic workspace at `out` (which must not exist) from `base` in `source`.
pub fn create(source: &Path, base: &str, out: &Path, drops: &[String]) -> Result<Snapshot> {
    let mut dropped = BTreeSet::from([ALWAYS_DROPPED.to_owned()]);
    for name in drops {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            fail!("--drop takes one top-level name, not {name:?}");
        }
        dropped.insert(name.clone());
    }
    if fs::symlink_metadata(out).is_ok() {
        fail!("{} already exists", out.display());
    }
    let commit = git::commit(source, base)?;
    let format = git::line(source, &["rev-parse", "--show-object-format"])?;

    let mut init = git::command(None);
    init.args([
        "init",
        "-q",
        "--template=",
        &format!("--initial-branch={BRANCH}"),
        &format!("--object-format={format}"),
    ])
    .arg(out);
    git::output(init, None)?;
    // `out` did not exist, so everything in it is ours to remove if a later step fails.
    fill(source, &commit, out, &dropped).inspect_err(|_| {
        let _ = fs::remove_dir_all(out);
    })
}

/// Copies the kept objects into the fresh repository at `out` and makes its one commit.
fn fill(source: &Path, commit: &str, out: &Path, dropped: &BTreeSet<String>) -> Result<Snapshot> {
    // Every tree and blob under the kept paths, then exactly those objects.
    let listing = git::run(
        source,
        &["ls-tree", "-r", "-t", "-z", "--full-tree", commit],
    )?;
    let mut objects = BTreeSet::new();
    for entry in git::entries(&listing)? {
        let top = entry.path.split('/').next().unwrap_or_default();
        if dropped.contains(top) {
            continue;
        }
        if entry.kind == "commit" {
            fail!(
                "{} is a submodule; synthetic workspaces do not support them",
                entry.path
            );
        }
        objects.insert(entry.oid);
    }
    let list: String = objects.iter().map(|oid| format!("{oid}\n")).collect();
    let pack = git::run_with_input(source, &["pack-objects", "--stdout", "-q"], list.as_bytes())?;
    git::run_with_input(out, &["unpack-objects", "-q"], &pack)?;

    // The root tree without the dropped entries, and one parentless commit.
    let root = git::run(source, &["ls-tree", "-z", "--full-tree", commit])?;
    let mut kept = Vec::new();
    for entry in git::entries(&root)? {
        if !dropped.contains(&entry.path) {
            kept.extend_from_slice(
                format!(
                    "{} {} {}\t{}\0",
                    entry.mode, entry.kind, entry.oid, entry.path
                )
                .as_bytes(),
            );
        }
    }
    let tree = String::from_utf8_lossy(&git::run_with_input(out, &["mktree", "-z"], &kept)?)
        .trim()
        .to_owned();
    let date = format!("@{EPOCH} +0000");
    let mut commit_tree = git::command(Some(out));
    commit_tree
        .args([
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &tree,
            "-m",
            MESSAGE,
        ])
        .env("GIT_AUTHOR_NAME", AUTHOR_NAME)
        .env("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_NAME", AUTHOR_NAME)
        .env("GIT_COMMITTER_EMAIL", AUTHOR_EMAIL)
        .env("GIT_COMMITTER_DATE", &date);
    let synthetic = String::from_utf8_lossy(&git::output(commit_tree, None)?)
        .trim()
        .to_owned();
    git::run(
        out,
        &[
            "-c",
            "core.logAllRefUpdates=false",
            "update-ref",
            &format!("refs/heads/{BRANCH}"),
            &synthetic,
        ],
    )?;
    git::run(out, &["read-tree", "--reset", "-u", "HEAD"])?;
    normalize_mtimes(out)?;
    git::run(out, &["update-index", "-q", "--refresh"])?;

    Ok(Snapshot {
        commit: synthetic,
        branch: BRANCH.to_owned(),
        object_set_sha256: object_set_hash(&all_objects(out)?),
    })
}

/// The outcome of [`check`]: one line per violation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub violations: Vec<String>,
    /// The git steps did not run because the filesystem steps found violations.
    pub git_skipped: bool,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks a workspace before launch against its expected object-set hash.
pub fn check(dir: &Path, object_set_sha256: &str) -> Result<Report> {
    let mut report = Report::default();
    let violations = &mut report.violations;
    let dot_git = dir.join(".git");

    // 1. `.git` is a real directory with no symlink anywhere inside it.
    match fs::symlink_metadata(&dot_git) {
        Ok(meta) if meta.is_dir() => {
            for link in symlinks(&dot_git)? {
                violations.push(format!("symlink inside .git: {link}"));
            }
            // 2. Only the entries a fresh synthetic workspace has.
            for name in names(&dot_git)? {
                if !GIT_ENTRIES.contains(&name.as_str()) {
                    violations.push(format!(".git entry not allowed: {name}"));
                }
            }
            if !fs::symlink_metadata(dot_git.join("HEAD")).is_ok_and(|meta| meta.is_file()) {
                violations.push(".git/HEAD is missing or not a file".into());
            }
            // Git skips a file under refs/ or objects/ it cannot read as a ref or object, so
            // those trees are checked on the filesystem too.
            layout_violations(&dot_git, violations)?;
            // 3. No alternates.
            let info = dot_git.join("objects/info");
            if fs::symlink_metadata(&info).is_ok() {
                for name in names(&info)? {
                    if name != "packs" {
                        violations.push(format!(".git/objects/info entry not allowed: {name}"));
                    }
                }
            }
            // 4. Configuration keys, read from that one file.
            check_config(&dot_git.join("config"), violations)?;
        }
        Ok(meta) if meta.file_type().is_symlink() => violations.push(".git is a symlink".into()),
        Ok(_) => violations.push(".git is not a directory (a gitdir file?)".into()),
        Err(_) => violations.push(".git is missing".into()),
    }

    if violations.is_empty() {
        // 5. Only HEAD and one branch.
        let refs = git_line(dir, &["for-each-ref", "--format=%(refname)"])?;
        let refs: Vec<&str> = refs.lines().collect();
        let branches: Vec<&&str> = refs
            .iter()
            .filter(|r| r.starts_with("refs/heads/"))
            .collect();
        for other in refs.iter().filter(|r| !r.starts_with("refs/heads/")) {
            violations.push(format!("ref not allowed: {other}"));
        }
        if branches.len() != 1 {
            violations.push(format!("expected one branch, found {}", branches.len()));
        }
        match git::run(dir, &guarded(&["symbolic-ref", "-q", "HEAD"])) {
            Ok(head) => {
                let head = String::from_utf8_lossy(&head).trim().to_owned();
                if branches
                    .first()
                    .is_none_or(|branch| **branch != head.as_str())
                {
                    violations.push(format!("HEAD points at {head}, not the one branch"));
                }
            }
            Err(_) => violations.push("HEAD is detached".into()),
        }
        // 6. Exactly the expected objects, all reachable.
        let all = all_objects(dir)?;
        if object_set_hash(&all) != object_set_sha256 {
            violations.push(format!(
                "object set differs: {} objects hash to {}, expected {object_set_sha256}",
                all.len(),
                object_set_hash(&all)
            ));
        }
        let reachable: BTreeSet<String> =
            git_line(dir, &["rev-list", "--objects", "--no-object-names", "HEAD"])
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect();
        let unreachable = all.iter().filter(|oid| !reachable.contains(*oid)).count();
        if unreachable > 0 {
            violations.push(format!("{unreachable} unreachable objects"));
        }
        // 7. The working tree is HEAD, with nothing untracked or ignored.
        let status = git::run(
            dir,
            &guarded(&[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored",
                "--ignore-submodules=none",
            ]),
        )?;
        for entry in status.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            violations.push(format!(
                "working tree differs: {}",
                String::from_utf8_lossy(entry)
            ));
        }
    } else {
        report.git_skipped = true;
    }

    // 8. Normalized mtimes (filesystem only).
    for path in mtime_violations(dir)? {
        report
            .violations
            .push(format!("mtime not normalized: {path}"));
    }
    Ok(report)
}

/// Pins git to the directory's own `.git` (never a repository found further up) and adds
/// `-c` settings that keep git from running workspace-configured programs.
fn guarded<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut out = vec![
        "--git-dir=.git",
        "--work-tree=.",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    out.extend_from_slice(args);
    out
}

fn git_line(dir: &Path, args: &[&str]) -> Result<String> {
    git::line(dir, &guarded(args))
}

fn check_config(path: &Path, violations: &mut Vec<String>) -> Result<()> {
    let mut command = git::command(None);
    command
        .args(["config", "--no-includes", "--list", "-z", "--file"])
        .arg(path);
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    let Ok(listing) = git::output(command, None) else {
        violations.push("config unreadable".into());
        return Ok(());
    };
    for item in listing.split(|b| *b == 0).filter(|i| !i.is_empty()) {
        let item = String::from_utf8_lossy(item);
        let (key, value) = item.split_once('\n').unwrap_or((&item, ""));
        if CONFIG_KEYS.contains(&key) {
            continue;
        }
        if BOOLEAN_CONFIG_KEYS.contains(&key) && matches!(value, "true" | "false") {
            continue;
        }
        violations.push(format!("config key not allowed: {key}"));
    }
    Ok(())
}

/// Every object in the repository at `dir`: loose and packed, reachable or not, sorted.
fn all_objects(dir: &Path) -> Result<Vec<String>> {
    let out = git_line(
        dir,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
    )?;
    let mut objects: Vec<String> = out.lines().map(str::to_owned).collect();
    objects.sort();
    objects.dedup();
    Ok(objects)
}

/// SHA-256 over the sorted object ids, one per line.
pub fn object_set_hash(objects: &[String]) -> String {
    let list: String = objects.iter().map(|oid| format!("{oid}\n")).collect();
    sha256_hex(list.as_bytes())
}

/// Reports anything under `.git/refs` and `.git/objects` that a synthetic workspace does not
/// have: only the one branch, `refs/tags`, loose objects, packs and `objects/info`.
fn layout_violations(dot_git: &Path, violations: &mut Vec<String>) -> Result<()> {
    let branch = format!("refs/heads/{BRANCH}");
    for top in ["refs", "objects"] {
        let root = dot_git.join(top);
        if fs::symlink_metadata(&root).is_err() {
            continue;
        }
        walk(&root, false, &mut |path, meta| {
            let relative = path
                .strip_prefix(dot_git)
                .expect("walk stays under .git")
                .to_string_lossy()
                .into_owned();
            let allowed = if meta.is_dir() {
                layout_dir(&relative)
            } else {
                meta.is_file() && (relative == branch || layout_file(&relative))
            };
            if !allowed {
                violations.push(format!("not allowed in .git: {relative}"));
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn layout_dir(relative: &str) -> bool {
    match relative.split('/').collect::<Vec<_>>()[..] {
        ["refs"] | ["refs", "heads" | "tags"] => true,
        ["objects"] | ["objects", "info" | "pack"] => true,
        ["objects", fan] => is_hex(fan, &[2]),
        _ => false,
    }
}

fn layout_file(relative: &str) -> bool {
    match relative.split('/').collect::<Vec<_>>()[..] {
        ["objects", "info", "packs"] => true,
        ["objects", "pack", name] => name
            .strip_prefix("pack-")
            .and_then(|name| name.rsplit_once('.'))
            .is_some_and(|(id, extension)| {
                is_hex(id, &[40, 64]) && ["pack", "idx", "rev"].contains(&extension)
            }),
        ["objects", fan, rest] => is_hex(fan, &[2]) && is_hex(rest, &[38, 62]),
        _ => false,
    }
}

fn is_hex(text: &str, lengths: &[usize]) -> bool {
    lengths.contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn names(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        out.push(entry?.file_name().to_string_lossy().into_owned());
    }
    out.sort();
    Ok(out)
}

fn symlinks(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(dir, false, &mut |path, meta| {
        if meta.file_type().is_symlink() {
            out.push(path.display().to_string());
        }
        Ok(())
    })?;
    Ok(out)
}

/// Calls `visit` for every path under `dir` (children before their directory, siblings in
/// name order), then `dir` itself, never following symlinks. A top-level `.git` is skipped
/// unless `include_git` is set or `dir` is itself a `.git`.
pub(crate) fn walk(
    dir: &Path,
    include_git: bool,
    visit: &mut dyn FnMut(&Path, &fs::Metadata) -> Result<()>,
) -> Result<()> {
    fn inner(
        path: &Path,
        skip_git: bool,
        visit: &mut dyn FnMut(&Path, &fs::Metadata) -> Result<()>,
    ) -> Result<()> {
        let context = |error: std::io::Error| crate::Error(format!("{}: {error}", path.display()));
        let meta = fs::symlink_metadata(path).map_err(context)?;
        if meta.is_dir() {
            let mut children: Vec<_> = fs::read_dir(path)
                .and_then(|entries| entries.collect::<std::io::Result<_>>())
                .map_err(context)?;
            children.sort_by_key(|entry| entry.file_name());
            for child in children {
                if skip_git && child.file_name() == ".git" {
                    continue;
                }
                inner(&child.path(), false, visit)?;
            }
        }
        visit(path, &meta)
    }
    let skip_git = !include_git && dir.file_name().is_none_or(|name| name != ".git");
    inner(dir, skip_git, visit)
}

fn normalize_mtimes(dir: &Path) -> Result<()> {
    let at = Timespec {
        tv_sec: EPOCH,
        tv_nsec: 0,
    };
    let times = Timestamps {
        last_access: at,
        last_modification: at,
    };
    walk(dir, false, &mut |path, _| {
        utimensat(CWD, path, &times, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| crate::Error(format!("{}: {error}", path.display())))
    })
}

fn mtime_violations(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(dir, false, &mut |path, meta| {
        if meta.mtime() != EPOCH || meta.mtime_nsec() != 0 {
            let shown = path.strip_prefix(dir).unwrap_or(path);
            out.push(if shown.as_os_str().is_empty() {
                ".".into()
            } else {
                shown.display().to_string()
            });
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Runs git for a test fixture with fixed identity and dates.
    pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
        let mut command = git::command(Some(dir));
        command
            .args(args)
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
            .env("GIT_AUTHOR_DATE", "@1700000000 +0000")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
            .env("GIT_COMMITTER_DATE", "@1700000000 +0000");
        String::from_utf8(git::output(command, None).unwrap())
            .unwrap()
            .trim()
            .to_owned()
    }

    /// A source repository with two commits, a symlink, an executable, an ignore rule and a
    /// tracker directory.
    pub(crate) fn source() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "--initial-branch=edge"]);
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::create_dir_all(root.join(".beads")).unwrap();
        fs::write(root.join("README.md"), "fixture\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/deep/mod.rs"), "// deep\n").unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(
            root.join(".beads/issues.jsonl"),
            "{\"id\":\"secret-later-history\"}\n",
        )
        .unwrap();
        fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
        let mut perms = fs::metadata(root.join("run.sh")).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(root.join("run.sh"), perms).unwrap();
        symlink("README.md", root.join("link")).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "first"]);
        fs::write(root.join("src/lib.rs"), "pub fn b() {}\n").unwrap();
        git(root, &["commit", "-q", "-am", "second"]);
        dir
    }

    fn synthetic(source: &Path) -> (tempfile::TempDir, std::path::PathBuf, Snapshot) {
        let parent = tempfile::tempdir().unwrap();
        let out = parent.path().join("ws");
        let snapshot = create(source, "HEAD", &out, &[]).unwrap();
        (parent, out, snapshot)
    }

    fn assert_flags(report: &Report, needle: &str) {
        assert!(
            report.violations.iter().any(|v| v.contains(needle)),
            "expected a violation containing {needle:?}, got {:?}",
            report.violations
        );
    }

    #[test]
    fn synthetic_workspace_passes() {
        let source = source();
        let (_parent, out, snapshot) = synthetic(source.path());
        let report = check(&out, &snapshot.object_set_sha256).unwrap();
        assert!(report.passed(), "{:?}", report.violations);
        assert!(!report.git_skipped);
        // One parentless commit with the fixed identity, on the one branch.
        assert_eq!(git(&out, &["rev-list", "--count", "HEAD"]), "1");
        assert_eq!(
            git(
                &out,
                &["log", "-1", "--format=%an <%ae> %at %cn <%ce> %ct %s"]
            ),
            format!(
                "{AUTHOR_NAME} <{AUTHOR_EMAIL}> {EPOCH} {AUTHOR_NAME} <{AUTHOR_EMAIL}> {EPOCH} {MESSAGE}"
            )
        );
        assert_eq!(git(&out, &["symbolic-ref", "HEAD"]), "refs/heads/main");
        // The working tree carries the content, the symlink and the executable bit.
        assert_eq!(
            fs::read_to_string(out.join("src/lib.rs")).unwrap(),
            "pub fn b() {}\n"
        );
        assert_eq!(
            fs::read_link(out.join("link")).unwrap(),
            Path::new("README.md")
        );
        assert_eq!(
            fs::metadata(out.join("run.sh")).unwrap().mode() & 0o111,
            0o111
        );
        assert_eq!(
            fs::symlink_metadata(out.join("link")).unwrap().mtime(),
            EPOCH
        );
    }

    #[test]
    fn beads_are_dropped_and_their_objects_never_copied() {
        let source = source();
        let beads_blob = git(source.path(), &["rev-parse", "HEAD:.beads/issues.jsonl"]);
        let (_parent, out, _) = synthetic(source.path());
        assert!(!out.join(".beads").exists());
        let mut probe = git::command(Some(&out));
        probe.args(["cat-file", "-e", &beads_blob]);
        assert!(git::output(probe, None).is_err());
        // The first commit's lib.rs is history, so it is absent too.
        let old_blob = git(source.path(), &["rev-parse", "HEAD~1:src/lib.rs"]);
        let mut probe = git::command(Some(&out));
        probe.args(["cat-file", "-e", &old_blob]);
        assert!(git::output(probe, None).is_err());
    }

    #[test]
    fn create_is_deterministic_and_drops_validate() {
        let source = source();
        let (_a, _, first) = synthetic(source.path());
        let (_b, _, second) = synthetic(source.path());
        assert_eq!(first, second);
        let parent = tempfile::tempdir().unwrap();
        for bad in ["", ".", "..", "src/deep"] {
            let out = parent.path().join("bad");
            assert!(
                create(source.path(), "HEAD", &out, &[bad.into()]).is_err(),
                "{bad:?}"
            );
            assert!(!out.exists());
        }
        // A further top-level drop works, and changes the commit.
        let dropped = create(
            source.path(),
            "HEAD",
            &parent.path().join("d"),
            &["src".into()],
        )
        .unwrap();
        assert!(!parent.path().join("d/src").exists());
        assert_ne!(dropped.commit, first.commit);
        // An existing output directory is refused.
        assert!(create(source.path(), "HEAD", &parent.path().join("d"), &[]).is_err());
    }

    #[test]
    fn failed_create_leaves_nothing() {
        let source = source();
        let head = git(source.path(), &["rev-parse", "HEAD"]);
        git(
            source.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{head},sub"),
            ],
        );
        git(source.path(), &["commit", "-q", "-m", "submodule"]);
        let parent = tempfile::tempdir().unwrap();
        let out = parent.path().join("ws");
        let error = create(source.path(), "HEAD", &out, &[]).unwrap_err();
        assert!(error.0.contains("submodule"), "{error}");
        assert!(!out.exists());
        // An unknown base fails before anything is made.
        assert!(create(source.path(), "no-such-rev", &out, &[]).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn clone_fails_check() {
        let source = source();
        let (_parent, _, snapshot) = synthetic(source.path());
        let clones = tempfile::tempdir().unwrap();
        let clone = clones.path().join("clone");
        let mut command = git::command(None);
        command.args(["clone", "-q"]).arg(source.path()).arg(&clone);
        git::output(command, None).unwrap();
        let report = check(&clone, &snapshot.object_set_sha256).unwrap();
        assert!(!report.passed());
        assert!(report.git_skipped);
        for needle in [
            "config key not allowed: remote.origin.url",
            ".git entry not allowed: logs",
            ".git entry not allowed: hooks",
            "mtime not normalized",
        ] {
            assert_flags(&report, needle);
        }
    }

    type Plant = fn(&Path);

    #[test]
    fn each_planted_violation_is_caught() {
        let source = source();
        let cases: Vec<(&str, Plant, &str)> = vec![
            (
                "loose unreachable object",
                |ws| {
                    let mut c = git::command(Some(ws));
                    c.args(["hash-object", "-w", "--stdin"]);
                    git::output(c, Some(b"stray")).unwrap();
                },
                "1 unreachable objects",
            ),
            (
                "packed unreachable object",
                |ws| {
                    let mut c = git::command(Some(ws));
                    c.args(["hash-object", "-w", "--stdin"]);
                    git::output(c, Some(b"stray")).unwrap();
                    git::run(
                        ws,
                        &["repack", "-a", "-d", "-n", "-q", "--keep-unreachable"],
                    )
                    .unwrap();
                    git::run(ws, &["prune-packed"]).unwrap();
                    let loose: usize = fs::read_dir(ws.join(".git/objects"))
                        .unwrap()
                        .filter(|e| e.as_ref().unwrap().file_name().len() == 2)
                        .count();
                    assert_eq!(loose, 0, "the objects are packed");
                },
                "1 unreachable objects",
            ),
            (
                "FETCH_HEAD",
                |ws| fs::write(ws.join(".git/FETCH_HEAD"), "x").unwrap(),
                "FETCH_HEAD",
            ),
            (
                "ORIG_HEAD",
                |ws| fs::write(ws.join(".git/ORIG_HEAD"), "x").unwrap(),
                "ORIG_HEAD",
            ),
            (
                "stash",
                |ws| {
                    git::run(
                        ws,
                        &[
                            "-c",
                            "core.logAllRefUpdates=false",
                            "update-ref",
                            "refs/stash",
                            "HEAD",
                        ],
                    )
                    .unwrap();
                },
                "refs/stash",
            ),
            (
                "alternates",
                |ws| fs::write(ws.join(".git/objects/info/alternates"), "/elsewhere\n").unwrap(),
                "alternates",
            ),
            (
                "hook",
                |ws| {
                    fs::create_dir(ws.join(".git/hooks")).unwrap();
                    fs::write(ws.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
                },
                "hooks",
            ),
            (
                "reflog",
                |ws| {
                    fs::create_dir_all(ws.join(".git/logs")).unwrap();
                    fs::write(ws.join(".git/logs/HEAD"), "x").unwrap();
                },
                "logs",
            ),
            (
                "symlink inside .git",
                |ws| symlink("/tmp", ws.join(".git/refs/elsewhere")).unwrap(),
                "symlink inside .git",
            ),
            (
                ".git as a symlink",
                |ws| {
                    fs::rename(ws.join(".git"), ws.join("real-git")).unwrap();
                    symlink("real-git", ws.join(".git")).unwrap();
                },
                ".git is a symlink",
            ),
            (
                "extra branch",
                |ws| {
                    git::run(
                        ws,
                        &[
                            "-c",
                            "core.logAllRefUpdates=false",
                            "update-ref",
                            "refs/heads/other",
                            "HEAD",
                        ],
                    )
                    .unwrap();
                    // Packed, so only the git step sees it.
                    git::run(ws, &["pack-refs", "--all"]).unwrap();
                },
                "expected one branch, found 2",
            ),
            (
                "loose extra branch",
                |ws| {
                    fs::copy(
                        ws.join(".git/refs/heads/main"),
                        ws.join(".git/refs/heads/x"),
                    )
                    .map(drop)
                    .unwrap()
                },
                "not allowed in .git: refs/heads/x",
            ),
            (
                "stray file under refs",
                |ws| fs::write(ws.join(".git/refs/notes.txt"), "not a ref").unwrap(),
                "not allowed in .git: refs/notes.txt",
            ),
            (
                "stray file under objects",
                |ws| fs::write(ws.join(".git/objects/stray.bin"), "data").unwrap(),
                "not allowed in .git: objects/stray.bin",
            ),
            (
                "stray file in an object fan-out directory",
                |ws| {
                    fs::create_dir_all(ws.join(".git/objects/ab")).unwrap();
                    fs::write(ws.join(".git/objects/ab/not-an-object"), "data").unwrap();
                },
                "not allowed in .git: objects/ab/not-an-object",
            ),
            (
                "stray file in the pack directory",
                |ws| fs::write(ws.join(".git/objects/pack/notes.txt"), "data").unwrap(),
                "not allowed in .git: objects/pack/notes.txt",
            ),
            (
                "HEAD missing",
                |ws| fs::remove_file(ws.join(".git/HEAD")).unwrap(),
                ".git/HEAD is missing",
            ),
            (
                "tag",
                |ws| {
                    git::run(
                        ws,
                        &[
                            "-c",
                            "core.logAllRefUpdates=false",
                            "update-ref",
                            "refs/tags/v1",
                            "HEAD",
                        ],
                    )
                    .unwrap();
                },
                "refs/tags/v1",
            ),
            (
                "remote",
                |ws| {
                    git::run(
                        ws,
                        &["remote", "add", "origin", "https://example.invalid/repo"],
                    )
                    .unwrap();
                },
                "remote.origin.url",
            ),
            (
                "non-boolean macOS key",
                |ws| {
                    git::run(ws, &["config", "core.ignorecase", "bogus"]).unwrap();
                },
                "core.ignorecase",
            ),
            (
                "untracked file",
                |ws| fs::write(ws.join("new.rs"), "x").unwrap(),
                "working tree differs: ?? new.rs",
            ),
            (
                "ignored file",
                |ws| fs::write(ws.join("debug.log"), "x").unwrap(),
                "working tree differs: !! debug.log",
            ),
            (
                "touched mtime",
                |ws| {
                    let at = Timespec {
                        tv_sec: EPOCH + 60,
                        tv_nsec: 0,
                    };
                    utimensat(
                        CWD,
                        ws.join("README.md"),
                        &Timestamps {
                            last_access: at,
                            last_modification: at,
                        },
                        AtFlags::empty(),
                    )
                    .unwrap();
                },
                "mtime not normalized: README.md",
            ),
            (
                "nanosecond mtime",
                |ws| {
                    let at = Timespec {
                        tv_sec: EPOCH,
                        tv_nsec: 1,
                    };
                    utimensat(
                        CWD,
                        ws.join("src/deep/mod.rs"),
                        &Timestamps {
                            last_access: at,
                            last_modification: at,
                        },
                        AtFlags::empty(),
                    )
                    .unwrap();
                },
                "mtime not normalized: src/deep/mod.rs",
            ),
            (
                "touched symlink",
                |ws| {
                    let at = Timespec {
                        tv_sec: EPOCH + 1,
                        tv_nsec: 0,
                    };
                    utimensat(
                        CWD,
                        ws.join("link"),
                        &Timestamps {
                            last_access: at,
                            last_modification: at,
                        },
                        AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .unwrap();
                },
                "mtime not normalized: link",
            ),
        ];
        for (name, plant, needle) in cases {
            let (_parent, out, snapshot) = synthetic(source.path());
            plant(&out);
            let report = check(&out, &snapshot.object_set_sha256).unwrap();
            assert!(!report.passed(), "{name}: passed");
            assert!(
                report.violations.iter().any(|v| v.contains(needle)),
                "{name}: expected {needle:?} in {:?}",
                report.violations
            );
        }
    }
}
