//! `agent-jail init`: the outer layer's PID 1. It binds the bridge, runs
//! the guard, spawns (never execs) the harness, bridges its connections,
//! reaps every child and returns the harness's exit status. Any guard miss
//! exits [`crate::GUARD_EXIT`] naming the condition; the harness never
//! starts.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::spec::{Mode, Spec};
use crate::{JAIL_DIR, Result};

/// Where `enter` records the host's network namespace.
pub const HOST_NETNS: &str = "/bench/jail/host-netns";

/// A guard miss: the condition's number and what was wrong.
#[derive(Debug, PartialEq, Eq)]
pub struct Miss {
    pub condition: u8,
    pub detail: String,
}

fn miss(condition: u8, detail: impl Into<String>) -> Miss {
    Miss {
        condition,
        detail: detail.into(),
    }
}

/// Condition 1: the network namespace is not the host's and holds only `lo`.
pub fn network(own: &str, host: &str, net_dev: &str) -> std::result::Result<(), Miss> {
    if own.trim().is_empty() || host.trim().is_empty() || own.trim() == host.trim() {
        return Err(miss(1, "the network namespace is the host's"));
    }
    let interfaces: Vec<&str> = net_dev
        .lines()
        .skip(2)
        .filter_map(|line| line.split(':').next())
        .map(str::trim)
        .collect();
    if interfaces != ["lo"] {
        return Err(miss(1, format!("interfaces other than lo: {interfaces:?}")));
    }
    Ok(())
}

/// Condition 2: each policy file is present and hash-matched, and dcg exists.
pub fn hooks_configured(spec: &Spec) -> std::result::Result<(), Miss> {
    for (path, sha256) in &spec.policy {
        let actual = crate::sha256_file(Path::new(path)).map_err(|err| miss(2, err))?;
        if &actual != sha256 {
            return Err(miss(2, format!("{path} does not match its manifest hash")));
        }
    }
    if !Path::new(&spec.dcg).is_file() {
        return Err(miss(2, format!("dcg {} is missing", spec.dcg)));
    }
    Ok(())
}

/// Condition 3: each hook check gives its expected verdict when run as the
/// harness runs it, through `/bin/sh -c`.
pub fn hooks_live(spec: &Spec, shell: &str) -> std::result::Result<(), Miss> {
    for check in &spec.hooks {
        let (status, stdout, stderr) = run_hook(shell, &check.command, &check.input)
            .map_err(|err| miss(3, format!("{}: {err}", check.name)))?;
        // An allow check needs a clean exit, not just the absence of a deny.
        let wrong = if check.deny {
            !crate::hook::denied(status, &stdout)
        } else {
            status != 0
        };
        if wrong {
            let said: String = stderr.trim().chars().take(400).collect();
            return Err(miss(3, format!("{} (exit {status}): {said}", check.name)));
        }
    }
    Ok(())
}

fn run_hook(shell: &str, command: &str, input: &str) -> Result<(i32, String, String)> {
    use std::io::{Read, Write};
    let mut child = Command::new(shell)
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let error_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = reader.join().unwrap_or_default();
    let errors = error_reader.join().unwrap_or_default();
    Ok((crate::wrapper::exit_code(status), text, errors))
}

/// Condition 4: every shell path and tool name is the wrapper, and `PATH`
/// holds only the wrapper directory.
pub fn wrappers(spec: &Spec, wrapper: &Path, path_var: &str) -> std::result::Result<(), Miss> {
    use std::os::unix::fs::MetadataExt;
    if path_var != crate::BIN_DIR {
        return Err(miss(
            4,
            format!("PATH is {path_var}, not {}", crate::BIN_DIR),
        ));
    }
    let reference = std::fs::metadata(wrapper)
        .map_err(|err| miss(4, format!("{}: {err}", wrapper.display())))?;
    for required in crate::SHELL_PATHS {
        if !spec.tools.contains_key(*required) {
            return Err(miss(4, format!("{required} is not a wrapper")));
        }
    }
    for path in spec.tools.keys() {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|err| miss(4, format!("{path}: {err}")))?;
        if !metadata.is_file()
            || metadata.dev() != reference.dev()
            || metadata.ino() != reference.ino()
        {
            return Err(miss(4, format!("{path} is not the wrapper")));
        }
    }
    Ok(())
}

