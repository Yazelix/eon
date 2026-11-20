//! The structural probe. `agent-jail probe` runs the jail with a stand-in
//! harness (`probe-harness`, the harness PID) that checks the outer layer,
//! starts non-harness outer processes (`probe-peer`) for the owner checks,
//! and runs `probe-inner` through `/bin/sh` for the inner layer, which also
//! tries to fetch with a real Nix client and with Cargo, each next to a
//! positive control with the same client and flags. Each check prints one
//! JSON line; the host adds the proxy-log expectations and writes
//! `meta/probe.json`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::manifest::Home;
use crate::spec::{Spec, StoreSpec};
use crate::task_env::{STORE, Store};
use crate::{BIN_DIR, HOME, JAIL_DIR, PRIVATE_DIR, Result, SOCK_DIR, WORKSPACE};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    pub layer: String,
    pub name: String,
    pub expected: String,
    pub observed: String,
    pub pass: bool,
}

struct Reporter {
    layer: &'static str,
}

impl Reporter {
    fn report(&self, name: &str, expected: &str, observed: impl Into<String>, pass: bool) {
        let check = Check {
            layer: self.layer.into(),
            name: name.into(),
            expected: expected.into(),
            observed: observed.into(),
            pass,
        };
        if let Ok(text) = serde_json::to_string(&check) {
            println!("{text}");
        }
    }

    /// `result` must be an error.
    fn fails<T>(&self, name: &str, result: std::io::Result<T>) {
        match result {
            Ok(_) => self.report(name, "fails", "succeeded", false),
            Err(err) => self.report(name, "fails", err.to_string(), true),
        }
    }
}

const TIMEOUT: Duration = Duration::from_secs(3);

fn tcp(address: &str) -> std::io::Result<TcpStream> {
    let address: SocketAddr = address.parse().map_err(std::io::Error::other)?;
    TcpStream::connect_timeout(&address, TIMEOUT)
}

fn udp_dns() -> std::io::Result<usize> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.send_to(&[0u8; 12], "1.1.1.1:53")?;
    let mut reply = [0u8; 512];
    socket.recv(&mut reply)
}

fn resolve(host: &str) -> std::io::Result<Vec<SocketAddr>> {
    (host, 443).to_socket_addrs().map(Iterator::collect)
}

/// Sends `request` through the bridge and returns the reply's first line.
pub fn through_bridge(request: &str) -> std::io::Result<String> {
    let mut stream = tcp(crate::BRIDGE)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.write_all(request.as_bytes())?;
    let mut reply = [0u8; 256];
    let count = stream.read(&mut reply)?;
    Ok(String::from_utf8_lossy(&reply[..count])
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned())
}

fn status_line(result: std::io::Result<String>) -> String {
    match result {
        Ok(line) if line.is_empty() => "closed".into(),
        Ok(line) => line,
        Err(err) => format!("error: {err}"),
    }
}

fn model_host(profile: &str) -> Result<String> {
    let loaded = crate::profile::load(profile)?;
    loaded
        .allowlist
        .hosts()
        .first()
        .cloned()
        .ok_or("the allowlist is empty".into())
}

fn policy_dir(harness: &str) -> &'static str {
    if harness == "claude" {
        "/etc/claude-code"
    } else {
        "/etc/codex"
    }
}

fn config_dir(harness: &str) -> String {
    format!("{HOME}/.{harness}")
}

fn sockets_under(root: &Path, found: &mut Vec<String>) {
    use std::os::unix::fs::FileTypeExt;
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if ["/proc", "/sys", "/dev", "/nix"]
            .iter()
            .any(|skip| path == Path::new(skip))
        {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_socket() {
            found.push(path.display().to_string());
        } else if metadata.is_dir() {
            sockets_under(&path, found);
        }
    }
}

fn cap_eff() -> String {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| {
            line.strip_prefix("CapEff:")
                .map(|value| value.trim().to_owned())
        })
        .unwrap_or_default()
}

fn empty_dir(path: &str) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

fn abstract_x11() -> std::io::Result<()> {
    use std::os::linux::net::SocketAddrExt;
    let address = std::os::unix::net::SocketAddr::from_abstract_name(b"/tmp/.X11-unix/X0")?;
    std::os::unix::net::UnixStream::connect_addr(&address).map(drop)
}

/// The Nix client variables neither layer may carry.
pub const NIX_CLIENT_VARS: &[&str] = &[
    "NIX_REMOTE",
    "NIX_CONFIG",
    "NIX_PATH",
    "NIX_STORE_DIR",
    "NIX_STATE_DIR",
    "NIX_CONF_DIR",
    "NIX_USER_CONF_FILES",
    "NIX_DAEMON_SOCKET_PATH",
    "NIX_SSL_CERT_FILE",
];

