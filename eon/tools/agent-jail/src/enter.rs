//! `agent-jail enter`: the host side. It checks the credential's lifetime,
//! builds a fresh run directory and home, starts the allowlist proxy, runs
//! the outer bwrap with init as PID 1 on the task store, enforces the wall
//! cap and the disk floor, then opens up the private overlay layers, scans
//! the run for token material and deletes the layers. `command` mode is the
//! same jail without a credential, a proxy or a bridge.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::manifest::Home;
use crate::pins::Pins;
use crate::profile::Loaded;
use crate::proxy::{Limits, Proxy, Resolver};
use crate::scan::{Finding, Secret};
use crate::spec::{COMMAND_SCRIPT, Mode, Spec, StoreSpec};
use crate::task_env::{CARGO_HOME, STORE, Store, TARGET_DIR, TaskEnv, Template};
use crate::{BIN_DIR, HOME, JAIL_DIR, PRIVATE_DIR, Result, SHELL_PATHS, SOCK_DIR, WORKSPACE};

/// Workspace paths a harness reads as project configuration; read-only.
pub const PROJECT_CONFIG: &[&str] = &[".claude", ".codex", ".agents", ".agent", ".gemini"];

/// What a run asks for.
#[derive(Clone, Debug)]
pub struct Options {
    pub profile: String,
    pub run: PathBuf,
    pub wall_cap: i64,
    pub probe: bool,
    pub mode: Mode,
    pub slots: BTreeMap<String, String>,
    /// A probe control variant; refused without `probe`.
    pub control: crate::control::Control,
    /// Command mode's env.json template.
    pub template: Option<Template>,
    /// Command mode only: bind the task env's bases read-write (the warm build).
    pub base_rw: bool,
}

/// The host facts a run uses. The CLI fills these from the pins and fixed
/// paths; tests fill them directly.
#[derive(Clone)]
pub struct Host {
    pub pins: Pins,
    /// The agent-jail binary bound as init and every wrapper.
    pub self_exe: PathBuf,
    /// The harness credential file on the host.
    pub auth: PathBuf,
    pub resolver: Resolver,
    pub limits: Limits,
    /// Unix seconds, for the credential gate.
    pub now: i64,
    /// The store, Cargo home and target bases the jail mounts.
    pub task: TaskEnv,
    /// The free space the run's filesystem must keep; the jail dies below it.
    pub disk_floor: DiskFloor,
}

/// Free bytes a run's filesystem must keep, and how often [`wait_capped`]
/// checks them after its first check at the start.
#[derive(Clone, Copy, Debug)]
pub struct DiskFloor {
    pub bytes: u64,
    pub every: Duration,
}

impl DiskFloor {
    /// `bytes`, checked every [`DISK_CHECK`].
    pub const fn new(bytes: u64) -> Self {
        Self {
            bytes,
            every: DISK_CHECK,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub control: crate::control::Control,
    pub store: Store,
    pub exit: i32,
    pub killed_at_cap: bool,
    pub killed_at_disk_floor: bool,
    pub quarantined: bool,
    /// The credential's minutes left; none in command mode.
    pub minutes_left: Option<i64>,
    /// Why `<run>/overlay` survived its deletion, if it did.
    pub overlay_left: Vec<String>,
}

/// The outer bwrap flags that make the jail's lifetime init's: init is the
/// PID namespace's PID 1 and dies with `enter`'s bwrap child.
pub const OUTER_LIFETIME: &[&str] = &["--unshare-pid", "--as-pid-1", "--die-with-parent"];

/// How the outer bwrap ended.
pub struct Waited {
    pub status: std::process::ExitStatus,
    pub killed_at_cap: bool,
    pub killed_at_disk_floor: bool,
}

/// How often the disk floor is checked.
pub const DISK_CHECK: Duration = Duration::from_secs(10);

/// Waits for the outer bwrap, killing it once `cap` has passed since
/// `started`, or when `floor`'s filesystem has fewer free bytes than it
/// asks (checked at once, then every [`DiskFloor::every`]; an upper layer
/// can fill the disk). Its `--die-with-parent` takes init with it, and init
/// is the PID namespace's PID 1, so the whole jail goes.
pub fn wait_capped(
    child: &mut std::process::Child,
    started: Instant,
    cap: Duration,
    floor: Option<(&Path, DiskFloor)>,
) -> Result<Waited> {
    let (mut killed_at_cap, mut killed_at_disk_floor) = (false, false);
    let mut checked: Option<Instant> = None;
    loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            return Ok(Waited {
                status,
                killed_at_cap,
                killed_at_disk_floor,
            });
        }
        if started.elapsed() > cap && !killed_at_cap {
            killed_at_cap = true;
            let _ = child.kill();
        }
        if let Some((path, floor)) = floor
            && !killed_at_disk_floor
            && checked.is_none_or(|checked| checked.elapsed() >= floor.every)
        {
            checked = Some(Instant::now());
            // An unreadable filesystem counts as full.
            if crate::free_bytes(path).unwrap_or(0) < floor.bytes {
                killed_at_disk_floor = true;
                let _ = child.kill();
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The run directory and its parts are created, never followed: a link
/// there would bind some other directory into the jail read-write.
fn refuse_link(path: &Path) -> Result {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{} is a symbolic link; the jail refuses it",
            path.display()
        )),
        _ => Ok(()),
    }
}

/// `enter`'s exit status: a quarantined run never reads as success.
pub const QUARANTINE_EXIT: i32 = 4;

pub fn exit_status(outcome: &Outcome) -> i32 {
    if outcome.quarantined {
        QUARANTINE_EXIT
    } else {
        outcome.exit
    }
}

/// The harness executable in the jail for `loaded`.
pub fn harness_exe(loaded: &Loaded) -> String {
    if loaded.profile.exe != "pinned" {
        return loaded.profile.exe.clone();
    }
    let pins: toml::Table = toml::from_str(crate::pins::HOST_TOOLS).unwrap_or_default();
    pins.get("tools")
        .and_then(|tools| tools.get(&loaded.profile.binary))
        .and_then(|tool| tool.get("path"))
        .and_then(|path| path.as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The jail mounts only the task store at /nix/store, so agent-jail must
/// be static or load from the store (built in the repository's dev shell);
/// `task-env store` copies what it loads.
pub fn check_self_exe(path: &Path) -> Result {
    match crate::elf::read(path)?.interpreter {
        Some(interpreter) if !interpreter.starts_with(&format!("{STORE}/")) => Err(format!(
            "{} loads {interpreter}, which the jail does not mount; build agent-jail in the dev shell (nix develop)",
            path.display()
        )),
        _ => Ok(()),
    }
}

/// The host credential file each harness reads.
pub fn default_auth(harness: &str) -> Result<PathBuf> {
    let home = crate::pins::passwd_home()?;
    match harness {
        "claude" => Ok(home.join(".claude/.credentials.json")),
        "codex" => Ok(home.join(".codex/auth.json")),
        other => Err(format!("no credential for harness {other}")),
    }
}

/// Wrapper path in the jail to the real tool.
pub fn tool_map(pins: &Pins) -> Result<BTreeMap<String, String>> {
    let mut tools: BTreeMap<String, String> = pins
        .inner
        .iter()
        .map(|(name, real)| (format!("{BIN_DIR}/{name}"), real.display().to_string()))
        .collect();
    let bash = pins
        .inner
        .get("bash")
        .ok_or("the inner packages provide no bash")?;
    for shell in SHELL_PATHS {
        tools.insert((*shell).to_owned(), bash.display().to_string());
    }
    Ok(tools)
}

/// Where the outer layer holds the real harness release for the inner
/// layer's binds; under the private directory, which the inner layer masks.
pub const RELEASE_COPY: &str = "/bench/jail-private/harness-release";

/// The pinned harness's release directory on the host and its mount point,
/// for profiles that mount one.
pub fn release_dir(loaded: &Loaded, host: &Host) -> Result<Option<(PathBuf, String)>> {
    let Some(mount) = &loaded.profile.release_mount else {
        return Ok(None);
    };
    let binary = host.pins.tool(&loaded.profile.binary)?;
    let release = binary
        .parent()
        .and_then(Path::parent)
        .ok_or("the pinned harness binary has no release directory")?;
    Ok(Some((release.to_path_buf(), mount.clone())))
}

/// Wrapper path to real path for every executable in the release except
/// the harness itself: the code-mode host, rg, the exec server and the
/// bundled bwrap all run in the inner layer when the harness spawns them.
pub fn release_tools(
    release: &Path,
    mount: &str,
    harness_exe: &str,
) -> Result<BTreeMap<String, String>> {
    use std::os::unix::fs::PermissionsExt;
    let mut tools = BTreeMap::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        let dir = release.join(&relative);
        for item in std::fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))? {
            let item = item.map_err(|err| err.to_string())?;
            let relative = relative.join(item.file_name());
            let metadata = std::fs::symlink_metadata(item.path()).map_err(|err| err.to_string())?;
            let inside = format!("{mount}/{}", relative.display());
            if metadata.is_dir() {
                stack.push(relative);
            } else if metadata.is_file()
                && metadata.permissions().mode() & 0o111 != 0
                && inside != harness_exe
            {
                tools.insert(inside, format!("{RELEASE_COPY}/{}", relative.display()));
            }
        }
    }
    Ok(tools)
}