/// Condition 5: argv is the profile template filled with the spec's slots.
pub fn argv(spec: &Spec) -> std::result::Result<(), Miss> {
    let mut loaded = crate::profile::load(&spec.profile).map_err(|err| miss(5, err))?;
    spec.control
        .apply_profile(&mut loaded)
        .map_err(|err| miss(5, err))?;
    match spec.mode {
        Mode::Harness => {
            loaded
                .check_argv(&spec.slots, &spec.argv)
                .map_err(|err| miss(5, err))?;
            if spec.exe != crate::enter::harness_exe(&loaded) {
                return Err(miss(
                    5,
                    format!("{} is not the profile's harness", spec.exe),
                ));
            }
        }
        Mode::StandIn => {
            if spec.exe != format!("{JAIL_DIR}/agent-jail")
                || spec.argv != ["probe-harness", spec.profile.as_str()]
            {
                return Err(miss(5, "the stand-in argv is not the probe's"));
            }
        }
    }
    Ok(())
}

/// Condition 7: every listed path sits on a read-only mount.
pub fn read_only(paths: &[String]) -> std::result::Result<(), Miss> {
    for path in paths {
        let stat =
            rustix::fs::statvfs(path.as_str()).map_err(|err| miss(7, format!("{path}: {err}")))?;
        if !stat.f_flag.contains(rustix::fs::StatVfsMountFlags::RDONLY) {
            return Err(miss(7, format!("{path} is writable")));
        }
    }
    Ok(())
}

/// Condition 8: same-user ptrace is contained.
pub fn ptrace(scope: &str) -> std::result::Result<(), Miss> {
    match scope.trim().parse::<u8>() {
        Ok(level) if level >= 1 => Ok(()),
        _ => Err(miss(
            8,
            format!("kernel.yama.ptrace_scope is {:?}, not >= 1", scope.trim()),
        )),
    }
}

/// All nine conditions, in order, against the live jail.
pub fn check(spec: &Spec) -> std::result::Result<(), Miss> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let own = std::fs::read_link("/proc/self/ns/net")
        .map(|link| link.display().to_string())
        .unwrap_or_default();
    network(&own, &read(HOST_NETNS), &read("/proc/net/dev"))?;
    hooks_configured(spec)?;
    hooks_live(spec, "/bin/sh")?;
    wrappers(
        spec,
        Path::new(&format!("{JAIL_DIR}/agent-jail")),
        &std::env::var("PATH").unwrap_or_default(),
    )?;
    argv(spec)?;
    pid_one(std::process::id())?;
    read_only(&spec.read_only)?;
    ptrace(&read("/proc/sys/kernel/yama/ptrace_scope"))?;
    crate::seal::sealed(&read("/proc/self/mountinfo"), crate::WORKSPACE).map_err(|err| miss(9, err))
}

/// Condition 6: init is the jail's PID 1, so the jail dies with it.
pub fn pid_one(pid: u32) -> std::result::Result<(), Miss> {
    if pid == 1 {
        Ok(())
    } else {
        Err(miss(6, format!("init runs as PID {pid}, not 1")))
    }
}

/// `agent-jail init`.
pub fn init() -> i32 {
    match run_init() {
        Ok(code) => code,
        Err(miss) => {
            eprintln!(
                "agent-jail guard: condition {}: {}",
                miss.condition, miss.detail
            );
            crate::GUARD_EXIT
        }
    }
}

