//! Retire build output while preserving work evidence. Errors always keep files.
//!
//! Inventory pins parent directory handles; removal uses no-follow, no-cross-mount
//! traversal and verifies entry identities. Readable target directories become
//! writable by handle before removing contents. CACHEDIR.TAG is removed last.
//! Interrupted removals can resume from the same tagged directory after guards pass;
//! no renames, manifests or persistent cleanup state are used. An empty target can
//! finish with rmdir even after interruption removed its tag. The inspecting process
//! is excluded from liveness checks because inventory holds traversal descriptors.
//! openat2 requires Linux 5.6+. A blocked syscall fails closed. Permission changes
//! affect target directories only, never evidence directories or hard-linked files.
mod fs;
#[cfg(test)]
mod tests;

use self::fs::{Dir, Stat, event, invalid};
use crate::config::Config;
use crate::launch::Failure;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const TAG: &str = "CACHEDIR.TAG";
const SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub const HELP: &str = "Usage: rota retire NAME [--project P] [--bead ID] [--target-only] [--dry-run] [--json]\n\
Guard: resolved cwd, executable, CARGO_TARGET_DIR, open-fd and mapped-file paths, plus cwd directory identity.\n\
Unreadable details of owned processes keep files and exit 1, naming the PID.\n\
The caller vouches that NAME and --bead identify the same finished task.\n\
Bind-mount aliases of agent directories are unsupported and may not be detected.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub repo: PathBuf,
    pub agents_root: PathBuf,
    pub done: Option<Done>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Done {
    pub command: Vec<String>,
    pub pointers: Vec<String>,
    pub equals: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cleanup {
    pub min_idle: String,
    pub store_prefixes: Vec<PathBuf>,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            min_idle: "2h".into(),
            store_prefixes: Vec::new(),
        }
    }
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && !path.as_os_str().as_bytes().contains(&0)
}

