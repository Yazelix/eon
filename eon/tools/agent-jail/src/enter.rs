//! `agent-jail enter`: the host side. It checks the credential's lifetime,
//! builds a fresh run directory and home, starts the allowlist proxy, runs
//! the outer bwrap with init as PID 1, enforces the wall cap, then scans the
//! run for token material.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::manifest::Home;
use crate::pins::Pins;
use crate::profile::Loaded;
use crate::proxy::{Limits, Proxy, Resolver};
use crate::spec::{Mode, Spec};
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
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub control: crate::control::Control,
    pub exit: i32,
    pub killed_at_cap: bool,
    pub quarantined: bool,
    pub minutes_left: i64,
}

/// The outer bwrap flags that make the jail's lifetime init's: init is the
/// PID namespace's PID 1 and dies with `enter`'s bwrap child.
pub const OUTER_LIFETIME: &[&str] = &["--unshare-pid", "--as-pid-1", "--die-with-parent"];

/// Waits for the outer bwrap, killing it once `cap` has passed since
/// `started`. Its `--die-with-parent` takes init with it, and init is the
/// PID namespace's PID 1, so the whole jail goes.
pub fn wait_capped(
    child: &mut std::process::Child,
    started: Instant,
    cap: Duration,
) -> Result<(std::process::ExitStatus, bool)> {
    let mut killed_at_cap = false;
    loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            return Ok((status, killed_at_cap));
        }
        if started.elapsed() > cap && !killed_at_cap {
            killed_at_cap = true;
            let _ = child.kill();
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

/// The ELF interpreter of `path`, if it names one (`PT_INTERP`).
pub fn elf_interpreter(path: &Path) -> Result<Option<String>> {
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let field = |offset: usize, size: usize| -> Result<u64> {
        let slice = bytes
            .get(offset..offset + size)
            .ok_or("truncated ELF header")?;
        Ok(slice
            .iter()
            .rev()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)))
    };
    if bytes.get(..5) != Some(b"\x7fELF\x02".as_slice()) || bytes.get(5) != Some(&1) {
        return Err(format!(
            "{} is not a little-endian 64-bit ELF file",
            path.display()
        ));
    }
    let (phoff, phentsize, phnum) = (field(0x20, 8)?, field(0x36, 2)?, field(0x38, 2)?);
    for index in 0..phnum {
        let header = usize::try_from(phoff + index * phentsize).map_err(|err| err.to_string())?;
        if field(header, 4)? == 3 {
            let offset = usize::try_from(field(header + 8, 8)?).map_err(|err| err.to_string())?;
            let size = usize::try_from(field(header + 32, 8)?).map_err(|err| err.to_string())?;
            let raw = bytes
                .get(offset..offset + size)
                .ok_or("truncated PT_INTERP")?;
            return Ok(Some(
                String::from_utf8_lossy(raw)
                    .trim_end_matches('\0')
                    .to_owned(),
            ));
        }
    }
    Ok(None)
}