fn run_init() -> std::result::Result<i32, Miss> {
    let spec = Spec::read().map_err(|err| miss(0, err))?;
    // First, while init is single-threaded.
    crate::seal::seal(crate::WORKSPACE, crate::INNER_WORKSPACE).map_err(|err| miss(9, err))?;
    let listener = crate::bridge::listen().map_err(|err| miss(0, format!("bridge: {err}")))?;
    check(&spec)?;
    let harness = Arc::new(AtomicU32::new(0));
    crate::bridge::serve(
        listener,
        Arc::clone(&harness),
        format!("{}/proxy", crate::SOCK_DIR).into(),
        "/proc".into(),
    );
    let child = Command::new(&spec.exe)
        .args(&spec.argv)
        .spawn()
        .map_err(|err| miss(0, format!("cannot start {}: {err}", spec.exe)))?;
    let pid = child.id();
    harness.store(pid, Ordering::SeqCst);
    // Reap every child; the harness's status is the jail's.
    loop {
        match rustix::process::wait(rustix::process::WaitOptions::empty()) {
            Ok(Some((reaped, status))) if reaped.as_raw_nonzero().get() as u32 == pid => {
                return Ok(status
                    .exit_status()
                    .unwrap_or_else(|| 128 + status.terminating_signal().unwrap_or(0)));
            }
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => {}
            Err(err) => return Err(miss(0, format!("wait: {err}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = "Inter-|   Receive\n face |bytes\n    lo: 0 0\n";

    #[test]
    fn network_condition() {
        assert!(network("net:[2]", "net:[1]", DEV).is_ok());
        assert_eq!(network("net:[1]", "net:[1]", DEV).unwrap_err().condition, 1);
        assert_eq!(network("", "net:[1]", DEV).unwrap_err().condition, 1);
        assert_eq!(network("net:[2]", "", DEV).unwrap_err().condition, 1);
        let eth = format!("{DEV}  eth0: 1 2\n");
        assert_eq!(
            network("net:[2]", "net:[1]", &eth).unwrap_err().condition,
            1
        );
    }

    #[test]
    fn ptrace_condition() {
        assert!(ptrace("1\n").is_ok() && ptrace("2").is_ok());
        assert_eq!(ptrace("0").unwrap_err().condition, 8);
        assert_eq!(ptrace("").unwrap_err().condition, 8);
    }

    #[test]
    fn init_must_be_pid_one() {
        assert!(pid_one(1).is_ok());
        assert_eq!(pid_one(2).unwrap_err().condition, 6);
    }

    #[test]
    fn the_full_check_on_the_host_stops_at_the_network() {
        // On the host there is no recorded host namespace and the test's
        // network namespace has more than lo: condition 1 must catch it
        // before anything else runs.
        let spec = spec(Mode::Harness, "claude", Vec::new(), "/bin/true");
        assert_eq!(check(&spec).unwrap_err().condition, 1);
    }

    #[test]
    fn read_only_condition() {
        assert!(
            read_only(&["/proc/sys".into()]).is_ok() || read_only(&["/nix/store".into()]).is_ok()
        );
        let dir = crate::test_dir("guard-ro");
        assert_eq!(
            read_only(&[dir.display().to_string()])
                .unwrap_err()
                .condition,
            7
        );
    }

    fn spec(mode: Mode, profile: &str, argv: Vec<String>, exe: &str) -> Spec {
        Spec {
            profile: profile.into(),
            mode,
            slots: [("prompt", "hi"), ("model", "m"), ("effort", "high")]
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
                .into(),
            exe: exe.into(),
            argv,
            tools: Default::default(),
            auth_path: String::new(),
            read_only: vec![],
            policy: Default::default(),
            hooks: vec![],
            inner_masks: vec![],
            dcg: "/nonexistent/dcg".into(),
            control: Default::default(),
        }
    }

    #[test]
    fn argv_condition() {
        let loaded = crate::profile::load("claude").unwrap();
        let exe = crate::enter::harness_exe(&loaded);
        let good = spec(Mode::Harness, "claude", vec![], &exe);
        let argv_ok = loaded.fill(&good.slots).unwrap();
        assert!(
            argv(&Spec {
                argv: argv_ok.clone(),
                ..good.clone()
            })
            .is_ok()
        );
        let mut extra = argv_ok.clone();
        extra.push("--dangerously-skip-permissions".into());
        assert_eq!(
            argv(&Spec {
                argv: extra,
                ..good.clone()
            })
            .unwrap_err()
            .condition,
            5
        );
        assert_eq!(
            argv(&Spec {
                argv: argv_ok,
                exe: "/bin/sh".into(),
                ..good
            })
            .unwrap_err()
            .condition,
            5
        );
        let stand_in = spec(
            Mode::StandIn,
            "codex",
            vec!["probe-harness".into(), "codex".into()],
            "/bench/jail/agent-jail",
        );
        assert!(argv(&stand_in).is_ok());
        assert_eq!(
            argv(&Spec {
                argv: vec!["probe-harness".into(), "claude".into()],
                ..stand_in
            })
            .unwrap_err()
            .condition,
            5
        );
    }

    #[test]
    fn missing_dcg_and_drifted_policy_are_refused() {
        let dir = crate::test_dir("guard-policy");
        let file = dir.join("policy.json");
        std::fs::write(&file, "{}").unwrap();
        let mut good = spec(Mode::StandIn, "claude", vec![], "");
        good.dcg = file.display().to_string();
        good.policy
            .insert(file.display().to_string(), crate::sha256_hex(b"{}"));
        assert!(hooks_configured(&good).is_ok());
        let mut drift = good.clone();
        drift
            .policy
            .insert(file.display().to_string(), crate::sha256_hex(b"[]"));
        assert_eq!(hooks_configured(&drift).unwrap_err().condition, 2);
        assert_eq!(
            hooks_configured(&Spec {
                dcg: "/nonexistent".into(),
                ..good
            })
            .unwrap_err()
            .condition,
            2
        );
    }

    #[test]
    fn hooks_live_condition_uses_the_shell() {
        let mut checks = spec(Mode::StandIn, "claude", vec![], "");
        checks.hooks = vec![crate::spec::HookCheck {
            name: "deny".into(),
            command: "cat >/dev/null; echo no >&2; exit 2".into(),
            input: "{}".into(),
            deny: true,
        }];
        assert!(hooks_live(&checks, "sh").is_ok());
        checks.hooks[0].command = "cat >/dev/null; exit 0".into();
        assert_eq!(hooks_live(&checks, "sh").unwrap_err().condition, 3);
        // An allow check fails on a crash or a missing hook, not only a deny.
        checks.hooks[0].deny = false;
        assert!(hooks_live(&checks, "sh").is_ok());
        for command in ["cat >/dev/null; exit 1", "/nonexistent/hook"] {
            checks.hooks[0].command = command.into();
            assert_eq!(
                hooks_live(&checks, "sh").unwrap_err().condition,
                3,
                "{command}"
            );
        }
    }

    #[test]
    fn wrappers_condition() {
        let dir = crate::test_dir("guard-wrappers");
        let wrapper = dir.join("agent-jail");
        std::fs::write(&wrapper, "x").unwrap();
        let other = dir.join("other");
        std::fs::write(&other, "x").unwrap();
        // A PATH other than the wrapper directory is refused.
        let shells = spec(Mode::StandIn, "claude", vec![], "");
        assert_eq!(
            wrappers(&shells, &wrapper, "/usr/bin")
                .unwrap_err()
                .condition,
            4
        );
        // Missing shell paths are refused.
        assert_eq!(
            wrappers(&shells, &wrapper, crate::BIN_DIR)
                .unwrap_err()
                .condition,
            4
        );
        // A tool path that is another file is refused. (The shell paths are
        // real host files outside a jail, so the in-jail run covers them.)
        let mut fake = spec(Mode::StandIn, "claude", vec![], "");
        for shell in crate::SHELL_PATHS {
            fake.tools.insert((*shell).to_owned(), String::new());
        }
        fake.tools
            .insert(other.display().to_string(), String::new());
        assert_eq!(
            wrappers(&fake, &wrapper, crate::BIN_DIR)
                .unwrap_err()
                .condition,
            4
        );
    }
}
