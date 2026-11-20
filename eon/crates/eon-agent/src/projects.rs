//! Project profiles (EON-C40 clause 6): `[projects.NAME]` caps, per-role guards and command
//! overlays, project selection by `--project` or the working directory's repository, and the
//! live-agent records that caps count.
//!
//! A live agent is the process an admission started, identified by its PID and start time:
//! rota's own process, which `launch` turns into the agent by exec and `run` keeps as the
//! run's supervisor (`run` also records its leader). Nothing an agent leaves behind counts.
use crate::cleanup::Done;
use crate::command;
use crate::config::{Config, Level, Role};
use crate::launch::{Failure, atomic_write};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub repo: PathBuf,
    pub agents_root: PathBuf,
    pub done: Option<Done>,
    /// Live work agents (implement and review) in total.
    pub max: Option<usize>,
    #[serde(default)]
    pub roles: BTreeMap<Role, RoleProfile>,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendProfile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleProfile {
    pub max: Option<usize>,
    pub guard: Option<Vec<String>>,
    /// Extra arguments by command executable, inserted at the command's `{overlay}`.
    #[serde(default)]
    pub overlay: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendProfile {
    pub max: usize,
}

/// How long a project guard may run, as backend guards may.
pub const GUARD_SECONDS: u64 = 15;

/// The commands a role reaches on a backend, by label: launch level commands where the backend
/// serves the role (or is the implement fallback), and `run.commands.ROLE` wherever it exists.
pub fn reached<'a>(
    config: &'a Config,
    backend: &'a crate::config::Backend,
    role: Role,
) -> Vec<(String, &'a [String])> {
    let mut commands = Vec::new();
    let fallback = role == Role::Implement && config.fallback.as_deref() == Some(&backend.name);
    if backend.roles.contains(&role) || fallback {
        for (level, name) in [(Level::Medium, "medium"), (Level::High, "high")] {
            if let Some(argv) = backend.commands.get(&level) {
                commands.push((name.to_string(), argv.as_slice()));
            }
        }
    }
    if let Some(argv) = backend.run.as_ref().and_then(|run| run.commands.get(&role)) {
        commands.push((format!("run {}", role.name()), argv.as_slice()));
    }
    commands
}