/// The jail mounts only `/nix/store`, so agent-jail must be static or load
/// from the store (built in the repository's dev shell).
pub fn check_self_exe(path: &Path) -> Result {
    match elf_interpreter(path)? {
        Some(interpreter) if !interpreter.starts_with("/nix/store/") => Err(format!(
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

/// The outer bwrap's arguments, ending with `-- /bench/jail/agent-jail init`.
#[allow(clippy::too_many_arguments)]
pub fn outer_args(
    loaded: &Loaded,
    home: &Home,
    host: &Host,
    run: &Path,
    tools: &BTreeMap<String, String>,
    env: &BTreeMap<String, String>,
    info_fd: i32,
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
            "--ro-bind",
            "/nix/store",
            "/nix/store",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--tmpfs",
            "/run",
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
        path(host.pins.tool("bwrap")?),
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
    if let Some((release, mount)) = release_dir(loaded, host)? {
        // The real release for the inner layer (masked there), then the
        // harness's view of it, whose other executables become wrappers.
        args.extend(["--ro-bind".into(), path(&release), RELEASE_COPY.into()]);
        args.extend(["--ro-bind".into(), path(&release), mount]);
    }
    args.extend(home.mounts(&run.join("home"), &host.auth));
    for name in ["passwd", "group", "hosts"] {
        args.extend([
            "--ro-bind".into(),
            path(&run.join("jail/etc").join(name)),
            format!("/etc/{name}"),
        ]);
    }
    args.extend([
        "--ro-bind".into(),
        path(host.pins.tool("ca-bundle")?),
        "/etc/ssl/certs/ca-certificates.crt".into(),
    ]);
    for wrapper in tools.keys() {
        args.extend(["--ro-bind".into(), path(&host.self_exe), wrapper.clone()]);
    }
    let env_tool = tools
        .get(&format!("{BIN_DIR}/env"))
        .ok_or("the inner packages provide no env")?;
    args.extend(["--ro-bind".into(), env_tool.clone(), "/usr/bin/env".into()]);
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

/// The harness's environment: the profile's plus the jail's own.
pub fn environment(loaded: &Loaded) -> BTreeMap<String, String> {
    let mut env = loaded.profile.env.clone();
    let proxy = format!("http://{}", crate::BRIDGE);
    for (name, value) in [
        ("HOME", HOME),
        ("USER", "bench"),
        ("LOGNAME", "bench"),
        ("PATH", BIN_DIR),
        ("SHELL", "/bin/bash"),
        ("TMPDIR", "/tmp"),
        ("LANG", "C.UTF-8"),
        ("TERM", "dumb"),
        ("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt"),
        ("NO_PROXY", ""),
        ("no_proxy", ""),
    ] {
        env.insert(name.into(), value.into());
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

/// Prepares the run directory and returns the spec, the gate result and
/// the bwrap arguments (with `info_fd` as the info descriptor).
fn prepare(
    options: &Options,
    host: &Host,
    loaded: &Loaded,
    home: &Home,
    info_fd: i32,
) -> Result<(Spec, crate::credential::Checked, Vec<String>)> {
    let run = &options.run;
    for fresh in ["home", "meta", "sock", "jail"] {
        if std::fs::symlink_metadata(run.join(fresh)).is_ok() {
            return Err(format!(
                "{} already exists; every run needs a fresh run directory",
                run.join(fresh).display()
            ));
        }
    }
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
    drop(credential);

    for dir in ["meta", "sock", "jail", "workspace"] {
        refuse_link(&run.join(dir))?;
        std::fs::create_dir_all(run.join(dir))
            .map_err(|err| format!("{}: {err}", run.join(dir).display()))?;
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

    let (exe, argv) = match options.mode {
        Mode::Harness => (harness_exe(loaded), loaded.fill(&options.slots)?),
        Mode::StandIn => (
            format!("{JAIL_DIR}/agent-jail"),
            vec!["probe-harness".into(), loaded.name.clone()],
        ),
    };
    let mut tools = tool_map(&host.pins)?;
    if let Some((release, mount)) = release_dir(loaded, host)? {
        tools.extend(release_tools(&release, &mount, &harness_exe(loaded))?);
    }
    let mut read_only = home.read_only();
    read_only.extend(["/".into(), JAIL_DIR.into(), SOCK_DIR.into()]);
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
        &environment(loaded),
        info_fd,
    )?;
    Ok((spec, checked, args))
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

/// Runs one jailed harness and returns its outcome.
pub fn enter(options: &Options, host: &Host) -> Result<Outcome> {
    let mut loaded = crate::profile::load(&options.profile)?;
    if options.control != crate::control::Control::None {
        if !options.probe || options.mode != Mode::Harness {
            return Err("a control variant runs only in a harness probe (--probe)".into());
        }
        options.control.apply_profile(&mut loaded)?;
    }
    if loaded.probe_only() && !options.probe {
        return Err(format!(
            "profile {} is probe-only in M2 (the host store is visible); pass --probe",
            loaded.name
        ));
    }
    if options.wall_cap <= 0 {
        return Err("--wall-cap must be a positive number of minutes".into());
    }
    check_self_exe(&host.self_exe)?;
    let mut home = Home::shipped(&loaded.profile.harness)?;
    options.control.apply_home(&mut home)?;
    refuse_link(&options.run)?;
    std::fs::create_dir_all(&options.run)
        .map_err(|err| format!("{}: {err}", options.run.display()))?;
    let (info_read, info_write) = std::io::pipe().map_err(|err| err.to_string())?;
    let (_spec, checked, args) = prepare(options, host, &loaded, &home, info_write.as_raw_fd())?;

    let meta = options.run.join("meta");
    let listener = UnixListener::bind(options.run.join("sock/proxy"))
        .map_err(|err| format!("proxy socket: {err}"))?;
    let log = File::create(meta.join("proxy.jsonl")).map_err(|err| err.to_string())?;
    let proxy = Proxy::new(
        loaded.allowlist.clone(),
        log,
        host.limits,
        host.resolver.clone(),
    );
    proxy.serve(listener);

    let stdout = File::create(meta.join("harness.out")).map_err(|err| err.to_string())?;
    let stderr = File::create(meta.join("harness.err")).map_err(|err| err.to_string())?;
    rustix::io::fcntl_setfd(info_write.as_fd(), rustix::io::FdFlags::empty())
        .map_err(|err| err.to_string())?;
    let spawned = Command::new(host.pins.tool("bwrap")?)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn();
    let _ = rustix::io::fcntl_setfd(info_write.as_fd(), rustix::io::FdFlags::CLOEXEC);
    drop(info_write);
    let mut child = spawned.map_err(|err| format!("cannot start bwrap: {err}"))?;
    let started = Instant::now();
    if let Some(pid) = read_child_pid(info_read, Duration::from_secs(10)) {
        proxy.set_init_pid(pid);
    }
    let cap = Duration::from_secs(u64::try_from(options.wall_cap).unwrap_or(1) * 60);
    let (status, killed_at_cap) = wait_capped(&mut child, started, cap)?;
    let findings = crate::scan::scan(&options.run, &checked.secrets, &checked.account_ids)?;
    crate::scan::quarantine(&options.run, &findings, &meta.join("token-scan.json"))?;
    let outcome = Outcome {
        control: options.control,
        exit: crate::wrapper::exit_code(status),
        killed_at_cap,
        quarantined: !findings.is_empty(),
        minutes_left: checked.minutes_left,
    };
    write(
        &meta.join("run.json"),
        &(serde_json::to_string_pretty(&outcome).map_err(|err| err.to_string())? + "\n"),
    )?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal little-endian ELF64 file with one PT_INTERP segment.
    fn elf(interpreter: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x40 + 0x38];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
        bytes[0x36..0x38].copy_from_slice(&0x38u16.to_le_bytes());
        bytes[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
        let header = 0x40;
        bytes[header..header + 4].copy_from_slice(&3u32.to_le_bytes());
        let offset = bytes.len() as u64;
        bytes[header + 8..header + 16].copy_from_slice(&offset.to_le_bytes());
        let text = format!("{interpreter}\0");
        bytes[header + 32..header + 40].copy_from_slice(&(text.len() as u64).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    #[test]
    fn elf_interpreters_are_read() {
        let dir = crate::test_dir("elf");
        let store = dir.join("store");
        std::fs::write(&store, elf("/nix/store/x-glibc/lib/ld-linux-x86-64.so.2")).unwrap();
        assert_eq!(
            elf_interpreter(&store).unwrap().as_deref(),
            Some("/nix/store/x-glibc/lib/ld-linux-x86-64.so.2")
        );
        assert!(check_self_exe(&store).is_ok());
        let host = dir.join("host");
        std::fs::write(&host, elf("/lib64/ld-linux-x86-64.so.2")).unwrap();
        assert!(check_self_exe(&host).unwrap_err().contains("nix develop"));
        let not_elf = dir.join("x");
        std::fs::write(&not_elf, "#!/bin/sh\n").unwrap();
        assert!(elf_interpreter(&not_elf).is_err());
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
            exit,
            killed_at_cap: false,
            quarantined,
            minutes_left: 60,
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

    #[test]
    fn the_environment_points_every_proxy_variable_at_the_bridge() {
        let env = environment(&crate::profile::load("codex").unwrap());
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:3128");
        assert_eq!(env["PATH"], BIN_DIR);
        assert_eq!(env["CODEX_HOME"], "/home/bench/.codex");
        assert!(!env.contains_key("SSH_AUTH_SOCK"));
    }
}
