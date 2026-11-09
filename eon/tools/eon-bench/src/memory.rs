//! Linux process PSS, with workload children excluded. All subjects live in
//! one private PID namespace so daemonized reference servers die with the run.

use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime},
};

use crate::{
    child::publish,
    environment,
    json::Value,
    orbit::Client,
    pty,
    result::{Cell, Grid, RunResult, Sample, utc_timestamp},
    sha256::Sha256,
    stats::Summary,
    suite::{self, Settings},
};

mod venus;

type Result<T = ()> = std::result::Result<T, String>;
const COUNT: usize = 50;
const USAGE: &str = "usage: eon-bench memory --out DIR [--subject eon|tmux|zellij]... [--eon PATH] [--renderer lavapipe|/dev/dri/renderD<N>] [--orbit PATH] [--eonbase PATH] [--tmux PATH] [--zellij PATH] [--settle-ms N] [--deadline SECONDS] [shared suite options]";

pub fn help() -> String {
    format!(
        r#"{USAGE}

Linux /proc/smaps_rollup memory; private headless display for Venus. Requires bubblewrap on PATH
(or EON_BENCH_BWRAP). Root is read-only; only --out and private tmp/HOMEs
are writable. PID/network namespaces contain even daemonized reference servers.

  --subject NAME   eon, tmux or zellij; repeat (default all three)
  --eon PATH       Nix out-link candidate, required for Eon native client
  --renderer NAME  lavapipe (default) or /dev/dri/renderD<N>
  --orbit PATH     default $EON_BENCH_ORBIT
  --eonbase PATH   unwrapped supervisor executable; default $EON_BENCH_EONBASE
  --tmux PATH      default $EON_BENCH_TMUX
  --zellij PATH    default $EON_BENCH_ZELLIJ
  --settle-ms N    wait after completion/attach (default 1000)
  --deadline N     seconds per subject launch (default 300)
{}

Two fresh engine launches per subject/round: empty and filled, 80x24.
Eon also launches one fresh native workspace per round for the Venus row. Workload
children are excluded. Startup includes the server plus one empty terminal;
Eon includes supervisor, Orbit host and Bend worker. Terminal cost for ALL
subjects is (total at 50 - total at 1)/49, with the server and engines included.
Eon is one eonbase/first terminal plus 49 independent Orbit engines; it does
not measure 50 workspace records. Filled is exactly 10,000 numbered 72-column
lines; DSR completion and the child grid are verified. Tmux/zellij use 10,000
history rows; Orbit retains its runtime byte budget.

Native client cost is after(server + frontend) minus before(server), with
50 filled terminals. Tmux and zellij use native clients on private PTYs.
Eon uses a fresh actual 50-tab workspace for its native Venus client;
its independent-engine 50-socket codec client remains a separate
server-attribution diagnostic, with every reader and
connection checked around sampling. Supervisor-alone is also diagnostic.
Zellij is background-created, then a temporary PTY client establishes 80x24
(its background default is 50x50); it detaches before output. Setup allocations
may remain. Venus process memory includes mapped graphics allocations, not total GPU memory;
this establishes no leak bound.

result.json uses suite schema 1: before/after PSS, RSS and Pss_Anon byte
counters and a denominator. Seconds are collection time, not throughput.
summary.md reports MiB median and nearest-rank p05-p95; with 10 samples the
range is min-max. RSS sums double-count shared pages; PSS is the primary
comparison. evidence/ retains identities, process sets, raw rollups, configs,
completion/oracle records and load. live-artifacts.json records observed
candidate executable path/backing-inode sharing with live processes before isolation;
other shared mappings remain uncontrolled. Natural load is recorded; no
quiet window is reserved. --quick is smoke only, not a publishable cohort.
A failed/interrupted run retains evidence and removes result.json/summary.md.
"#,
        suite::settings_help()
    )
}

pub struct Options {
    settings: Settings,
    subjects: Vec<String>,
    eon: PathBuf,
    renderer: crate::launcher::Renderer,
    orbit: PathBuf,
    eonbase: PathBuf,
    tmux: PathBuf,
    zellij: PathBuf,
    settle: Duration,
    deadline: Duration,
}

pub fn options(args: &[String]) -> Result<Options> {
    let (settings, own) = suite::parse(
        args,
        USAGE,
        help,
        &[
            "--subject",
            "--eon",
            "--renderer",
            "--orbit",
            "--eonbase",
            "--tmux",
            "--zellij",
            "--settle-ms",
            "--deadline",
        ],
    )?;
    let mut values = std::collections::BTreeMap::new();
    let mut subjects = Vec::new();
    for (flag, value) in own {
        if flag == "--subject" {
            if !["eon", "tmux", "zellij"].contains(&value.as_str()) {
                return Err(format!("unknown subject {value}"));
            }
            if !subjects.contains(&value) {
                subjects.push(value);
            }
        } else if values.insert(flag.clone(), value).is_some() {
            return Err(format!("{flag} is repeated"));
        }
    }
    if subjects.is_empty() {
        subjects = ["eon", "tmux", "zellij"].map(str::to_owned).to_vec();
    }
    let path = |flag: &str, env: &str| {
        values
            .get(flag)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os(env).map(PathBuf::from))
            .unwrap_or_default()
    };
    let duration = |flag: &str, default| {
        values
            .get(flag)
            .map_or(Ok(default), |s| suite::number(flag, s))
    };
    let result = Options {
        eon: values.get("--eon").map(PathBuf::from).unwrap_or_default(),
        renderer: crate::launcher::Renderer(
            values
                .get("--renderer")
                .cloned()
                .unwrap_or_else(|| "lavapipe".into()),
        ),
        orbit: path("--orbit", "EON_BENCH_ORBIT"),
        eonbase: path("--eonbase", "EON_BENCH_EONBASE"),
        tmux: path("--tmux", "EON_BENCH_TMUX"),
        zellij: path("--zellij", "EON_BENCH_ZELLIJ"),
        settle: Duration::from_millis(duration("--settle-ms", 1000)?),
        deadline: Duration::from_secs(duration("--deadline", 300)?.max(1)),
        settings,
        subjects,
    };
    result.renderer.render_node()?;
    if result.subjects.iter().any(|s| s == "eon") {
        crate::launcher::candidate(&result.eon)?;
    }
    for subject in &result.subjects {
        let paths = match subject.as_str() {
            "eon" => vec![(&result.orbit, "--orbit"), (&result.eonbase, "--eonbase")],
            "tmux" => vec![(&result.tmux, "--tmux")],
            _ => vec![(&result.zellij, "--zellij")],
        };
        for (path, flag) in paths {
            if !path.is_absolute() || !path.is_file() {
                return Err(format!("{flag} must name an absolute executable path"));
            }
        }
    }
    Ok(result)
}