/// Directories the inner layer covers with a fresh tmpfs.
pub fn inner_masks() -> Vec<String> {
    [
        "/tmp",
        "/run",
        HOME,
        SOCK_DIR,
        PRIVATE_DIR,
        "/etc/claude-code",
        "/etc/codex",
    ]
    .map(String::from)
    .into()
}

/// A writable mount of a task env base: an overlay with the run's private
/// upper and work layers, or (the warm build) the base itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layer {
    Overlay {
        lower: PathBuf,
        upper: PathBuf,
        work: PathBuf,
    },
    Base(PathBuf),
}

/// The run's private layers, under `<run>/overlay`.
pub const LAYERS: [&str; 2] = ["cargo", "target"];

/// The Cargo home and target mounts of a run: overlays over the task env's
/// bases, or with `base_rw` the bases read-write.
pub fn layers(task: &TaskEnv, run: &Path, base_rw: bool) -> Vec<(Layer, &'static str)> {
    [(task.cargo(), CARGO_HOME), (task.target(), TARGET_DIR)]
        .into_iter()
        .zip(LAYERS)
        .map(|((base, dest), name)| {
            let layer = if base_rw {
                Layer::Base(base.to_path_buf())
            } else {
                let private = run.join("overlay").join(name);
                Layer::Overlay {
                    lower: base.to_path_buf(),
                    upper: private.join("upper"),
                    work: private.join("work"),
                }
            };
            (layer, dest)
        })
        .collect()
}

pub fn layer_args(layer: &Layer, dest: &str) -> Vec<String> {
    let path = |path: &Path| path.display().to_string();
    match layer {
        Layer::Overlay { lower, upper, work } => vec![
            "--overlay-src".into(),
            path(lower),
            "--overlay".into(),
            path(upper),
            path(work),
            dest.into(),
        ],
        Layer::Base(base) => vec!["--bind".into(), path(base), dest.into()],
    }
}

/// `<run>/overlay` may hold only `{cargo,target}/{upper,work}`, each an
/// empty directory, with no link on the way; absent parts are fine, and
/// only a missing path is absent.
pub fn check_overlay(run: &Path) -> Result {
    let overlay = run.join("overlay");
    crate::task_env::refuse_links(&overlay)?;
    match std::fs::symlink_metadata(&overlay) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("{}: {err}", overlay.display())),
        Ok(_) => {}
    }
    let entries = |dir: &Path| -> Result<Vec<PathBuf>> {
        let metadata =
            std::fs::symlink_metadata(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        if !metadata.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        std::fs::read_dir(dir)
            .map_err(|err| format!("{}: {err}", dir.display()))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|err| err.to_string())
            })
            .collect()
    };
    let named = |path: &Path, names: &[&str]| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| names.contains(&name))
    };
    for layer in entries(&overlay)? {
        if !named(&layer, &LAYERS) {
            return Err(format!(
                "{} is not a private layer; the run refuses it",
                layer.display()
            ));
        }
        for part in entries(&layer)? {
            if !named(&part, &["upper", "work"]) {
                return Err(format!(
                    "{} is not a private layer; the run refuses it",
                    part.display()
                ));
            }
            if !entries(&part)?.is_empty() {
                return Err(format!(
                    "{} is not empty; every run needs fresh private layers",
                    part.display()
                ));
            }
        }
    }
    Ok(())
}