/// The store mount check on a `/proc/self/mountinfo` text: exactly one
/// mount at /nix/store, which is the task store (or for the host-test
/// store the host's own), and no other mount whose root lies in the host's
/// /nix/store unless the spec allows that root.
pub fn store_mounts(mountinfo: &str, store: &StoreSpec) -> std::result::Result<(), String> {
    let mounts: Vec<(String, String)> = mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            Some((
                crate::seal::unescape(fields.get(3)?),
                crate::seal::unescape(fields.get(4)?),
            ))
        })
        .collect();
    let host_store = |root: &str| root == STORE || root.starts_with(&format!("{STORE}/"));
    let mut problems = Vec::new();
    let at_store: Vec<&String> = mounts
        .iter()
        .filter(|(_, point)| point == STORE)
        .map(|(root, _)| root)
        .collect();
    match (at_store.as_slice(), &store.store) {
        ([root], Store::Task { .. }) if host_store(root) => {
            problems.push(format!("/nix/store is the host store ({root})"));
        }
        ([root], Store::HostTest) if root.as_str() != STORE => {
            problems.push(format!("/nix/store mounts {root}, not the host store"));
        }
        ([_], _) => {}
        (roots, _) => problems.push(format!("{} mounts at /nix/store", roots.len())),
    }
    for (root, point) in &mounts {
        if point != STORE && host_store(root) && !store.allowed_roots.contains(root) {
            problems.push(format!("{point} is bound from the host store's {root}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Excluded packages in `store` or on `PATH`; a store or PATH entry it
/// cannot read is an error, never a clean answer.
fn excluded_found(store: &Path, exclude: &[String]) -> std::result::Result<Vec<String>, String> {
    let mut found = Vec::new();
    if exclude.is_empty() {
        return Ok(found);
    }
    let unreadable = |path: &Path, err: std::io::Error| format!("{}: {err}", path.display());
    for entry in std::fs::read_dir(store).map_err(|err| unreadable(store, err))? {
        let name = entry
            .map_err(|err| unreadable(store, err))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if exclude
            .iter()
            .any(|pname| crate::task_env::is_package(&name, pname))
        {
            found.push(store.join(name).display().to_string());
        }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':').filter(|dir| !dir.is_empty()) {
        for pname in exclude {
            let tool = Path::new(dir).join(pname);
            match std::fs::symlink_metadata(&tool) {
                Ok(_) => found.push(tool.display().to_string()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(unreadable(&tool, err)),
            }
        }
    }
    Ok(found)
}

/// Checks both layers share: the network, host paths, sockets, variables,
/// the store.
fn common(reporter: &Reporter, spec: Option<&Spec>) {
    let interfaces: Vec<String> = std::fs::read_to_string("/proc/net/dev")
        .unwrap_or_default()
        .lines()
        .skip(2)
        .filter_map(|line| line.split(':').next().map(|name| name.trim().to_owned()))
        .collect();
    reporter.report(
        "only lo",
        "[\"lo\"]",
        format!("{interfaces:?}"),
        interfaces == ["lo"],
    );
    reporter.fails("tcp 140.82.112.3:443", tcp("140.82.112.3:443"));
    reporter.fails("tcp [2606:4700::1111]:443", tcp("[2606:4700::1111]:443"));
    reporter.fails("udp 1.1.1.1:53", udp_dns());
    reporter.fails("resolve github.com", resolve("github.com"));
    reporter.fails("abstract @/tmp/.X11-unix/X0", abstract_x11());
    for path in [
        "/home/lucca",
        "/home/lucca/pjs/eon/eon",
        "/home/lucca/.cache/eon-agents",
        "/home/lucca/.secrets",
        "/mnt/build",
        "/run/user/1000",
        "/nix/var/nix/daemon-socket",
        "/nix/var",
        "/bench/bin/br",
        "/bench/bin/nix",
    ] {
        let present = std::fs::symlink_metadata(path).is_ok();
        reporter.report(
            &format!("absent {path}"),
            "absent",
            if present { "present" } else { "absent" },
            !present,
        );
    }
    for name in [
        "SSH_AUTH_SOCK",
        "GPG_AGENT_INFO",
        "DBUS_SESSION_BUS_ADDRESS",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_RUNTIME_DIR",
    ]
    .iter()
    .chain(NIX_CLIENT_VARS)
    {
        let set = std::env::var_os(name).is_some();
        reporter.report(
            &format!("unset {name}"),
            "unset",
            if set { "set" } else { "unset" },
            !set,
        );
    }
    let caps = cap_eff();
    reporter.report(
        "CapEff",
        "0000000000000000",
        caps.clone(),
        caps == "0000000000000000",
    );
    write_fails(reporter, &format!("{STORE}/agent-jail-probe"));
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mounts = spec
        .ok_or("no spec".to_owned())
        .and_then(|spec| store_mounts(&mountinfo, &spec.store));
    reporter.report(
        "no host store mount",
        "the task store at /nix/store, nothing bound from the host store",
        format!("{mounts:?}"),
        mounts.is_ok(),
    );
    let found = spec
        .ok_or("no spec".to_owned())
        .and_then(|spec| excluded_found(Path::new(STORE), &spec.store.exclude));
    reporter.report(
        "no excluded package in the store or on PATH",
        "[]",
        format!("{found:?}"),
        found.is_ok_and(|found| found.is_empty()),
    );
}

fn write_fails(reporter: &Reporter, path: &str) {
    reporter.fails(
        &format!("write {path}"),
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path),
    );
}

/// `agent-jail probe-harness <profile>`: the outer layer, as the harness.
pub fn probe_harness(profile: &str) -> i32 {
    let reporter = Reporter { layer: "outer" };
    let Ok(loaded) = crate::profile::load(profile) else {
        return 1;
    };
    let harness = loaded.profile.harness.clone();
    let Ok(home) = Home::shipped(&harness) else {
        return 1;
    };
    let Ok(host) = model_host(profile) else {
        return 1;
    };
    let spec = Spec::read().ok();
    common(&reporter, spec.as_ref());
    let open = crate::fds::open_beyond(&[]);
    reporter.report(
        "no inherited descriptor >= 3",
        "[]",
        format!("{open:?}"),
        matches!(&open, Ok(open) if open.is_empty()),
    );

    // The harness's own route out, and the proxy's refusals.
    let line = status_line(through_bridge(&format!(
        "CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n"
    )));
    reporter.report(
        "bridge CONNECT own host",
        "HTTP/1.1 200",
        line.clone(),
        line.starts_with("HTTP/1.1 200"),
    );
    for (name, request, expected) in [
        (
            "bridge CONNECT github.com",
            "CONNECT github.com:443 HTTP/1.1\r\n\r\n".to_owned(),
            "403",
        ),
        (
            "bridge CONNECT IP literal",
            "CONNECT 140.82.112.3:443 HTTP/1.1\r\n\r\n".to_owned(),
            "403",
        ),
        (
            "bridge CONNECT own host port 80",
            format!("CONNECT {host}:80 HTTP/1.1\r\n\r\n"),
            "403",
        ),
        (
            "bridge plain GET",
            format!("GET http://{host}/ HTTP/1.1\r\n\r\n"),
            "400",
        ),
    ] {
        let line = status_line(through_bridge(&request));
        reporter.report(name, expected, line.clone(), line.contains(expected));
    }

    // Non-harness outer processes: straight to the socket, through the
    // bridge, and at the harness's memory.
    let me = std::process::id().to_string();
    for (name, args, expected) in [
        (
            "peer straight to /bench/sock/proxy",
            vec!["socket", host.as_str()],
            "refused",
        ),
        (
            "peer through the bridge",
            vec!["bridge", host.as_str()],
            "refused",
        ),
        (
            "peer reads the harness's memory",
            vec!["mem", me.as_str()],
            "refused",
        ),
    ] {
        let output = Command::new(format!("{JAIL_DIR}/agent-jail"))
            .arg("probe-peer")
            .args(&args)
            .stdin(Stdio::null())
            .output();
        let observed = output.map_or_else(
            |err| err.to_string(),
            |out| String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        );
        reporter.report(
            name,
            expected,
            observed.clone(),
            observed.starts_with(expected),
        );
    }
    // Real tools run directly in the outer layer, not through a wrapper.
    let real = |name: &str| {
        spec.as_ref()
            .and_then(|spec| spec.tools.get(&format!("{BIN_DIR}/{name}")).cloned())
            .unwrap_or_default()
    };
    for (name, program, args) in [
        (
            "outer curl github.com direct",
            real("curl"),
            vec!["--noproxy", "*", "-sS", "-m", "5", "https://github.com"],
        ),
        (
            "outer curl own host through the bridge",
            real("curl"),
            vec![
                "-sS",
                "-m",
                "10",
                "-x",
                "http://127.0.0.1:3128",
                "https://HOST/",
            ],
        ),
        (
            "outer git ls-remote",
            real("git"),
            vec!["ls-remote", "https://github.com/Yazelix/eon"],
        ),
    ] {
        let args: Vec<String> = args.iter().map(|arg| arg.replace("HOST", &host)).collect();
        let mut command = Command::new(&program);
        for name in crate::spec::PROXY_VARS {
            command.env_remove(name);
        }
        let status = command
            .args(&args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // The program must have run and failed; a missing binary proves nothing.
        let failed = matches!(&status, Ok(status) if !status.success());
        reporter.report(name, "fails", format!("{status:?}"), failed);
    }

    // The harness's own file access never follows a workspace link, so no
    // inner-layer command can swap a checked file for the credential.
    let link = Path::new(crate::WORKSPACE).join(".agent-jail-probe-link");
    let _ = std::fs::remove_file(&link);
    let followed = std::os::unix::fs::symlink(&home.manifest.auth, &link)
        .and_then(|()| std::fs::read(&link).map(|bytes| bytes.len()));
    let _ = std::fs::remove_file(&link);
    reporter.report(
        "workspace link to the credential is not followed",
        "ELOOP",
        format!("{followed:?}"),
        matches!(&followed, Err(err) if err.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())),
    );

    // Codex's own file tool, run for real past any hook: its apply_patch
    // must meet the seal (ELOOP), never resolve a workspace link itself.
    if let Some(mount) = &loaded.profile.release_mount {
        codex_patch_links(&reporter, &format!("{mount}/bin/codex"));
    }

    // Policy and the read-only root.
    let auth = std::fs::metadata(&home.manifest.auth)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    reporter.report(
        "auth readable by the harness",
        "non-empty",
        format!("{auth} bytes"),
        auth > 0,
    );
    let dir = policy_dir(&harness);
    for path in [
        "/agent-jail-probe".to_owned(),
        format!("{dir}/agent-jail-probe"),
        format!("{HOME}/agent-jail-probe"),
        format!("{JAIL_DIR}/agent-jail-probe"),
        format!("{WORKSPACE}/.claude/agent-jail-probe"),
        format!("{WORKSPACE}/.codex/agent-jail-probe"),
    ] {
        write_fails(&reporter, &path);
    }
    for path in home.policy().into_iter().map(|(path, _)| path) {
        reporter.fails(
            &format!("open {path} for writing"),
            std::fs::OpenOptions::new().write(true).open(&path),
        );
    }
    for from in [
        config_dir(&harness),
        dir.to_owned(),
        HOME.into(),
        "/home".into(),
        "/bin".into(),
        "/etc".into(),
    ] {
        reporter.fails(
            &format!("rename {from}"),
            std::fs::rename(&from, format!("{from}.moved")),
        );
    }

    // The inner layer, through the shell path every harness command takes.
    match Command::new("/bin/sh")
        .args([
            "-c",
            &format!("{JAIL_DIR}/agent-jail probe-inner {profile}"),
        ])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = text.lines().filter(|line| line.starts_with('{')).collect();
            reporter.report(
                "inner layer ran",
                "checks",
                format!("{} checks", lines.len()),
                !lines.is_empty(),
            );
            for line in lines {
                println!("{line}");
            }
        }
        Err(err) => reporter.report("inner layer ran", "checks", err.to_string(), false),
    }
    let where_line = Command::new("/bin/bash")
        .args(["-c", &format!("{JAIL_DIR}/agent-jail where")])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    reporter.report(
        "bash -c lands in the inner layer",
        "AGENT-JAIL-WHERE: inner",
        where_line.clone(),
        where_line == "AGENT-JAIL-WHERE: inner",
    );
    let direct = where_marker(&harness);
    reporter.report(
        "the stand-in itself is outer",
        "outer",
        direct,
        direct == "outer",
    );
    0
}

/// `agent-jail probe-inner <profile>`.
pub fn probe_inner(profile: &str) -> i32 {
    let reporter = Reporter { layer: "inner" };
    let Ok(loaded) = crate::profile::load(profile) else {
        return 1;
    };
    let harness = loaded.profile.harness.clone();
    let Ok(home) = Home::shipped(&harness) else {
        return 1;
    };
    let spec = Spec::read().ok();
    common(&reporter, spec.as_ref());
    reporter.fails("tcp to the bridge", tcp(crate::BRIDGE));
    let path = std::env::var("PATH").unwrap_or_default();
    let want = spec
        .as_ref()
        .map(crate::wrapper::inner_path)
        .unwrap_or_default();
    reporter.report(
        "PATH is the wrapper directory then the dev shell's",
        &want,
        path.clone(),
        !want.is_empty() && path == want,
    );
    store_fetches(&reporter, spec.as_ref().map(|spec| spec.store.nix.as_str()));
    cargo_fetches(&reporter);
    // Functional: the shells harness commands run in need a usable /dev.
    let dev_null = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .and_then(|mut file| file.write_all(b"x"));
    reporter.report(
        "write /dev/null",
        "works",
        format!("{dev_null:?}"),
        dev_null.is_ok(),
    );
    let shell = Command::new("/bin/bash")
        .args(["-c", "echo x >/dev/null && echo shell-ok"])
        .stdin(Stdio::null())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|err| err.to_string());
    reporter.report(
        "bash -c with a /dev/null redirect",
        "shell-ok",
        shell.clone(),
        shell == "shell-ok",
    );
    for name in crate::spec::PROXY_VARS {
        let set = std::env::var_os(name).is_some();
        reporter.report(
            &format!("unset {name}"),
            "unset",
            if set { "set" } else { "unset" },
            !set,
        );
    }
    for dir in [SOCK_DIR, PRIVATE_DIR] {
        reporter.report(
            &format!("{dir} empty"),
            "empty",
            if empty_dir(dir) { "empty" } else { "not empty" },
            empty_dir(dir),
        );
    }
    // Commands still see ordinary links in the workspace.
    let target = Path::new(crate::WORKSPACE).join(".agent-jail-probe-target");
    let link = Path::new(crate::WORKSPACE).join(".agent-jail-probe-inner-link");
    let _ = std::fs::remove_file(&link);
    let resolved = std::fs::write(&target, "linked")
        .and_then(|()| std::os::unix::fs::symlink(".agent-jail-probe-target", &link))
        .and_then(|()| std::fs::read_to_string(&link));
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_file(&target);
    reporter.report(
        "workspace links resolve in the inner layer",
        "linked",
        format!("{resolved:?}"),
        matches!(&resolved, Ok(text) if text == "linked"),
    );
    let auth = std::fs::read(&home.manifest.auth)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    reporter.report(
        "auth reads 0 bytes",
        "0 bytes",
        format!("{auth} bytes"),
        auth == 0,
    );
    for (path, _) in home.policy() {
        let present = Path::new(&path).exists();
        reporter.report(
            &format!("policy {path} hidden"),
            "absent",
            if present { "present" } else { "absent" },
            !present,
        );
    }
    let mut sockets = Vec::new();
    sockets_under(Path::new("/"), &mut sockets);
    reporter.report(
        "no sockets",
        "[]",
        format!("{sockets:?}"),
        sockets.is_empty(),
    );
    for path in [
        format!("{JAIL_DIR}/agent-jail-probe"),
        format!("{WORKSPACE}/.claude/agent-jail-probe"),
        format!("{WORKSPACE}/.codex/agent-jail-probe"),
    ] {
        write_fails(&reporter, &path);
    }
    let marker = where_marker(&harness);
    reporter.report("where", "inner", marker, marker == "inner");
    0
}

/// Where the probe's Nix keeps its own throwaway store: a store directory
/// of its own, so Nix builds without its sandbox (a store rooted elsewhere
/// would force one, which can see no /tmp file and cannot mount /proc in a
/// nested jail); only the network differs between check and control.
const PROBE_NIX_STORE: &str = "local?store=/tmp/agent-jail-probe-nix/store&state=/tmp/agent-jail-probe-nix/state&log=/tmp/agent-jail-probe-nix/log";
/// The probe's Nix keeps its cache beside its store, off the read-only home.
const PROBE_NIX_ENV: &[(&str, &str)] = &[("XDG_CACHE_HOME", "/tmp/agent-jail-probe-nix/cache")];
/// The control's fixed-output file and its bytes.
const PROBE_FOD: &str = "/tmp/agent-jail-probe-fod";
const PROBE_FOD_TEXT: &[u8] = b"agent-jail probe fixed-output file\n";

/// Runs `program` with `args` (and extra `env`) for up to `seconds`;
/// returns its exit code and its standard output followed by its error.
fn output(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
    cwd: Option<&Path>,
    seconds: u64,
) -> (i32, String) {
    let mut command = Command::new(program);
    command.args(args).envs(env.iter().copied());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    match crate::capture(&mut command, b"", Duration::from_secs(seconds)) {
        Ok((code, out, err)) => (code, format!("{out}{err}")),
        Err(err) => (-1, format!("{program}: {err}")),
    }
}

/// The last lines of `text`, for a check's observation.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(4)..].join(" | ")
}