/// Namespace containment also covers Zellij's double-forked server.
pub fn run(args: &[String], options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("memory currently requires Linux smaps_rollup; macOS is not measured".into());
    }
    fs::create_dir_all(&options.settings.out).map_err(error)?;
    remove_results(&options.settings.out);
    live_artifacts(options)?;
    let exe = std::env::current_exe().map_err(error)?;
    let out = fs::canonicalize(&options.settings.out).map_err(error)?;
    let mut arguments = args.to_vec();
    if let Some(index) = arguments.iter().position(|arg| arg == "--out") {
        arguments[index + 1] = out.display().to_string();
    }
    let mut command = crate::isolation::command(
        &out,
        &[
            &options.orbit,
            &options.eonbase,
            &options.tmux,
            &options.zellij,
        ],
    );
    if options.subjects.iter().any(|s| s == "eon") {
        crate::isolation::display(&mut command, options.renderer.render_node()?);
        let index = arguments
            .iter()
            .position(|s| s == "--eon")
            .ok_or("--eon is required for Eon")?;
        arguments[index + 1] = crate::launcher::candidate(&options.eon)?
            .display()
            .to_string();
    }
    command
        .arg("--")
        .arg(exe)
        .arg("memory-inner")
        .args(&arguments)
        .stdin(Stdio::null());
    let result = crate::isolation::run(&mut command);
    if result.is_err() {
        remove_results(&options.settings.out);
    }
    suite::interrupt::resend();
    result
}

fn remove_results(out: &Path) {
    for file in ["result.json", "summary.md"] {
        let _ = fs::remove_file(out.join(file));
    }
}

fn payload() -> Vec<u8> {
    (1..=10_000)
        .map(|i| {
            format!(
                "{i:05} {}{}",
                &"abcdefghijklmnopqrstuvwxyz0123456789".repeat(2)[..66],
                if i == 10_000 { "" } else { "\r\n" }
            )
        })
        .collect::<String>()
        .into_bytes()
}

fn hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.hex()
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Observe executable sharing without entering or changing any live runtime.
fn live_artifacts(options: &Options) -> Result {
    use std::os::unix::fs::MetadataExt;
    let mut candidates = Vec::new();
    let mut observed = Vec::new();
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    for (name, path) in [("orbit", &options.orbit), ("eonbase", &options.eonbase)] {
        if path.as_os_str().is_empty() {
            continue;
        }
        let path = fs::canonicalize(path).map_err(error)?;
        let identity = fs::metadata(&path).map_err(error)?;
        let mut matches = Vec::new();
        let mut backing_matches = Vec::new();
        for entry in fs::read_dir("/proc").map_err(error)?.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if entry.metadata().is_ok_and(|m| m.uid() != uid) || Process::at(pid).is_err() {
                continue;
            }
            let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
                continue;
            };
            if exe == path {
                matches.push(u64::from(pid).into());
            }
            if fs::metadata(entry.path().join("exe"))
                .is_ok_and(|m| m.dev() == identity.dev() && m.ino() == identity.ino())
            {
                backing_matches.push(u64::from(pid).into());
            }
            if name == "orbit"
                && exe.file_name().is_some_and(|n| {
                    n.to_string_lossy().contains("eon") || n.to_string_lossy().contains("orbit")
                })
            {
                observed.push(object(vec![
                    ("pid", u64::from(pid).into()),
                    ("exe", exe.display().to_string().into()),
                ]));
            }
        }
        candidates.push(object(vec![
            ("subject", name.into()),
            ("exe", path.display().to_string().into()),
            ("matching_live_path_pids", Value::Array(matches)),
            ("matching_backing_file_pids", Value::Array(backing_matches)),
            ("device", identity.dev().into()),
            ("inode", identity.ino().into()),
        ]));
    }
    fs::write(options.settings.out.join("live-artifacts.json"), object(vec![
        ("sampled_at", utc_timestamp(SystemTime::now()).into()), ("candidates", Value::Array(candidates)),
        ("observed_same_uid_eon_executables", Value::Array(observed)),
        ("scope", "Readable same-UID live executable paths only; other shared mappings remain uncontrolled".into()),
    ]).to_pretty()).map_err(error)
}

/// A PID plus /proc start-time; comm can itself contain spaces or parentheses.
#[derive(Clone)]
struct Process {
    pid: u32,
    start: u64,
    parent: u32,
}
impl Process {
    fn at(pid: u32) -> Result<Self> {
        let stat = read(&PathBuf::from(format!("/proc/{pid}/stat")))?;
        let fields: Vec<_> = stat
            .rsplit_once(") ")
            .ok_or("bad proc stat")?
            .1
            .split_whitespace()
            .collect();
        if fields.first().is_none_or(|s| matches!(*s, "Z" | "X")) {
            return Err(format!("process {pid} exited"));
        }
        Ok(Self {
            pid,
            parent: fields[1].parse().map_err(error)?,
            start: fields
                .get(19)
                .ok_or("no process start time")?
                .parse()
                .map_err(error)?,
        })
    }
    fn alive(&self) -> bool {
        Self::at(self.pid).is_ok_and(|p| p.start == self.start)
    }
    fn signal(&self, signal: i32) {
        #[cfg(target_os = "linux")]
        // SAFETY: pidfd pins one process identity; only signal after checking start time.
        unsafe {
            let fd = libc::syscall(libc::SYS_pidfd_open, self.pid, 0);
            if fd >= 0 {
                if self.alive() {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd,
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    );
                }
                libc::close(fd as i32);
            }
        }
    }
}

const METRICS: [(&str, &str); 3] = [
    ("pss_bytes", "Pss:"),
    ("rss_bytes", "Rss:"),
    ("pss_anon_bytes", "Pss_Anon:"),
];
type Memory = [u64; 3];

fn metric(text: &str, field: &str) -> Result<u64> {
    let mut value = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            let fields: Vec<_> = rest.split_whitespace().collect();
            if fields.len() != 2 || fields[1] != "kB" || value.is_some() {
                return Err(format!("invalid {field} rollup"));
            }
            value = Some(
                fields[0]
                    .parse::<u64>()
                    .map_err(error)?
                    .checked_mul(1024)
                    .ok_or("rollup byte count overflow")?,
            );
        }
    }
    value.ok_or(format!("rollup has no {field}"))
}