/// The outer bwrap's arguments, ending with `-- /bench/jail/agent-jail init`.
/// Nothing is bound from the host's /nix/store: the store is the task's,
/// and bwrap, env and every inner tool resolve inside it.
#[allow(clippy::too_many_arguments)]
pub fn outer_args(
    loaded: &Loaded,
    home: &Home,
    host: &Host,
    run: &Path,
    tools: &BTreeMap<String, String>,
    env: &BTreeMap<String, String>,
    info_fd: i32,
    layers: &[(Layer, &str)],
    auth: &Path,
) -> Result<Vec<String>> {
    let path = |path: &Path| path.display().to_string();
    let mut args: Vec<String> = OUTER_LIFETIME.iter().map(|arg| (*arg).to_owned()).collect();
    args.extend(
        [
            "--unshare-user",
            "--unshare-net",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--new-session",
            "--clearenv",
            "--hostname",
            "bench",
        ]
        .map(String::from),
    );
    args.extend([
        "--ro-bind".into(),
        path(host.task.store_dir()),
        STORE.into(),
    ]);
    args.extend(
        [
            "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--tmpfs", "/run",
        ]
        .map(String::from),
    );
    args.extend(["--info-fd".into(), info_fd.to_string()]);
    args.extend(["--ro-bind".into(), path(&run.join("jail")), JAIL_DIR.into()]);
    args.extend([
        "--ro-bind".into(),
        path(&host.self_exe),
        format!("{JAIL_DIR}/agent-jail"),
    ]);
    args.extend([
        "--ro-bind".into(),
        path(&host.pins.host("bwrap")?),
        format!("{JAIL_DIR}/bwrap"),
    ]);
    args.extend(["--tmpfs".into(), PRIVATE_DIR.into()]);
    args.extend(["--ro-bind".into(), path(&run.join("sock")), SOCK_DIR.into()]);
    args.extend([
        "--bind".into(),
        path(&run.join("workspace")),
        WORKSPACE.into(),
    ]);
    for name in PROJECT_CONFIG {
        args.extend([
            "--ro-bind".into(),
            path(&run.join("workspace").join(name)),
            format!("{WORKSPACE}/{name}"),
        ]);
    }
    for (layer, dest) in layers {
        args.extend(layer_args(layer, dest));
    }
    if let Some((release, mount)) = release_dir(loaded, host)? {
        // The real release for the inner layer (masked there), then the
        // harness's view of it, whose other executables become wrappers.
        args.extend(["--ro-bind".into(), path(&release), RELEASE_COPY.into()]);
        args.extend(["--ro-bind".into(), path(&release), mount]);
    }
    args.extend(home.mounts(&run.join("home"), auth));
    for name in ["passwd", "group", "hosts"] {
        args.extend([
            "--ro-bind".into(),
            path(&run.join("jail/etc").join(name)),
            format!("/etc/{name}"),
        ]);
    }
    args.extend([
        "--ro-bind".into(),
        path(&run.join("jail").join(CA_COPY)),
        "/etc/ssl/certs/ca-certificates.crt".into(),
    ]);
    for wrapper in tools.keys() {
        args.extend(["--ro-bind".into(), path(&host.self_exe), wrapper.clone()]);
    }
    let env_tool = tools
        .get(&format!("{BIN_DIR}/env"))
        .ok_or("the inner packages provide no env")?;
    args.extend([
        "--ro-bind".into(),
        path(&crate::pins::host_path(
            host.task.store_dir(),
            Path::new(env_tool),
        )),
        "/usr/bin/env".into(),
    ]);
    for (name, value) in env {
        args.extend(["--setenv".into(), name.clone(), value.clone()]);
    }
    args.extend([
        "--chdir".into(),
        WORKSPACE.into(),
        "--remount-ro".into(),
        "/".into(),
    ]);
    args.extend(["--".into(), format!("{JAIL_DIR}/agent-jail"), "init".into()]);
    Ok(args)
}

/// The host CA bundle's copy in `<run>/jail` (followed once on the host).
pub const CA_COPY: &str = "ca-certificates.crt";
/// The empty file command mode mounts where the credential would be.
pub const NO_CREDENTIAL: &str = "no-credential";

/// The harness's environment: env.json's variables, then the profile's,
/// then the jail's own, which always win. Command mode has no proxy.
pub fn environment(loaded: &Loaded, task: &TaskEnv, mode: Mode) -> BTreeMap<String, String> {
    let mut env = task.variables().clone();
    env.extend(loaded.profile.env.clone());
    let proxy = format!("http://{}", crate::BRIDGE);
    for (name, value) in [
        ("HOME", HOME),
        ("USER", "bench"),
        ("LOGNAME", "bench"),
        ("PATH", BIN_DIR),
        ("SHELL", "/bin/bash"),
        ("TMPDIR", "/tmp"),
        // The Nix cc wrapper's purity check needs the build's top directory; `nix develop`
        // sets it to its temp dir, and the dev shell's own value (`/build`) never exists here.
        ("NIX_BUILD_TOP", "/tmp"),
        ("LANG", "C.UTF-8"),
        ("TERM", "dumb"),
        ("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt"),
        ("CARGO_HOME", CARGO_HOME),
        ("CARGO_TARGET_DIR", TARGET_DIR),
        ("CARGO_NET_OFFLINE", "true"),
        ("CARGO_INCREMENTAL", "0"),
    ] {
        env.insert(name.into(), value.into());
    }
    if mode == Mode::Command {
        return env;
    }
    for name in ["NO_PROXY", "no_proxy"] {
        env.insert(name.into(), String::new());
    }
    for name in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ] {
        env.insert(name.into(), proxy.clone());
    }
    env
}