/// A network failure in a fetch client's words, not a refusal of its own.
fn network_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    !lower.contains("experimental")
        && [
            "resolve host",
            "could not resolve",
            "couldn't resolve",
            "unable to download",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// The pinned Nix: two fetches that must fail on the network, each with a
/// control that must succeed, and the default store, which must reach
/// neither a daemon nor the read-only /nix/store.
fn store_fetches(reporter: &Reporter, nix: Option<&str>) {
    let nix = nix.unwrap_or("/nonexistent/nix");
    let flags = [
        "--extra-experimental-features",
        "nix-command flakes",
        "--option",
        "sandbox",
        "false",
        "--option",
        "download-attempts",
        "1",
        "--store",
        PROBE_NIX_STORE,
    ];
    let fetch = |url: &str, sha256: &str| {
        let expr =
            format!("(import <nix/fetchurl.nix> {{ url = \"{url}\"; sha256 = \"{sha256}\"; }})");
        let mut args = flags.to_vec();
        args.extend([
            "build",
            "--impure",
            "--no-link",
            "--print-out-paths",
            "--expr",
            &expr,
        ]);
        output(nix, &args, PROBE_NIX_ENV, None, 300)
    };
    let (code, text) = fetch(
        "https://github.com/Yazelix/eon/archive/refs/heads/edge.tar.gz",
        &"0".repeat(64),
    );
    reporter.report(
        "store nix fetchurl from the network",
        "a download or resolve error",
        format!("exit {code}: {}", tail(&text)),
        code != 0 && network_error(&text),
    );
    let control = std::fs::write(PROBE_FOD, PROBE_FOD_TEXT).map(|()| {
        fetch(
            &format!("file://{PROBE_FOD}"),
            &crate::sha256_hex(PROBE_FOD_TEXT),
        )
    });
    let (code, text) = control.unwrap_or_else(|err| (-1, err.to_string()));
    reporter.report(
        "store nix fetchurl from a file (control)",
        "the file's store path",
        format!("exit {code}: {}", tail(&text)),
        code == 0 && text.contains("/tmp/agent-jail-probe-nix/store/"),
    );
    let mut args = flags.to_vec();
    args.extend(["flake", "metadata", "github:Yazelix/eon"]);
    let (code, text) = output(nix, &args, PROBE_NIX_ENV, None, 300);
    reporter.report(
        "store nix flake metadata github:Yazelix/eon",
        "a download or resolve error",
        format!("exit {code}: {}", tail(&text)),
        code != 0 && network_error(&text),
    );
    let mut args = flags.to_vec();
    args.extend(["eval", "--expr", "1 + 1"]);
    let (code, text) = output(nix, &args, PROBE_NIX_ENV, None, 120);
    reporter.report(
        "store nix eval (control)",
        "2",
        format!("exit {code}: {}", tail(&text)),
        code == 0 && text.lines().next() == Some("2"),
    );
    // Without a daemon socket or a /nix/var, Nix either fails or falls
    // back to a chroot store in the jail's own home; both reach nothing.
    let (code, text) = output(
        nix,
        &[
            "--extra-experimental-features",
            "nix-command",
            "store",
            "info",
        ],
        &[],
        None,
        120,
    );
    let refused = code != 0
        && !text.contains("experimental")
        && [
            "daemon-socket",
            "Read-only file system",
            "Permission denied",
            "cannot connect",
        ]
        .iter()
        .any(|needle| text.contains(needle));
    let private = code == 0 && text.contains(&format!("{HOME}/.local/share/nix/root"));
    reporter.report(
        "default store has no daemon and no writable /nix/store",
        "an error, or a chroot store in the jail home",
        format!("exit {code}: {}", tail(&text)),
        refused || private,
    );
}

/// Writes a one-dependency crate at `dir` and runs `cargo fetch` there.
fn cargo_fetch(dir: &Path, dependency: &str, env: &[(&str, &str)]) -> (i32, String) {
    let written = std::fs::create_dir_all(dir.join("src"))
        .and_then(|()| std::fs::write(dir.join("src/lib.rs"), ""))
        .and_then(|()| {
            std::fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"agent-jail-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{dependency}\n\n[workspace]\n"
                ),
            )
        });
    match written {
        Ok(()) => output("cargo", &["fetch"], env, Some(dir), 300),
        Err(err) => (-1, err.to_string()),
    }
}