struct Session<'a> {
    options: &'a Options,
    subject: &'a str,
    dir: PathBuf,
    evidence: PathBuf,
    exe: PathBuf,
    end: Instant,
    children: Vec<Child>,
    required: Vec<Process>,
    terminals: Vec<PathBuf>,
    server: Option<Process>,
    presentation: Option<Process>,
    client: Option<Process>,
    worker_sets: Vec<Vec<Process>>,
    serial: usize,
    native: bool,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let owned = self.owned();
        for p in &owned {
            p.signal(libc::SIGTERM);
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) && owned.iter().any(Process::alive) {
            std::thread::sleep(Duration::from_millis(10));
        }
        for p in &owned {
            p.signal(libc::SIGKILL);
        }
        for child in &mut self.children {
            let _ = child.wait();
        }
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "log") {
                    let _ = fs::copy(entry.path(), self.evidence.join(entry.file_name()));
                }
            }
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl<'a> Session<'a> {
    fn new(options: &'a Options, subject: &'a str, id: usize) -> Result<Self> {
        let dir = PathBuf::from(format!("/tmp/m{}-{id}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(error)?;
        let evidence = fs::canonicalize(&options.settings.out)
            .map_err(error)?
            .join("evidence")
            .join(format!("{id:03}-{subject}"));
        fs::create_dir_all(&evidence).map_err(error)?;
        for name in ["home", "config", "cache", "state", "data", "run", "sockets"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(dir.join(name))
                .map_err(error)?;
        }
        fs::write(dir.join("tmux.conf"), "set -g exit-empty off\nset -g status off\nset -g history-limit 10000\nset -g default-size 80x24\n").map_err(error)?;
        fs::write(dir.join("zellij.kdl"), "pane_frames false\nscroll_buffer_size 10000\nsession_serialization false\nshow_release_notes false\nshow_startup_tips false\nweb_server false\nsession_name \"bench\"\n").map_err(error)?;
        fs::write(
            dir.join("config/config.toml"),
            "[terminal]\ncolumns = 80\nrows = 24\n",
        )
        .map_err(error)?;
        fs::copy(dir.join("tmux.conf"), evidence.join("tmux.conf")).map_err(error)?;
        fs::copy(dir.join("zellij.kdl"), evidence.join("zellij.kdl")).map_err(error)?;
        fs::copy(
            dir.join("config/config.toml"),
            evidence.join("eon-config.toml"),
        )
        .map_err(error)?;
        Ok(Self {
            options,
            subject,
            dir,
            evidence,
            exe: std::env::current_exe().map_err(error)?,
            end: Instant::now()
                .checked_add(options.deadline)
                .ok_or("deadline out of range")?,
            children: Vec::new(),
            required: Vec::new(),
            terminals: Vec::new(),
            server: None,
            presentation: None,
            client: None,
            worker_sets: Vec::new(),
            serial: 0,
            native: false,
        })
    }
    fn command(&self, path: &Path) -> Command {
        let mut c = Command::new(path);
        // Keep only the tool search path and terminfo. No ambient Eon, display,
        // multiplexer, activation token or session bus can escape into a subject.
        c.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("LANG", "C.UTF-8")
            .env("TERM", "xterm-256color")
            .env("HOME", self.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("XDG_DATA_HOME", self.dir.join("data"))
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env("EON_RUNTIME_DIR", self.dir.join("r"))
            .env("EON_CONFIG_HOME", self.dir.join("config"))
            .env("ZELLIJ_SOCKET_DIR", self.dir.join("sockets"))
            .env("EON_BENCH_MEMORY_ROOT", &self.dir)
            .current_dir(&self.dir)
            .stdin(Stdio::null());
        if let Some(v) = std::env::var_os("TERMINFO_DIRS") {
            c.env("TERMINFO_DIRS", v);
        }
        environment::die_with_parent(&mut c);
        c
    }
    fn owned(&self) -> Vec<Process> {
        let marker = format!("EON_BENCH_MEMORY_ROOT={}", self.dir.display());
        fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                fs::read(format!("/proc/{pid}/environ"))
                    .is_ok_and(|v| v.split(|b| *b == 0).any(|v| v == marker.as_bytes()))
            })
            .filter_map(|pid| Process::at(pid).ok())
            .collect()
    }
    fn tick(&self) -> Result {
        suite::interrupt::check()?;
        if Instant::now() >= self.end {
            return Err(format!(
                "{} launch exceeded {}s",
                self.subject,
                self.options.deadline.as_secs()
            ));
        }
        if self.server.as_ref().is_some_and(|p| !p.alive())
            || self.required.iter().any(|p| !p.alive())
        {
            return Err("server exited early".into());
        }
        Ok(())
    }
    fn wait(&self, what: &str, mut ready: impl FnMut() -> bool) -> Result {
        while !ready() {
            self.tick()
                .map_err(|e| format!("{what}: {e}; evidence {}", self.evidence.display()))?;
            std::thread::sleep(Duration::from_millis(10));
        }
        self.tick()
    }
    fn settle(&self) -> Result {
        let start = Instant::now();
        self.wait("settle", || start.elapsed() >= self.options.settle)
    }
    fn spawn(&mut self, mut c: Command, tty: bool) -> Result<u32> {
        self.serial += 1;
        fs::write(
            self.evidence
                .join(format!("process-{}.command.txt", self.serial)),
            format!("{c:?}"),
        )
        .map_err(error)?;
        let log = File::create(self.evidence.join(format!("process-{}.log", self.serial)))
            .map_err(error)?;
        if tty {
            let (mut master, slave) = pty::open(24, 80).map_err(error)?;
            c.stdin(slave.try_clone().map_err(error)?)
                .stdout(slave.try_clone().map_err(error)?)
                .stderr(slave);
            // SAFETY: these async-signal-safe calls run only before exec.
            unsafe {
                c.pre_exec(|| {
                    if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = c.spawn().map_err(error)?;
            let pid = child.id();
            self.children.push(child);
            // The PTY is drained without interpreting output; native clients
            // render to it but no desktop or terminal emulator is involved.
            std::thread::spawn(move || {
                let mut log = log;
                let mut bytes = [0; 65536];
                loop {
                    match master.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = log.write_all(&bytes[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(pid)
        } else {
            c.stdout(log.try_clone().map_err(error)?).stderr(log);
            let child = c.spawn().map_err(error)?;
            let pid = child.id();
            self.children.push(child);
            Ok(pid)
        }
    }
    fn call(&mut self, c: Command) -> Result {
        let pid = self.spawn(c, false)?;
        let index = self.children.iter().position(|c| c.id() == pid).unwrap();
        loop {
            if let Some(status) = self.children[index].try_wait().map_err(error)? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "command exited {status}; see {}",
                        self.evidence.display()
                    ))
                };
            }
            self.tick()?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn output(&mut self, command: Command) -> Result<String> {
        self.call(command)?;
        read(&self.evidence.join(format!("process-{}.log", self.serial)))
    }
    fn tmux(&self) -> Command {
        let mut c = self.command(&self.options.tmux);
        c.arg("-S").arg(self.dir.join("tmux.sock"));
        c
    }
    fn zellij(&self) -> Command {
        let mut c = self.command(&self.options.zellij);
        c.arg("--config").arg(self.dir.join("zellij.kdl"));
        c
    }
    fn child_args(&self, dir: &Path) -> Vec<String> {
        child_args(&self.exe, dir, &self.dir, self.options.deadline)
    }
    fn layout(&self, dir: &Path) -> Result<PathBuf> {
        let args = self.child_args(dir);
        // JSON string escaping is also valid for these KDL basic strings.
        let quote = |s: &str| Value::from(s).to_pretty().trim().to_owned();
        let text = format!(
            "layout {{\n pane command={} {{\n args {};\n }}\n}}\n",
            quote(&args[0]),
            args[1..]
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let path = dir.join("layout.kdl");
        fs::write(&path, text).map_err(error)?;
        fs::copy(
            &path,
            self.evidence
                .join(format!("terminal-{}.layout.kdl", self.terminals.len())),
        )
        .map_err(error)?;
        Ok(path)
    }
    fn terminal(&mut self) -> Result {
        let index = self.terminals.len();
        let dir = self.dir.join(index.to_string());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(error)?;
        let mut start_client = None;
        match self.subject {
            "eon" => {
                if index == 0 {
                    self.start_supervisor(&dir)?;
                } else {
                    let socket = dir.join("s");
                    let mut c = self.command(&self.options.orbit);
                    c.arg("serve")
                        .arg(&socket)
                        .arg("--")
                        .args(self.child_args(&dir));
                    let host = self.spawn(c, false)?;
                    self.required.push(Process::at(host)?);
                    self.wait("Orbit socket", || socket.exists())?;
                    let mut client = Client::new(UnixStream::connect(socket).map_err(error)?)?;
                    client.attach(24, 80)?;
                    let reader = client.discard_in_background()?;
                    self.wait("child grid", || dir.join("ready").exists())?;
                    if !reader.wait_quiet() {
                        return Err("Orbit did not settle".into());
                    }
                    reader.close()?;
                    self.wait("Orbit detach", || {
                        crate::orbit::procfs::connections(&dir.join("s")) == Some(0)
                    })?;
                }
            }
            "tmux" => {
                let mut c = self.tmux();
                if index == 0 {
                    c.args(["new-session", "-d", "-s", "bench", "-x", "80", "-y", "24"]);
                } else {
                    c.args(["new-window", "-d", "-t", "bench"]);
                }
                c.arg("--").args(self.child_args(&dir));
                self.call(c)?;
            }
            _ => {
                let layout = self.layout(&dir)?;
                let mut c = self.zellij();
                if index == 0 {
                    c.arg("--layout-string").arg(read(&layout)?).args([
                        "attach",
                        "--create-background",
                        "bench",
                    ]);
                    self.call(c)?;
                    // Background creation returns before the session handles its first message.
                    self.wait("Zellij background workload", || {
                        self.owned().iter().any(|p| {
                            fs::read(format!("/proc/{}/cmdline", p.pid)).is_ok_and(|args| {
                                args.split(|b| *b == 0).nth(1) == Some(b"child".as_slice())
                            })
                        })
                    })?;
                    let mut c = self.zellij();
                    c.args(["attach", "bench"]);
                    start_client = Some(self.spawn(c, true)?);
                    self.required.push(Process::at(start_client.unwrap())?);
                } else {
                    c.args(["--session", "bench", "action", "new-tab", "--layout"])
                        .arg(layout);
                    self.call(c)?;
                }
            }
        }
        self.wait("80x24 child ready", || dir.join("ready").exists())?;
        let workload = Process::at(read(&dir.join("ready"))?.parse().map_err(error)?)?;
        self.required.push(workload.clone());
        if self.subject == "eon" {
            let host = Process::at(workload.parent)?;
            let children = read(&PathBuf::from(format!(
                "/proc/{}/task/{}/children",
                host.pid, host.pid
            )))?
            .split_whitespace()
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(error)?;
            let worker = choose_worker(host.pid, workload.pid, &children)?;
            let pair = vec![host, Process::at(worker)?];
            self.required.extend(pair.clone());
            self.worker_sets.push(pair);
        }
        if let Some(pid) = start_client {
            self.stop_client(pid)?;
        }
        publish(&dir.join("go"), "")?;
        self.wait("completion reply", || dir.join("done").exists())?;
        if read(&dir.join("done"))? != "0" {
            return Err("completion writer failed".into());
        }
        let report = Value::parse(&read(&dir.join("report.json"))?)?;
        let bytes = fs::metadata(self.dir.join("payload.vt"))
            .map_err(error)?
            .len();
        if report.get("bytes").and_then(Value::as_u64) != Some(bytes) {
            return Err("writer byte mismatch".into());
        }
        fs::copy(
            dir.join("report.json"),
            self.evidence
                .join(format!("terminal-{index}-completion.json")),
        )
        .map_err(error)?;
        self.terminals.push(dir);
        Ok(())
    }
    fn stop_client(&mut self, pid: u32) -> Result {
        self.required.retain(|p| p.pid != pid);
        Process::at(pid)?.signal(libc::SIGTERM);
        let child = self
            .children
            .iter_mut()
            .find(|c| c.id() == pid)
            .ok_or("missing client")?;
        let start = Instant::now();
        while child.try_wait().map_err(error)?.is_none() {
            if start.elapsed() > Duration::from_secs(2) {
                child.kill().map_err(error)?;
                child.wait().map_err(error)?;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    fn start_server(&mut self) -> Result {
        if self.subject == "tmux" {
            let mut c = self.tmux();
            c.arg("-D").arg("-f").arg(self.dir.join("tmux.conf"));
            let pid = self.spawn(c, false)?;
            self.server = Some(Process::at(pid)?);
            self.wait("tmux socket", || self.dir.join("tmux.sock").exists())?;
        }
        self.terminal()?;
        if self.subject == "zellij" {
            let mut found = None;
            self.wait("zellij daemon", || {
                found = self.owned().into_iter().find(|p| {
                    fs::read(format!("/proc/{}/cmdline", p.pid))
                        .is_ok_and(|v| v.split(|b| *b == 0).any(|s| s == b"--server"))
                });
                found.is_some()
            })?;
            self.server = found;
        }
        Ok(())
    }
    fn measure(&mut self, label: &str, measured: &[Process]) -> Result<Memory> {
        validate_set(measured)?;
        self.tick()?;
        let started = utc_timestamp(SystemTime::now());
        let load = environment::load()?;
        let mut total = [0; 3];
        let mut rows = Vec::new();
        let owned = self.owned();
        for process in measured {
            if !owned
                .iter()
                .any(|p| p.pid == process.pid && p.start == process.start)
            {
                return Err("measurement process is not owned/live".into());
            }
        }
        for process in &owned {
            let rollup = read(&PathBuf::from(format!(
                "/proc/{}/smaps_rollup",
                process.pid
            )))?;
            if !process.alive() {
                return Err("process exited/reused during rollup".into());
            }
            let mut values = [0; 3];
            for (index, (_, field)) in METRICS.iter().enumerate() {
                values[index] = metric(&rollup, field)?;
            }
            let included = measured
                .iter()
                .any(|p| p.pid == process.pid && p.start == process.start);
            if included {
                for (sum, value) in total.iter_mut().zip(values) {
                    *sum += value;
                }
            }
            let role = if self.worker_sets.iter().any(|set| set[0].pid == process.pid) {
                "orbit-host"
            } else if self.worker_sets.iter().any(|set| set[1].pid == process.pid) {
                "bend-worker"
            } else if self.server.as_ref().is_some_and(|p| p.pid == process.pid) {
                "supervisor-server"
            } else if self.client.as_ref().is_some_and(|p| p.pid == process.pid) {
                if self.subject == "eon" && !self.native {
                    "diagnostic-codec-client"
                } else {
                    "native-client"
                }
            } else if included {
                "reference-server"
            } else {
                "excluded-workload-or-support"
            };
            rows.push(object(vec![
                ("role", role.into()),
                ("pid", u64::from(process.pid).into()),
                ("start_ticks", process.start.into()),
                ("included", included.into()),
                ("pss_bytes", values[0].into()),
                ("rss_bytes", values[1].into()),
                ("pss_anon_bytes", values[2].into()),
                ("parent_pid", u64::from(process.parent).into()),
                ("rollup", rollup.into()),
                (
                    "exe",
                    fs::read_link(format!("/proc/{}/exe", process.pid))
                        .map_err(error)?
                        .display()
                        .to_string()
                        .into(),
                ),
                (
                    "cmdline",
                    String::from_utf8_lossy(
                        &fs::read(format!("/proc/{}/cmdline", process.pid)).map_err(error)?,
                    )
                    .replace('\0', " ")
                    .into(),
                ),
            ]));
        }
        let record = object(vec![
            ("measure", label.into()),
            ("started_at", started.into()),
            ("finished_at", utc_timestamp(SystemTime::now()).into()),
            ("load_start", load.into()),
            ("load_end", environment::load()?.into()),
            ("pss_bytes", total[0].into()),
            ("rss_bytes", total[1].into()),
            ("pss_anon_bytes", total[2].into()),
            ("processes", Value::Array(rows)),
        ]);
        fs::write(
            self.evidence.join(format!("{label}.json")),
            record.to_pretty(),
        )
        .map_err(error)?;
        Ok(total)
    }
    fn server_set(&self) -> Vec<Process> {
        self.server.clone().into_iter().collect()
    }
    fn terminal_set(&self) -> Result<Vec<Process>> {
        let server = self.server.as_ref().ok_or("missing measurement server")?;
        let set = engine_set(server, &self.worker_sets);
        validate_set(&set)?;
        Ok(set)
    }
    fn attach(&mut self) -> Result<Process> {
        let mut c = match self.subject {
            "eon" => {
                let client = self
                    .presentation
                    .clone()
                    .ok_or("missing diagnostic presentation")?;
                self.client = Some(client.clone());
                publish(&self.dir.join("client-go"), "")?;
                self.wait("50 attached sockets", || {
                    self.dir.join("client-ready").exists()
                })?;
                fs::copy(
                    self.dir.join("client-ready"),
                    self.evidence.join("attached-orbit-oracle.json"),
                )
                .map_err(error)?;
                self.settle()?;
                self.check_codec("attached")?;
                return Ok(client);
            }
            "tmux" => {
                let mut c = self.tmux();
                c.args(["attach-session", "-t", "bench"]);
                c
            }
            _ => {
                let mut c = self.zellij();
                c.args(["attach", "bench"]);
                c
            }
        };
        c.env("EON_BENCH_MEMORY_DIR", &self.dir);
        let pid = self.spawn(c, true)?;
        self.client = Some(Process::at(pid)?);
        self.required.push(Process::at(pid)?);
        self.settle()?;
        if self.subject != "eon" {
            loop {
                let mut c = if self.subject == "tmux" {
                    self.tmux()
                } else {
                    self.zellij()
                };
                if self.subject == "tmux" {
                    c.args(["list-clients", "-F", "#{client_pid}"]);
                } else {
                    c.args(["--session", "bench", "action", "list-clients"]);
                }
                let clients = self.output(c)?;
                let count = clients
                    .lines()
                    .filter(|line| {
                        line.split_whitespace()
                            .next()
                            .is_some_and(|s| s.parse::<u32>().is_ok())
                    })
                    .count();
                if count == 1 {
                    fs::write(self.evidence.join("attached-clients.txt"), clients)
                        .map_err(error)?;
                    break;
                }
                self.tick()?;
                std::thread::sleep(Duration::from_millis(50));
            }
            let mut c = if self.subject == "tmux" {
                self.tmux()
            } else {
                self.zellij()
            };
            if self.subject == "tmux" {
                c.args(["capture-pane", "-p", "-t", "bench"]);
            } else {
                c.args(["--session", "bench", "action", "dump-screen"]);
            }
            let screen = self.output(c)?;
            verify_screen(&screen)?;
            fs::write(self.evidence.join("attached-screen.txt"), screen).map_err(error)?;
            self.settle()?;
        }
        Process::at(pid)
    }
}

fn sample_metric(sample: &Sample, metric: &str) -> Result<f64> {
    let counter = |name| {
        sample
            .counters
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v as f64)
            .ok_or(format!("missing counter {name}"))
    };
    let denominator = counter("denominator")?;
    if denominator == 0.0 {
        return Err("zero memory denominator".into());
    }
    Ok(
        (counter(&format!("{metric}_after"))? - counter(&format!("{metric}_before"))?)
            / denominator
            / 1048576.0,
    )
}

fn verify_screen(screen: &str) -> Result {
    let expected = String::from_utf8(payload()).map_err(error)?;
    let actual: Vec<_> = screen
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let expected: Vec<_> = expected.split("\r\n").skip(10000 - 24).collect();
    if actual.len() != expected.len() || actual.iter().zip(expected).any(|(a, e)| a.trim_end() != e)
    {
        return Err("filled viewport differs from the last 24 numbered lines".into());
    }
    Ok(())
}

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

/// A separate, explicitly diagnostic process. It holds one canonical-codec
/// connection and discard thread per terminal; no terminal state or rendering.
fn client(dir: &Path) -> Result {
    let mut readers = Vec::new();
    let mut oracles = Vec::new();
    for i in 0..COUNT {
        let mut client =
            Client::new(UnixStream::connect(dir.join(i.to_string()).join("s")).map_err(error)?)?;
        let frame = client.attach(24, 80)?;
        verify_screen(
            &frame
                .rows
                .iter()
                .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        oracles.push(object(vec![
            ("terminal", (i as u64).into()),
            ("rows", u64::from(frame.dimensions.rows).into()),
            ("cols", u64::from(frame.dimensions.cols).into()),
            ("history_rows", frame.scroll_position.history_rows.into()),
            ("viewport_matches", true.into()),
        ]));
        readers.push(client.discard_in_background()?);
    }
    if readers.iter().any(|r| !r.wait_quiet()) {
        return Err("client frames did not settle".into());
    }
    publish(
        &dir.join("client-ready"),
        &Value::Array(oracles).to_pretty(),
    )?;
    loop {
        for reader in &readers {
            reader.ensure_connected()?;
        }
        if let Ok(token) = read(&dir.join("client-check")) {
            for reader in &readers {
                reader.ensure_connected()?;
            }
            publish(&dir.join("client-checked"), &token)?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Stand-in for Venus only for the terminal-mode supervisor start row.
/// The parent measures the supervisor, never this diagnostic process.
pub fn presentation() -> Result {
    if std::env::var("EON_VENUS_PRESENTATION_CONTROL").as_deref() == Ok("stdin-ready-focus-v1") {
        std::io::stdout().write_all(b"ready-v1").map_err(error)?;
        std::io::stdout().flush().map_err(error)?;
    }
    let socket = std::env::args().next_back().ok_or("missing Orbit socket")?;
    let start = Instant::now();
    while !Path::new(&socket).exists() {
        if start.elapsed() > Duration::from_secs(20) {
            return Err("supervisor Orbit socket missing".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let reader = loop {
        let mut client = Client::new(UnixStream::connect(&socket).map_err(error)?)?;
        match client.attach(24, 80) {
            Ok(_) => break client.discard_in_background()?,
            Err(e) if e == "Orbit answered Busy" && start.elapsed() < Duration::from_secs(20) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e),
        }
    };
    let root = std::env::var_os("EON_BENCH_MEMORY_ROOT").ok_or("missing private root")?;
    let root = PathBuf::from(root);
    publish(
        &root.join("supervisor-client-ready"),
        &format!("{}\n{socket}", std::process::id()),
    )?;
    while !root.join("supervisor-detach").exists() {
        reader.ensure_connected()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    reader.close()?;
    while crate::orbit::procfs::connections(Path::new(&socket)) != Some(0) {
        std::thread::sleep(Duration::from_millis(5));
    }
    publish(&root.join("supervisor-detached"), "")?;
    while !root.join("client-go").exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
    client(&root)
}

pub fn run_inner(options: &Options) -> Result {
    let settings = &options.settings;
    let out = fs::canonicalize(&settings.out).map_err(error)?;
    remove_results(&out);
    if let Some(cpus) = &settings.cpus {
        environment::pin(cpus)?;
    }
    suite::interrupt::install()?;
    let started_at = utc_timestamp(SystemTime::now());
    let load_start = environment::load()?;
    let build = environment::build()?;
    let mut tools = vec![("method".into(), "Linux smaps_rollup PSS/RSS/Pss_Anon bytes; child excluded; common 1-to-50 slope; native client=server+frontend delta; Venus native workspace delta; codec server attribution separate".into()),
        ("grid".into(), "80x24".into()), ("history".into(), "Orbit runtime byte budget; tmux/zellij 10000 rows; retention not independently counted".into()),
        ("settle_ms".into(), options.settle.as_millis().to_string()), ("terminals".into(), COUNT.to_string())];
    for (name, path) in [
        ("eon", &options.eon),
        ("orbit", &options.orbit),
        ("eonbase", &options.eonbase),
        ("tmux", &options.tmux),
        ("zellij", &options.zellij),
    ] {
        if !path.as_os_str().is_empty() {
            tools.push((
                name.into(),
                format!(
                    "{} sha256:{}",
                    path.display(),
                    hash(&fs::read(path).map_err(error)?)
                ),
            ));
        }
    }
    tools.push((
        "venus-requested-renderer".into(),
        options.renderer.0.clone(),
    ));
    let filled = payload();
    fs::write(out.join("filled.vt"), &filled).map_err(error)?;
    let mut cells: Vec<Cell> = options
        .subjects
        .iter()
        .flat_map(|subject| {
            let names = if subject == "eon" {
                vec![
                    "server-start",
                    "empty-terminal",
                    "filled-terminal",
                    "client-50-filled",
                    "diagnostic-supervisor-alone",
                    "diagnostic-codec-server-attach",
                ]
            } else {
                vec![
                    "server-start",
                    "empty-terminal",
                    "filled-terminal",
                    "client-50-filled",
                ]
            };
            names
                .into_iter()
                .map(|workload| Cell {
                    workload: workload.into(),
                    corpus_sha256: hash(
                        if [
                            "server-start",
                            "empty-terminal",
                            "diagnostic-supervisor-alone",
                        ]
                        .contains(&workload)
                        {
                            b""
                        } else {
                            &filled
                        },
                    ),
                    input_bytes: 0,
                    layer: "process-pss".into(),
                    subject: subject.clone(),
                    subject_version: tools
                        .iter()
                        .find(|(n, _)| n == if subject == "eon" { "orbit" } else { subject })
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    samples: Vec::new(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut id = 0;
    for round in 0..settings.warmups + settings.samples {
        let (kind, number, of) = suite::round_label(settings, round);
        for subject in &options.subjects {
            for loaded in [false, true] {
                suite::interrupt::check()?;
                let load = suite::wait_for_load("memory", settings)?;
                id += 1;
                eprintln!(
                    "memory: {kind} {number}/{of} {subject} {}",
                    if loaded { "filled" } else { "empty" }
                );
                let start = Instant::now();
                let mut s = Session::new(options, subject, id)?;
                fs::write(
                    s.dir.join("payload.vt"),
                    if loaded { &filled[..] } else { b"" },
                )
                .map_err(error)?;
                if round == 0 && !loaded {
                    let path = if subject == "eon" {
                        &options.eonbase
                    } else if subject == "tmux" {
                        &options.tmux
                    } else {
                        &options.zellij
                    };
                    let mut c = s.command(path);
                    c.arg(if subject == "tmux" { "-V" } else { "--version" });
                    let version = s.output(c)?;
                    tools.push((format!("{subject}-version"), version.trim().to_owned()));
                    for cell in cells.iter_mut().filter(|c| c.subject == *subject) {
                        cell.subject_version
                            .push_str(&format!(" {}", version.trim()));
                    }
                }
                s.start_server()?;
                s.settle()?;
                let one = s.measure("one-terminal", &s.terminal_set()?)?;
                let mut measures = Vec::new();
                if !loaded {
                    let server = s.measure("server-start", &s.terminal_set()?)?;
                    measures.push(("server-start", [0; 3], server, 1));
                    if subject == "eon" {
                        let alone = s.measure("diagnostic-supervisor-alone", &s.server_set())?;
                        measures.push(("diagnostic-supervisor-alone", [0; 3], alone, 1));
                    }
                }
                for _ in 1..COUNT {
                    s.terminal()?;
                }
                s.settle()?;
                let all = s.measure("all-terminals", &s.terminal_set()?)?;
                measures.push((
                    if loaded {
                        "filled-terminal"
                    } else {
                        "empty-terminal"
                    },
                    one,
                    all,
                    (COUNT - 1) as u64,
                ));
                if loaded {
                    let client = s.attach()?;
                    let after = if subject == "eon" {
                        s.check_codec("before-snapshot")?;
                        let after = s.measure("attached-servers", &s.terminal_set()?)?;
                        s.check_codec("after-snapshot")?;
                        after
                    } else {
                        let mut set = s.terminal_set()?;
                        set.push(client.clone());
                        s.measure("attached-total", &set)?
                    };
                    s.measure("client-process", &[client])?;
                    measures.push((
                        if subject == "eon" {
                            "diagnostic-codec-server-attach"
                        } else {
                            "client-50-filled"
                        },
                        all,
                        after,
                        1,
                    ));
                }
                let seconds = start.elapsed().as_secs_f64();
                let load_end = (environment::load()? * 1000.0).round() as u64;
                if kind == "sample" {
                    for (measure, before, after, denominator) in measures {
                        let cell = cells
                            .iter_mut()
                            .find(|c| c.subject == *subject && c.workload == measure)
                            .unwrap();
                        cell.samples.push(Sample {
                            seconds,
                            load,
                            counters: {
                                let mut counters = vec![
                                    ("denominator".into(), denominator),
                                    ("load_end_milli".into(), load_end),
                                    ("launch".into(), id as u64),
                                ];
                                for (index, (name, _)) in METRICS.iter().enumerate() {
                                    counters.push((format!("{name}_before"), before[index]));
                                    counters.push((format!("{name}_after"), after[index]));
                                }
                                counters
                            },
                        });
                    }
                }
            }
            if subject == "eon" {
                let load = suite::wait_for_load("memory Venus", settings)?;
                id += 1;
                eprintln!(
                    "memory: {kind} {number}/{of} eon Venus {}",
                    options.renderer.0
                );
                let start = Instant::now();
                let mut s = Session::new(options, subject, id)?;
                s.native = true;
                fs::write(s.dir.join("payload.vt"), &filled).map_err(error)?;
                let (before, after, gpu) = s.venus()?;
                tools.push((format!("venus-{id}-gpu"), gpu));
                if kind == "sample" {
                    let mut counters = vec![
                        ("denominator".into(), 1),
                        ("launch".into(), id as u64),
                        (
                            "load_end_milli".into(),
                            (environment::load()? * 1000.0).round() as u64,
                        ),
                    ];
                    for (index, (name, _)) in METRICS.iter().enumerate() {
                        counters.push((format!("{name}_before"), before[index]));
                        counters.push((format!("{name}_after"), after[index]));
                    }
                    cells
                        .iter_mut()
                        .find(|c| c.subject == *subject && c.workload == "client-50-filled")
                        .unwrap()
                        .samples
                        .push(Sample {
                            seconds: start.elapsed().as_secs_f64(),
                            load,
                            counters,
                        });
                }
            }
        }
    }
    let result = RunResult {
        suite: "memory".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: settings.quick,
        corpus_version: 1,
        warmups: settings.warmups,
        samples: settings.samples,
        started_at,
        finished_at: utc_timestamp(SystemTime::now()),
        environment: suite::environment(
            settings,
            environment::kernel(),
            load_start,
            Grid { rows: 24, cols: 80 },
            tools,
            build,
        )?,
        cells,
    };
    suite::interrupt::check()?;
    let result = suite::write(&out, &result)?;
    let mut table = String::from(
        "MiB, median (nearest-rank p05–p95); workload children excluded. Terminal cost is the common (50 minus 1)/49 slope. Native client cost includes server + frontend.\n\n",
    );
    for diagnostic in [false, true] {
        if diagnostic {
            table.push_str("\nSeparate diagnostics (not native-client comparisons):\n\n");
        }
        table.push_str(
            "| Subject | Measure | n | PSS | RSS | Pss_Anon |\n|---|---|---:|---:|---:|---:|\n",
        );
        for cell in result
            .cells
            .iter()
            .filter(|c| c.workload.starts_with("diagnostic-") == diagnostic)
        {
            table.push_str(&format!(
                "| {} | {} | {} |",
                cell.subject,
                cell.workload,
                cell.samples.len()
            ));
            for (name, _) in METRICS {
                let mut values = cell
                    .samples
                    .iter()
                    .map(|s| sample_metric(s, name))
                    .collect::<Result<Vec<_>>>()?;
                let summary = Summary::of(&values).ok_or("missing samples")?;
                values.sort_by(f64::total_cmp);
                let p05 = values[(values.len() * 5).div_ceil(100) - 1];
                table.push_str(&format!(
                    " {:.3} ({:.3}–{:.3}) |",
                    summary.median, p05, summary.p95
                ));
            }
            table.push('\n');
        }
    }
    if result.cells.iter().any(|cell| {
        cell.subject == "eon" && cell.workload == "client-50-filled" && !cell.samples.is_empty()
    }) {
        table.push_str(&format!("\nVenus client: actual 50-tab Eon workspace, one visible tab, {}. GPU and ICD per launch in evidence; GPU allocations outside smaps are not counted.\n", options.renderer.0));
    }
    table.push_str("\nEon engine rows: one eonbase supervisor/first terminal plus 49 independent Orbit hosts and workers: a terminal-engine cohort, not 50 Eon workspace records. Its codec diagnostic attaches all 50; native references display one tab. Zellij background creation starts at 50x50, so a temporary native client establishes 80x24 and detaches before output; setup allocations may remain. live-artifacts.json records whether candidate executable paths/backing inodes were live-mapped at run start; other shared mappings remain uncontrolled.\n");
    fs::write(out.join("summary.md"), &table).map_err(error)?;
    println!("{table}\nresult: {}", out.join("result.json").display());
    Ok(())
}

fn choose_worker(host: u32, workload: u32, children: &[u32]) -> Result<u32> {
    if host == workload || !children.contains(&workload) {
        return Err("workload is not the host's child".into());
    }
    let workers: Vec<_> = children
        .iter()
        .copied()
        .filter(|pid| *pid != workload)
        .collect();
    match workers.as_slice() {
        [worker] => Ok(*worker),
        _ => Err("expected exactly one Bend worker beside workload".into()),
    }
}

fn engine_set(server: &Process, engines: &[Vec<Process>]) -> Vec<Process> {
    std::iter::once(server.clone())
        .chain(engines.iter().flatten().cloned())
        .collect()
}

fn validate_set(set: &[Process]) -> Result {
    if set.is_empty() {
        return Err("empty measurement process set".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    if set.iter().any(|p| !seen.insert(p.pid)) {
        return Err("duplicate measurement PID".into());
    }
    Ok(())
}

impl Session<'_> {
    fn start_supervisor(&mut self, dir: &Path) -> Result {
        let link = self.dir.join("memory-presentation");
        std::os::unix::fs::symlink(&self.exe, &link).map_err(error)?;
        let base = self.dir.join("eonbase");
        std::os::unix::fs::symlink(&self.options.eonbase, &base).map_err(error)?;
        let mut c = self.command(&base);
        c.env("EON_ORBIT", &self.options.orbit)
            .env("EON_VENUS", link)
            .arg("--")
            .args(self.child_args(dir));
        let pid = self.spawn(c, false)?;
        self.server = Some(Process::at(pid)?);
        self.wait("supervisor diagnostic client", || {
            self.dir.join("supervisor-client-ready").exists()
        })?;
        let info = read(&self.dir.join("supervisor-client-ready"))?;
        let mut lines = info.lines();
        let presentation = Process::at(
            lines
                .next()
                .ok_or("no presentation PID")?
                .parse()
                .map_err(error)?,
        )?;
        if presentation.parent != pid {
            return Err("presentation has wrong parent".into());
        }
        let socket = PathBuf::from(lines.next().ok_or("no managed Orbit socket")?);
        std::os::unix::fs::symlink(socket, dir.join("s")).map_err(error)?;
        self.required.push(presentation.clone());
        self.presentation = Some(presentation);
        publish(&self.dir.join("supervisor-detach"), "")?;
        self.wait("supervisor diagnostic detach", || {
            self.dir.join("supervisor-detached").exists()
        })
    }
    fn check_codec(&mut self, token: &str) -> Result {
        publish(&self.dir.join("client-check"), token)?;
        self.wait("diagnostic reader health", || {
            read(&self.dir.join("client-checked")).is_ok_and(|s| s == token)
        })?;
        for dir in &self.terminals {
            let socket = fs::canonicalize(dir.join("s")).map_err(error)?;
            if crate::orbit::procfs::connections(&socket) != Some(1) {
                return Err("diagnostic must hold exactly 50 live connections".into());
            }
        }
        if self.terminals.len() != COUNT {
            return Err("diagnostic terminal count changed".into());
        }
        let client = self.client.as_ref().ok_or("no diagnostic client")?;
        fs::write(
            self.evidence.join(format!("codec-health-{token}.json")),
            object(vec![
                ("token", token.into()),
                ("connections", (COUNT as u64).into()),
                ("client_pid", u64::from(client.pid).into()),
                ("client_start_ticks", client.start.into()),
                ("readers_healthy", true.into()),
            ])
            .to_pretty(),
        )
        .map_err(error)?;
        Ok(())
    }
}

fn child_args(exe: &Path, dir: &Path, root: &Path, deadline: Duration) -> Vec<String> {
    vec![
        exe.display().to_string(),
        "child".into(),
        dir.display().to_string(),
        "--rows".into(),
        "24".into(),
        "--cols".into(),
        "80".into(),
        "--release-wait".into(),
        deadline.as_secs().saturating_add(10).to_string(),
        "--".into(),
        exe.display().to_string(),
        "write".into(),
        root.join("payload.vt").display().to_string(),
        "--report".into(),
        dir.join("report.json").display().to_string(),
    ]
}

/// Internal configured shell: one existing completion child per workspace terminal.
pub fn workspace_workload() -> Result {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("missing workload root")?);
    let deadline = Duration::from_secs(
        std::env::args()
            .nth(2)
            .ok_or("missing deadline")?
            .parse()
            .map_err(error)?,
    );
    let terminal = std::env::var("EON_TERMINAL").map_err(error)?;
    let dir = root.join(hash(terminal.as_bytes()));
    fs::create_dir(&dir).map_err(error)?;
    fs::write(dir.join("identity"), terminal).map_err(error)?;
    let args = child_args(
        &std::env::current_exe().map_err(error)?,
        &dir,
        &root,
        deadline,
    );
    let code = crate::child::run(&crate::child::options(&args[2..])?)?;
    if code == 0 {
        Ok(())
    } else {
        Err(format!("workload exited {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollup_requires_exact_pss_field_units_and_nonoverflowing_bytes() {
        assert_eq!(
            metric("Rss: 100 kB\nPss: 42 kB\nPss_Anon: 9 kB\n", "Pss:").unwrap(),
            43008
        );
        for bad in [
            "Pss_Anon: 42 kB",
            "Pss: 42 MB",
            "Pss: nope kB",
            "Pss: 1 kB\nPss: 2 kB",
            "Pss: 18446744073709551615 kB",
        ] {
            assert!(metric(bad, "Pss:").is_err(), "{bad}");
        }
    }
    #[test]
    fn filled_payload_has_ten_thousand_numbered_72_column_lines() {
        let payload = String::from_utf8(payload()).unwrap();
        let lines: Vec<_> = payload.split("\r\n").collect();
        assert_eq!(lines.len(), 10000);
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(line.len(), 72);
            assert!(line.starts_with(&format!("{:05} ", i + 1)));
        }
        assert!(!payload.ends_with('\n'));
    }
    #[test]
    fn signed_memory_deltas_and_viewport_oracle_keep_failures_visible() {
        let mut sample = Sample {
            seconds: 1.0,
            load: 2.0,
            counters: vec![
                ("pss_bytes_before".into(), 4096),
                ("pss_bytes_after".into(), 2048),
                ("denominator".into(), 2),
            ],
        };
        assert_eq!(sample_metric(&sample, "pss_bytes").unwrap(), -1.0 / 1024.0);
        sample.counters[2].1 = 0;
        assert!(sample_metric(&sample, "pss_bytes").is_err());
        let payload = String::from_utf8(payload()).unwrap();
        let viewport = payload
            .split("\r\n")
            .skip(9976)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(verify_screen(&viewport).is_ok());
        assert!(verify_screen(&viewport.replace("10000", "99999")).is_err());
        assert!(verify_screen("").is_err());
    }
    #[test]
    fn process_sets_include_server_and_engines_but_exclude_workloads() {
        let process = |pid| Process {
            pid,
            start: 1,
            parent: 0,
        };
        let server = process(1);
        for n in [1, COUNT] {
            let mut pairs = Vec::new();
            let mut workloads = Vec::new();
            for i in 0..n {
                let host = 10 + i as u32 * 3;
                let workload = host + 1;
                let worker = choose_worker(host, workload, &[workload, host + 2]).unwrap();
                pairs.push(vec![process(host), process(worker)]);
                workloads.push(workload);
            }
            let set = engine_set(&server, &pairs);
            validate_set(&set).unwrap();
            assert_eq!(set.len(), 1 + 2 * n);
            assert_eq!(set[0].pid, server.pid);
            assert!(set.iter().all(|p| !workloads.contains(&p.pid)));
        }
        assert!(choose_worker(10, 11, &[12]).is_err());
        assert!(choose_worker(10, 11, &[11, 12, 13]).is_err());
        assert!(validate_set(&[]).is_err());
        assert!(validate_set(&[process(1), process(1)]).is_err());
        let reference = engine_set(&server, &[]);
        assert_eq!(reference.len(), 1);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn pid_identity_does_not_accept_a_different_start_time() {
        let mut p = Process::at(std::process::id()).unwrap();
        assert!(p.alive());
        p.start += 1;
        assert!(!p.alive());
    }
}