fn write(path: &Path, text: &str) -> Result {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

/// What the post-run scan looks for.
pub struct Scan {
    pub secrets: Vec<Secret>,
    pub account_ids: Vec<String>,
    pub minutes_left: Option<i64>,
}

/// Prepares the run directory and returns the spec, the scan's needles and
/// the bwrap arguments (with `info_fd` as the info descriptor).
fn prepare(
    options: &Options,
    host: &Host,
    loaded: &Loaded,
    home: &Home,
    info_fd: i32,
) -> Result<(Spec, Scan, Vec<String>)> {
    let run = &options.run;
    for fresh in ["home", "meta", "sock", "jail"] {
        if std::fs::symlink_metadata(run.join(fresh)).is_ok() {
            return Err(format!(
                "{} already exists; every run needs a fresh run directory",
                run.join(fresh).display()
            ));
        }
    }
    check_overlay(run)?;
    let scan = if options.mode == Mode::Command {
        Scan {
            secrets: Vec::new(),
            account_ids: Vec::new(),
            minutes_left: None,
        }
    } else {
        let mut credential = String::new();
        File::open(&host.auth)
            .and_then(|file| file.take(1 << 20).read_to_string(&mut credential))
            .map_err(|_| {
                format!(
                    "{}: cannot read the credential file",
                    loaded.profile.harness
                )
            })?;
        let checked = crate::credential::gate(
            &loaded.profile.harness,
            &credential,
            host.now,
            options.wall_cap,
        )?;
        Scan {
            secrets: checked.secrets,
            account_ids: checked.account_ids,
            minutes_left: Some(checked.minutes_left),
        }
    };

    for dir in ["meta", "sock", "jail", "workspace"] {
        refuse_link(&run.join(dir))?;
        std::fs::create_dir_all(run.join(dir))
            .map_err(|err| format!("{}: {err}", run.join(dir).display()))?;
    }
    let layers = layers(&host.task, run, options.base_rw);
    for (layer, _) in &layers {
        if let Layer::Overlay { upper, work, .. } = layer {
            for dir in [upper, work] {
                std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
            }
        }
    }
    home.build(&run.join("home"))?;
    for name in PROJECT_CONFIG {
        let path = run.join("workspace").join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "the workspace's {name} is a symbolic link; the jail refuses it"
                ));
            }
            Ok(_) => {}
            Err(_) => {
                std::fs::create_dir(&path).map_err(|err| format!("{}: {err}", path.display()))?
            }
        }
    }

    let template: Vec<String> = options
        .template
        .map(|which| host.task.template(which).to_vec())
        .unwrap_or_default();
    let (exe, argv) = match options.mode {
        Mode::Harness => (harness_exe(loaded), loaded.fill(&options.slots)?),
        Mode::StandIn => (
            format!("{JAIL_DIR}/agent-jail"),
            vec!["probe-harness".into(), loaded.name.clone()],
        ),
        Mode::Command => {
            if template.is_empty() {
                return Err("env.json's template is empty".into());
            }
            (
                "/bin/sh".to_owned(),
                COMMAND_SCRIPT
                    .iter()
                    .map(|part| (*part).to_owned())
                    .chain(template.clone())
                    .collect(),
            )
        }
    };
    let mut tools = tool_map(&host.pins)?;
    if let Some((release, mount)) = release_dir(loaded, host)? {
        tools.extend(release_tools(&release, &mount, &harness_exe(loaded))?);
    }
    let mut read_only = home.read_only();
    read_only.extend(["/".into(), JAIL_DIR.into(), SOCK_DIR.into(), STORE.into()]);
    // The directories holding policy files (condition 7), not only the files.
    for (path, _) in home.policy() {
        if let Some(parent) = Path::new(&path)
            .parent()
            .map(|parent| parent.display().to_string())
            && !read_only.contains(&parent)
        {
            read_only.push(parent);
        }
    }
    read_only.extend(
        PROJECT_CONFIG
            .iter()
            .map(|name| format!("{WORKSPACE}/{name}")),
    );
    // Mount roots in the host store the probe may meet: agent-jail itself
    // when installed there, and for the host-test store every single file
    // bound out of it.
    let mut allowed_roots = Vec::new();
    if host.self_exe.starts_with(STORE) {
        allowed_roots.push(host.self_exe.display().to_string());
    }
    if *host.task.store() == Store::HostTest {
        allowed_roots.extend(tools.values().cloned());
        allowed_roots.push(host.pins.tool("bwrap")?.display().to_string());
    }
    let spec = Spec {
        profile: loaded.name.clone(),
        mode: options.mode,
        slots: options.slots.clone(),
        exe,
        argv,
        tools: tools.clone(),
        auth_path: home.manifest.auth.clone(),
        read_only,
        policy: home.policy().into_iter().collect(),
        hooks: crate::hook::guard_checks(&loaded.profile.harness, options.control),
        inner_masks: inner_masks(),
        dcg: host.pins.tool("dcg")?.display().to_string(),
        control: options.control,
        store: StoreSpec {
            store: host.task.store().clone(),
            exclude: host.task.exclude().to_vec(),
            allowed_roots,
            nix: host.pins.tool("nix")?.display().to_string(),
        },
        inner_path: host.task.path().to_vec(),
        template,
    };
    let jail = run.join("jail");
    write(
        &jail.join("spec.json"),
        &serde_json::to_string_pretty(&spec).map_err(|err| err.to_string())?,
    )?;
    write(&jail.join("empty-mcp.json"), "{\"mcpServers\":{}}\n")?;
    write(
        &jail.join("dcg/config.toml"),
        include_str!("../dcg-config.toml"),
    )?;
    let bundle = host.pins.host("ca-bundle")?;
    std::fs::copy(&bundle, jail.join(CA_COPY))
        .map_err(|err| format!("{}: {err}", bundle.display()))?;
    let auth = if options.mode == Mode::Command {
        write(&jail.join(NO_CREDENTIAL), "")?;
        jail.join(NO_CREDENTIAL)
    } else {
        host.auth.clone()
    };
    let netns = std::fs::read_link("/proc/self/ns/net")
        .map_err(|err| format!("host network namespace: {err}"))?;
    write(&jail.join("host-netns"), &netns.display().to_string())?;
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    write(
        &jail.join("etc/passwd"),
        &format!("bench:x:{uid}:{gid}:bench:{HOME}:/bin/bash\n"),
    )?;
    write(&jail.join("etc/group"), &format!("bench:x:{gid}:\n"))?;
    write(
        &jail.join("etc/hosts"),
        "127.0.0.1 localhost\n::1 localhost\n",
    )?;
    for placeholder in ["agent-jail", "bwrap"] {
        write(&jail.join(placeholder), "")?;
    }
    let args = outer_args(
        loaded,
        home,
        host,
        run,
        &tools,
        &environment(loaded, &host.task, options.mode),
        info_fd,
        &layers,
        &auth,
    )?;
    Ok((spec, scan, args))
}