pub fn validate(config: &Config) -> Result<(), String> {
    if crate::run::parse_timeout(&config.cleanup.min_idle).is_none()
        || config
            .cleanup
            .store_prefixes
            .iter()
            .any(|p| !absolute(p) || p == Path::new("/"))
    {
        return Err(
            "cleanup needs a positive min_idle duration and absolute non-root store_prefixes"
                .into(),
        );
    }
    for (name, project) in &config.projects {
        if !identifier(name)
            || !absolute(&project.repo)
            || !absolute(&project.agents_root)
            || project.agents_root == Path::new("/")
        {
            return Err(
                "projects need identifier names and absolute repo/agents_root without traversal"
                    .into(),
            );
        }
        if let Some(done) = &project.done {
            if done.command.first().is_none_or(|s| s.is_empty())
                || done.command.iter().any(|s| s.contains('\0'))
                || !done.command.iter().any(|s| s.contains("{bead}"))
                || done.pointers.is_empty()
                || done.pointers.iter().any(|s| {
                    (!s.is_empty() && !s.starts_with('/'))
                        || s.as_bytes()
                            .windows(2)
                            .any(|p| p[0] == b'~' && !b"01".contains(&p[1]))
                        || s.ends_with('~')
                })
            {
                return Err(
                    "project done needs command with {bead} and valid JSON pointers".into(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct Options {
    pub name: String,
    pub project: Option<String>,
    pub bead: Option<String>,
    pub target_only: bool,
    pub dry_run: bool,
    pub json: bool,
}

pub fn parse(args: &[String]) -> Result<Options, Failure> {
    let mut result = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" | "--bead" => {
                let value = args
                    .next()
                    .filter(|s| !s.is_empty() && !s.starts_with('-'))
                    .ok_or_else(|| (2, format!("{arg} needs a value")))?;
                let slot = if arg == "--project" {
                    &mut result.project
                } else {
                    &mut result.bead
                };
                if slot.replace(value.clone()).is_some() {
                    return Err((2, format!("duplicate {arg}")));
                }
            }
            "--target-only" => result.target_only = true,
            "--dry-run" => result.dry_run = true,
            "--json" => result.json = true,
            _ if !arg.starts_with('-') && result.name.is_empty() => result.name = arg.clone(),
            _ => return Err((2, "unknown retire option or extra name".into())),
        }
    }
    if !identifier(&result.name) {
        return Err((2, "retire needs one identifier NAME".into()));
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    name: OsString,
    stat: Stat,
    link: Option<OsString>,
    children: Vec<Node>,
}

fn scan(parent: &Dir, name: &OsStr) -> io::Result<Node> {
    let stat = parent.stat(name)?;
    let mut node = Node {
        name: name.into(),
        stat,
        link: None,
        children: Vec::new(),
    };
    if node.stat.is(libc::S_IFDIR) {
        let dir = parent.child(name)?;
        if !node.stat.identity(&dir.stat(OsStr::new("."))?) {
            return Err(invalid("directory replaced while opening"));
        }
        for name in dir.names()? {
            node.children.push(scan(&dir, &name)?);
        }
    } else if node.stat.is(libc::S_IFLNK) {
        node.link = Some(parent.link(name)?);
    }
    if node.stat != parent.stat(name)? {
        return Err(invalid("entry changed during scan"));
    }
    Ok(node)
}

fn tag(dir: &Dir) -> io::Result<Option<Vec<u8>>> {
    match dir.stat(OsStr::new(TAG)) {
        Ok(stat) if stat.is(libc::S_IFREG) => {
            let bytes = dir.read(OsStr::new(TAG), &stat)?;
            Ok(bytes.starts_with(SIGNATURE).then_some(bytes))
        }
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn size(node: &Node, seen: &mut BTreeSet<(u64, u64)>) -> u64 {
    let own = if seen.insert((node.stat.dev, node.stat.ino)) {
        node.stat.blocks * 512
    } else {
        0
    };
    own + node.children.iter().map(|n| size(n, seen)).sum::<u64>()
}

fn newest(node: &Node) -> (i64, i64) {
    node.children
        .iter()
        .map(newest)
        .fold((node.stat.mtime, node.stat.nanos), std::cmp::max)
}

#[derive(Serialize)]
struct PathReport {
    path: PathBuf,
    kind: String,
    bytes: u64,
    outcome: String,
    reason: Option<String>,
}

struct Candidate {
    parent: Dir,
    node: Node,
    report: usize,
    target: bool,
}

#[derive(Serialize)]
struct Report {
    name: String,
    project: String,
    bead: Option<String>,
    dry_run: bool,
    target_only: bool,
    done: String,
    error: Option<String>,
    freed_bytes: u64,
    paths: Vec<PathReport>,
}

impl Report {
    fn add(&mut self, path: PathBuf, kind: &str, bytes: u64, reason: Option<String>) -> usize {
        let index = self.paths.len();
        self.paths.push(PathReport {
            path,
            kind: kind.into(),
            bytes,
            outcome: "kept".into(),
            reason,
        });
        index
    }
    fn keep_all(&mut self, reason: &str) {
        for path in &mut self.paths {
            if path.outcome == "kept" && path.reason.is_none() {
                path.reason = Some(reason.into());
            }
        }
    }
}

fn store_link(node: &Node, prefixes: &[PathBuf]) -> bool {
    node.link.as_ref().is_some_and(|text| {
        let path = Path::new(text);
        absolute(path) && prefixes.iter().any(|prefix| path.starts_with(prefix))
    })
}

struct Inventory<'a> {
    root: &'a Dir,
    project: &'a Project,
    config: &'a Cleanup,
    options: &'a Options,
    report: &'a mut Report,
    candidates: Vec<Candidate>,
    seen: BTreeSet<(u64, u64)>,
}

impl Inventory<'_> {
    fn add(
        &mut self,
        parent: &Dir,
        path: PathBuf,
        node: Node,
        kind: &str,
        reason: Option<String>,
    ) -> io::Result<()> {
        if path.to_str().is_none() {
            return Err(invalid("cannot report a non-UTF-8 cleanup path"));
        }
        let bytes = size(&node, &mut self.seen);
        let index = self.report.add(path, kind, bytes, reason);
        self.candidates.push(Candidate {
            parent: parent.child(OsStr::new("."))?,
            node,
            report: index,
            target: kind == "target",
        });
        Ok(())
    }

    fn target(&mut self, parent: &Dir, path: &Path, name: &OsStr) -> io::Result<()> {
        let stat = match parent.stat(name) {
            Ok(stat) => stat,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let tagged = stat.is(libc::S_IFDIR) && tag(&parent.child(name)?)?.is_some();
        let empty = stat.is(libc::S_IFDIR) && !tagged && parent.child(name)?.names()?.is_empty();
        let reason = (!(tagged || empty))
            .then(|| "target is not a plain directory with a regular Cargo CACHEDIR.TAG".into());
        let node = if tagged || empty {
            let node = scan(parent, name)?;
            if !node.stat.identity(&stat) || (empty && !node.children.is_empty()) {
                return Err(invalid("target changed during inspection"));
            }
            node
        } else {
            Node {
                name: name.into(),
                stat,
                link: None,
                children: Vec::new(),
            }
        };
        self.add(parent, path.join(name), node, "target", reason)
    }

    fn work_links(&mut self, dir: &Dir, path: &Path, top: bool) -> io::Result<u64> {
        let mut bytes = 0;
        for name in dir.names()? {
            if top && name == "target" {
                continue;
            }
            let stat = dir.stat(&name)?;
            if stat.is(libc::S_IFDIR) {
                let child = dir.child(&name)?;
                if !stat.identity(&child.stat(OsStr::new("."))?) {
                    return Err(invalid("work directory replaced"));
                }
                bytes += self.work_links(&child, &path.join(&name), false)?;
            } else {
                let node = scan(dir, &name)?;
                if store_link(&node, &self.config.store_prefixes) {
                    self.add(dir, path.join(&name), node, "store_link", None)?;
                } else {
                    bytes += size(&node, &mut self.seen);
                }
            }
            if stat != dir.stat(&name)? {
                return Err(invalid("work entry changed during scan"));
            }
        }
        Ok(bytes)
    }

    fn collect(mut self) -> io::Result<Vec<Candidate>> {
        let root = self.root;
        let path = self.project.agents_root.clone();
        self.target(
            root,
            &path,
            OsStr::new(&format!("{}-target", self.options.name)),
        )?;
        let work = OsString::from(format!("{}-work", self.options.name));
        match root.stat(&work) {
            Ok(stat) if stat.is(libc::S_IFDIR) => {
                let dir = root.child(&work)?;
                if !stat.identity(&dir.stat(OsStr::new("."))?) {
                    return Err(invalid("work directory changed"));
                }
                let work_path = path.join(&work);
                self.target(&dir, &work_path, OsStr::new("target"))?;
                if !self.options.target_only {
                    let bytes = self.work_links(&dir, &work_path, true)?;
                    self.report.add(
                        work_path,
                        "work",
                        bytes,
                        Some("work evidence is preserved outside the direct tagged target".into()),
                    );
                }
            }
            Ok(_) => {
                self.report.add(
                    path.join(&work),
                    "target",
                    0,
                    Some("work parent is not a plain directory".into()),
                );
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        if !self.options.target_only {
            for (suffix, kind) in [("-prompt.md", "prompt"), ("-launch.sh", "launcher")] {
                let name = OsString::from(format!("{}{suffix}", self.options.name));
                match root.stat(&name) {
                    Ok(stat) => {
                        let reason = (!stat.is(libc::S_IFREG)).then(|| "not a regular file".into());
                        let file_path = path.join(&name);
                        self.add(
                            root,
                            file_path,
                            Node {
                                name,
                                stat,
                                link: None,
                                children: Vec::new(),
                            },
                            kind,
                            reason,
                        )?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(self.candidates)
    }
}

fn done(project: &Project, options: &Options) -> io::Result<bool> {
    let (Some(check), Some(bead)) = (&project.done, &options.bead) else {
        return Ok(false);
    };
    let repo = project
        .repo
        .to_str()
        .ok_or_else(|| invalid("repo is not UTF-8"))?;
    let argv: Vec<_> = check
        .command
        .iter()
        .map(|s| {
            crate::run::fill(
                s,
                &[
                    ("{bead}", bead),
                    ("{repo}", repo),
                    ("{name}", &options.name),
                ],
            )
        })
        .collect();
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    let output = crate::readings::probe_command(command, 30, &project.repo).map_err(invalid)?;
    let value: serde_json::Value =
        serde_json::from_str(&output).map_err(|e| invalid(format!("done JSON: {e}")))?;
    let value = check
        .pointers
        .iter()
        .find_map(|p| value.pointer(p))
        .ok_or_else(|| invalid("done JSON has no configured pointer"))?;
    let status = value
        .as_str()
        .ok_or_else(|| invalid("done status is not a string"))?;
    Ok(status == check.equals)
}

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => (),
            c => result.push(c.as_os_str()),
        }
    }
    result
}

fn resolved(path: &Path) -> io::Result<PathBuf> {
    let mut prefix = path;
    let mut tail = Vec::new();
    loop {
        match std::fs::canonicalize(prefix) {
            Ok(mut path) => {
                for part in tail.into_iter().rev() {
                    path.push(part);
                }
                return Ok(normalized(&path));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut parts = prefix.components();
                let part = parts.next_back().ok_or(e)?;
                tail.push(part.as_os_str().to_owned());
                prefix = parts.as_path();
                if prefix.as_os_str().is_empty() {
                    return Err(invalid("cannot resolve process path"));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn mapped_paths(maps: &[u8]) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for mut line in maps.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        for _ in 0..5 {
            line = line.trim_ascii_start();
            if line.is_empty() {
                return Err(invalid("incomplete process maps entry"));
            }
            let end = line
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap_or(line.len());
            line = &line[end..];
        }
        let path = line.trim_ascii_start();
        if path.is_empty() || path.starts_with(b"[") {
            continue;
        }
        if !path.starts_with(b"/") {
            return Err(invalid("unrecognized process maps path"));
        }
        paths.push(PathBuf::from(OsStr::from_bytes(path)));
        // proc_pid_maps(5): a newline and a literal backslash-012 are ambiguous; check both.
        let mut decoded = Vec::new();
        let mut i = 0;
        while i < path.len() {
            if path[i..].starts_with(b"\\012") {
                decoded.push(b'\n');
                i += 4;
            } else {
                decoded.push(path[i]);
                i += 1;
            }
        }
        if decoded != path {
            paths.push(PathBuf::from(OsStr::from_bytes(&decoded)));
        }
    }
    Ok(paths)
}

fn process_uid(status: &str) -> io::Result<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|uid| uid.parse().ok())
        .ok_or_else(|| invalid("cannot determine process effective UID"))
}

fn live(
    config: &Config,
    project: &Project,
    options: &Options,
    proc: &Path,
    cache_ids: &BTreeSet<(u64, u64)>,
) -> io::Result<Option<String>> {
    let roots = [
        project.agents_root.join(&options.name),
        project.agents_root.join(format!("{}-work", options.name)),
        project.agents_root.join(format!("{}-target", options.name)),
    ];
    let roots: Vec<_> = roots
        .iter()
        .map(|p| resolved(p))
        .collect::<io::Result<_>>()?;
    let targets = [roots[2].clone(), roots[1].join("target")];
    let mut directory_ids = cache_ids.clone();
    for root in &roots {
        match std::fs::metadata(root) {
            Ok(stat) if stat.is_dir() => {
                directory_ids.insert((stat.dev(), stat.ino()));
            }
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    let uid = unsafe { libc::geteuid() };
    let mut found = None;
    for entry in std::fs::read_dir(proc)? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let path = entry.path();
        let inspect = || -> io::Result<Option<bool>> {
            let status = std::fs::read_to_string(path.join("status"))?;
            // Non-dumpable tasks can have root-owned /proc directories without changing UID.
            if process_uid(&status)? != uid {
                return Ok(None);
            }
            event("process_owned")?;
            let state = status
                .lines()
                .find_map(|s| s.strip_prefix("State:"))
                .and_then(|s| s.split_whitespace().next())
                .ok_or_else(|| invalid("cannot determine process state"))?;
            if matches!(state, "Z" | "X" | "x") {
                return Ok(None);
            }
            let cwd = std::fs::read_link(path.join("cwd"))?;
            let exe = std::fs::read_link(path.join("exe"))?;
            let env = std::fs::read(path.join("environ"))?;
            let cwd = resolved(&cwd)?;
            let exe = resolved(&exe)?;
            let in_session = |p: &Path| roots.iter().any(|root| p.starts_with(root));
            let mut in_use = in_session(&cwd) || in_session(&exe);
            for value in env
                .split(|b| *b == 0)
                .filter_map(|v| v.strip_prefix(b"CARGO_TARGET_DIR="))
            {
                if !value.is_empty() {
                    let target = resolved(&cwd.join(OsStr::from_bytes(value)))?;
                    in_use |= targets.iter().any(|t| target.starts_with(t));
                }
            }
            for fd in std::fs::read_dir(path.join("fd"))
                .map_err(|e| invalid(format!("file descriptors: {e}")))?
            {
                let reference = std::fs::read_link(fd?.path())
                    .map_err(|e| invalid(format!("file descriptor link: {e}")))?;
                if reference.is_absolute() {
                    in_use |= in_session(&resolved(&reference)?);
                }
            }
            let maps = std::fs::read(path.join("maps"))
                .map_err(|e| invalid(format!("memory maps: {e}")))?;
            for reference in mapped_paths(&maps)? {
                in_use |= in_session(&resolved(&reference)?);
            }
            if !in_use {
                in_use = fs::ancestors_match(&path.join("cwd"), &directory_ids)?;
            }
            if process_uid(&std::fs::read_to_string(path.join("status"))?)? != uid {
                return Ok(None);
            }
            Ok(Some(in_use))
        };
        match inspect() {
            Ok(Some(in_use)) => {
                if in_use {
                    found = Some(format!(
                        "live session: pid {pid} uses its worktree, work directory or target (cwd, executable, target environment, open fd or mapping)"
                    ));
                }
            }
            Ok(None) => (),
            Err(e) => {
                let gone =
                    std::fs::metadata(&path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
                let other = std::fs::read_to_string(path.join("status"))
                    .ok()
                    .and_then(|status| process_uid(&status).ok())
                    .is_some_and(|owner| owner != uid);
                if !gone && !other {
                    return Err(invalid(format!("cannot inspect process {pid}: {e}")));
                }
            }
        }
    }
    let (sessions, problems) = crate::session::list(&config.state_file)?;
    if !problems.is_empty() {
        return Err(invalid(problems.join("; ")));
    }
    for session in sessions {
        if session.name == options.name && session.finished.is_none() {
            match std::fs::metadata(proc.join(session.pid.to_string())) {
                Ok(_) => {
                    found = Some(format!(
                        "live registry session: {} (pid {})",
                        session.id, session.pid
                    ))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => {
                    return Err(invalid(format!(
                        "cannot inspect registry pid {}: {e}",
                        session.pid
                    )));
                }
            }
        }
    }
    Ok(found)
}

// Only a validated target may remove regular files beneath the work directory.
fn remove_tree(parent: &Dir, node: &Node, target_root: bool) -> io::Result<()> {
    if !node.stat.identity(&parent.stat(&node.name)?) {
        return Err(invalid("entry replaced before removal"));
    }
    if !node.stat.is(libc::S_IFDIR) {
        return parent.unlink(&node.name, false);
    }
    let dir = parent.child(&node.name)?;
    if !node.stat.identity(&dir.stat(OsStr::new("."))?) {
        return Err(invalid("directory replaced before chmod"));
    }
    dir.writable()?;
    for child in &node.children {
        if target_root && child.name == TAG {
            continue;
        }
        remove_tree(&dir, child, false)?;
    }
    if target_root {
        event("before_tag_unlink")?;
        dir.unlink(OsStr::new(TAG), false)?;
        event("after_tag_unlink")?;
        event("final_rmdir")?;
    }
    parent.unlink(&node.name, true)
}

fn remove_candidate(candidate: &Candidate) -> io::Result<()> {
    event("before_remove")?;
    let actual = scan(&candidate.parent, &candidate.node.name)?;
    if actual != candidate.node {
        return Err(invalid("candidate replaced or changed after inspection"));
    }
    drop(actual);
    candidate.parent.check_write()?;
    if candidate.target && tag(&candidate.parent.child(&candidate.node.name)?)?.is_none() {
        if candidate.node.children.is_empty() {
            return candidate.parent.unlink(&candidate.node.name, true);
        }
        return Err(invalid("target tag changed"));
    }
    remove_tree(&candidate.parent, &candidate.node, candidate.target)
}

fn execute(
    config: &Config,
    project: &Project,
    options: &Options,
    proc: &Path,
    report: &mut Report,
) -> io::Result<u8> {
    let root = Dir::absolute(&project.agents_root)?;
    let candidates = Inventory {
        root: &root,
        project,
        config: &config.cleanup,
        options,
        report,
        candidates: Vec::new(),
        seen: BTreeSet::new(),
    }
    .collect()?;
    if report.paths.is_empty() {
        return Ok(0);
    }
    let passed = done(project, options)?;
    report.done = if passed { "passed" } else { "not passed" }.into();
    let cache_ids = candidates
        .iter()
        .filter(|c| c.target && c.node.stat.is(libc::S_IFDIR))
        .map(|c| (c.node.stat.dev, c.node.stat.ino))
        .collect();
    if let Some(reason) = live(config, project, options, proc, &cache_ids)? {
        report.keep_all(&reason);
        return Ok(3);
    }
    if !options.target_only && !passed {
        report.keep_all("done check did not pass; full retire requires --bead and project.done");
        return Ok(3);
    }
    let idle = crate::run::parse_timeout(&config.cleanup.min_idle).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| invalid(e.to_string()))?
        .as_nanos() as i128;
    let mut code = 0;
    for candidate in &candidates {
        let entry = &mut report.paths[candidate.report];
        if entry.reason.is_some() {
            code = 3;
            continue;
        }
        let (seconds, nanos) = newest(&candidate.node);
        if candidate.target
            && !candidate.node.children.is_empty()
            && !passed
            && now - (i128::from(seconds) * 1_000_000_000 + i128::from(nanos))
                < i128::from(idle) * 1_000_000_000
        {
            entry.reason = Some(format!(
                "target changed within min_idle ({})",
                config.cleanup.min_idle
            ));
            code = 3;
            continue;
        }
        if options.dry_run {
            entry.outcome = "would_remove".into();
            continue;
        }
        if let Some(reason) = live(config, project, options, proc, &cache_ids)? {
            report.keep_all(&reason);
            return Ok(3);
        }
        if let Err(e) = remove_candidate(candidate) {
            report.paths[candidate.report].outcome = "failed".into();
            report.paths[candidate.report].reason = Some(e.to_string());
            return Err(e);
        }
        let entry = &mut report.paths[candidate.report];
        entry.outcome = "removed".into();
        report.freed_bytes += entry.bytes;
    }
    if report
        .paths
        .iter()
        .any(|p| p.kind != "work" && p.outcome == "kept")
    {
        code = 3;
    }
    Ok(code)
}

pub fn run(config: &Config, options: &Options) -> Result<u8, Failure> {
    let (name, project) = match &options.project {
        Some(name) => config.projects.get_key_value(name),
        None if config.projects.len() == 1 => config.projects.first_key_value(),
        None => None,
    }
    .ok_or_else(|| {
        (
            2,
            "choose a configured cleanup project with --project".into(),
        )
    })?;
    let mut report = Report {
        name: options.name.clone(),
        project: name.clone(),
        bead: options.bead.clone(),
        dry_run: options.dry_run,
        target_only: options.target_only,
        done: "not checked".into(),
        error: None,
        freed_bytes: 0,
        paths: Vec::new(),
    };
    let result = execute(config, project, options, Path::new("/proc"), &mut report);
    if let Err(e) = &result {
        report.error = Some(e.to_string());
        report.keep_all(&format!("inspection/removal failed: {e}"));
    }
    if options.json {
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        for path in &report.paths {
            println!(
                "{} {} ({} bytes){}",
                path.outcome,
                path.path.display(),
                path.bytes,
                path.reason
                    .as_ref()
                    .map(|s| format!(": {s}"))
                    .unwrap_or_default()
            );
        }
        if report.paths.is_empty() && result.is_ok() {
            println!("nothing found");
        }
        println!(
            "retire {}: freed {} bytes",
            options.name, report.freed_bytes
        );
    }
    result.map_err(|e| (1, e.to_string()))
}