/// Cargo in the jail is offline by default and has only the Cargo home's
/// crates; with the network asked for, it meets no network.
fn cargo_fetches(reporter: &Reporter) {
    let root = Path::new("/tmp/agent-jail-probe-cargo");
    let absent = "serde = \"=1.0.0\"";
    let (code, text) = cargo_fetch(&root.join("absent"), absent, &[]);
    reporter.report(
        "cargo fetch of an uncached version offline",
        "an offline resolution error",
        format!("exit {code}: {}", tail(&text)),
        code != 0 && text.contains("offline"),
    );
    let (code, text) = cargo_fetch(
        &root.join("cached"),
        "serde = { version = \"=1.0.229\", default-features = false }",
        &[],
    );
    reporter.report(
        "cargo fetch of a cached crate offline (control)",
        "exit 0",
        format!("exit {code}: {}", tail(&text)),
        code == 0,
    );
    let (code, text) = cargo_fetch(
        &root.join("online"),
        absent,
        &[
            ("CARGO_NET_OFFLINE", "false"),
            ("CARGO_NET_RETRY", "0"),
            ("CARGO_HTTP_TIMEOUT", "10"),
        ],
    );
    reporter.report(
        "cargo fetch with CARGO_NET_OFFLINE=false",
        "a network error",
        format!("exit {code}: {}", tail(&text)),
        code != 0 && network_error(&text),
    );
}