pub fn validate(config: &Config) -> Result<(), String> {
    for (name, project) in &config.projects {
        let at = format!("projects.{name}");
        if project.max == Some(0) {
            return Err(format!("{at}.max must be at least 1"));
        }
        for (backend, profile) in &project.backends {
            if !config.backends.iter().any(|b| &b.name == backend) {
                return Err(format!(
                    "{at}.backends.{backend} is not a configured backend"
                ));
            }
            if profile.max == 0 {
                return Err(format!("{at}.backends.{backend}.max must be at least 1"));
            }
        }
        for (role, profile) in &project.roles {
            let at = format!("{at}.roles.{}", role.name());
            match profile.max {
                Some(_) if *role == Role::Lead => {
                    return Err(format!(
                        "{at}.max is invalid: leads are not capped by projects"
                    ));
                }
                Some(0) => return Err(format!("{at}.max must be at least 1")),
                _ => (),
            }
            if profile.guard.as_ref().is_some_and(|g| {
                g.first().is_none_or(|p| p.is_empty()) || g.iter().any(|a| a.contains('\0'))
            }) {
                return Err(format!("{at}.guard needs a nonempty command without NUL"));
            }
            for (executable, args) in &profile.overlay {
                let at = format!("{at}.overlay.{executable}");
                if executable.is_empty()
                    || executable == "."
                    || executable == ".."
                    || executable.contains(['/', '\0'])
                {
                    return Err(format!("{at}: overlay keys are executable base names"));
                }
                if args.iter().any(|a| {
                    a.contains('\0')
                        || [
                            command::PROMPT,
                            command::ADD_DIRS,
                            command::OVERLAY,
                            command::DIR,
                        ]
                        .iter()
                        .any(|p| a.contains(p))
                }) {
                    return Err(format!(
                        "{at}: overlay arguments take no NUL, {{prompt}}, {{add_dirs}}, {{overlay}} or {{dir}}"
                    ));
                }
                for backend in &config.backends {
                    for (label, argv) in reached(config, backend, *role) {
                        if command::executable(argv) == executable
                            && !argv.iter().any(|a| a == command::OVERLAY)
                        {
                            return Err(format!(
                                "{at}: backend {}'s {label} command has no {{overlay}} argument",
                                backend.name
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// A selected project and how it was chosen.
pub struct Selected<'a> {
    pub name: &'a str,
    pub project: &'a Project,
    pub from_flag: bool,
}

impl Selected<'_> {
    /// The stderr line that names the selection.
    pub fn line(&self) -> String {
        let from = if self.from_flag {
            "from --project"
        } else {
            "from the working directory's repository"
        };
        format!("project: {} ({from})", self.name)
    }

    fn role(&self, role: Role) -> Option<&RoleProfile> {
        self.project.roles.get(&role)
    }
}

/// `--project NAME`, or the project whose `repo` is the repository holding `cwd` (canonical).
pub fn select<'a>(
    config: &'a Config,
    flag: Option<&str>,
    cwd: &Path,
) -> Result<Option<Selected<'a>>, Failure> {
    if let Some(name) = flag {
        let (name, project) = config
            .projects
            .get_key_value(name)
            .ok_or_else(|| (2, format!("unknown project {name}")))?;
        return Ok(Some(Selected {
            name,
            project,
            from_flag: true,
        }));
    }
    let Some(repo) = repository_of(cwd) else {
        return Ok(None);
    };
    let matches: Vec<_> = config
        .projects
        .iter()
        .filter(|(_, p)| fs::canonicalize(&p.repo).unwrap_or_else(|_| p.repo.clone()) == repo)
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [(name, project)] => Ok(Some(Selected {
            name,
            project,
            from_flag: false,
        })),
        [(a, _), (b, _), ..] => Err((
            2,
            format!(
                "projects {a} and {b} both hold repository {}; pass --project",
                repo.display()
            ),
        )),
    }
}

/// The main checkout of the git repository holding `dir`, found without running git: the
/// nearest ancestor with `.git`. A `.git` file (linked worktree, submodule or separate git dir)
/// is followed through `gitdir:` and `commondir`, each relative to the file naming it.
pub fn repository_of(dir: &Path) -> Option<PathBuf> {
    for at in dir.ancestors() {
        let dot_git = at.join(".git");
        let Ok(meta) = fs::metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return Some(at.to_path_buf());
        }
        let text = fs::read_to_string(&dot_git).ok()?;
        let Some(gitdir) = text.lines().next().and_then(|l| l.strip_prefix("gitdir:")) else {
            return Some(at.to_path_buf());
        };
        let Ok(gitdir) = fs::canonicalize(at.join(gitdir.trim())) else {
            return Some(at.to_path_buf());
        };
        let Ok(common) = fs::read_to_string(gitdir.join("commondir")) else {
            // A submodule or separate git dir: the checkout is where the `.git` file is.
            return Some(at.to_path_buf());
        };
        let common = fs::canonicalize(gitdir.join(common.trim())).ok()?;
        return Some(match common.file_name() {
            Some(name) if name == ".git" => common.parent()?.to_path_buf(),
            // A worktree of a bare repository.
            _ => common,
        });
    }
    None
}

/// The process identity a record keeps: PID and start time in clock ticks since boot.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Record {
    pub schema: u32,
    pub id: String,
    pub project: String,
    pub role: Role,
    pub backend: String,
    /// The inode of the admitter's PID namespace; PIDs are checked only within it.
    pub pid_ns: u64,
    pub processes: Vec<Process>,
    pub started: u64,
}

const SCHEMA: u32 = 1;

/// Where a state file's live-agent records are.
pub fn records_dir(state_file: &Path) -> PathBuf {
    crate::session::state_dir(state_file).join("agents")
}

/// What liveness checks compare against.
pub struct Scanner {
    pub proc: PathBuf,
    pub euid: u32,
    pub pid_ns: u64,
}

enum State {
    Live,
    Gone,
}

impl Scanner {
    pub fn system() -> Result<Self, String> {
        Ok(Self {
            proc: PathBuf::from("/proc"),
            euid: rustix::process::geteuid().as_raw(),
            pid_ns: own_pid_ns()?,
        })
    }

    /// This process's identity as this scanner's proc sees it (`self`), which stays right when
    /// the proc belongs to another PID namespace than `getpid` answers in.
    pub fn own(&self) -> Result<Process, String> {
        let link = self.proc.join("self");
        let pid = fs::read_link(&link)
            .ok()
            .and_then(|target| target.to_str()?.parse().ok())
            .ok_or_else(|| format!("cannot read {}", link.display()))?;
        self.identity(pid)
    }

    /// The identity of `pid`, read from this scanner's proc.
    pub fn identity(&self, pid: u32) -> Result<Process, String> {
        let stat = fs::read_to_string(self.proc.join(pid.to_string()).join("stat"))
            .map_err(|e| format!("cannot read process {pid}: {e}"))?;
        let (_, start_ticks) = parse_stat(&stat).ok_or_else(|| format!("bad stat for {pid}"))?;
        Ok(Process { pid, start_ticks })
    }

    /// Whether `process` still runs as this user. Only a gone process (absent, another start
    /// time or user, a zombie) is Gone; anything unreadable otherwise is an error.
    fn state(&self, process: &Process) -> Result<State, String> {
        let dir = self.proc.join(process.pid.to_string());
        let read = |name: &str| match fs::read_to_string(dir.join(name)) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == ErrorKind::NotFound || e.raw_os_error() == Some(libc::ESRCH) => {
                Ok(None)
            }
            Err(e) => Err(format!("cannot read {}: {e}", dir.join(name).display())),
        };
        let Some(stat) = read("stat")? else {
            return Ok(State::Gone);
        };
        let (state, start_ticks) =
            parse_stat(&stat).ok_or_else(|| format!("cannot parse {}/stat", dir.display()))?;
        if start_ticks != process.start_ticks || matches!(state, 'Z' | 'X' | 'x') {
            return Ok(State::Gone);
        }
        let Some(status) = read("status")? else {
            return Ok(State::Gone);
        };
        let euid = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|uids| uids.split_whitespace().nth(1))
            .and_then(|uid| uid.parse::<u32>().ok())
            .ok_or_else(|| format!("cannot parse {}/status", dir.display()))?;
        Ok(if euid == self.euid {
            State::Live
        } else {
            State::Gone
        })
    }
}