/// Reads bwrap's info JSON until `child-pid` parses; bwrap may keep the
/// descriptor open, so this never waits for end of file.
fn read_child_pid(mut reader: std::io::PipeReader, timeout: Duration) -> Option<u32> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = Vec::new();
        let mut chunk = [0u8; 256];
        while let Ok(count) = reader.read(&mut chunk) {
            if count == 0 {
                break;
            }
            text.extend_from_slice(&chunk[..count]);
            if let Ok(value) = serde_json::from_slice::<Value>(&text) {
                let _ = sender.send(value.get("child-pid").and_then(Value::as_u64));
                return;
            }
        }
        let _ = sender.send(None);
    });
    receiver
        .recv_timeout(timeout)
        .ok()
        .flatten()
        .and_then(|pid| u32::try_from(pid).ok())
}

/// Runs one jail and returns its outcome.
pub fn enter(options: &Options, host: &Host) -> Result<Outcome> {
    // First, before this opens anything: what the host left open for us.
    let inherited = crate::fds::inherited()?;
    let mut loaded = crate::profile::load(&options.profile)?;
    if options.control != crate::control::Control::None {
        if !options.probe || options.mode != Mode::Harness {
            return Err("a control variant runs only in a harness probe (--probe)".into());
        }
        options.control.apply_profile(&mut loaded)?;
    }
    if options.wall_cap <= 0 {
        return Err("--wall-cap must be a positive number of minutes".into());
    }
    if (options.mode == Mode::Command) != options.template.is_some() {
        return Err("a template runs only in command mode, and command mode runs one".into());
    }
    if options.base_rw && options.template != Some(Template::Build) {
        return Err("--base-rw runs only command --template build".into());
    }
    check_self_exe(&host.self_exe)?;
    host.task.check_tools(&host.self_exe)?;
    // Exclusive for the warm build, shared for every other run, so no
    // --base-rw changes a lower layer under a mounted overlay.
    let _lock = host.task.lock(options.base_rw)?;
    if options.base_rw {
        host.task.check_base_rw()?;
    }
    let mut home = Home::shipped(&loaded.profile.harness)?;
    options.control.apply_home(&mut home)?;
    refuse_link(&options.run)?;
    std::fs::create_dir_all(&options.run)
        .map_err(|err| format!("{}: {err}", options.run.display()))?;
    let (info_read, info_write) = std::io::pipe().map_err(|err| err.to_string())?;
    let (_spec, scan, args) = prepare(options, host, &loaded, &home, info_write.as_raw_fd())?;

    let meta = options.run.join("meta");
    // Bound and spawned while enter is single-threaded, so clearing the
    // info pipe's close-on-exec races no other spawn; the proxy starts after.
    let listener = if options.mode == Mode::Command {
        None
    } else {
        Some(
            UnixListener::bind(options.run.join("sock/proxy"))
                .map_err(|err| format!("proxy socket: {err}"))?,
        )
    };
    // Every fallible step happens before the spawn: from the spawn on,
    // nothing returns before `supervise` has scanned the run.
    let create = |name: &str| File::create(meta.join(name)).map_err(|err| format!("{name}: {err}"));
    let stdout = create("harness.out")?;
    let stderr = create("harness.err")?;
    let proxy_log = if listener.is_some() {
        Some(create("proxy.jsonl")?)
    } else {
        None
    };
    rustix::io::fcntl_setfd(info_write.as_fd(), rustix::io::FdFlags::empty())
        .map_err(|err| err.to_string())?;
    let spawned = crate::fds::launcher(
        &host.pins.host("bash")?,
        &inherited,
        &host.pins.host("bwrap")?,
        &args,
    )
    .stdin(Stdio::null())
    .stdout(stdout)
    .stderr(stderr)
    .spawn();
    let _ = rustix::io::fcntl_setfd(info_write.as_fd(), rustix::io::FdFlags::CLOEXEC);
    drop(info_write);
    let child = spawned.map_err(|err| format!("cannot start bwrap: {err}"))?;
    let started = Instant::now();
    let proxy = match listener.zip(proxy_log) {
        Some((listener, log)) => {
            let proxy = Proxy::new(
                loaded.allowlist.clone(),
                log,
                host.limits,
                host.resolver.clone(),
            );
            proxy.serve(listener);
            Some(proxy)
        }
        None => None,
    };
    if let Some(pid) = read_child_pid(info_read, Duration::from_secs(10))
        && let Some(proxy) = &proxy
    {
        proxy.set_init_pid(pid);
    }
    let cap = Duration::from_secs(u64::try_from(options.wall_cap).unwrap_or(1) * 60);
    let (waited, findings, overlay_left) = supervise(
        child,
        started,
        cap,
        host.disk_floor,
        &options.run,
        &scan,
        !options.base_rw,
    )?;
    let outcome = Outcome {
        control: options.control,
        store: host.task.store().clone(),
        exit: crate::wrapper::exit_code(waited.status),
        killed_at_cap: waited.killed_at_cap,
        killed_at_disk_floor: waited.killed_at_disk_floor,
        quarantined: !findings.is_empty(),
        minutes_left: scan.minutes_left,
        overlay_left,
    };
    write(
        &meta.join("run.json"),
        &(serde_json::to_string_pretty(&outcome).map_err(|err| err.to_string())? + "\n"),
    )?;
    Ok(outcome)
}