/// `inner` when every functional sign of the inner layer holds.
fn where_marker(harness: &str) -> &'static str {
    let Ok(home) = Home::shipped(harness) else {
        return "unknown";
    };
    // The bridge goes last: in the outer layer an earlier sign fails first,
    // so the probe never opens a stray bridge connection.
    let auth_empty = || std::fs::read(&home.manifest.auth).map_or(true, |bytes| bytes.is_empty());
    let policy_hidden = || {
        home.policy()
            .iter()
            .all(|(path, _)| !Path::new(path).exists())
    };
    if auth_empty()
        && policy_hidden()
        && empty_dir(SOCK_DIR)
        && empty_dir(PRIVATE_DIR)
        && tcp(crate::BRIDGE).is_err()
    {
        "inner"
    } else {
        "outer"
    }
}

/// `agent-jail where`: prints `AGENT-JAIL-WHERE: inner|outer`.
pub fn where_command() -> i32 {
    let harness = crate::spec::Spec::read()
        .ok()
        .and_then(|spec| crate::profile::load(&spec.profile).ok())
        .map(|loaded| loaded.profile.harness)
        .unwrap_or_default();
    println!("AGENT-JAIL-WHERE: {}", where_marker(&harness));
    0
}

/// `agent-jail probe-peer socket|bridge <host>` and `probe-peer mem <pid>`:
/// a non-harness outer process; prints `refused …` or `ALLOWED …`.
pub fn probe_peer(kind: &str, arg: &str) -> i32 {
    let verdict = match kind {
        "socket" => {
            let reply = std::os::unix::net::UnixStream::connect(format!("{SOCK_DIR}/proxy"))
                .and_then(|mut stream| {
                    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                    stream.write_all(format!("CONNECT {arg}:443 HTTP/1.1\r\n\r\n").as_bytes())?;
                    let mut reply = [0u8; 64];
                    let count = stream.read(&mut reply)?;
                    Ok(String::from_utf8_lossy(&reply[..count]).into_owned())
                });
            match reply {
                Ok(text) if text.starts_with("HTTP/1.1 200") => format!("ALLOWED {text}"),
                other => format!("refused {other:?}"),
            }
        }
        "bridge" => match through_bridge(&format!("CONNECT {arg}:443 HTTP/1.1\r\n\r\n")) {
            Ok(line) if line.starts_with("HTTP/1.1 200") => format!("ALLOWED {line}"),
            other => format!("refused {other:?}"),
        },
        "mem" => match std::fs::File::open(format!("/proc/{arg}/mem")) {
            Ok(_) => "ALLOWED opened".into(),
            Err(err) => format!("refused {err}"),
        },
        _ => "unknown peer check".into(),
    };
    println!("{verdict}");
    0
}