fn own_pid_ns() -> Result<u64, String> {
    fs::metadata("/proc/self/ns/pid")
        .map(|meta| meta.ino())
        .map_err(|e| format!("cannot read the PID namespace: {e}"))
}

/// The state letter and start time (field 22) of a `/proc/PID/stat` line, read after the last
/// `)` so a command name with spaces or parentheses cannot shift fields.
fn parse_stat(stat: &str) -> Option<(char, u64)> {
    let fields: Vec<&str> = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    let state = fields.first()?.chars().next()?;
    Some((state, fields.get(19)?.parse().ok()?))
}

/// The live records under `dir`, each holding only its live processes (all of them for a record
/// from another PID namespace). With `prune`, records whose processes are all gone are removed,
/// and only once every record was judged: a record that cannot be read, parsed or judged fails
/// the scan, and then nothing is removed.
pub fn scan(dir: &Path, scanner: &Scanner, prune: bool) -> Result<Vec<Record>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "cannot read live-agent records {}: {e}",
                dir.display()
            ));
        }
    };
    let mut live = Vec::new();
    let mut gone = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read live-agent records: {e}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let path = entry.path();
        let unreadable =
            |why: String| format!("unreadable live-agent record {}: {why}", path.display());
        let text = match fs::read(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(unreadable(e.to_string())),
        };
        let schema = serde_json::from_slice::<serde_json::Value>(&text)
            .map_err(|e| unreadable(e.to_string()))?
            .get("schema")
            .and_then(|s| s.as_u64());
        if schema != Some(u64::from(SCHEMA)) {
            return Err(unreadable(format!("unknown schema {schema:?}")));
        }
        let mut record: Record =
            serde_json::from_slice(&text).map_err(|e| unreadable(e.to_string()))?;
        if record.pid_ns == scanner.pid_ns {
            let mut processes = Vec::new();
            for process in record.processes {
                let state = scanner
                    .state(&process)
                    .map_err(|e| format!("cannot scan /proc for project caps: {e}"))?;
                if matches!(state, State::Live) {
                    processes.push(process);
                }
            }
            record.processes = processes;
        }
        if record.processes.is_empty() {
            gone.push(path);
        } else {
            live.push(record);
        }
    }
    if prune {
        for path in gone {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != ErrorKind::NotFound => {
                    return Err(format!("cannot remove {}: {e}", path.display()));
                }
                _ => (),
            }
        }
    }
    live.sort_by_key(|r| r.started);
    Ok(live)
}

/// A record written for an admission; dropping it unlinks the file until [`Admitted::keep`].
pub struct Admitted {
    path: PathBuf,
    record: Record,
    keep: bool,
}