/// Everything after the spawn: waits for the jail under `cap` and `floor`,
/// then finishes the run. Waiting can fail only while the jail may still
/// run, so a failure kills and reaps it first, and the run is always
/// scanned before any error returns.
fn supervise(
    mut child: std::process::Child,
    started: Instant,
    cap: Duration,
    floor: DiskFloor,
    run: &Path,
    scan: &Scan,
    overlays: bool,
) -> Result<(Waited, Vec<Finding>, Vec<String>)> {
    let waited = wait_capped(&mut child, started, cap, Some((run, floor)));
    if waited.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let finished = finish(run, scan, overlays);
    let waited = waited?;
    let (findings, overlay_left) = finished?;
    Ok((waited, findings, overlay_left))
}

/// After the jail: opens up the private layers (the harness could write
/// there too, so they are scanned like the rest of the run), scans the run
/// and quarantines, then deletes the layers. With `overlays` (every run but
/// --base-rw) `<run>/overlay` must be a directory. An error opening up is a
/// finding and never skips the scan; one deleting is returned for run.json.
fn finish(run: &Path, scan: &Scan, overlays: bool) -> Result<(Vec<Finding>, Vec<String>)> {
    let overlay = run.join("overlay");
    let finding = |form: String| Finding {
        path: PathBuf::from("overlay"),
        credential: "open-up".into(),
        form,
    };
    let mut findings = Vec::new();
    if overlays {
        match std::fs::symlink_metadata(&overlay) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => findings.push(finding(format!("{} is not a directory", overlay.display()))),
            Err(err) => findings.push(finding(format!("{}: {err}", overlay.display()))),
        }
    }
    findings.extend(crate::run_dir::open_up(&overlay).into_iter().map(finding));
    // The proxy socket is the one special file enter makes; the scan
    // reports it if it stays, and any other.
    let _ = std::fs::remove_file(run.join("sock/proxy"));
    findings.extend(crate::scan::scan(run, &scan.secrets, &scan.account_ids)?);
    crate::scan::quarantine(run, &findings, &run.join("meta/token-scan.json"))?;
    let overlay_left = match std::fs::remove_dir_all(&overlay) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            vec![format!("{}: {err}", overlay.display())]
        }
        _ => Vec::new(),
    };
    Ok((findings, overlay_left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_jail_must_load_from_the_store() {
        let dir = crate::test_dir("elf");
        let elf = crate::elf::tests::elf;
        let store = dir.join("store");
        std::fs::write(
            &store,
            elf(Some("/nix/store/x-glibc/lib/ld-linux-x86-64.so.2"), &[]),
        )
        .unwrap();
        assert!(check_self_exe(&store).is_ok());
        let host = dir.join("host");
        std::fs::write(&host, elf(Some("/lib64/ld-linux-x86-64.so.2"), &[])).unwrap();
        assert!(check_self_exe(&host).unwrap_err().contains("nix develop"));
        let not_elf = dir.join("x");
        std::fs::write(&not_elf, "#!/bin/sh\n").unwrap();
        assert!(check_self_exe(&not_elf).is_err());
    }

    /// A host on a fixture task env whose pins point at fake store files.
    fn fixture_host(dir: &Path) -> Host {
        let hash = crate::task_env::tests::HASH;
        let task = crate::task_env::tests::fixture(&dir.join("env"), &["cc", "tool"]);
        let jail = |name: &str| PathBuf::from(format!("/nix/store/{hash}-tool/bin/{name}"));
        let ca = dir.join("ca.crt");
        std::fs::write(&ca, "").unwrap();
        let tools: BTreeMap<String, PathBuf> = ["bwrap", "dcg", "bash", "nix"]
            .map(|name| (name.to_owned(), jail(name)))
            .into_iter()
            .chain([("ca-bundle".to_owned(), ca)])
            .collect();
        let inner: BTreeMap<String, PathBuf> = ["bash", "env", "ls"]
            .map(|name| (name.to_owned(), jail(name)))
            .into();
        Host {
            pins: Pins {
                store: task.store_dir().to_path_buf(),
                tools,
                inner,
            },
            self_exe: dir.join("agent-jail"),
            auth: dir.join("credential.json"),
            resolver: Proxy::system_resolver(),
            limits: Limits::default(),
            now: 0,
            task,
            disk_floor: DiskFloor::new(0),
        }
    }

    #[test]
    fn the_outer_layer_mounts_the_task_store_and_overlays_only() {
        let dir = crate::test_dir("outer-args");
        let host = fixture_host(&dir);
        let loaded = crate::profile::load("claude").unwrap();
        let home = Home::shipped("claude").unwrap();
        let run = dir.join("run");
        let tools = tool_map(&host.pins).unwrap();
        let env = environment(&loaded, &host.task, Mode::Harness);
        let args = outer_args(
            &loaded,
            &home,
            &host,
            &run,
            &tools,
            &env,
            9,
            &layers(&host.task, &run, false),
            &host.auth,
        )
        .unwrap();
        let joined = args.join(" ");
        let store = host.task.store_dir().display().to_string();
        let hash = crate::task_env::tests::HASH;
        for needle in [
            format!("--ro-bind {store} /nix/store"),
            format!(
                "--overlay-src {0}/env/cargo --overlay {1}/overlay/cargo/upper {1}/overlay/cargo/work /bench/cargo",
                dir.display(),
                run.display()
            ),
            format!(
                "--overlay-src {0}/env/target --overlay {1}/overlay/target/upper {1}/overlay/target/work /bench/target",
                dir.display(),
                run.display()
            ),
            format!("--ro-bind {store}/{hash}-tool/bin/bwrap /bench/jail/bwrap"),
            format!("--ro-bind {store}/{hash}-tool/bin/env /usr/bin/env"),
            format!(
                "--ro-bind {}/jail/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt",
                run.display()
            ),
        ] {
            assert!(joined.contains(&needle), "{needle} missing from {joined}");
        }
        // No bind source anywhere is a host store path.
        for pair in args.windows(2) {
            if matches!(pair[0].as_str(), "--ro-bind" | "--bind" | "--overlay-src") {
                assert!(
                    !pair[1].starts_with("/nix/store"),
                    "{} binds {}",
                    pair[0],
                    pair[1]
                );
            }
        }
        // The warm build binds the bases themselves.
        let base = layers(&host.task, &run, true);
        let target = format!("{}/env/target", dir.display());
        assert_eq!(
            layer_args(&base[1].0, base[1].1),
            ["--bind", target.as_str(), "/bench/target"]
        );
    }

    #[test]
    fn variables_follow_env_json_then_the_profile_then_the_jail() {
        let dir = crate::test_dir("environment");
        let host = fixture_host(&dir);
        let (_, mut file) = crate::task_env::EnvFile::read(host.task.dir()).unwrap();
        file.variables.extend(
            [
                ("CODEX_HOME", "/nix/store/x"),
                ("CARGO_NET_OFFLINE", "false"),
                ("PATH", "/nix/store/y"),
            ]
            .map(|(key, value)| (key.to_owned(), value.to_owned())),
        );
        std::fs::write(
            host.task.dir().join("env.json"),
            crate::task_env::json_bytes(&file).unwrap(),
        )
        .unwrap();
        let task = TaskEnv::load(host.task.dir()).unwrap();
        let codex = crate::profile::load("codex").unwrap();
        let env = environment(&codex, &task, Mode::Harness);
        assert_eq!(env["CC"], file.variables["CC"]);
        assert_eq!(env["CODEX_HOME"], "/home/bench/.codex");
        assert_eq!(env["PATH"], BIN_DIR);
        assert_eq!(env["CARGO_NET_OFFLINE"], "true");
        assert_eq!(env["CARGO_HOME"], "/bench/cargo");
        assert_eq!(env["CARGO_TARGET_DIR"], "/bench/target");
        assert_eq!(env["CARGO_INCREMENTAL"], "0");
        assert_eq!(env["NIX_BUILD_TOP"], "/tmp");
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:3128");
        let command = environment(&codex, &task, Mode::Command);
        for name in crate::spec::PROXY_VARS {
            assert!(!command.contains_key(*name), "{name}");
        }
    }

    #[test]
    fn private_layers_must_be_absent_or_empty() {
        let run = crate::test_dir("private-layers");
        assert!(check_overlay(&run).is_ok());
        for part in ["cargo/upper", "cargo/work", "target/upper"] {
            std::fs::create_dir_all(run.join("overlay").join(part)).unwrap();
        }
        assert!(check_overlay(&run).is_ok());
        std::fs::write(run.join("overlay/cargo/upper/x"), "").unwrap();
        assert!(check_overlay(&run).unwrap_err().contains("not empty"));
        std::fs::remove_file(run.join("overlay/cargo/upper/x")).unwrap();
        std::fs::create_dir(run.join("overlay/other")).unwrap();
        assert!(
            check_overlay(&run)
                .unwrap_err()
                .contains("not a private layer")
        );
        std::fs::remove_dir(run.join("overlay/other")).unwrap();
        std::fs::create_dir(run.join("overlay/target/elsewhere")).unwrap();
        assert!(
            check_overlay(&run)
                .unwrap_err()
                .contains("not a private layer")
        );
        std::fs::remove_dir(run.join("overlay/target/elsewhere")).unwrap();
        std::os::unix::fs::symlink("/tmp", run.join("overlay/target/work")).unwrap();
        assert!(check_overlay(&run).unwrap_err().contains("not a directory"));
        std::fs::remove_file(run.join("overlay/target/work")).unwrap();
        let linked = crate::test_dir("private-layers-link");
        std::os::unix::fs::symlink(run.join("overlay"), linked.join("overlay")).unwrap();
        assert!(
            check_overlay(&linked)
                .unwrap_err()
                .contains("symbolic link")
        );
        // Only a missing overlay is absent: under a file it cannot be looked at.
        let file = linked.join("file");
        std::fs::write(&file, "").unwrap();
        assert!(
            check_overlay(&file)
                .unwrap_err()
                .contains("Not a directory")
        );
    }

    fn no_secrets() -> Scan {
        Scan {
            secrets: Vec::new(),
            account_ids: Vec::new(),
            minutes_left: None,
        }
    }

    /// A finished run directory: meta/ and the four empty private layers.
    fn finished_run(name: &str) -> PathBuf {
        let run = crate::test_dir(name);
        std::fs::create_dir(run.join("meta")).unwrap();
        for part in ["cargo/upper", "cargo/work", "target/upper", "target/work"] {
            std::fs::create_dir_all(run.join("overlay").join(part)).unwrap();
        }
        run
    }

    #[test]
    fn a_missing_or_odd_overlay_quarantines_a_run_that_mounted_one() {
        let run = finished_run("finish-clean");
        let (findings, left) = finish(&run, &no_secrets(), true).unwrap();
        assert!(findings.is_empty() && left.is_empty(), "{findings:?}");
        assert!(!run.join("overlay").exists());
        // --base-rw mounts no overlay: none is expected.
        assert!(finish(&run, &no_secrets(), false).unwrap().0.is_empty());
        // Every other run must find it as a directory.
        let quarantined = |run: &Path| {
            let findings = finish(run, &no_secrets(), true).unwrap().0;
            let report = std::fs::read_to_string(run.join("meta/token-scan.json")).unwrap();
            assert!(report.contains("\"quarantined\": true"), "{report}");
            findings
        };
        let found = quarantined(&run);
        assert!(found[0].form.contains("No such file"), "{found:?}");
        std::fs::write(run.join("overlay"), "").unwrap();
        let found = quarantined(&run);
        assert!(found[0].form.contains("not a directory"), "{found:?}");
        let elsewhere = crate::test_dir("finish-elsewhere");
        std::fs::write(elsewhere.join("keep"), "x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, run.join("overlay")).unwrap();
        let found = quarantined(&run);
        assert!(found[0].form.contains("not a directory"), "{found:?}");
        assert!(elsewhere.join("keep").exists());
        // A special file in an upper layer is opened up as a finding too.
        let run = finished_run("finish-fifo");
        crate::mknod(
            &run.join("overlay/target/upper/fifo"),
            rustix::fs::FileType::Fifo,
            0,
        )
        .unwrap();
        let forms: Vec<(String, String)> = quarantined(&run)
            .into_iter()
            .map(|finding| (finding.credential, finding.form))
            .collect();
        assert!(
            forms
                .iter()
                .any(|(label, form)| label == "open-up" && form.contains("a fifo")),
            "{forms:?}"
        );
        assert!(
            forms.iter().any(|(label, _)| label == "special-file"),
            "{forms:?}"
        );
    }

    #[test]
    fn a_failed_wait_still_tears_down_and_scans_the_run() {
        let run = finished_run("supervise");
        std::fs::write(
            run.join("overlay/target/upper/leak"),
            format!("sk-ant-oat01-{}", "x".repeat(48)),
        )
        .unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        // Reaped behind the child's back: waiting on it can only fail.
        rustix::process::waitpid(
            Some(rustix::process::Pid::from_child(&child)),
            rustix::process::WaitOptions::empty(),
        )
        .unwrap();
        assert!(child.try_wait().is_err());
        let result = supervise(
            child,
            Instant::now(),
            Duration::from_secs(60),
            DiskFloor::new(0),
            &run,
            &no_secrets(),
            true,
        );
        assert!(result.is_err());
        let report = std::fs::read_to_string(run.join("meta/token-scan.json")).unwrap();
        assert!(report.contains("\"quarantined\": true"), "{report}");
        assert!(!run.join("overlay").exists());
    }

    #[test]
    fn the_disk_floor_is_checked_at_once_and_then_every_interval() {
        let sleeper = || {
            std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap()
        };
        let dir = crate::test_dir("disk-floor");
        let started = Instant::now();
        // A floor no filesystem meets kills at the first check.
        let unreachable = DiskFloor::new(u64::MAX);
        let waited = wait_capped(
            &mut sleeper(),
            started,
            Duration::from_secs(20),
            Some((&dir, unreachable)),
        )
        .unwrap();
        assert!(waited.killed_at_disk_floor && !waited.killed_at_cap);
        assert!(!waited.status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
        // A floor met at first, then not (an unreadable filesystem counts
        // as full), kills at the next check.
        let started = Instant::now();
        let gone = dir.clone();
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::remove_dir(gone).unwrap();
        });
        let every = DiskFloor {
            bytes: 1,
            every: Duration::from_millis(20),
        };
        let waited = wait_capped(
            &mut sleeper(),
            started,
            Duration::from_secs(5),
            Some((&dir, every)),
        )
        .unwrap();
        remover.join().unwrap();
        assert!(waited.killed_at_disk_floor && !waited.killed_at_cap);
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn base_rw_and_templates_belong_to_command_mode() {
        let dir = crate::test_dir("base-rw");
        let host = fixture_host(&dir);
        let options = Options {
            profile: "claude".into(),
            run: dir.join("run"),
            wall_cap: 5,
            probe: false,
            mode: Mode::Command,
            slots: BTreeMap::new(),
            control: crate::control::Control::None,
            template: Some(Template::Test),
            base_rw: true,
        };
        assert!(enter(&options, &host).unwrap_err().contains("--base-rw"));
        let harness = Options {
            mode: Mode::Harness,
            base_rw: false,
            ..options.clone()
        };
        assert!(enter(&harness, &host).unwrap_err().contains("template"));
        let no_template = Options {
            template: None,
            base_rw: false,
            ..options
        };
        assert!(enter(&no_template, &host).unwrap_err().contains("template"));
        assert!(!dir.join("run").exists());
    }

    #[test]
    fn linked_run_directories_are_refused() {
        let dir = crate::test_dir("refuse-link");
        std::os::unix::fs::symlink("/home", dir.join("workspace")).unwrap();
        assert!(
            refuse_link(&dir.join("workspace"))
                .unwrap_err()
                .contains("symbolic link")
        );
        assert!(refuse_link(&dir.join("absent")).is_ok());
        std::fs::create_dir(dir.join("real")).unwrap();
        assert!(refuse_link(&dir.join("real")).is_ok());
    }

    #[test]
    fn a_quarantined_run_never_exits_zero() {
        let outcome = |exit, quarantined| Outcome {
            control: crate::control::Control::None,
            store: Store::HostTest,
            exit,
            killed_at_cap: false,
            killed_at_disk_floor: false,
            quarantined,
            minutes_left: Some(60),
            overlay_left: Vec::new(),
        };
        assert_eq!(exit_status(&outcome(0, false)), 0);
        assert_eq!(exit_status(&outcome(0, true)), QUARANTINE_EXIT);
        assert_eq!(exit_status(&outcome(3, true)), QUARANTINE_EXIT);
        assert_eq!(exit_status(&outcome(7, false)), 7);
    }

    #[test]
    fn release_executables_other_than_the_harness_become_wrappers() {
        let release = crate::test_dir("release");
        for (name, mode) in [
            ("bin/codex", 0o755),
            ("bin/codex-code-mode-host", 0o755),
            ("codex-path/rg", 0o755),
            ("codex-package.json", 0o644),
        ] {
            use std::os::unix::fs::PermissionsExt;
            let file = release.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "x").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        std::os::unix::fs::symlink("bin/codex", release.join("codex")).unwrap();
        let tools = release_tools(
            &release,
            "/bench/tools/codex",
            "/bench/tools/codex/bin/codex",
        )
        .unwrap();
        let expected: BTreeMap<String, String> = [
            (
                "/bench/tools/codex/bin/codex-code-mode-host",
                "/bench/jail-private/harness-release/bin/codex-code-mode-host",
            ),
            (
                "/bench/tools/codex/codex-path/rg",
                "/bench/jail-private/harness-release/codex-path/rg",
            ),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .into();
        assert_eq!(tools, expected);
    }

    #[test]
    fn harness_exes_come_from_profiles_and_pins() {
        assert!(harness_exe(&crate::profile::load("claude").unwrap()).starts_with("/nix/store/"));
        assert_eq!(
            harness_exe(&crate::profile::load("codex").unwrap()),
            "/bench/tools/codex/bin/codex"
        );
    }
}