/// Drives the real `codex --codex-run-as-apply-patch` at workspace links
/// into the harness's writable state, which the inner layer cannot see:
/// a read through a link (update and move into the workspace), a write
/// through a link, and a create through a dangling link must all fail and
/// leave the state untouched.
fn codex_patch_links(reporter: &Reporter, codex: &str) {
    let state = Path::new("/home/bench/.codex/tmp");
    let workspace = Path::new(crate::WORKSPACE);
    let secret = state.join("agent-jail-probe-secret");
    let target = state.join("agent-jail-probe-target");
    let created = state.join("agent-jail-probe-new");
    let copy = workspace.join(".agent-jail-probe-copy");
    let links = [
        (".agent-jail-probe-rl", &secret),
        (".agent-jail-probe-wl", &target),
        (".agent-jail-probe-nl", &created),
    ];
    let setup = std::fs::write(&secret, "AGENT-JAIL-PROBE-SECRET\n")
        .and_then(|()| std::fs::write(&target, "a\n"))
        .and_then(|()| {
            links.iter().try_for_each(|(name, to)| {
                let _ = std::fs::remove_file(workspace.join(name));
                std::os::unix::fs::symlink(to, workspace.join(name))
            })
        });
    let patch = |body: &str| {
        std::process::Command::new(codex)
            .arg("--codex-run-as-apply-patch")
            .arg(format!("*** Begin Patch\n{body}*** End Patch\n"))
            .current_dir(workspace)
            .stdin(std::process::Stdio::null())
            .output()
            .map(|output| output.status.success())
    };
    let read = patch(
        "*** Update File: .agent-jail-probe-rl\n*** Move to: .agent-jail-probe-copy\n@@\n-AGENT-JAIL-PROBE-SECRET\n+changed\n",
    );
    let write = patch("*** Update File: .agent-jail-probe-wl\n@@\n-a\n+b\n");
    let create = patch("*** Add File: .agent-jail-probe-nl\n+created\n");
    let copied = std::fs::symlink_metadata(&copy).is_ok();
    let target_text = std::fs::read_to_string(&target).unwrap_or_default();
    let made = created.exists();
    for (name, _) in &links {
        let _ = std::fs::remove_file(workspace.join(name));
    }
    let _ = std::fs::remove_file(&copy);
    for path in [&secret, &target, &created] {
        let _ = std::fs::remove_file(path);
    }
    let ran = |result: &std::io::Result<bool>| setup.is_ok() && matches!(result, Ok(false));
    reporter.report(
        "codex apply_patch reads no file through a workspace link",
        "patch fails, nothing copied",
        format!("setup {setup:?}, patch {read:?}, copied {copied}"),
        ran(&read) && !copied,
    );
    reporter.report(
        "codex apply_patch writes no file through a workspace link",
        "patch fails, target unchanged",
        format!("patch {write:?}, target {target_text:?}"),
        ran(&write) && target_text == "a\n",
    );
    reporter.report(
        "codex apply_patch creates no file through a dangling workspace link",
        "patch fails, nothing created",
        format!("patch {create:?}, created {made}"),
        ran(&create) && !made,
    );
}

