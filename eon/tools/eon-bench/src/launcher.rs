//! Shared private display, environment and owned-process lifecycle for display suites.
use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::unix::{
        fs::{DirBuilderExt, FileExt, FileTypeExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

use rustix::process::Signal;

use crate::{
    bundle::{HostSet, Jail},
    json::Value,
};

type Result<T = ()> = std::result::Result<T, String>;
fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;

fn capture_output(command: &mut Command, runtime: &Path) -> std::io::Result<Output> {
    let spool = |name| {
        let path = runtime.join(name);
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        fs::remove_file(path)?;
        Ok::<_, std::io::Error>(file)
    };
    let stdout = spool("command.stdout")?;
    let stderr = spool("command.stderr")?;
    // A detached helper can keep its descriptors open after the command exits.
    // Snapshot regular files instead of waiting for EOF on inherited pipes.
    let status = command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .status()?;
    let snapshot = |file: File| {
        let len = file.metadata()?.len();
        if len > OUTPUT_LIMIT {
            return Err(std::io::Error::other("subprocess output exceeds 4 MiB"));
        }
        let mut bytes = vec![0; len as usize];
        file.read_exact_at(&mut bytes, 0)?;
        Ok(bytes)
    };
    Ok(Output {
        status,
        stdout: snapshot(stdout)?,
        stderr: snapshot(stderr)?,
    })
}

struct Roots(Vec<PathBuf>, bool);
impl Roots {
    fn create() -> Result<Self> {
        let parent = crate::environment::user_runtime_parent();
        let mut roots = Self(Vec::new(), false);
        for index in 0..4096 {
            let path = parent.join(format!(
                "b{:03x}",
                (SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(failure)?
                    .as_nanos() as usize
                    + index)
                    % 4096
            ));
            if path.as_os_str().len() > 19 {
                return Err("Eon test needs a runtime root <=19 bytes".into());
            }
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => roots.0.push(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(failure(error)),
            }
            if roots.0.len() == 2 {
                return Ok(roots);
            }
        }
        Err("no private runtime roots available".into())
    }
}
impl Drop for Roots {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        for root in &self.0 {
            let _ = fs::remove_dir_all(root);
        }
    }
}

pub(crate) struct Processes {
    pub(crate) children: Vec<Child>,
    pub(crate) known: BTreeMap<u32, String>,
    marker: String,
}
/// What `/proc/<pid>/stat` says about a process.
#[derive(Debug)]
pub(crate) struct Stat {
    /// The start time, which tells this lifetime from a later one reusing the PID.
    pub(crate) start: String,
    /// Not yet a zombie.
    pub(crate) running: bool,
    /// The kernel has begun `do_exit`, which sets `PF_EXITING` (0x4 in the
    /// flags field) before it drops the process's memory and executable, and
    /// only then marks it a zombie. Meanwhile its `exe`, `environ` and `maps`
    /// no longer read, though its state still says running.
    pub(crate) exiting: bool,
    pub(crate) parent: u32,
}
impl Stat {
    fn parse(stat: &str) -> Option<Self> {
        // The command name may hold spaces and `)`, so split after its last `)`.
        let fields = stat
            .rsplit_once(") ")?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        let flags: u32 = fields.get(6)?.parse().ok()?;
        Some(Self {
            start: fields.get(19)?.to_string(),
            running: fields.first() != Some(&"Z"),
            exiting: flags & 0x4 != 0,
            parent: fields.get(1)?.parse().ok()?,
        })
    }
    /// The lifetime that started at `identity`, running and not exiting: its
    /// `/proc` entries must read.
    pub(crate) fn intact(&self, identity: &str) -> bool {
        self.start == identity && self.running && !self.exiting
    }
}
pub(crate) fn process_state(pid: u32) -> Option<Stat> {
    Stat::parse(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}
/// Whether the lifetime that started at `identity` still exists and is not a
/// zombie; it may be exiting, and an exiting process still holds its
/// descriptors and, briefly, its memory.
pub(crate) fn live_identity(pid: u32, identity: &str) -> bool {
    process_state(pid).is_some_and(|stat| stat.start == identity && stat.running)
}

/// Only use for an inventory after entering the display suite's PID namespace.
pub(crate) fn namespace_processes() -> Result<BTreeMap<u32, String>> {
    Ok(fs::read_dir("/proc")
        .map_err(failure)?
        .filter_map(|entry| {
            let pid = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let stat = process_state(pid)?;
            stat.running.then_some((pid, stat.start))
        })
        .collect())
}
/// Sends `signal` to `pid` only if `still_same` holds once a pidfd pins the
/// process, so a reused PID is never signalled; without pidfds (an old
/// kernel, or not Linux) nothing is sent.
#[cfg(target_os = "linux")]
pub(crate) fn signal_pinned(pid: u32, still_same: impl FnOnce() -> bool, signal: Signal) -> bool {
    use rustix::process::{Pid, PidfdFlags, pidfd_open, pidfd_send_signal};
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return false;
    };
    let Ok(pidfd) = pidfd_open(pid, PidfdFlags::empty()) else {
        return false;
    };
    still_same() && pidfd_send_signal(&pidfd, signal).is_ok()
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn signal_pinned(_: u32, _: impl FnOnce() -> bool, _: Signal) -> bool {
    false
}
impl Processes {
    pub(crate) fn remember(&mut self, pid: u32) -> Option<String> {
        let Stat {
            start: identity,
            running,
            ..
        } = process_state(pid)?;
        let owned = fs::read(format!("/proc/{pid}/environ"))
            .ok()?
            .split(|&b| b == 0)
            .any(|item| item == self.marker.as_bytes());
        if !running || !owned || !live_identity(pid, &identity) {
            return None;
        }
        self.known.insert(pid, identity.clone());
        Some(identity)
    }
    fn descendants(&mut self, pid: u32, identity: &str) {
        if !live_identity(pid, identity) {
            return;
        }
        let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) else {
            return;
        };
        if !live_identity(pid, identity) {
            return;
        }
        for child in children
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
        {
            let Some(Stat {
                start,
                running: true,
                parent,
                ..
            }) = process_state(child)
            else {
                continue;
            };
            if parent != pid || !live_identity(pid, identity) {
                continue;
            }
            // Ownership and both identities must survive the environment read.
            if self.remember(child).as_deref() != Some(start.as_str()) {
                self.known.remove(&child);
                continue;
            }
            if !live_identity(pid, identity)
                || process_state(child).is_none_or(|now| now.start != start || now.parent != pid)
            {
                self.known.remove(&child);
                continue;
            }
            self.descendants(child, &start);
        }
    }
    pub(crate) fn cleanup(&mut self) -> bool {
        // Saved PIDs are never roots for traversal until their start identity matches.
        for (pid, identity) in self.known.clone() {
            self.descendants(pid, &identity);
        }
        for signal in [Signal::TERM, Signal::KILL] {
            for (&pid, identity) in &self.known {
                signal_pinned(pid, || live_identity(pid, identity), signal);
            }
            let deadline = Instant::now() + Duration::from_secs(1);
            while self
                .known
                .iter()
                .any(|(&pid, identity)| live_identity(pid, identity))
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
        }
        for child in &mut self.children {
            let _ = child.try_wait();
        }
        !self
            .known
            .iter()
            .any(|(&pid, identity)| live_identity(pid, identity))
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub(crate) enum BusPolicy {
    Session,
    Private,
}

pub(crate) enum WindowMode {
    Fullscreen,
    Floating,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputSize {
    pub(crate) width: u64,
    pub(crate) height: u64,
}

impl OutputSize {
    pub(crate) const DEFAULT: Self = Self {
        width: 1280,
        height: 800,
    };

    pub(crate) fn parse(text: &str) -> Result<Self> {
        let invalid = || {
            format!(
                "expected WIDTHxHEIGHT with sides from 1 to {}",
                crate::picture::MAX_SIDE
            )
        };
        let (width, height) = text.split_once('x').ok_or_else(invalid)?;
        let side = |value: &str| {
            value
                .parse::<u64>()
                .ok()
                .filter(|&size| {
                    value.bytes().all(|byte| byte.is_ascii_digit())
                        && (1..=crate::picture::MAX_SIDE as u64).contains(&size)
                })
                .ok_or_else(invalid)
        };
        Ok(Self {
            width: side(width)?,
            height: side(height)?,
        })
    }

    pub(crate) fn check_outputs(self, outputs: &Value) -> Result {
        let mut active = outputs
            .as_array()
            .ok_or("Sway returned no outputs")?
            .iter()
            .filter(|o| o.get("active").and_then(Value::as_bool) == Some(true));
        let matches = active.next().is_some_and(|output| {
            output
                .get("current_mode")
                .is_some_and(|mode| self.matches(mode))
                && output.get("scale").and_then(Value::as_f64) == Some(1.0)
        });
        if !matches || active.next().is_some() {
            return Err(format!(
                "expected one private output at {self}, scale 1: {outputs:?}"
            ));
        }
        Ok(())
    }

    pub(crate) fn matches(self, dimensions: &Value) -> bool {
        dimensions.get("width").and_then(Value::as_u64) == Some(self.width)
            && dimensions.get("height").and_then(Value::as_u64) == Some(self.height)
    }
}

impl std::fmt::Display for OutputSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// How every candidate program runs: the launch, CLI calls, helpers and Stop.
/// Chosen with the candidate, never from a program's path; bench tools such as
/// `swaymsg` and `grim` always run directly.
pub(crate) enum Route {
    Direct,
    Jail(Box<Jail>),
}

/// Suites supply their own config, trace consumer and workloads. The launcher owns
/// only the private display and process lifetime, including failure cleanup.
pub(crate) struct Headless {
    pub(crate) processes: Processes,
    roots: Roots,
    eon_child: Option<usize>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) eon: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) output_size: OutputSize,
    route: Route,
}
impl Headless {
    pub(crate) fn new(out: &Path, eon: &Path, window_mode: WindowMode) -> Result<Self> {
        Self::with_size(out, eon, window_mode, OutputSize::DEFAULT)
    }
    pub(crate) fn with_size(
        out: &Path,
        eon: &Path,
        window_mode: WindowMode,
        output_size: OutputSize,
    ) -> Result<Self> {
        Self::create(out, candidate(eon)?, window_mode, output_size)
    }
    /// A Nix-free bundle candidate whose every program runs in one jail built
    /// from `host` (crate::bundle).
    pub(crate) fn bundle(
        out: &Path,
        eon: &Path,
        window_mode: WindowMode,
        host: HostSet,
    ) -> Result<Self> {
        let mut launch = Self::create(out, eon.to_path_buf(), window_mode, OutputSize::DEFAULT)?;
        let xdg = launch.out.join("xdg");
        for directory in ["data", "config"] {
            fs::create_dir_all(xdg.join(directory)).map_err(failure)?;
        }
        let runtime = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]);
        let eon_root = PathBuf::from(&launch.env["EON_RUNTIME_DIR"]);
        // Sway runs any command its IPC socket is sent, outside the jail, so
        // the socket lives where no jail bind reaches.
        let mut ipc = runtime.into_os_string();
        ipc.push("-ipc");
        let ipc = PathBuf::from(ipc);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&ipc)
            .map_err(failure)?;
        launch.roots.0.push(ipc.clone());
        launch.env.insert(
            "SWAYSOCK".into(),
            ipc.join("sway.sock").display().to_string(),
        );
        let runtime = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]);
        let mounts = Jail::mounts(
            &host,
            Path::new(crate::bundle::HOST_ROOT),
            &[&launch.out, &runtime, &eon_root],
        )?;
        let log = File::create(launch.out.join("jail.log")).map_err(failure)?;
        let mut holder = None;
        let env = launch.env.clone();
        let jail = Jail::start(
            crate::isolation::bwrap_path().into(),
            host,
            mounts,
            xdg,
            &env,
            |program, args| {
                let mut command = launch.command(program);
                command
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(log.try_clone().map_err(failure)?)
                    .stderr(log.try_clone().map_err(failure)?);
                holder = Some(command.spawn().map_err(failure)?);
                Ok(())
            },
        );
        if let Some(holder) = holder {
            launch.adopt(holder);
        }
        launch.route = Route::Jail(Box::new(jail?));
        Ok(launch)
    }
    fn create(
        out: &Path,
        eon: PathBuf,
        window_mode: WindowMode,
        output_size: OutputSize,
    ) -> Result<Self> {
        let roots = Roots::create()?;
        let out = fs::canonicalize(out).map_err(failure)?;
        let runtime = roots.0[0].clone();
        let eon_root = roots.0[1].clone();
        for directory in ["home", "config", "state"] {
            fs::create_dir(out.join(directory)).map_err(failure)?;
        }
        let sway_config = out.join("sway.conf");
        let window_rule = match window_mode {
            WindowMode::Fullscreen => "fullscreen enable",
            WindowMode::Floating => "floating enable",
        };
        fs::write(&sway_config, format!("output * mode {output_size}\noutput * scale 1\noutput * bg #202020 solid_color\nseat seat0 fallback true\nxwayland disable\ndefault_border none\nfocus_follows_mouse no\nfor_window [app_id=\"eon\"] {window_rule}\n")).map_err(failure)?;
        let env = BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").map_err(failure)?),
            ("HOME".into(), out.join("home").display().to_string()),
            (
                "EON_CONFIG_HOME".into(),
                out.join("config").display().to_string(),
            ),
            (
                "XDG_STATE_HOME".into(),
                out.join("state").display().to_string(),
            ),
            ("XDG_RUNTIME_DIR".into(), runtime.display().to_string()),
            ("EON_RUNTIME_DIR".into(), eon_root.display().to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".into(),
                format!("unix:path={}/bus", runtime.display()),
            ),
            ("WLR_BACKENDS".into(), "headless".into()),
            ("WLR_RENDERER".into(), "pixman".into()),
            ("WLR_LIBINPUT_NO_DEVICES".into(), "1".into()),
            ("LANG".into(), "C.UTF-8".into()),
        ]);
        Ok(Self {
            processes: Processes {
                children: Vec::new(),
                known: BTreeMap::new(),
                marker: format!("EON_RUNTIME_DIR={}", eon_root.display()),
            },
            roots,
            eon_child: None,
            env,
            eon,
            out,
            output_size,
            route: Route::Direct,
        })
    }
    pub(crate) fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env_clear().envs(&self.env).stdin(Stdio::null());
        command
    }
    pub(crate) fn output_until(
        &self,
        deadline: Instant,
        program: &Path,
        args: &[&str],
    ) -> Result<Output> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("command deadline elapsed")?;
        let mut command = self.command("timeout");
        command
            .args([
                "--kill-after=0.2s",
                &format!("{}s", remaining.as_secs_f64()),
            ])
            .arg(program)
            .args(args);
        capture_output(&mut command, Path::new(&self.env["XDG_RUNTIME_DIR"])).map_err(failure)
    }
    /// `output_until` for a candidate program, through the candidate's route.
    pub(crate) fn candidate_output_until(
        &self,
        deadline: Instant,
        program: &Path,
        args: &[&str],
    ) -> Result<Output> {
        let (program, args) = self.routed(program, args)?;
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        self.inheriting(|| self.output_until(deadline, &program, &args))?
    }
    fn routed(&self, program: &Path, args: &[&str]) -> Result<(PathBuf, Vec<String>)> {
        Ok(match &self.route {
            Route::Direct => (
                program.to_path_buf(),
                args.iter().map(|&arg| arg.into()).collect(),
            ),
            Route::Jail(jail) => (
                jail.bwrap().to_path_buf(),
                jail.join(&self.env, program, args)?,
            ),
        })
    }
    fn inheriting<R>(&self, spawn: impl FnOnce() -> R) -> Result<R> {
        match &self.route {
            Route::Direct => Ok(spawn()),
            Route::Jail(jail) => jail.inherit(spawn),
        }
    }
    /// Binds `source` read-only at `destination` in the bundle jail for
    /// every later candidate call.
    pub(crate) fn jail_bind(&mut self, source: &Path, destination: &Path) -> Result {
        let Route::Jail(jail) = &mut self.route else {
            return Err("jail binds need the bundle jail".into());
        };
        jail.bind(source, destination)
    }
    /// Audits candidate process origins in the bundle jail (crate::bundle).
    pub(crate) fn audit_origins(&self, tree: &Path, required: &[&str]) -> Result<Value> {
        let Route::Jail(jail) = &self.route else {
            return Err("origin audits need the bundle jail".into());
        };
        jail.audit(&self.out, tree, required)
    }
    pub(crate) fn spawn(&mut self, program: &str, args: &[&str], log: &str) -> Result {
        let output = File::create(self.out.join(log)).map_err(failure)?;
        self.spawn_logged(program, args, output.try_clone().map_err(failure)?, output)
    }
    pub(crate) fn spawn_separate(
        &mut self,
        program: &str,
        args: &[&str],
        log: &str,
        stderr: &str,
    ) -> Result {
        self.spawn_logged(
            program,
            args,
            File::create(self.out.join(log)).map_err(failure)?,
            File::create(self.out.join(stderr)).map_err(failure)?,
        )
    }
    fn spawn_logged(&mut self, program: &str, args: &[&str], output: File, stderr: File) -> Result {
        let child = self
            .command(program)
            .args(args)
            .stdout(output)
            .stderr(stderr)
            .spawn()
            .map_err(failure)?;
        self.adopt(child);
        Ok(())
    }
    fn adopt(&mut self, child: Child) {
        self.processes.remember(child.id());
        self.processes.children.push(child);
    }
    pub(crate) fn start_compositor(&mut self, bus: BusPolicy) -> Result {
        self.start_compositor_until(bus, Instant::now() + Duration::from_secs(15))
    }
    pub(crate) fn start_compositor_until(&mut self, bus: BusPolicy, deadline: Instant) -> Result {
        self.start_sway_until("sway", bus, deadline)
    }
    pub(crate) fn start_sway_until(
        &mut self,
        sway: &str,
        bus: BusPolicy,
        deadline: Instant,
    ) -> Result {
        let deadline = deadline.min(Instant::now() + Duration::from_secs(15));
        let runtime = PathBuf::from(&self.env["XDG_RUNTIME_DIR"]);
        let sway_config = self.out.join("sway.conf");
        let address = self.env["DBUS_SESSION_BUS_ADDRESS"].clone();
        match bus {
            BusPolicy::Session => {
                let config = std::env::var_os("EON_BENCH_DBUS_SESSION_CONFIG")
                    .ok_or("missing EON_BENCH_DBUS_SESSION_CONFIG; use the Nix bench package")?;
                let config = fs::canonicalize(config).map_err(failure)?;
                if !config.starts_with("/nix/store") || !config.is_file() {
                    return Err("D-Bus session configuration must be a Nix store file".into());
                }
                self.spawn(
                    "dbus-daemon",
                    &[
                        &format!("--config-file={}", config.display()),
                        "--nofork",
                        &format!("--address={address}"),
                    ],
                    "dbus.log",
                )?;
            }
            BusPolicy::Private => {
                let bus_config = self.out.join("dbus.conf");
                // No host service directories: the private bus must not activate desktop daemons.
                fs::write(
                    &bus_config,
                    r#"<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                )
                .map_err(failure)?;
                self.spawn(
                    "dbus-daemon",
                    &[
                        "--nofork",
                        &format!(
                            "--config-file={}",
                            bus_config.to_str().ok_or("non-UTF8 output path")?
                        ),
                        &format!("--address={address}"),
                    ],
                    "dbus.log",
                )?;
                wait_for_bus(
                    self.processes
                        .children
                        .last_mut()
                        .ok_or("missing D-Bus child")?,
                    &runtime.join("bus"),
                    deadline,
                )?;
            }
        }
        self.spawn(
            sway,
            &[
                "--unsupported-gpu",
                "--config",
                sway_config.to_str().ok_or("non-UTF8 output path")?,
            ],
            "sway.log",
        )?;
        loop {
            if self
                .processes
                .children
                .last_mut()
                .ok_or("missing Sway child")?
                .try_wait()
                .map_err(failure)?
                .is_some()
            {
                return Err("private Sway exited; see sway.log".into());
            }
            let files: Vec<_> = fs::read_dir(&runtime)
                .map_err(failure)?
                .filter_map(|entry| entry.ok())
                .collect();
            let wayland = files.iter().find(|entry| {
                entry.file_name().to_string_lossy().starts_with("wayland-")
                    && !entry.file_name().to_string_lossy().ends_with(".lock")
            });
            // A preset SWAYSOCK names where Sway must put its command socket.
            let ipc = match self.env.get("SWAYSOCK") {
                Some(preset) => Path::new(preset).exists().then(|| PathBuf::from(preset)),
                None => files
                    .iter()
                    .find(|entry| {
                        entry.file_name().to_string_lossy().starts_with("sway-ipc.")
                            && entry.file_name().to_string_lossy().ends_with(".sock")
                    })
                    .map(|entry| entry.path()),
            };
            if let (Some(wayland), Some(ipc)) = (wayland, ipc) {
                self.env.insert(
                    "WAYLAND_DISPLAY".into(),
                    wayland.file_name().to_string_lossy().into(),
                );
                self.env
                    .insert("SWAYSOCK".into(), ipc.display().to_string());
                // Socket creation can precede the IPC event loop becoming responsive.
                let reply = self.output_until(
                    deadline.min(Instant::now() + Duration::from_secs(1)),
                    Path::new("swaymsg"),
                    &["-t", "get_outputs", "-r"],
                )?;
                fs::write(
                    self.out.join("sway-ipc.log"),
                    format!(
                        "{}\n{}{}",
                        reply.status,
                        String::from_utf8_lossy(&reply.stdout),
                        String::from_utf8_lossy(&reply.stderr)
                    ),
                )
                .map_err(failure)?;
                if reply.status.success() {
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err("private Sway did not become ready; see sway.log".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
    pub(crate) fn start_eon(&mut self) -> Result {
        self.start_eon_with(&[])
    }
    pub(crate) fn start_eon_with(&mut self, args: &[&str]) -> Result {
        if self.eon_child.is_some() {
            return Err("private Eon already started".into());
        }
        let (program, args) = self.routed(&self.eon, args)?;
        let log = File::create(self.out.join("eon.log")).map_err(failure)?;
        let mut command = self.command(program);
        command
            .args(args)
            .stdout(log.try_clone().map_err(failure)?)
            .stderr(log);
        let child = self.inheriting(|| command.spawn())?.map_err(failure)?;
        self.adopt(child);
        self.eon_child = Some(self.processes.children.len() - 1);
        Ok(())
    }

    pub(crate) fn presentation(&self, pid: u32) -> Result<Stat> {
        let supervisor = self
            .eon_child
            .map(|index| self.processes.children[index].id())
            .ok_or("private supervisor has not started")?;
        process_state(pid)
            .filter(|stat| stat.parent == supervisor && stat.running && !stat.exiting)
            .ok_or_else(|| {
                "private compositor's Eon client is not a live child of the supervisor".into()
            })
    }

    /// End the private native client while keeping the supervisor and its PTYs.
    pub(crate) fn end_presentation(&self, pid: u32) -> Result<String> {
        let start = self.presentation(pid)?.start;
        if !signal_pinned(
            pid,
            || self.presentation(pid).is_ok_and(|stat| stat.start == start),
            Signal::TERM,
        ) {
            return Err("could not signal the pinned private Eon client".into());
        }
        Ok(start)
    }
    pub(crate) fn stop_eon(&mut self) -> Result {
        self.stop_eon_report().map(|_| ())
    }
    pub(crate) fn stop_eon_report(&mut self) -> Result<Option<String>> {
        let Some(index) = self.eon_child else {
            return Ok(None);
        };
        let stopped = self
            .candidate_output_until(
                Instant::now() + Duration::from_secs(10),
                &self.eon,
                &["window", "stop", "all", "--json"],
            )
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).map(Some).map_err(failure)
                } else {
                    Err(format!(
                        "private Stop failed ({}): {} {}",
                        output.status,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ))
                }
            });
        if stopped.is_ok() {
            let deadline = Instant::now() + Duration::from_secs(15);
            let child = &mut self.processes.children[index];
            loop {
                if child.try_wait().map_err(failure)?.is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("private supervisor did not exit after Stop".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        stopped
    }
}
impl Drop for Headless {
    fn drop(&mut self) {
        if !self.processes.cleanup() {
            self.roots.1 = true;
            eprintln!(
                "eon-bench: private processes still live; preserving {:?}",
                self.roots.0
            );
        }
    }
}
pub(crate) fn candidate(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path).map_err(failure)?;
    if !path.starts_with("/nix/store") || !path.is_file() {
        return Err("--eon must name a Nix candidate, not a profile/checkout fallback".into());
    }
    Ok(path)
}
fn wait_for_bus(child: &mut Child, path: &Path, deadline: Instant) -> Result {
    loop {
        if child.try_wait().map_err(failure)?.is_some() {
            return Err("private D-Bus exited; see dbus.log".into());
        }
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("private D-Bus did not become ready; see dbus.log".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Explicit GPU selection for measured suites; response leaves its existing
/// automatic adapter policy alone. Hardware needs a GPU compositor to advertise dmabuf.
pub(crate) struct Renderer(pub(crate) String);
impl Renderer {
    pub(crate) fn apply(&self, launch: &mut Headless) -> Result {
        if let Some(node) = self.render_node()? {
            let egl = std::env::var("EON_BENCH_EGL")
                .map_err(|_| "missing EON_BENCH_EGL (use the Nix bench package)")?;
            if !Path::new(&egl).is_file() {
                return Err("Mesa EGL vendor file is missing".into());
            }
            launch
                .env
                .insert("__EGL_VENDOR_LIBRARY_FILENAMES".into(), egl);
            for (source, target) in [
                ("EON_BENCH_GBM", "GBM_BACKENDS_PATH"),
                ("EON_BENCH_DRI", "LIBGL_DRIVERS_PATH"),
            ] {
                let directory = std::env::var(source)
                    .map_err(|_| format!("missing {source} (use the Nix bench package)"))?;
                if !Path::new(&directory).is_dir() {
                    return Err(format!("{source} directory is missing"));
                }
                launch.env.insert(target.into(), directory);
            }
            let icds = std::env::var_os("EON_BENCH_VULKAN")
                .ok_or("missing EON_BENCH_VULKAN (use the Nix bench package)")?;
            launch
                .env
                .insert("VK_DRIVER_FILES".into(), hardware_icds(Path::new(&icds))?);
            launch.env.insert("WLR_RENDERER".into(), "gles2".into());
            launch
                .env
                .insert("WLR_RENDER_DRM_DEVICE".into(), node.display().to_string());
        } else {
            let icd = std::env::var("EON_BENCH_LAVAPIPE")
                .map_err(|_| "missing EON_BENCH_LAVAPIPE (use the Nix bench package)")?;
            if !Path::new(&icd).is_file() {
                return Err("lavapipe ICD is missing".into());
            }
            launch.env.insert("VK_DRIVER_FILES".into(), icd);
        }
        Ok(())
    }

    pub(crate) fn render_node(&self) -> Result<Option<&Path>> {
        if self.0 == "lavapipe" {
            return Ok(None);
        }
        let path = Path::new(&self.0);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if path.parent() != Some(Path::new("/dev/dri"))
            || !name
                .strip_prefix("renderD")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            || !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_char_device())
        {
            return Err(
                "--renderer must be lavapipe or an existing /dev/dri/renderD<N> device".into(),
            );
        }
        Ok(Some(path))
    }
}

fn hardware_icds(directory: &Path) -> Result<String> {
    let mut manifests = fs::read_dir(directory)
        .map_err(failure)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(failure)?;
    manifests.retain(|path| {
        path.extension().is_some_and(|ext| ext == "json")
            && !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .contains("lvp")
    });
    manifests.sort();
    if manifests.is_empty() {
        return Err("no pinned hardware Vulkan ICD".into());
    }
    std::env::join_paths(manifests)
        .map_err(failure)?
        .into_string()
        .map_err(|_| "non-UTF8 Vulkan manifests".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_vulkan_omits_lavapipe_and_requires_a_manifest() {
        let root = std::env::temp_dir().join(format!("bench-icd-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("lvp_icd.json"), "").unwrap();
        assert!(hardware_icds(&root).is_err());
        for name in ["z.json", "a.json", "readme"] {
            fs::write(root.join(name), "").unwrap();
        }
        let selected = hardware_icds(&root).unwrap();
        assert_eq!(
            std::env::split_paths(&selected).collect::<Vec<_>>(),
            [root.join("a.json"), root.join("z.json")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn output_check_requires_one_active_output_at_the_requested_mode_and_scale() {
        let size = OutputSize::DEFAULT;
        let active = format!(
            r#"{{"active":true,"current_mode":{{"width":{},"height":{}}},"scale":1}}"#,
            size.width, size.height,
        );
        let check = |outputs: &str| size.check_outputs(&Value::parse(outputs).unwrap());
        assert!(check(&format!(r#"[{{"active":false}},{active}]"#)).is_ok());
        assert!(check(&format!("[{active},{active}]")).is_err());
        assert!(
            check(&format!(
                "[{}]",
                active.replace(r#""scale":1"#, r#""scale":2"#)
            ))
            .is_err()
        );
        assert!(
            check(&format!(
                "[{}]",
                active.replace(&format!(r#""height":{}"#, size.height), r#""height":1"#),
            ))
            .is_err()
        );
    }

    /// A `/proc/<pid>/stat` line, with a command name that holds `) ` itself.
    fn stat_line(state: char, flags: u32, start: u64) -> String {
        format!(
            "4242 (sh) -c ) x) {state} 7 4242 4242 0 -1 {flags} 120 0 0 0 1 2 0 0 20 0 1 0 {start} \
             2768896 222 18446744073709551615 1 1 0 0 0 0 65536 4 65538 0 0 0 17 3 0 0 0 0 0"
        )
    }

    #[test]
    fn stat_parse_reads_exiting_and_zombie_states() {
        // 0x400100 is PF_RANDOMIZE | PF_SUPERPRIV: set flags that are not PF_EXITING.
        let running = Stat::parse(&stat_line('R', 0x40_0100, 81)).unwrap();
        assert_eq!((running.start.as_str(), running.parent), ("81", 7));
        assert!(running.running && !running.exiting);
        assert!(running.intact("81"));
        assert!(!running.intact("82"), "a later lifetime of the same PID");
        let exiting = Stat::parse(&stat_line('R', 0x40_0104, 81)).unwrap();
        assert!(exiting.running && exiting.exiting);
        assert!(!exiting.intact("81"));
        let zombie = Stat::parse(&stat_line('Z', 0x40_0104, 81)).unwrap();
        assert!(!zombie.running && !zombie.intact("81"));
        assert!(Stat::parse("4242 (sh) R 7").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn command_output_does_not_wait_on_a_helpers_inherited_descriptors() {
        let root = std::env::temp_dir().join(format!("bench-output-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & printf '%s' \"$!\"; printf note >&2"]);
        let out = capture_output(&mut command, &root);
        fs::remove_dir(root).unwrap();
        let out = out.unwrap();
        assert!(out.status.success());
        assert_eq!(out.stderr, b"note");
        let pid = std::str::from_utf8(&out.stdout).unwrap().parse().unwrap();
        let Stat {
            start: identity,
            running,
            ..
        } = process_state(pid).expect("helper is still alive");
        assert!(running);
        signal_pinned(pid, || live_identity(pid, &identity), Signal::KILL);
    }

    #[test]
    fn an_exited_bus_is_a_startup_failure() {
        let mut child = Command::new("sh").args(["-c", "exit 1"]).spawn().unwrap();
        child.wait().unwrap();
        assert!(
            wait_for_bus(
                &mut child,
                Path::new("/missing-private-bus"),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err()
            .contains("D-Bus exited")
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_refuses_reused_roots_and_unowned_descendants() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 2 & wait"])
            .env_remove("EON_RUNTIME_DIR")
            .spawn()
            .unwrap();
        let mut processes = Processes {
            children: Vec::new(),
            known: BTreeMap::from([(child.id(), "different-lifetime".into())]),
            marker: "EON_RUNTIME_DIR=private-bench-test".into(),
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        processes.descendants(child.id(), "different-lifetime");
        assert_eq!(processes.known.len(), 1);
        let identity = process_state(child.id()).unwrap().start;
        processes.descendants(child.id(), &identity);
        assert_eq!(processes.known.len(), 1);
        assert!(processes.remember(child.id()).is_none());
        assert!(processes.cleanup());
        assert!(child.try_wait().unwrap().is_none());
        // Let the short-lived unowned fixture reap its own child.
        assert!(child.wait().unwrap().success());
    }
}