impl Admitted {
    pub fn write(
        state_file: &Path,
        project: &str,
        role: Role,
        backend: &str,
        scanner: &Scanner,
    ) -> Result<Self, Failure> {
        // The file is named by the random id alone: a project name never becomes a path.
        let id = crate::run::random()?;
        let own = scanner.own().map_err(|e| (1, e))?;
        let record = Record {
            schema: SCHEMA,
            id: id.clone(),
            project: project.into(),
            role,
            backend: backend.into(),
            pid_ns: scanner.pid_ns,
            processes: vec![own],
            started: crate::quota::now_ms() / 1000,
        };
        let admitted = Self {
            path: records_dir(state_file).join(format!("{id}.json")),
            record,
            keep: false,
        };
        admitted.save()?;
        Ok(admitted)
    }

    fn save(&self) -> Result<(), Failure> {
        let bytes = serde_json::to_vec(&self.record).map_err(|e| (1, e.to_string()))?;
        atomic_write(&self.path, &bytes).map_err(|e| {
            (
                1,
                format!(
                    "cannot write live-agent record {}: {e}",
                    self.path.display()
                ),
            )
        })
    }

    /// Adds another process (a run's leader) to the record.
    pub fn add(&mut self, process: Process) -> Result<(), Failure> {
        self.record.processes.push(process);
        self.save()
    }

    /// Keeps the record once the agent runs; scans prune it when its processes are gone.
    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn plural(n: usize, one: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    }
}

/// The agents' PIDs (each record's first process), in numeric order.
fn pids(records: &[&Record]) -> String {
    let mut pids: Vec<u32> = records
        .iter()
        .filter_map(|record| record.processes.first().map(|p| p.pid))
        .collect();
    pids.sort_unstable();
    let mut text = String::new();
    for (i, pid) in pids.iter().enumerate() {
        let _ = write!(text, "{}{pid}", if i == 0 { "" } else { ", " });
    }
    text
}

/// A total or role cap that `role` would exceed; leads are never capped.
pub fn hold(selected: &Selected<'_>, role: Role, live: &[Record]) -> Option<String> {
    if role == Role::Lead {
        return None;
    }
    let ours: Vec<&Record> = live.iter().filter(|r| r.project == selected.name).collect();
    let name = selected.name;
    if let Some(max) = selected.project.max
        && ours.len() >= max
    {
        return Some(format!(
            "project {name}: {} (max {max}; pids {})",
            plural(ours.len(), "live agent"),
            pids(&ours)
        ));
    }
    let max = selected.role(role).and_then(|r| r.max)?;
    let same: Vec<&Record> = ours.into_iter().filter(|r| r.role == role).collect();
    (same.len() >= max).then(|| {
        format!(
            "project {name}: {} (roles.{}.max {max}; pids {})",
            plural(same.len(), &format!("live {} agent", role.name())),
            role.name(),
            pids(&same)
        )
    })
}

/// A project backend cap that `backend` has reached for `role`; leads are never capped.
pub fn backend_hold(
    selected: &Selected<'_>,
    role: Role,
    backend: &str,
    live: &[Record],
) -> Option<String> {
    let max = selected.project.backends.get(backend)?.max;
    if role == Role::Lead {
        return None;
    }
    let on: Vec<&Record> = live
        .iter()
        .filter(|r| r.project == selected.name && r.backend == backend)
        .collect();
    (on.len() >= max).then(|| {
        format!(
            "project {}: {} on {backend} (backends.{backend}.max {max}; pids {})",
            selected.name,
            plural(on.len(), "live agent"),
            pids(&on)
        )
    })
}

/// The overlay `role` applies to a command: empty without a project or overlays, and an
/// error naming the executable when the role sets overlays but none for it.
pub fn overlay<'a>(
    selected: Option<&Selected<'a>>,
    role: Role,
    argv: &[String],
) -> Result<&'a [String], String> {
    let Some(selected) = selected else {
        return Ok(&[]);
    };
    let Some(profile) = selected.project.roles.get(&role) else {
        return Ok(&[]);
    };
    if profile.overlay.is_empty() {
        return Ok(&[]);
    }
    let executable = command::executable(argv);
    profile
        .overlay
        .get(executable)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            format!(
                "project {} role {} has no overlay for {executable}",
                selected.name,
                role.name()
            )
        })
}

/// Runs `role`'s guard in `cwd`; the error is the hold reason.
pub fn guard(selected: &Selected<'_>, role: Role, cwd: &Path) -> Result<(), String> {
    let Some(argv) = selected.role(role).and_then(|r| r.guard.as_ref()) else {
        return Ok(());
    };
    crate::readings::guard(argv, GUARD_SECONDS, cwd)
        .map_err(|why| format!("project {}: {} guard {why}", selected.name, role.name()))
}

#[cfg(test)]
mod tests;