/// The checks (`layer: name`) each harness's probe must report, exactly, so
/// a probe that stops early or skips a layer fails.
pub fn expected_checks(harness: &str) -> &'static str {
    match harness {
        "claude" => include_str!("../probe-checks/claude.txt"),
        "codex" => include_str!("../probe-checks/codex.txt"),
        _ => "",
    }
}

pub fn same_checks(checks: &[Check], expected: &str) -> Check {
    let mut seen: Vec<String> = checks
        .iter()
        .map(|check| format!("{}: {}", check.layer, check.name))
        .collect();
    seen.sort();
    let mut want: Vec<String> = expected.lines().map(str::to_owned).collect();
    want.sort();
    let missing: Vec<&String> = want.iter().filter(|name| !seen.contains(name)).collect();
    let unexpected: Vec<&String> = seen.iter().filter(|name| !want.contains(name)).collect();
    Check {
        layer: "host".into(),
        name: "the probe reported exactly the expected checks".into(),
        expected: format!("{} checks", want.len()),
        observed: format!(
            "{} checks; missing {missing:?}; unexpected {unexpected:?}",
            seen.len()
        ),
        pass: !want.is_empty() && seen == want,
    }
}

/// Proxy-log verdicts the stand-in run must leave, exactly.
pub const EXPECTED_VERDICTS: &[(&str, usize)] = &[
    ("allow", 1),
    ("bad-request", 1),
    ("deny-host", 3),
    ("deny-owner", 2),
    ("deny-peer", 1),
];

/// Host side: runs the stand-in, collects its checks and the proxy-log
/// expectations, writes `meta/probe.json` and returns it.
pub fn run(options: &crate::enter::Options, host: &crate::enter::Host) -> Result<Value> {
    let outcome = crate::enter::enter(options, host)?;
    let meta = options.run.join("meta");
    let out = crate::run_dir::read_inside(&meta, Path::new("harness.out"), 4 << 20)?;
    let mut checks: Vec<Check> = out
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let log = crate::run_dir::read_inside(&meta, Path::new("proxy.jsonl"), 4 << 20)?;
    let verdicts: Vec<String> = log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            line.get("verdict")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    for (verdict, count) in EXPECTED_VERDICTS {
        let seen = verdicts.iter().filter(|seen| seen == verdict).count();
        checks.push(Check {
            layer: "host".into(),
            name: format!("proxy log {verdict}"),
            expected: count.to_string(),
            observed: seen.to_string(),
            pass: seen == *count,
        });
    }
    let total: usize = EXPECTED_VERDICTS.iter().map(|(_, count)| count).sum();
    checks.push(Check {
        layer: "host".into(),
        name: "proxy log has no other lines (none from the inner layer)".into(),
        expected: total.to_string(),
        observed: verdicts.len().to_string(),
        pass: verdicts.len() == total,
    });
    checks.push(Check {
        layer: "host".into(),
        name: "stand-in exit and token scan".into(),
        expected: "exit 0, not quarantined, private layers deleted".into(),
        observed: format!(
            "exit {}, quarantined {}, layers left {:?}",
            outcome.exit, outcome.quarantined, outcome.overlay_left
        ),
        pass: outcome.exit == 0 && !outcome.quarantined && outcome.overlay_left.is_empty(),
    });
    let harness = crate::profile::load(&options.profile)?.profile.harness;
    checks.push(same_checks(&checks, expected_checks(&harness)));
    let pass = checks.iter().all(|check| check.pass);
    let report = json!({ "profile": options.profile, "pass": pass, "checks": checks });
    std::fs::write(
        meta.join("probe.json"),
        serde_json::to_string_pretty(&report).map_err(|err| err.to_string())? + "\n",
    )
    .map_err(|err| err.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_store_fails_the_exclusion_check() {
        let exclude = vec!["agent-jail-no-such-tool".to_owned()];
        let dir = crate::test_dir("probe-excluded");
        assert!(excluded_found(&dir.join("absent"), &exclude).is_err());
        std::fs::write(dir.join("file"), "").unwrap();
        assert!(excluded_found(&dir.join("file"), &exclude).is_err());
        let store = dir.join("store");
        std::fs::create_dir(&store).unwrap();
        assert_eq!(excluded_found(&store, &exclude), Ok(Vec::new()));
        std::fs::create_dir(
            store.join("0123456789abcdfghijklmnpqrsvwxyz-agent-jail-no-such-tool-0.7.2"),
        )
        .unwrap();
        assert_eq!(excluded_found(&store, &exclude).unwrap().len(), 1);
        // Nothing excluded: nothing to look for.
        assert_eq!(excluded_found(&dir.join("absent"), &[]), Ok(Vec::new()));
    }

    fn check(layer: &str, name: &str) -> Check {
        Check {
            layer: layer.into(),
            name: name.into(),
            expected: String::new(),
            observed: String::new(),
            pass: true,
        }
    }

    fn store_spec(store: Store, allowed: &[&str]) -> StoreSpec {
        StoreSpec {
            store,
            exclude: vec![],
            allowed_roots: allowed.iter().map(|root| (*root).to_owned()).collect(),
            nix: String::new(),
        }
    }

    fn line(root: &str, point: &str) -> String {
        format!("36 35 98:0 {root} {point} ro,nosuid - ext4 /dev/x ro\n")
    }

    #[test]
    fn store_mount_fixtures() {
        let task = Store::Task {
            env_sha256: "0".repeat(64),
        };
        let task_store = line("/eon-agents/env/store/nix/store", "/nix/store");
        let jail = line("/eon-agents/target/agent-jail", "/bench/jail/agent-jail")
            + &line("/eon-agents/env/store/nix/store/x-bash/bin/bash", "/bin/sh");
        assert_eq!(
            store_mounts(
                &(task_store.clone() + &jail),
                &store_spec(task.clone(), &[])
            ),
            Ok(())
        );
        // A stray single-file bind out of the host store.
        let stray = line("/nix/store/y-coreutils/bin/env", "/usr/bin/env");
        assert!(
            store_mounts(
                &(task_store.clone() + &stray),
                &store_spec(task.clone(), &[])
            )
            .unwrap_err()
            .contains("/usr/bin/env")
        );
        // The host store itself at /nix/store, or no store, or two.
        let host = line("/nix/store", "/nix/store");
        assert!(
            store_mounts(&host, &store_spec(task.clone(), &[]))
                .unwrap_err()
                .contains("host store")
        );
        assert!(
            store_mounts(&jail, &store_spec(task.clone(), &[]))
                .unwrap_err()
                .contains("0 mounts")
        );
        assert!(
            store_mounts(
                &(task_store.clone() + &host),
                &store_spec(task.clone(), &[])
            )
            .is_err()
        );
        // agent-jail installed in the store is the one named exception.
        let installed = line(
            "/nix/store/z-agent-jail/bin/agent-jail",
            "/bench/jail/agent-jail",
        );
        let spec = store_spec(task, &["/nix/store/z-agent-jail/bin/agent-jail"]);
        assert_eq!(store_mounts(&(task_store + &installed), &spec), Ok(()));
        // The host-test store: exactly the host store, plus allowed files.
        let tests = store_spec(Store::HostTest, &["/nix/store/y-coreutils/bin/env"]);
        assert_eq!(store_mounts(&(host.clone() + &stray), &tests), Ok(()));
        let other = line("/nix/store/w-curl/bin/curl", "/bench/bin/curl");
        assert!(
            store_mounts(&(host + &other), &tests)
                .unwrap_err()
                .contains("curl")
        );
        assert!(
            store_mounts(&line("/x/store", "/nix/store"), &tests)
                .unwrap_err()
                .contains("not the host store")
        );
    }

    #[test]
    fn network_errors_are_told_from_refusals() {
        assert!(network_error(
            "error: unable to download 'https://x': Could not resolve hostname (6)"
        ));
        assert!(network_error(
            "[6] Couldn't resolve host name (Could not resolve host: index.crates.io)"
        ));
        assert!(!network_error(
            "error: experimental Nix feature 'flakes' is disabled; unable to download"
        ));
        assert!(!network_error("error: you're using offline mode"));
    }

    #[test]
    fn a_probe_missing_or_adding_a_check_fails() {
        for harness in ["claude", "codex"] {
            let names: Vec<Check> = expected_checks(harness)
                .lines()
                .map(|line| {
                    let (layer, name) = line.split_once(": ").unwrap();
                    check(layer, name)
                })
                .collect();
            assert!(names.len() > 90, "{harness}");
            assert!(same_checks(&names, expected_checks(harness)).pass);
            // An inner layer that stopped after its common checks.
            let early: Vec<Check> = names
                .iter()
                .filter(|check| check.name != "auth reads 0 bytes")
                .cloned()
                .collect();
            assert!(!same_checks(&early, expected_checks(harness)).pass);
            let mut extra = names.clone();
            extra.push(check("inner", "something new"));
            assert!(!same_checks(&extra, expected_checks(harness)).pass);
        }
        assert!(!same_checks(&[], expected_checks("agy")).pass);
    }
}
