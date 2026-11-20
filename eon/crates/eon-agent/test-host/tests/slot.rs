#![cfg(target_os = "linux")]

use fs2::FileExt;
use rustix::process::{Pid, Signal};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Fixture {
    dir: tempfile::TempDir,
    shared: bool,
    _case: MutexGuard<'static, ()>,
}

// Keep subprocess-heavy cases from contending at idle priority under the build wrapper.
static CASE: Mutex<()> = Mutex::new(());

/// Removes the slot and handoff names a parent run may have exported, current and removed,
/// so a suite started inside a slot or delegate behaves as a fresh one.
fn scrub(command: &mut Command) -> &mut Command {
    for name in ["ROTA_SLOT", "ROTA_SLOT_TOKEN"] {
        command.env_remove(name);
    }
    for name in ["QUEUED_NS", "CLASS", "RUN_KEY", "DELEGATE"] {
        command.env_remove(format!("ROTA_SLOT_{name}"));
        command.env_remove(format!("EON_SLOT_{name}"));
    }
    command
}

fn delegate(f: &Fixture, pause: bool) {
    let helper = f.dir.path().join("fake/local.sh");
    fs::write(
        &helper,
        format!(
            r#"set -eu
# No alias: the handoff uses only the ROTA_SLOT_* names.
[ -z "${{EON_SLOT_QUEUED_NS+x}}${{EON_SLOT_CLASS+x}}${{EON_SLOT_RUN_KEY+x}}${{EON_SLOT_DELEGATE+x}}" ] || exit 88
flock -n legacy/old-1.lock true
flock -n state/slots/queue/.lock true
[ ! -e state/slots/boat/lease.lock ] || flock -n state/slots/boat/lease.lock true
printf '%s %s' "$ROTA_SLOT_QUEUED_NS" "$ROTA_SLOT_CLASS" > provider/handoff
printf '%s' "$ROTA_SLOT_RUN_KEY" > provider/run-key
touch provider/delegated
{}
exec "$@"
"#,
            if pause {
                "while [ ! -f provider/continue ]; do sleep 0.01; done"
            } else {
                ""
            }
        ),
    )
    .unwrap();
    let config = f.dir.path().join("config.toml");
    let text = fs::read_to_string(&config).unwrap().replace(
        "[slots]\n",
        &format!("[slots]\nlocal_runner = ['bash', '{}']\n", helper.display()),
    );
    fs::write(config, text).unwrap();
}

#[test]
fn delegated_fallback_releases_capacity_and_preserves_exact_command() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    let out = f
        .boat_command(&[
            "slot",
            "run",
            "--",
            "sh",
            "-c",
            "printf '%s' \"$1\"; test -z \"${ROTA_SLOT_QUEUED_NS+x}${ROTA_SLOT_CLASS+x}${ROTA_SLOT_RUN_KEY+x}\" || exit 89; exit 7",
            "sh",
            "literal $word with spaces",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"literal $word with spaces");
    assert!(f.dir.path().join("provider/delegated").exists());
    assert!(!f.dir.path().join("provider/calls").exists());
    let key = fs::read_to_string(f.dir.path().join("provider/run-key")).unwrap();
    let events = fs::read_to_string(f.root().join("log.jsonl")).unwrap();
    for line in events.lines() {
        let event: Value = serde_json::from_str(line).unwrap();
        assert_eq!(event["run_key"], key);
    }
}

#[test]
fn delayed_delegate_waiters_keep_fifo_age_and_reject_recycled_pid() {
    let f = Fixture::boat_fixture();
    delegate(&f, true);
    let helper = f.dir.path().join("fake/local.sh");
    let script = fs::read_to_string(&helper).unwrap();
    fs::write(
        &helper,
        script.replace(
            "exec \"$@\"",
            r#"bash -c '
mkdir -p legacy/queue
ticket=legacy/queue/$ROTA_SLOT_QUEUED_NS.$$.run
stat=$(cat /proc/$$/stat); set -- ${stat##*) }
printf "%s\n" "${20}" > "$ticket"
echo "$ticket" > provider/ticket
while [ ! -f provider/release ]; do sleep 0.01; done
rm "$ticket"
'"#,
        ),
    )
    .unwrap();
    let mut run = Running(
        f.boat_command(&["slot", "run", "--", "true"])
            .spawn()
            .unwrap(),
    );
    until(|| {
        f.dir
            .path()
            .join("provider/delegated")
            .exists()
            .then_some(())
    });
    // Deliberately cross several Linux start ticks before spawning the helper.
    std::thread::sleep(Duration::from_millis(30));
    fs::write(f.dir.path().join("provider/continue"), "go").unwrap();
    let ticket = until(|| fs::read_to_string(f.dir.path().join("provider/ticket")).ok());
    let path = f.dir.path().join(ticket.trim());
    let waiter = f.list()["waiters"].as_array().unwrap().clone();
    assert_eq!(waiter.len(), 1);
    assert!(waiter[0]["legacy"].as_bool().unwrap());
    let handoff = fs::read_to_string(f.dir.path().join("provider/handoff")).unwrap();
    assert_eq!(
        waiter[0]["queued_ns"].as_u64().unwrap().to_string(),
        handoff.split_whitespace().next().unwrap()
    );
    fs::write(path, "1\n").unwrap();
    assert!(f.list()["waiters"].as_array().unwrap().is_empty());
    fs::write(f.dir.path().join("provider/release"), "go").unwrap();
    assert!(run.wait().success());
}

#[test]
fn delegated_fallback_rechecks_source_after_waiting_in_local_queue() {
    for added_untracked in [false, true] {
        let f = Fixture::boat_fixture();
        delegate(&f, true);
        fs::write(f.dir.path().join("provider/mode"), "unreachable").unwrap();
        let mut child = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !f.dir.path().join("provider/delegated").exists() {
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("delegate never queued");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(
            f.dir.path().join(if added_untracked {
                "new-input"
            } else {
                "source"
            }),
            "changed while queued",
        )
        .unwrap();
        fs::write(f.dir.path().join("provider/continue"), "go").unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(125));
        assert!(!f.dir.path().join("local-ran").exists());
    }
}

#[test]
fn delegated_pre_execution_barrier_still_uses_local_queue() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    f.pending_boat(false);
    let result = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(f.dir.path().join("local-ran").exists());
    assert!(f.dir.path().join("provider/delegated").exists());
    assert!(f.root().join("boat/current.json").exists());
}

#[test]
fn delegated_remote_verdict_log_loss_and_own_kill_never_replay() {
    for (mode, expected) in [
        ("normal", 7),
        ("incomplete-log", 125),
        ("disconnected", 125),
    ] {
        let f = Fixture::boat_fixture();
        delegate(&f, false);
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        fs::write(f.dir.path().join("provider/code"), "7").unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(expected), "{mode}: {out:?}");
        assert!(!f.dir.path().join("provider/delegated").exists());
        assert!(!f.dir.path().join("local-ran").exists());
        let record: Value =
            serde_json::from_slice(&fs::read(f.root().join("boat/current.json")).unwrap()).unwrap();
        if mode == "disconnected" {
            assert_eq!(record["kill_sent"], true);
            assert_eq!(record["exit_code"], Value::Null);
        } else {
            assert_eq!(record["exit_code"], 7);
        }
        assert_eq!(record["queue_wait_avoided_seconds"], Value::Null);
        assert_eq!(record["trial_usage_reason"], "account-counter-delta");
    }
}

#[test]
fn delegated_nesting_reuses_verified_ownership_without_second_target_wait() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    let helper = f.dir.path().join("fake/local.sh");
    let script = fs::read_to_string(&helper).unwrap().replace(
        "exec \"$@\"",
        "echo call >> provider/delegate-calls\nexec flock \"$CARGO_TARGET_DIR/lock\" \"$@\"",
    );
    fs::write(&helper, script).unwrap();
    let target = f.dir.path().join("provider/target");
    fs::create_dir(&target).unwrap();
    let mut command = Command::new("timeout");
    command
        .args([
            "5",
            env!("CARGO_BIN_EXE_rota-test-host"),
            "slot",
            "run",
            "--",
            env!("CARGO_BIN_EXE_rota-test-host"),
            "slot",
            "run",
            "--",
            "true",
        ])
        .current_dir(f.dir.path())
        .env("CARGO_TARGET_DIR", &target)
        .env("ROTA_CONFIG", f.dir.path().join("config.toml"));
    let result = scrub(&mut command).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        fs::read_to_string(f.dir.path().join("provider/delegate-calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(
        f.events().iter().filter(|e| e["event"] == "queue").count(),
        1
    );
}

#[test]
fn fallback_refuses_symlink_and_unsafe_mode_runs_parent() {
    for symlink in [true, false] {
        let f = Fixture::boat_fixture();
        delegate(&f, false);
        f.pending_boat(false);
        let outside = tempfile::tempdir().unwrap();
        let runs = f.root().join("runs");
        if symlink {
            std::os::unix::fs::symlink(outside.path(), &runs).unwrap();
        } else {
            fs::create_dir(&runs).unwrap();
            fs::set_permissions(&runs, fs::Permissions::from_mode(0o775)).unwrap();
        }
        let output = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(125), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unsafe-boat-state-directory"));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(!f.dir.path().join("provider/delegated").exists());
    }
}

#[test]
fn billing_mismatch_and_wrong_vm_type_refuse_before_execution() {
    for mode in ["paid-mismatch", "wrong-type"] {
        let f = Fixture::boat_fixture();
        delegate(&f, false);
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{mode}: {out:?}");
        assert!(f.dir.path().join("local-ran").exists());
        assert!(!f.dir.path().join("provider/attached").exists());
        assert!(!f.dir.path().join("provider/upload").exists());
        if mode == "wrong-type" {
            assert!(f.dir.path().join("provider/deleted").exists());
        }
    }
}

#[test]
fn post_start_plan_and_reserve_loss_clean_up_without_local_replay() {
    for mode in ["post-plan-loss", "post-reserve-loss"] {
        let f = Fixture::boat_fixture();
        delegate(&f, false);
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(125), "{mode}: {out:?}");
        assert!(f.dir.path().join("provider/deleted").exists());
        assert!(!f.dir.path().join("provider/delegated").exists());
        assert!(!f.dir.path().join("local-ran").exists());
        let record = f.list()["boat"].clone();
        assert_eq!(record["reason"], "remote-budget-observation-or-reserve");
        assert_eq!(record["kill_sent"], true);
    }
}

#[test]
fn delegated_pid_signal_releases_target_and_stops_command() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    let helper = f.dir.path().join("fake/local.sh");
    fs::write(
        &helper,
        fs::read_to_string(&helper)
            .unwrap()
            .replace("exec \"$@\"", "exec flock provider/guard-lock \"$@\""),
    )
    .unwrap();
    let mut run = f.spawn(&[
        "slot",
        "run",
        "--",
        "sh",
        "-c",
        "echo $$ > provider/command-pid; exec sleep 120",
    ]);
    until(|| {
        f.dir
            .path()
            .join("provider/command-pid")
            .exists()
            .then_some(())
    });
    let _ = rustix::process::kill_process(Pid::from_raw(run.pid() as i32).unwrap(), Signal::TERM);
    assert_eq!(run.wait().code(), Some(143));
    let lock = File::open(f.dir.path().join("provider/guard-lock")).unwrap();
    assert!(FileExt::try_lock_exclusive(&lock).is_ok());
    let pid = fs::read_to_string(f.dir.path().join("provider/command-pid")).unwrap();
    until(
        || match fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
            Err(_) => Some(()),
            Ok(stat) => stat
                .rsplit_once(')')
                .is_some_and(|(_, fields)| fields.starts_with(" Z "))
                .then_some(()),
        },
    );
}

#[test]
fn queued_remote_request_observes_boat_off_before_claim() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    f.policy("quiet-until", "9999999999");
    let mut run = Running(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| f.root().join("queue").exists().then_some(()));
    let ticket = f.queued(run.pid());
    fs::remove_file(f.dir.path().join("legacy/boat-on")).unwrap();
    f.policy("quiet-until", "0");
    assert!(run.wait().success());
    assert!(f.dir.path().join("local-ran").exists());
    assert_eq!(
        fs::read_to_string(f.dir.path().join("provider/calls")).unwrap(),
        "limits\n"
    );
    let handoff = fs::read_to_string(f.dir.path().join("provider/handoff")).unwrap();
    assert_eq!(handoff, format!("{} nix", ticket["queued_ns"]));
}

#[test]
fn changed_source_is_refused_before_local_admission() {
    let f = Fixture::boat_fixture();
    delegate(&f, false);
    fs::write(f.dir.path().join("provider/mode"), "mutate-source").unwrap();
    let out = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(125));
    assert!(!f.dir.path().join("provider/delegated").exists());
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn native_local_grant_rechecks_source_and_logs_125() {
    let f = Fixture::boat_fixture();
    let lock = f.hold(1);
    fs::write(f.dir.path().join("provider/mode"), "unreachable").unwrap();
    let mut run = Running(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    f.queued(run.pid());
    until(|| {
        f.events()
            .iter()
            .any(|e| e["event"] == "fallback")
            .then_some(())
    });
    fs::write(f.dir.path().join("source"), "changed after admission check").unwrap();
    drop(lock);
    assert_eq!(run.wait().code(), Some(125));
    assert!(!f.dir.path().join("local-ran").exists());
    assert_eq!(f.events().last().unwrap()["exit_code"], 125);
}

#[test]
fn local_queue_failure_after_remote_attempt_is_unresolved_125() {
    for mode in [
        "break-local-queue",
        "break-local-slot",
        "break-local-holder",
        "break-fallback-record",
    ] {
        let f = Fixture::boat_fixture();
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(125), "{mode}");
        assert_eq!(f.events().last().unwrap()["exit_code"], 125, "{mode}");
        assert!(!f.dir.path().join("local-ran").exists());
    }
}

#[test]
fn remote_without_legacy_policy_requires_state_boat_on() {
    let f = Fixture::boat_fixture();
    let config = f.dir.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replace(
            &format!("legacy_dir = '{}/legacy'\n", f.dir.path().display()),
            "",
        ),
    )
    .unwrap();
    let args = ["slot", "run", "--", "nix", "build", ".#default"];
    assert!(f.boat_command(&args).output().unwrap().status.success());
    assert!(f.dir.path().join("local-ran").exists());
    assert!(!f.dir.path().join("provider/calls").exists());
    fs::remove_file(f.dir.path().join("local-ran")).unwrap();
    fs::write(f.root().join("boat-on"), "enabled").unwrap();
    assert!(f.boat_command(&args).output().unwrap().status.success());
    assert!(!f.dir.path().join("local-ran").exists());
    assert!(f.dir.path().join("provider/attached").exists());
}

#[test]
fn delegated_terminal_signals_stop_foreground_group_and_keep_input() {
    struct Session {
        child: Child,
        root: PathBuf,
    }
    impl Drop for Session {
        fn drop(&mut self) {
            let session = self.child.id() as i32;
            for name in ["helper-pid", "command-pid"] {
                if let Ok(text) = fs::read_to_string(self.root.join(name))
                    && let Ok(pid) = text.trim().parse::<i32>()
                    && pid > 0
                {
                    let pid = Pid::from_raw(pid).unwrap();
                    if let (Ok(group), Ok(owner)) = (
                        rustix::process::getpgid(Some(pid)),
                        rustix::process::getsid(Some(pid)),
                    ) && group.as_raw_pid() != session
                        && owner.as_raw_pid() == session
                    {
                        let _ = rustix::process::kill_process_group(group, Signal::KILL);
                    }
                }
            }
            let _ =
                rustix::process::kill_process_group(Pid::from_raw(session).unwrap(), Signal::KILL);
            let _ = self.child.wait();
        }
    }
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let f = Fixture::boat_fixture();
        delegate(&f, false);
        let helper = f.dir.path().join("fake/local.sh");
        fs::write(
            &helper,
            fs::read_to_string(&helper).unwrap().replace(
                "exec \"$@\"",
                "echo $$ > provider/helper-pid\nexec flock provider/guard-lock \"$@\"",
            ),
        )
        .unwrap();
        let flags = rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY;
        let master = rustix::pty::openpt(flags).unwrap();
        rustix::pty::grantpt(&master).unwrap();
        rustix::pty::unlockpt(&master).unwrap();
        let name = rustix::pty::ptsname(&master, Vec::new()).unwrap();
        let slave =
            File::from(rustix::fs::open(&name, flags.into(), rustix::fs::Mode::empty()).unwrap());
        let mut master = File::from(master);
        let tty = slave.try_clone().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "\"$@\" <&0 & owner=$!; echo $owner > provider/wrapper-pid; wait $owner; echo $? > provider/wrapper-status; while :; do sleep 1; done", "sh", env!("CARGO_BIN_EXE_rota-test-host"), "slot", "run", "--", "sh", "-c", "read token; test \"$token\" = start || exit 42; echo $$ > provider/command-pid; exec sleep 120"])
            .env("ROTA_CONFIG", f.dir.path().join("config.toml"))
            .current_dir(f.dir.path()).stdin(slave.try_clone().unwrap()).stdout(slave.try_clone().unwrap()).stderr(slave);
        scrub(&mut command);
        platform::session_on_exec(&mut command, tty);
        let _session = Session {
            child: command.spawn().unwrap(),
            root: f.dir.path().join("provider"),
        };
        master.write_all(b"start\n").unwrap();
        until(|| {
            f.dir
                .path()
                .join("provider/command-pid")
                .exists()
                .then_some(())
        });
        let wrapper: i32 = fs::read_to_string(f.dir.path().join("provider/wrapper-pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let _ = rustix::process::kill_process(
            Pid::from_raw(wrapper).unwrap(),
            Signal::from_named_raw(signal).unwrap(),
        );
        let status = until(|| {
            fs::read_to_string(f.dir.path().join("provider/wrapper-status"))
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
        });
        assert_eq!(status, 128 + signal);
        assert_eq!(
            rustix::termios::tcgetpgrp(&master).unwrap().as_raw_pid(),
            _session.child.id() as i32
        );
        let pid = fs::read_to_string(f.dir.path().join("provider/command-pid")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let stopped = loop {
            let dead =
                fs::read_to_string(format!("/proc/{}/stat", pid.trim())).map_or(true, |stat| {
                    stat.rsplit_once(')')
                        .is_some_and(|(_, fields)| fields.starts_with(" Z "))
                });
            if dead || Instant::now() >= deadline {
                break dead;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            stopped,
            "terminal foreground command survived signal {signal}"
        );
        let lock = File::open(f.dir.path().join("provider/guard-lock")).unwrap();
        assert!(FileExt::try_lock_exclusive(&lock).is_ok());
    }
}

impl Fixture {
    fn new(count: u16) -> Self {
        Self::configure(count, true, "")
    }

    fn configure(count: u16, legacy: bool, extra: &str) -> Self {
        let case = CASE.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().display();
        let legacy_config = if legacy {
            fs::create_dir_all(dir.path().join("legacy/queue")).unwrap();
            format!("legacy_dir = '{path}/legacy'\n")
        } else {
            String::new()
        };
        let locks = if legacy {
            let paths: Vec<_> = (1..=count + 1)
                .map(|slot| {
                    let lock = dir.path().join(format!("legacy/old-{slot}.lock"));
                    File::create(&lock).unwrap();
                    fs::set_permissions(&lock, fs::Permissions::from_mode(0o444)).unwrap();
                    format!("'{}'", lock.display())
                })
                .collect();
            format!("lock_files = [{}]\n", paths.join(", "))
        } else {
            String::new()
        };
        fs::write(
            dir.path().join("config.toml"),
            format!(
                r#"
state_file = '{path}/state/last-pick'
log_file = '{path}/state/decisions.jsonl'

[slots]
count = {count}
{legacy_config}{locks}{extra}

[[backends]]
name = 'fake'
roles = ['implement']
guard = ['false']
allow_unknown_quota = true
[backends.commands]
medium = ['never-start-this', '{{prompt}}']
high = ['never-start-this', '{{prompt}}']
"#
            ),
        )
        .unwrap();
        Self {
            dir,
            shared: legacy,
            _case: case,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("state/slots")
    }

    fn boat_fixture() -> Self {
        let fixture = Self::configure(
            1,
            true,
            "[slots.boat]\nallow_shapes = ['nix-gate']\nlarge_max_memory_gib = 8\n",
        );
        fixture.policy("boat-on", "enabled");
        for name in ["fake", "provider"] {
            fs::create_dir(fixture.dir.path().join(name)).unwrap();
        }
        let boat = fixture.dir.path().join("fake/boat");
        // Build sandboxes provide Bash on PATH, not /usr/bin/env.
        let bash = Command::new("sh")
            .args(["-c", "command -v bash"])
            .output()
            .unwrap();
        assert!(bash.status.success());
        let bash = String::from_utf8(bash.stdout).unwrap();
        let (_, body) = include_str!("fixtures/boat.sh").split_once('\n').unwrap();
        fs::write(&boat, format!("#!{}\n{body}", bash.trim())).unwrap();
        fs::set_permissions(&boat, fs::Permissions::from_mode(0o755)).unwrap();
        let nix = fixture.dir.path().join("fake/nix");
        fs::write(&nix, "#!/bin/sh\ntouch local-ran\n").unwrap();
        fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            fixture.dir.path().join(".gitignore"),
            "config.toml\nstate/\nlegacy/\nfake/\nprovider/\nlocal-ran\nsecret\n",
        )
        .unwrap();
        fs::write(fixture.dir.path().join("source"), "captured\n").unwrap();
        fs::write(fixture.dir.path().join("Cargo.lock"), "test lock\n").unwrap();
        fs::write(fixture.dir.path().join("flake.lock"), "{}\n").unwrap();
        fs::write(fixture.dir.path().join("secret"), "PRIVATE-SENTINEL").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Build",
                "-c",
                "user.email=build@invalid",
                "commit",
                "-qm",
                "source",
            ],
        ] {
            assert!(
                Command::new("git")
                    .current_dir(fixture.dir.path())
                    .args(args)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        fixture
    }

    fn boat_command(&self, args: &[&str]) -> Command {
        let mut command = self.rota(args);
        command
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().join("fake").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("BOAT_TEST_ROOT", self.dir.path().join("provider"));
        command
    }

    fn pending_boat(&self, expired: bool) {
        for directory in [
            self.dir.path().join("state"),
            self.root(),
            self.root().join("boat"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let now = ns() / 1_000_000_000;
        let record = serde_json::json!({
            "schema": 1, "run_id": "11111111111111111111111111111111", "pid": 999999,
            "phase": "creating", "where": "boat", "vm_id": null, "command_pid": null,
            "tree": "unknown", "base": "unknown", "started": now - 4000,
            "expires_at": if expired { now - 1 } else { now + 4000 },
            "duration_seconds": null, "exit_code": null, "usage": null,
            "environment_version": 1, "reason": null, "credentials_absent": false
        });
        let path = self.root().join("boat/current.json");
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// The test host, with Boat as its route.
    fn rota(&self, args: &[&str]) -> Command {
        self.program(env!("CARGO_BIN_EXE_rota-test-host"), args)
    }

    fn program(&self, binary: &str, args: &[&str]) -> Command {
        let mut command = Command::new(binary);
        command
            .current_dir(self.dir.path())
            .env("ROTA_CONFIG", self.dir.path().join("config.toml"))
            .env("ROTA_SLOT_POLL_MS", "50");
        scrub(&mut command).args(args);
        command
    }

    fn output(&self, args: &[&str]) -> Output {
        self.rota(args).output().unwrap()
    }

    fn spawn(&self, args: &[&str]) -> Running {
        self.spawn_in(self.dir.path(), args)
    }

    fn spawn_in(&self, cwd: &Path, args: &[&str]) -> Running {
        Running(
            self.rota(args)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }

    fn hold(&self, slot: u16) -> File {
        let lock = if self.shared {
            self.dir.path().join(format!("legacy/old-{slot}.lock"))
        } else {
            fs::create_dir_all(self.root()).unwrap();
            let path = self.root().join(format!("slot-{slot}.lock"));
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap();
            path
        };
        let lock = File::open(lock).unwrap();
        lock.try_lock_exclusive().unwrap();
        lock
    }

    fn list(&self) -> Value {
        let output = self.output(&["slot", "ls", "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn queued(&self, pid: u32) -> Value {
        until(|| {
            self.list()["waiters"]
                .as_array()
                .unwrap()
                .iter()
                .find(|ticket| ticket["pid"] == pid)
                .cloned()
        })
    }

    fn holder(&self, pid: u32) -> Value {
        until(|| {
            self.list()["holders"]
                .as_array()
                .unwrap()
                .iter()
                .find(|holder| holder["pid"] == pid)
                .cloned()
        })
    }

    fn policy(&self, name: &str, value: &str) {
        fs::write(self.dir.path().join("legacy").join(name), value).unwrap();
    }

    fn ticket_path(&self, ticket: &Value) -> PathBuf {
        self.root()
            .join("queue")
            .join(format!("{}-{}.json", ticket["queued_ns"], ticket["pid"]))
    }

    fn events(&self) -> Vec<Value> {
        fs::read_to_string(self.root().join("log.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn grants(&self) -> Vec<(u32, u16)> {
        self.events()
            .iter()
            .filter(|event| event["event"] == "grant")
            .map(|event| {
                (
                    event["pid"].as_u64().unwrap() as u32,
                    event["slot"].as_u64().unwrap() as u16,
                )
            })
            .collect()
    }

    fn yielding_cwd(&self) -> PathBuf {
        let cwd = self.dir.path().join("yielding");
        fs::create_dir(&cwd).unwrap();
        self.policy("yield-roots", cwd.to_str().unwrap());
        cwd
    }
}

struct Running(Child);

impl Running {
    fn pid(&self) -> u32 {
        self.0.id()
    }

    fn wait(&mut self) -> ExitStatus {
        until(|| self.0.try_wait().unwrap())
    }

    fn success(&mut self) {
        let status = self.wait();
        assert!(status.success(), "{status}: {}", self.stderr());
    }

    fn stderr(&mut self) -> String {
        let mut text = String::new();
        self.0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn until<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for slot state"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[test]
fn review_missing_build_identity_proof_retires_vm_without_a_local_duplicate() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "identity-present").unwrap();
    let result = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(125));
    assert_eq!(f.list()["boat"]["reason"], "remote-build-identity-unproven");
    assert!(f.dir.path().join("provider/deleted").exists());
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn review_lost_result_deletes_the_vm_with_its_unfinished_children() {
    let f = Fixture::boat_fixture();
    let args = ["slot", "run", "--", "nix", "build", ".#default"];
    fs::write(f.dir.path().join("provider/mode"), "disconnected").unwrap();
    assert_eq!(
        f.boat_command(&args).output().unwrap().status.code(),
        Some(125)
    );
    // This run is the only one, so the deletion below is its own.
    let calls = calls(&f);
    let start = position(&calls, "new", 0).unwrap();
    assert!(position(&calls, "delete", start).is_some(), "{calls:?}");
    assert!(f.dir.path().join("provider/deleted").exists());
    assert_eq!(
        fs::read_to_string(f.dir.path().join("provider/state"))
            .unwrap()
            .trim(),
        "deleted"
    );
    assert!(!f.dir.path().join("provider/old-command").exists());
    assert_eq!(f.list()["boat"]["vm_id"], "bx_test");
    assert_eq!(f.list()["boat"]["phase"], "stopped");
    // Its deletion was confirmed, so the next run starts its own fresh VM.
    fs::write(f.dir.path().join("provider/mode"), "").unwrap();
    assert!(f.boat_command(&args).output().unwrap().status.success());
    assert_eq!(f.list()["boat"]["vm_id"], "bx_next");
}

#[test]
fn review_incomplete_remote_log_is_not_accepted_as_a_successful_gate() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "incomplete-log").unwrap();
    let result = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(125));
    assert_eq!(f.list()["boat"]["reason"], "remote-log-incomplete");
    assert_eq!(f.list()["boat"]["phase"], "stopped");
    assert!(f.dir.path().join("provider/deleted").exists());
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn review_crash_recovery_deletes_uncertain_vm_instead_of_reviving_old_work() {
    let f = Fixture::boat_fixture();
    f.pending_boat(false);
    let current = f.root().join("boat/current.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&current).unwrap()).unwrap();
    record["phase"] = "running".into();
    record["vm_id"] = "bx_test".into();
    record["command_pid"] = 123.into();
    record["credentials_absent"] = true.into();
    fs::write(current, serde_json::to_vec(&record).unwrap()).unwrap();
    fs::write(f.dir.path().join("provider/state"), "running").unwrap();
    fs::write(f.dir.path().join("provider/old-command"), "uncertain build").unwrap();
    let output = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!f.dir.path().join("provider/old-command").exists());
    assert_eq!(f.list()["boat"]["vm_id"], "bx_next");
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert!(calls.find("delete\n").unwrap() < calls.find("new\n").unwrap());
    assert!(!calls.lines().any(|line| line == "resume"));
}

#[test]
fn review_canceled_preparation_returns_signal_status_after_cleanup() {
    let f = Fixture::boat_fixture();
    let args = ["slot", "run", "--", "nix", "build", ".#default"];
    fs::write(f.dir.path().join("provider/mode"), "info-hold").unwrap();
    let mut run = Running(
        f.boat_command(&args)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| {
        f.dir
            .path()
            .join("provider/preflight")
            .exists()
            .then_some(())
    });
    rustix::process::kill_process(Pid::from_raw(run.pid() as i32).unwrap(), Signal::TERM).unwrap();
    assert_eq!(run.wait().code(), Some(143));
    assert_eq!(f.list()["boat"]["phase"], "stopped");
    assert!(f.dir.path().join("provider/deleted").exists());
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn quiet_exemption_runs_locally_while_remote_front_ticket_waits() {
    let f = Fixture::boat_fixture();
    f.policy("quiet-until", FUTURE);
    let mut older = Running(
        f.boat_command(&["slot", "run", "--front", "--", "nix", "build", ".#default"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    f.queued(older.pid());
    let mut exempt = Running(
        f.boat_command(&[
            "slot",
            "run",
            "--quiet-exempt",
            "--",
            "nix",
            "build",
            ".#default",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    );
    until(|| f.dir.path().join("local-ran").exists().then_some(()));
    exempt.success();
    assert!(older.0.try_wait().unwrap().is_none());
    assert!(!f.dir.path().join("provider/attached").exists());
    assert!(!f.root().join("boat/lease.lock").exists());
    assert!(f.events().iter().any(|event| {
        event["pid"] == exempt.pid()
            && event["event"] == "exit"
            && event["where"] == "local"
            && event["fallback_reason"] == "timing-local"
    }));
    f.policy("quiet-until", "0");
    older.success();
}

#[test]
fn malformed_quiet_policy_refuses_listing_and_exempt_admission() {
    let f = Fixture::new(1);
    for text in ["not-an-epoch", "-1", "+1", "", "18446744073709551616"] {
        f.policy("quiet-until", text);
        assert!(!f.output(&["slot", "ls", "--json"]).status.success());
        let output = f.output(&[
            "slot",
            "run",
            "--quiet-exempt",
            "--",
            "sh",
            "-c",
            "touch policy-bypassed",
        ]);
        assert!(!output.status.success());
        assert!(!f.dir.path().join("policy-bypassed").exists());
    }
    let path = f.dir.path().join("legacy/quiet-until");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(!f.output(&["slot", "ls", "--json"]).status.success());
    fs::remove_dir(path).unwrap();
    assert!(f.output(&["slot", "ls", "--json"]).status.success());
    assert!(f.output(&["slot", "run", "--", "true"]).status.success());
}

#[test]
fn listing_counts_a_held_extra_slot_after_its_policy_expires() {
    let f = Fixture::new(2);
    let _held = f.hold(3);
    assert_eq!(f.list()["holders"], serde_json::json!([{"slot": 3}]));
}

#[test]
fn listing_refuses_broken_local_observation_but_allows_absent_optional_slot() {
    let f = Fixture::new(1);
    assert_eq!(f.list()["boat_lease_held"], false);
    assert!(!f.root().join("boat/lease.lock").exists());
    fs::remove_file(f.dir.path().join("legacy/old-2.lock")).unwrap();
    assert!(f.output(&["slot", "ls", "--json"]).status.success());
    fs::remove_file(f.dir.path().join("legacy/old-1.lock")).unwrap();
    assert!(!f.output(&["slot", "ls", "--json"]).status.success());
}

#[test]
fn listing_does_not_ignore_queue_lock_inspection_errors() {
    let f = Fixture::new(1);
    fs::create_dir_all(f.root()).unwrap();
    fs::write(f.root().join("queue"), "not a queue directory").unwrap();
    assert!(!f.output(&["slot", "ls", "--json"]).status.success());
}

#[test]
fn listing_observes_pre_record_lease_without_changing_its_inode() {
    let f = Fixture::boat_fixture();
    let path = f.root().join("boat/lease.lock");
    for directory in [f.dir.path().join("state"), f.root(), f.root().join("boat")] {
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let lock = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    use std::os::unix::fs::MetadataExt;
    let inode = lock.metadata().unwrap().ino();
    FileExt::lock_exclusive(&lock).unwrap();
    assert_eq!(f.list()["boat_lease_held"], true);
    assert!(!f.root().join("boat/current.json").exists());
    assert_eq!(path.metadata().unwrap().ino(), inode);
    FileExt::unlock(&lock).unwrap();
    assert_eq!(f.list()["boat_lease_held"], false);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
    let listing = f.list();
    assert!(listing["boat_lease_held"].is_null());
    assert!(listing["boat_error"].as_str().unwrap().contains("unsafe"));
    assert_eq!(path.metadata().unwrap().ino(), inode);
}

#[test]
fn review_bad_boat_record_does_not_hide_local_holders() {
    let f = Fixture::boat_fixture();
    f.pending_boat(false);
    let path = f.root().join("boat/current.json");
    fs::write(&path, "{broken").unwrap();
    let _local = f.hold(1);
    let output = f.output(&["slot", "ls", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listing["holders"], serde_json::json!([{"slot": 1}]));
    assert_eq!(listing["boat_error"], "boat-record-format");
    let text = f.output(&["slot", "ls"]);
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("slot 1 occupied"));
    assert!(String::from_utf8_lossy(&text.stdout).contains("boat-record-format"));
    assert_eq!(fs::read_to_string(path).unwrap(), "{broken");
}

#[test]
fn review_unresolved_create_uses_local_fallback_without_another_vm_start() {
    let f = Fixture::boat_fixture();
    f.pending_boat(false);
    for mode in ["", "reconcile-unreachable"] {
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let output = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(f.dir.path().join("local-ran").exists());
        fs::remove_file(f.dir.path().join("local-ran")).unwrap();
        assert_eq!(f.list()["boat"]["phase"], "creating");
    }
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert!(!calls.lines().any(|line| matches!(line, "new" | "resume")));
}

#[test]
fn review_expired_create_with_no_vm_is_reconciled_and_remote_work_can_resume() {
    let f = Fixture::boat_fixture();
    f.pending_boat(true);
    fs::write(f.dir.path().join("provider/mode"), "unsafe").unwrap();
    assert!(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(f.list()["boat"]["phase"], "stopped");
    assert_eq!(f.list()["boat"]["reason"], "expired-create-absent");
    fs::write(f.dir.path().join("provider/mode"), "").unwrap();
    assert!(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(f.list()["boat"]["vm_id"], "bx_test");
}

#[test]
fn review_canceling_reconciliation_never_falls_back_or_starts_a_vm() {
    let f = Fixture::boat_fixture();
    f.pending_boat(false);
    fs::write(f.dir.path().join("provider/mode"), "reconcile-hold").unwrap();
    let mut run = Running(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| {
        f.dir
            .path()
            .join("provider/preflight")
            .exists()
            .then_some(())
    });
    rustix::process::kill_process(Pid::from_raw(run.pid() as i32).unwrap(), Signal::TERM).unwrap();
    assert_eq!(run.wait().code(), Some(143));
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn review_refused_create_does_not_trap_the_next_gate_in_an_exit_125_loop() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "create-refused").unwrap();
    for _ in 0..2 {
        let output = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| *line == "new").count(), 1);
}

#[test]
fn review_local_fallback_retains_full_output_and_status_in_a_private_run() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "exhausted").unwrap();
    fs::write(f.dir.path().join("fake/nix"), "#!/bin/sh\nhead -c 1100000 /dev/zero | tr '\\0' x\nprintf 'stdout-tail\\n'\nprintf 'stderr-tail\\n' >&2\nexit 37\n").unwrap();
    let output = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert!(output.stdout.ends_with(b"stdout-tail\n"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stderr-tail"));
    let run = fs::read_dir(f.root().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let receipt: Value =
        serde_json::from_slice(&fs::read(run.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt["where"], "local");
    assert_eq!(receipt["exit_code"], 37);
    assert_eq!(receipt["output_complete"], true);
    assert!(
        receipt["fallback_reason"]
            .as_str()
            .unwrap()
            .contains("budget")
    );
    let log = fs::read(run.join("output.log")).unwrap();
    assert_eq!(log.iter().filter(|byte| **byte == b'x').count(), 1100000);
    assert!(String::from_utf8_lossy(&log).contains("stdout-tail"));
    assert!(String::from_utf8_lossy(&log).contains("stderr-tail"));
    assert_eq!(
        fs::metadata(run.join("output.log"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
}

#[test]
fn review_two_shared_locks_disable_timed_extra_admission() {
    let f = Fixture::new(2);
    let path = f.dir.path().join("config.toml");
    let third = f.dir.path().join("legacy/old-3.lock");
    let config = fs::read_to_string(&path)
        .unwrap()
        .replace(&format!(", '{}'", third.display()), "");
    fs::write(path, config).unwrap();
    fs::remove_file(&third).unwrap();
    let output = f.output(&["slot", "ls", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first = f.hold(1);
    let second = f.hold(2);
    f.policy("extra-slot-until", "4102444800");
    let mut run = f.spawn(&["slot", "run", "--front", "--", "true"]);
    assert_eq!(f.queued(run.pid())["slots"], serde_json::json!([1, 2]));
    drop(second);
    run.success();
    assert_eq!(f.grants(), [(run.pid(), 2)]);
    assert!(!third.exists());
    drop(first);
}

#[test]
fn boat_holds_only_remote_capacity_and_returns_exact_result_and_full_log() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/hold"), "").unwrap();
    fs::write(f.dir.path().join("provider/code"), "37").unwrap();
    fs::write(f.dir.path().join("source"), "uncommitted\n").unwrap();
    let mut remote = Running(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| {
        f.dir
            .path()
            .join("provider/attached")
            .exists()
            .then_some(())
    });
    let listed = f.list();
    assert_eq!(listed["holders"][0]["where"], "boat");
    assert_eq!(listed["holders"][0]["vm_id"], "bx_test");
    assert_eq!(listed["holders"][0]["credentials_absent"], true);
    let local = f.output(&["slot", "run", "--", "true"]);
    assert!(
        local.status.success(),
        "local capacity blocked by remote run"
    );
    fs::remove_file(f.dir.path().join("provider/hold")).unwrap();
    assert_eq!(remote.wait().code(), Some(37));
    let mut stdout = String::new();
    remote
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.contains("remote stdout"));
    assert!(!stdout.contains("\"type\":\"stdout\""));
    assert!(remote.stderr().contains("remote stderr"));
    let record = f.list()["boat"].clone();
    assert_eq!(record["phase"], "stopped");
    assert_eq!(record["exit_code"], 37);
    assert_eq!(record["usage"]["running"], false);
    assert_eq!(
        fs::read(f.dir.path().join("provider/upload/source")).unwrap(),
        b"uncommitted\n"
    );
    assert!(!f.dir.path().join("provider/upload/secret").exists());
    let log = f
        .root()
        .join("runs")
        .join(record["run_id"].as_str().unwrap())
        .join("output.log");
    assert_eq!(fs::metadata(log).unwrap().len(), 1100000);
    assert!(!f.dir.path().join("local-ran").exists());
}

#[test]
fn boat_policy_budget_and_connectivity_negatives_use_local_queue() {
    let f = Fixture::boat_fixture();
    let profile = f
        .boat_command(&["slot", "run", "--", "nix", "profile", "list"])
        .output()
        .unwrap();
    assert!(profile.status.success());
    assert!(!f.dir.path().join("provider/calls").exists());
    assert!(!f.root().join("runs").exists());
    for mode in ["exhausted", "unreachable", "unsafe"] {
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        fs::remove_file(f.dir.path().join("local-ran")).unwrap();
        let result = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        // Under the rota host the fallback line names rota (was a hard-coded `eon agent slot:`).
        let stderr = String::from_utf8_lossy(&result.stderr);
        // Under the rota host every fallback line names rota (were hard-coded `eon agent slot:`),
        // including the non-terminal local fallback's output line.
        assert!(
            stderr.contains("rota slot: where=local reason="),
            "{stderr}"
        );
        if mode != "unreachable" {
            assert!(stderr.contains("rota slot: local output "), "{stderr}");
        }
        assert!(!stderr.contains("eon agent"), "{stderr}");
        assert!(f.dir.path().join("local-ran").exists());
    }
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert!(!calls.lines().any(|line| line == "new"));
}

#[test]
fn boat_unqualified_caps_and_timing_do_not_query_or_wait_for_the_provider() {
    let f = Fixture::boat_fixture();
    let args = ["slot", "run", "--", "nix", "build", ".#default"];
    let high = f
        .boat_command(&args)
        .env("HEAVY_MEM", "12G")
        .output()
        .unwrap();
    assert!(high.status.success());
    assert!(String::from_utf8_lossy(&high.stderr).contains("unqualified-ceiling"));
    let invalid = f
        .boat_command(&args)
        .env("HEAVY_MEM", "13G")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let quiet = f
        .boat_command(&[
            "slot",
            "run",
            "--quiet-exempt",
            "--",
            "nix",
            "build",
            ".#default",
        ])
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(String::from_utf8_lossy(&quiet.stderr).contains("timing-local"));
    assert!(!f.dir.path().join("provider/calls").exists());
    assert!(!f.root().join("boat/lease.lock").exists());
}

const PINNED: &str = "eon-builds-nix-2-34-7-large-klt8h-v1";

fn calls(f: &Fixture) -> Vec<String> {
    fs::read_to_string(f.dir.path().join("provider/calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn position(calls: &[String], verb: &str, after: usize) -> Option<usize> {
    calls[after..]
        .iter()
        .position(|c| c == verb)
        .map(|i| i + after)
}

#[test]
fn boat_runs_every_qualified_cap_on_the_pinned_large_image_and_deletes_it() {
    for (cap, observed) in [
        ("6G", "6442450944 null 2147483648\n"),
        ("8G", "8589934592 null 2147483648\n"),
    ] {
        let f = Fixture::boat_fixture();
        let provider = f.dir.path().join("provider");
        fs::write(provider.join("observed-memory"), observed).unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .env("HEAVY_MEM", cap)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{cap}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            fs::read_to_string(provider.join("type")).unwrap().trim(),
            "large"
        );
        let new = fs::read_to_string(provider.join("new-args")).unwrap();
        assert!(
            new.contains(&format!(
                "--type large --ttl 3600 --from {PINNED} --no-snapshots"
            )),
            "{new}"
        );
        assert_eq!(
            fs::read_to_string(provider.join("requested-memory")).unwrap(),
            observed
        );
        // The named row is read before the start and again before anything runs on the VM.
        let calls = calls(&f);
        let before = position(&calls, "snapshots", 0).unwrap();
        let start = position(&calls, "new", before).unwrap();
        let again = position(&calls, "snapshots", start).unwrap();
        let push = position(&calls, "push", again).unwrap();
        assert!(position(&calls, "delete", push).is_some(), "{calls:?}");
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c.as_str(), "resume" | "snapshot"))
        );
        assert!(!f.root().join("boat/warm.json").exists());
        assert_eq!(f.list()["boat"]["phase"], "stopped");
        let run = f.list()["boat"]["run_id"].as_str().unwrap().to_owned();
        let receipt: Value = serde_json::from_slice(
            &fs::read(f.root().join("runs").join(run).join("receipt.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["image"], PINNED);
        assert!(!f.dir.path().join("local-ran").exists());
    }
}

#[test]
fn boat_without_a_pinned_image_for_the_cap_stays_local_before_any_provider_start() {
    let f = Fixture::boat_fixture();
    let config = f.dir.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("large_max_memory_gib = 8", "large_max_memory_gib = 0"),
    )
    .unwrap();
    let out = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .env("HEAVY_MEM", "6G")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("where=local reason=boat-capacity class=nix cap=6G image-unpinned"),
        "{stderr}"
    );
    assert!(f.dir.path().join("local-ran").exists());
    assert!(!calls(&f).iter().any(|c| c == "new"));
}

#[test]
fn boat_unverified_or_mismatched_image_before_start_stays_local_with_its_reason() {
    for (mode, reason) in [
        (
            "image-absent",
            "boat-image-unverified check=snapshot-absent",
        ),
        ("image-wrong-id", "boat-image-mismatch check=snapshot-id"),
        (
            "environment-version",
            "boat-image-mismatch check=environment",
        ),
    ] {
        let f = Fixture::boat_fixture();
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let args = ["slot", "run", "--", "nix", "build", ".#default"];
        let out = f.boat_command(&args).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{mode}: {stderr}");
        assert!(
            stderr.contains(&format!("where=local reason={reason}")),
            "{mode}: {stderr}"
        );
        assert!(f.dir.path().join("local-ran").exists(), "{mode}");
        assert!(!calls(&f).iter().any(|c| c == "new"), "{mode}");
        // Nothing was created, so nothing is left to reconcile: the next run goes remote.
        assert!(!f.root().join("boat/current.json").exists(), "{mode}");
        fs::remove_file(f.dir.path().join("local-ran")).unwrap();
        fs::write(f.dir.path().join("provider/mode"), "").unwrap();
        let out = f.boat_command(&args).output().unwrap();
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!f.dir.path().join("local-ran").exists(), "{mode}");
        assert_eq!(f.list()["boat"]["phase"], "stopped", "{mode}");
    }
}

#[test]
fn boat_image_refused_after_start_retires_the_vm_without_reading_from_it() {
    for (mode, reason) in [
        ("marker-absent", "boat-image-unverified check=recipe-marker"),
        ("marker-mismatch", "boat-image-mismatch check=recipe-marker"),
        ("image-drift", "boat-image-mismatch check=snapshot-id"),
        ("wrong-type", "boat-image-mismatch check=vm-identity"),
        (
            "identity-missing",
            "boat-image-unverified check=vm-identity",
        ),
        ("vm-inactive", "boat-image-unverified check=vm-state"),
    ] {
        let f = Fixture::boat_fixture();
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{mode}: {stderr}");
        assert!(
            stderr.contains(&format!("where=local reason={reason}")),
            "{mode}: {stderr}"
        );
        assert!(f.dir.path().join("local-ran").exists(), "{mode}");
        let calls = calls(&f);
        let start = position(&calls, "new", 0).unwrap();
        for verb in ["push", "scp"] {
            assert!(
                !calls.contains(&verb.to_owned()),
                "{mode}: {verb} in {calls:?}"
            );
        }
        assert!(
            position(&calls, "delete", start).is_some(),
            "{mode}: {calls:?}"
        );
        assert!(!f.dir.path().join("provider/detached").exists(), "{mode}");
        assert_eq!(f.list()["boat"]["phase"], "stopped", "{mode}");
    }
}

#[test]
fn boat_unavailable_large_type_never_waits_for_a_busy_lease() {
    let f = Fixture::boat_fixture();
    let directory = f.root().join("boat");
    fs::create_dir_all(&directory).unwrap();
    for dir in [f.dir.path().join("state"), f.root(), directory.clone()] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let lease = File::create(directory.join("lease.lock")).unwrap();
    lease.try_lock_exclusive().unwrap();
    fs::write(f.dir.path().join("provider/mode"), "large-unavailable").unwrap();
    let result = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("machine-unavailable"));
    assert_eq!(
        fs::read_to_string(f.dir.path().join("provider/calls")).unwrap(),
        "limits\n"
    );
}

#[test]
fn boat_reports_oom_cause_and_preparation_through_exit_and_receipt() {
    for (mode, code, cause, preparation) in [
        ("prep-oom", 137, "job-limit", true),
        ("job-oom", 137, "job-limit", false),
        ("vm-oom", 125, "vm-wide", false),
        ("mixed-oom", 125, "vm-wide", false),
        ("unknown-oom", 125, "unknown", false),
        ("normal", 137, "none", false),
    ] {
        let f = Fixture::boat_fixture();
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        fs::write(f.dir.path().join("provider/code"), "137").unwrap();
        let out = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(code), "{mode}: {stderr}");
        assert_eq!(
            stderr.contains("OOM-KILLED"),
            cause == "job-limit",
            "{mode}: {stderr}"
        );
        assert_eq!(
            stderr.contains("Boat VM out of memory"),
            cause == "vm-wide",
            "{mode}: {stderr}"
        );
        let event = f.events().pop().unwrap();
        assert_eq!(event["memory"]["cause"], cause);
        assert_eq!(event["memory"]["preparation"], preparation);
        assert_eq!(f.list()["boat"]["memory"], event["memory"]);
        assert!(!f.dir.path().join("local-ran").exists());
    }
}

#[test]
fn boat_pre_work_memory_refusals_require_cleanup_and_block_repeat_starts() {
    for (mode, reason) in [
        ("headroom", "remote-memory-headroom"),
        ("memory-unavailable", "remote-memory-unavailable"),
    ] {
        let f = Fixture::boat_fixture();
        let provider = f.dir.path().join("provider");
        fs::write(provider.join("mode"), mode).unwrap();
        let args = ["slot", "run", "--", "nix", "build", ".#default"];
        for expected in [reason, "qualification-stale"] {
            let out = f
                .boat_command(&args)
                .env("HEAVY_MEM", "6G")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{mode}: {stderr}");
            assert!(stderr.contains(expected), "{mode}: {stderr}");
            fs::remove_file(f.dir.path().join("local-ran")).unwrap();
        }
        let calls = fs::read_to_string(provider.join("calls")).unwrap();
        for verb in ["new", "stop", "delete"] {
            assert_eq!(calls.lines().filter(|line| *line == verb).count(), 1);
        }
        assert_eq!(
            fs::read_to_string(provider.join("state")).unwrap().trim(),
            "deleted"
        );
        let evidence = fs::read(f.root().join("boat/large_qualification_hold.json")).unwrap();
        let record: Value = serde_json::from_slice(&evidence).unwrap();
        assert_eq!(record["reason"], reason);
        if mode == "headroom" {
            assert_eq!(record["evidence"]["available"], 1073741824u64);
        } else {
            assert!(record["evidence"].is_null());
        }
    }
}

#[test]
fn boat_gate_exit_96_does_not_fall_back_or_hold_qualification() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/code"), "96").unwrap();
    let out = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(96));
    assert!(!f.dir.path().join("local-ran").exists());
    assert!(!f.root().join("boat/large_qualification_hold.json").exists());
}

#[test]
fn canceling_boat_preflight_exits_without_starting_remote_or_local_work() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "preflight-hold").unwrap();
    let mut run = Running(
        f.boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| {
        f.dir
            .path()
            .join("provider/preflight")
            .exists()
            .then_some(())
    });
    let listing = f.list();
    assert_eq!(listing["boat_lease_held"], true);
    assert_eq!(listing["boat_lease_pid"], run.pid());
    assert!(listing.get("boat").is_none());
    assert!(listing["waiters"].as_array().unwrap().is_empty());
    rustix::process::kill_process(Pid::from_raw(run.pid() as i32).unwrap(), Signal::TERM).unwrap();
    assert_eq!(run.wait().code(), Some(143));
    assert!(!f.dir.path().join("local-ran").exists());
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert!(!calls.lines().any(|line| matches!(line, "new" | "resume")));
}

#[test]
fn lost_boat_contact_after_start_stops_vm_without_duplicate_local_run() {
    let f = Fixture::boat_fixture();
    fs::write(f.dir.path().join("provider/mode"), "disconnected").unwrap();
    let result = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(125));
    assert!(!f.dir.path().join("local-ran").exists());
    assert_eq!(f.list()["boat"]["phase"], "stopped");
    assert!(
        fs::read_to_string(f.dir.path().join("provider/calls"))
            .unwrap()
            .lines()
            .any(|line| line == "stop")
    );
}

#[test]
fn slow_boat_stop_reads_final_usage_before_one_delete() {
    // delayed-stop reaches stopped after a few polls; stuck-stop never does within the
    // stop wait, so retirement falls through to the one delete.
    for mode in ["delayed-stop", "stuck-stop"] {
        let f = Fixture::boat_fixture();
        fs::write(f.dir.path().join("provider/mode"), mode).unwrap();
        let started = Instant::now();
        let result = f
            .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(f.list()["boat"]["phase"], "stopped", "{mode}");
        let calls = calls(&f);
        let stop = position(&calls, "stop", 0).unwrap();
        let usage = position(&calls, "usage", stop).unwrap();
        let delete = position(&calls, "delete", usage).unwrap();
        assert_eq!(
            calls.iter().filter(|c| *c == "delete").count(),
            1,
            "{mode}: {calls:?}"
        );
        assert!(!calls[stop..delete].contains(&"delete".to_owned()));
        assert!(f.dir.path().join("provider/deleted").exists(), "{mode}");
        assert!(f.list()["boat"]["usage"].is_object(), "{mode}");
        // Retirement fits the cleanup budget (60 s) even when the VM never stops.
        assert!(started.elapsed() < Duration::from_secs(60 + 20), "{mode}");
    }
}

#[test]
fn timeout_wrapped_gate_warms_its_inputs_and_runs_its_original_argv_on_boat() {
    let f = Fixture::boat_fixture();
    let config = f.dir.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replace(
            "allow_shapes = ['nix-gate']",
            "allow_shapes = ['nix-gate', 'cargo-clippy']\ntimeout_seconds = 7200",
        ),
    )
    .unwrap();
    fs::write(f.dir.path().join("provider/mode"), "paid").unwrap();
    let argv = ["timeout", "85m", "nix", "build", ".#default"];
    let mut args = vec!["slot", "run", "--"];
    args.extend(argv);
    let result = f.boat_command(&args).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // The envelope reached Boat rather than the local queue.
    assert!(!f.dir.path().join("local-ran").exists());
    let calls = fs::read_to_string(f.dir.path().join("provider/calls")).unwrap();
    assert!(calls.lines().any(|line| line == "new"));
    let detached = fs::read(f.dir.path().join("provider/detached")).unwrap();
    let words: Vec<_> = detached
        .split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect();
    assert_eq!(words[0], "1", "a wrapped gate still warms the gate inputs");
    assert_eq!(words[1..], argv, "the VM runs the original argv");
    assert!(!calls.lines().any(|line| line == "snapshot"));
    assert!(!f.root().join("boat/warm.json").exists());
}

const FUTURE: &str = "4102444800";

#[test]
fn same_class_waiters_run_in_arrival_order() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let mut waiters = Vec::new();
    for name in ["first", "second", "third"] {
        let run = f.spawn(&[
            "slot",
            "run",
            "--",
            "sh",
            "-c",
            "echo \"$1\" >> order",
            "sh",
            name,
        ]);
        f.queued(run.pid());
        waiters.push(run);
    }
    drop(slot);
    for run in &mut waiters {
        run.success();
    }
    assert_eq!(
        fs::read_to_string(f.dir.path().join("order")).unwrap(),
        "first\nsecond\nthird\n"
    );
}

#[test]
fn nix_argv_and_explicit_class_respect_slots() {
    let f = Fixture::configure(
        2,
        false,
        r#"
[[slots.classes]]
name = 'nix'
match_argv = [['nix', 'build'], ['nix', 'profile']]
slots = [2]
[[slots.classes]]
name = 'run'
slots = [1, 2]
"#,
    );
    let nix = f.dir.path().join("nix");
    fs::write(&nix, "#!/bin/sh\nprintf '%s' \"$ROTA_SLOT\"\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let slot2 = f.hold(2);
    let mut direct = f.spawn(&["slot", "run", "--", nix.to_str().unwrap(), "build"]);
    let shell = format!(": && {} profile", nix.display());
    let mut joined = f.spawn(&["slot", "run", "--", "sh", "-c", &shell]);
    for run in [&direct, &joined] {
        let ticket = f.queued(run.pid());
        assert_eq!(ticket["class"], "nix");
        assert_eq!(ticket["slots"], serde_json::json!([2]));
    }
    let output = f.output(&[
        "slot",
        "run",
        "--class",
        "run",
        "--",
        nix.to_str().unwrap(),
        "build",
    ]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1");
    drop(slot2);
    direct.success();
    joined.success();
    assert!(f.grants().contains(&(direct.pid(), 2)));
    assert!(f.grants().contains(&(joined.pid(), 2)));
}

#[test]
fn dead_ticket_is_removed() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let mut dead = f.spawn(&["slot", "run", "--", "true"]);
    let ticket = f.queued(dead.pid());
    let file = f
        .root()
        .join("queue")
        .join(format!("{}-{}.json", ticket["queued_ns"], dead.pid()));
    let hidden = f.root().join("queue/.ticket-dead");
    File::create(&hidden).unwrap();
    let mut next = f.spawn(&["slot", "run", "--", "true"]);
    f.queued(next.pid());
    dead.0.kill().unwrap();
    dead.wait();
    until(|| (!file.exists() && !hidden.exists()).then_some(()));
    drop(slot);
    next.success();
}

#[test]
fn listing_shows_holders_and_ordered_waiters() {
    let f = Fixture::new(1);
    let mut holder = Running(
        f.rota(&["slot", "run", "--", "sh", "-c", "read release"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let record = f.holder(holder.pid());
    assert_eq!(record["slot"], 1);
    let mut first = f.spawn(&["slot", "run", "--", "true"]);
    let ticket = f.queued(first.pid());
    let mut second = f.spawn(&["slot", "run", "--", "true"]);
    f.queued(second.pid());
    let queue = File::open(f.root().join("queue/.lock")).unwrap();
    FileExt::lock_exclusive(&queue).unwrap();
    for path in [
        f.root().join("holders/1.json"),
        f.root()
            .join("queue")
            .join(format!("{}-{}.json", ticket["queued_ns"], first.pid())),
    ] {
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["future_field"] = true.into();
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    }
    drop(queue);
    let list = f.list();
    assert_eq!(list["holders"][0]["pid"], holder.pid());
    assert_eq!(list["waiters"][0]["pid"], first.pid());
    assert_eq!(list["waiters"][1]["pid"], second.pid());
    let output = f.output(&["slot", "ls"]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(&format!("slot 1 held pid={}", holder.pid())));
    assert!(text.contains(&format!("waiting pid={}", first.pid())));
    writeln!(holder.0.stdin.take().unwrap(), "release").unwrap();
    holder.success();
    first.success();
    second.success();
}

#[test]
fn malformed_live_ticket_warns_once_and_keeps_its_place() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let stderr = f.dir.path().join("stderr");
    let mut run = Running(
        f.rota(&["slot", "run", "--", "true"])
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let own = f.queued(run.pid());
    let timestamp = own["queued_ns"].as_u64().unwrap() - 1;
    let malformed = |stamp, pid| {
        let queue = File::open(f.root().join("queue/.lock")).unwrap();
        FileExt::lock_exclusive(&queue).unwrap();
        let path = f.root().join("queue").join(format!("{stamp}-{pid}.json"));
        let file = File::create(&path).unwrap();
        file.try_lock_exclusive().unwrap();
        (path, file)
    };
    let (bad, file) = malformed(timestamp, 999999);
    drop(slot);
    let list = f.list();
    assert_eq!(list["waiters"][0]["pid"], 999999);
    assert_eq!(list["waiters"][0]["malformed"], true);
    assert_eq!(list["waiters"][0]["slots"], serde_json::json!([1]));
    assert_eq!(list["waiters"][1]["pid"], run.pid());
    let wait_for_warning = |path: &Path| {
        until(|| {
            fs::read_to_string(&stderr)
                .unwrap()
                .contains(path.to_str().unwrap())
                .then_some(())
        });
    };
    wait_for_warning(&bad);
    let (later, later_file) = malformed(timestamp + 2, 999998);
    wait_for_warning(&later);
    drop(later_file);
    drop(file);
    assert!(
        run.wait().success(),
        "{}",
        fs::read_to_string(&stderr).unwrap()
    );
    assert_eq!(
        fs::read_to_string(&stderr)
            .unwrap()
            .matches(bad.to_str().unwrap())
            .count(),
        1
    );
    assert!(!bad.exists());
}

#[test]
fn queue_creation_failure_refuses() {
    let f = Fixture::new(1);
    let root = f.root();
    fs::create_dir_all(&root).unwrap();
    let queue = root.join("queue");
    File::create(&queue).unwrap();
    let output = f.output(&["slot", "run", "--front", "--", "touch", "started"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains(queue.to_str().unwrap()));
    assert!(!f.dir.path().join("started").exists());
}

#[test]
fn nested_run_reuses_the_slot() {
    let f = Fixture::new(1);
    let mut run = f.spawn(&[
        "slot",
        "run",
        "--",
        "sh",
        "-c",
        "exec \"$1\" slot run -- sh -c 'test \"$ROTA_SLOT\" = 1'",
        "sh",
        env!("CARGO_BIN_EXE_rota"),
    ]);
    run.success();
    assert_eq!(f.grants(), [(run.pid(), 1)]);
    assert_eq!(f.events().len(), 3);
}

#[test]
fn nested_runs_reuse_a_pid_one_holder_across_namespaces() {
    let sandbox = [
        "--die-with-parent",
        "--dev-bind",
        "/",
        "/",
        "--unshare-pid",
        "--as-pid-1",
        "--proc",
        "/proc",
    ];
    let probe = Command::new("bwrap")
        .args(sandbox)
        .args(["--", "true"])
        .output();
    let unavailable = match probe {
        Ok(output) if output.status.success() => None,
        Ok(output) => Some(format!(
            "{}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Some(error.to_string()),
    };
    if let Some(reason) = unavailable {
        assert!(
            std::env::var("ROTA_TEST_ALLOW_NO_BWRAP").ok().as_deref() == Some("1"),
            "Bubblewrap PID namespaces unavailable: {reason}. Set ROTA_TEST_ALLOW_NO_BWRAP=1 to permit this test to skip"
        );
        let _ = writeln!(
            std::io::stderr().lock(),
            "SKIPPED nested_runs_reuse_a_pid_one_holder_across_namespaces (ROTA_TEST_ALLOW_NO_BWRAP=1): {reason}"
        );
        return;
    }

    let f = Fixture::new(1);
    let binary = env!("CARGO_BIN_EXE_rota");
    let script = r#"test "$PPID" = 1
        "$1" slot run -- sh -c 'test "$ROTA_SLOT" = 1'
        exec bwrap --die-with-parent --dev-bind / / --unshare-pid --as-pid-1 --proc /proc -- \
            "$1" slot run -- sh -c 'test "$ROTA_SLOT" = 1'"#;
    let mut run = Running(
        Command::new("bwrap")
            .current_dir(f.dir.path())
            .env("ROTA_CONFIG", f.dir.path().join("config.toml"))
            .env_remove("ROTA_SLOT")
            .env_remove("ROTA_SLOT_TOKEN")
            .args(sandbox)
            .args([
                "--", binary, "slot", "run", "--", "bash", "-ec", script, "bash", binary,
            ])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    run.success();
    assert_eq!(f.grants(), [(1, 1)]);
    assert_eq!(f.events().len(), 3);
}

#[test]
fn background_child_does_not_hold_slot() {
    let f = Fixture::new(1);
    let mut run = f.spawn(&[
        "slot",
        "run",
        "--",
        "sh",
        "-c",
        "sleep 120 </dev/null >/dev/null 2>&1 & echo $! > descendant",
    ]);
    run.success();
    let pid: i32 = fs::read_to_string(f.dir.path().join("descendant"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    struct Descendant(i32);
    impl Drop for Descendant {
        fn drop(&mut self) {
            let _ = rustix::process::kill_process(Pid::from_raw(self.0).unwrap(), Signal::TERM);
        }
    }
    let _descendant = Descendant(pid);
    assert!(Path::new(&format!("/proc/{pid}")).exists());
    let mut next = f.spawn(&["slot", "run", "--", "true"]);
    next.success();
    assert!(Path::new(&format!("/proc/{pid}")).exists());
    assert_eq!(f.grants(), [(run.pid(), 1), (next.pid(), 1)]);
}

#[test]
fn command_status_and_events() {
    let f = Fixture::configure(1, false, "[slots.env]\nCARGO_BUILD_JOBS = '6'");
    let literal = "command argument with spaces ; $(false)";
    let script = r#"printf '%s:%s:%s' "$ROTA_SLOT" "$CARGO_BUILD_JOBS" "$1"
        stat=$(cat "/proc/$$/stat")
        set -- ${stat##*) }
        printf '%s\n' "${17}"
        exit 42"#;
    let output = f.output(&["slot", "run", "--", "sh", "-c", script, "sh", literal]);
    assert_eq!(output.status.code(), Some(42));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout
            .strip_prefix(&format!("1:6:{literal}"))
            .unwrap()
            .trim(),
        "19"
    );
    let output = f.output(&["slot", "run", "--", "sh", "-c", "kill -TERM $$"]);
    assert_eq!(output.status.code(), Some(143));
    let events = f.events();
    let names: Vec<_> = events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["queue", "grant", "exit", "queue", "grant", "exit"]);
    for event in &events[..3] {
        assert_eq!(event["pid"], events[0]["pid"]);
        assert_eq!(event["class"], "run");
        assert_eq!(event["folder"], f.dir.path().to_str().unwrap());
        assert!(event["time"].is_u64());
        assert_eq!(event["command"].as_str().unwrap().chars().count(), 80);
    }
    assert_eq!(events[2]["exit_code"], 42);
    assert_eq!(events[5]["exit_code"], 143);
}

#[test]
fn queued_waiter_sees_extra_slot() {
    let f = Fixture::new(3);
    let _slots: Vec<_> = (1..=3).map(|slot| f.hold(slot)).collect();
    let mut run = f.spawn(&["slot", "run", "--class", "nix", "--", "true"]);
    assert_eq!(f.queued(run.pid())["slots"], serde_json::json!([2]));
    f.policy("extra-slot-until", FUTURE);
    run.success();
    assert_eq!(f.grants(), [(run.pid(), 4)]);
}

#[test]
fn one_slot_and_stalled_override_extra() {
    let f = Fixture::new(2);
    let slot1 = f.hold(1);
    f.policy("extra-slot-until", FUTURE);
    f.policy("one-slot-until", FUTURE);
    let mut run = f.spawn(&["slot", "run", "--class", "nix", "--front", "--", "true"]);
    assert_eq!(f.queued(run.pid())["slots"], serde_json::json!([1]));
    f.policy("slot2-stalled", "");
    f.policy("one-slot-until", "invalid");
    assert_eq!(f.queued(run.pid())["slots"], serde_json::json!([1]));
    fs::remove_file(f.dir.path().join("legacy/slot2-stalled")).unwrap();
    run.success();
    assert_eq!(f.grants(), [(run.pid(), 2)]);
    drop(slot1);
}

#[test]
fn quiet_exemption_and_any_slot() {
    let f = Fixture::new(2);
    let slot1 = f.hold(1);
    let slot2 = f.hold(2);
    f.policy("quiet-until", FUTURE);
    let mut normal = f.spawn(&["slot", "run", "--", "true"]);
    assert_eq!(f.queued(normal.pid())["slots"], serde_json::json!([]));
    let mut front = f.spawn(&["slot", "run", "--class", "nix", "--front", "--", "true"]);
    assert_eq!(f.queued(front.pid())["slots"], serde_json::json!([]));
    let mut exempt = f.spawn(&[
        "slot",
        "run",
        "--class",
        "nix",
        "--quiet-exempt",
        "--",
        "true",
    ]);
    assert_eq!(f.queued(exempt.pid())["slots"], serde_json::json!([2]));
    drop(slot2);
    exempt.success();
    assert_eq!(f.queued(normal.pid())["slots"], serde_json::json!([]));
    assert_eq!(f.queued(front.pid())["slots"], serde_json::json!([]));
    let _slot2 = f.hold(2);
    drop(slot1);
    f.policy("quiet-until", "0");
    front.success();
    normal.success();
    assert_eq!(
        f.grants(),
        [(exempt.pid(), 2), (front.pid(), 1), (normal.pid(), 1)]
    );
}

#[test]
fn yield_keeps_slot_one_for_general_run() {
    let f = Fixture::new(2);
    let cwd = f.yielding_cwd();
    let slot1 = f.hold(1);
    let slot2 = f.hold(2);
    let mut run = f.spawn_in(&cwd, &["slot", "run", "--", "true"]);
    f.queued(run.pid());
    let mut nix = f.spawn(&["slot", "run", "--class", "nix", "--", "true"]);
    f.queued(nix.pid());
    drop(slot1);
    run.success();
    f.queued(nix.pid());
    drop(slot2);
    nix.success();
    assert_eq!(f.grants(), [(run.pid(), 1), (nix.pid(), 2)]);
}

#[test]
fn yielding_run_yields_when_policy_forces_slot_one() {
    let f = Fixture::new(2);
    let cwd = f.yielding_cwd();
    let slot1 = f.hold(1);
    f.policy("one-slot-until", FUTURE);
    let mut run = f.spawn_in(&cwd, &["slot", "run", "--", "true"]);
    f.queued(run.pid());
    let mut nix = f.spawn(&["slot", "run", "--class", "nix", "--", "true"]);
    assert_eq!(f.queued(nix.pid())["slots"], serde_json::json!([1]));
    assert_eq!(f.list()["waiters"][0]["pid"], nix.pid());
    drop(slot1);
    nix.success();
    run.success();
    assert_eq!(f.grants(), [(nix.pid(), 1), (run.pid(), 1)]);
}

#[test]
fn failed_holder_write_releases_slot_and_ticket() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let holders = f.root().join("holders");
    fs::create_dir_all(f.root()).unwrap();
    File::create(&holders).unwrap();
    let mut run = f.spawn(&["slot", "run", "--", "touch", "started"]);
    f.queued(run.pid());
    drop(slot);
    assert_eq!(run.wait().code(), Some(1));
    assert!(run.stderr().contains("grant slot 1"));
    assert!(!f.dir.path().join("started").exists());
    let list = f.list();
    assert_eq!(list["holders"], serde_json::json!([]));
    assert_eq!(list["waiters"], serde_json::json!([]));
    fs::remove_file(holders).unwrap();
    let mut next = f.spawn(&["slot", "run", "--", "true"]);
    next.success();
    assert_eq!(f.grants(), [(next.pid(), 1)]);
}

#[test]
fn legacy_tickets_and_locks_are_shared() {
    let f = Fixture::configure(1, true, "");
    let lock = f.hold(1);
    let old = Running(Command::new("sleep").arg("120").spawn().unwrap());
    let queue = f.dir.path().join("legacy/queue");
    let live = queue.join(format!("{}.{}.run", ns(), old.pid()));
    let reused = queue.join(format!("1.{}.run", old.pid()));
    let dead = queue.join("1.4294967295.run");
    for ticket in [&live, &reused, &dead] {
        File::create(ticket).unwrap();
    }
    let mut native = f.spawn(&["slot", "run", "--", "true"]);
    f.queued(native.pid());
    let list = f.list();
    assert_eq!(list["holders"], serde_json::json!([{"slot": 1}]));
    assert_eq!(list["waiters"].as_array().unwrap().len(), 2);
    assert_eq!(list["waiters"][0]["pid"], old.pid());
    assert_eq!(list["waiters"][0]["legacy"], true);
    drop(lock);
    let list = f.list();
    assert_eq!(list["waiters"][0]["pid"], old.pid());
    assert_eq!(list["waiters"][0]["slots"], serde_json::json!([1]));
    fs::remove_file(&live).unwrap();
    native.success();
    assert!(reused.exists());
    assert!(dead.exists());
    assert_eq!(f.grants(), [(native.pid(), 1)]);
}

#[test]
fn flat_policy_requires_legacy_configuration() {
    let f = Fixture::configure(2, false, "");
    fs::create_dir_all(f.root()).unwrap();
    fs::write(f.root().join("quiet-until"), FUTURE).unwrap();
    let mut run = f.spawn(&["slot", "run", "--", "true"]);
    run.success();
}

#[test]
fn own_corrupt_ticket_cannot_bypass_quiet_or_class() {
    let f = Fixture::new(2);
    let slot2 = f.hold(2);
    f.policy("quiet-until", FUTURE);
    let stderr = f.dir.path().join("stderr");
    let mut run = Running(
        f.rota(&["slot", "run", "--class", "nix", "--", "true"])
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let own = f.queued(run.pid());
    let own_path = f.ticket_path(&own);
    let wait_for_scan = |path: &Path| {
        until(|| {
            fs::read_to_string(&stderr)
                .unwrap()
                .contains(path.to_str().unwrap())
                .then_some(())
        });
        assert!(
            f.list()["waiters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|ticket| ticket["pid"] == run.pid())
        );
    };
    let queue = File::open(f.root().join("queue/.lock")).unwrap();
    FileExt::lock_exclusive(&queue).unwrap();
    fs::write(&own_path, b"{").unwrap();
    drop(queue);
    wait_for_scan(&own_path);

    let queue = File::open(f.root().join("queue/.lock")).unwrap();
    FileExt::lock_exclusive(&queue).unwrap();
    let later = f.root().join("queue").join(format!(
        "{}-999999.json",
        own["queued_ns"].as_u64().unwrap() + 1
    ));
    let later_file = File::create(&later).unwrap();
    later_file.try_lock_exclusive().unwrap();
    f.policy("quiet-until", "0");
    drop(queue);
    wait_for_scan(&later);
    drop(later_file);
    drop(slot2);
    assert!(
        run.wait().success(),
        "{}",
        fs::read_to_string(&stderr).unwrap()
    );
    assert_eq!(f.grants(), [(run.pid(), 2)]);
}

#[test]
fn live_holder_record_cannot_supply_a_nested_token() {
    let f = Fixture::new(1);
    let mut holder = Running(
        f.rota(&["slot", "run", "--", "sh", "-c", "read release"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert!(f.holder(holder.pid()).get("nesting_token_hash").is_none());
    let record: Value =
        serde_json::from_slice(&fs::read(f.root().join("holders/1.json")).unwrap()).unwrap();
    let digest = record["nesting_token_hash"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(record.get("nesting_token").is_none());
    let mut reader = Running(
        f.rota(&["slot", "run", "--", "true"])
            .env("ROTA_SLOT", "1")
            .env("ROTA_SLOT_TOKEN", digest)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    f.queued(reader.pid());
    writeln!(holder.0.stdin.take().unwrap(), "release").unwrap();
    holder.success();
    reader.success();
    assert_eq!(f.grants(), [(holder.pid(), 1), (reader.pid(), 1)]);
}

#[test]
fn log_failure_preserves_child_status_and_releases_slot() {
    let f = Fixture::new(1);
    fs::create_dir_all(f.root().join("log.jsonl")).unwrap();
    let mut run = f.spawn(&["slot", "run", "--", "sh", "-c", "touch started; exit 42"]);
    assert_eq!(run.wait().code(), Some(42));
    let stderr = run.stderr();
    for event in ["queue", "grant", "exit"] {
        assert!(stderr.contains(&format!("cannot log {event}")));
    }
    assert!(f.dir.path().join("started").exists());
    assert_eq!(
        f.list(),
        serde_json::json!({"holders": [], "waiters": [], "boat_lease_held": false})
    );
    let _slot = f.hold(1);
}

#[test]
fn scans_skip_io_errors_in_other_tickets() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let legacy_queue = f.dir.path().join("legacy/queue");
    fs::remove_dir(&legacy_queue).unwrap();
    File::create(&legacy_queue).unwrap();
    let mut run = f.spawn(&["slot", "run", "--", "true"]);
    let own = f.queued(run.pid());
    let _file = if rustix::process::geteuid().as_raw() != 0 {
        let unreadable = f.root().join("queue").join(format!(
            "{}-999999.json",
            own["queued_ns"].as_u64().unwrap() - 1
        ));
        let queue = File::open(f.root().join("queue/.lock")).unwrap();
        FileExt::lock_exclusive(&queue).unwrap();
        let file = File::create(&unreadable).unwrap();
        file.try_lock_exclusive().unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        Some(file)
    } else {
        None
    };
    assert_eq!(f.list()["waiters"].as_array().unwrap().len(), 1);
    drop(slot);
    run.success();
}

#[test]
fn legacy_dir_requires_shared_lock_paths() {
    let f = Fixture::configure(1, false, "legacy_dir = '/path/to/legacy'");
    let output = f.output(&["slot", "ls"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("slots.lock_files"));
}

#[test]
fn ls_observes_existing_and_free_legacy_locks_without_opening_them() {
    let f = Fixture::new(1);
    let held = f.hold(1);
    assert_eq!(f.list()["holders"], serde_json::json!([{"slot": 1}]));
    assert!(!f.root().join("queue").exists());
    drop(held);

    let mut run = f.spawn(&["slot", "run", "--", "true"]);
    run.success();
    let path = f.dir.path().join("legacy/old-1.lock");
    let mut events = File::from(
        rustix::fs::inotify::init(
            rustix::fs::inotify::CreateFlags::CLOEXEC | rustix::fs::inotify::CreateFlags::NONBLOCK,
        )
        .unwrap(),
    );
    rustix::fs::inotify::add_watch(&events, &path, rustix::fs::inotify::WatchFlags::OPEN).unwrap();
    assert_eq!(f.list()["holders"], serde_json::json!([]));
    assert_eq!(
        events.read(&mut [0; 256]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn ls_is_read_only_while_a_waiter_is_live() {
    let f = Fixture::new(1);
    let slot = f.hold(1);
    let mut run = f.spawn(&["slot", "run", "--", "true"]);
    f.queued(run.pid());
    let queue_path = f.root().join("queue");
    fs::set_permissions(&queue_path, fs::Permissions::from_mode(0o555)).unwrap();
    assert_eq!(f.list()["waiters"][0]["pid"], run.pid());
    fs::set_permissions(&queue_path, fs::Permissions::from_mode(0o700)).unwrap();
    drop(slot);
    run.success();
    let dead = queue_path.join("1-999999.json");
    File::create(&dead).unwrap();
    fs::set_permissions(&queue_path, fs::Permissions::from_mode(0o555)).unwrap();
    assert_eq!(f.list()["waiters"], serde_json::json!([]));
    assert!(dead.exists());
    fs::set_permissions(&queue_path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn killed_holder_leaves_a_stale_record_and_slot_is_reused() {
    let f = Fixture::new(1);
    let mut holder = Running(
        f.rota(&[
            "slot",
            "run",
            "--",
            "sh",
            "-c",
            "printf '%s' \"$ROTA_SLOT_TOKEN\" > inherited-token; read release",
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    );
    f.holder(holder.pid());
    let token = until(|| {
        fs::read_to_string(f.dir.path().join("inherited-token"))
            .ok()
            .filter(|token| token.len() == 32)
    });
    let record = f.root().join("holders/1.json");
    assert!(!fs::read_to_string(&record).unwrap().contains(&token));
    holder.0.kill().unwrap();
    holder.wait();
    drop(holder.0.stdin.take());
    let probe = File::open(&record).unwrap();
    FileExt::try_lock_shared(&probe).unwrap();
    let foreign = f.hold(1);
    assert_eq!(f.list()["holders"], serde_json::json!([{"slot": 1}]));
    let mut next = Running(
        f.rota(&["slot", "run", "--", "true"])
            .env("ROTA_SLOT", "1")
            .env("ROTA_SLOT_TOKEN", token)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    f.queued(next.pid());
    drop(probe);
    drop(foreign);
    next.success();
    assert_eq!(f.grants(), [(holder.pid(), 1), (next.pid(), 1)]);
    assert!(!record.exists());
}

#[test]
#[ignore = "needs host PID namespace lock observation; see DEVELOPMENT.md"]
fn active_legacy_and_native_waiters_run_exclusively_and_finish() {
    let f = Fixture::new(1);
    let blocker = f.hold(1);
    let script = r#"set -eu
        ticket=$2/$(date +%s%N).$$.run
        trap 'rm -f "$ticket"' EXIT
        : > "$ticket"
        touch registered
        read enter
        exec 9<"$1"
        flock -n 9
        rm "$ticket"
        mkdir inside
        echo legacy >> order
        touch legacy-started
        read finish
        rmdir inside"#;
    let mut old = Running(
        Command::new("bash")
            .current_dir(f.dir.path())
            .args(["-c", script, "bash"])
            .arg(f.dir.path().join("legacy/old-1.lock"))
            .arg(f.dir.path().join("legacy/queue"))
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    until(|| f.dir.path().join("registered").exists().then_some(()));
    let mut native = f.spawn(&[
        "slot",
        "run",
        "--",
        "sh",
        "-ec",
        "mkdir inside; echo native >> order; rmdir inside",
    ]);
    f.queued(native.pid());
    drop(blocker);
    let list = f.list();
    assert_eq!(list["waiters"][0]["pid"], old.pid());
    assert_eq!(list["waiters"][0]["slots"], serde_json::json!([1]));
    writeln!(old.0.stdin.as_mut().unwrap(), "enter").unwrap();
    until(|| f.dir.path().join("legacy-started").exists().then_some(()));
    let list = f.list();
    assert_eq!(list["holders"], serde_json::json!([{"slot": 1}]));
    assert_eq!(list["waiters"][0]["pid"], native.pid());
    writeln!(old.0.stdin.take().unwrap(), "finish").unwrap();
    old.success();
    native.success();
    assert_eq!(
        fs::read_to_string(f.dir.path().join("order")).unwrap(),
        "legacy\nnative\n"
    );
    assert_eq!(f.grants(), [(native.pid(), 1)]);
}

#[test]
fn built_in_nix_inference_and_read_only_commands() {
    let f = Fixture::configure(2, false, "");
    let nix = f.dir.path().join("nix");
    fs::write(&nix, "#!/bin/sh\nprintf '%s' \"${ROTA_SLOT:-no-slot}\"\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let nix = nix.to_str().unwrap();
    let slot2 = f.hold(2);
    let mut heavy = f.spawn(&["slot", "run", "--", nix, "build"]);
    let ticket = f.queued(heavy.pid());
    assert_eq!(ticket["class"], "nix");
    assert_eq!(ticket["slots"], serde_json::json!([2]));
    let script = format!("{nix} --option example 'two words' build --json --dry-run .#package");
    let cd_script = format!("cd . && exec {nix} -v path-info .#package");
    for args in [
        &[nix, "build", ".#package", "-L", "--dry-run"][..],
        &[nix, "build", "--json", "--dry-run", ".#package"],
        &[nix, "-v", "eval"],
        &[nix, "--option", "example", "value", "build", "--dry-run"],
        &[nix, "path-info"],
        &["sh", "-c", &script],
        &["bash", "-c", &cd_script],
        &[
            "timeout",
            "20m",
            nix,
            "build",
            "--max-jobs",
            "2",
            "--cores",
            "6",
            "--no-link",
            "--dry-run",
            ".#package",
        ],
    ] {
        let mut command = vec!["slot", "run", "--"];
        command.extend_from_slice(args);
        let mut direct = Running(
            f.rota(&command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        direct.success();
        let mut output = String::new();
        direct
            .0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "no-slot");
    }
    assert_eq!(f.events().len(), 1);
    let mut forced = f.spawn(&[
        "slot",
        "run",
        "--class",
        "nix",
        "--",
        nix,
        "build",
        "--dry-run",
    ]);
    assert_eq!(f.queued(forced.pid())["slots"], serde_json::json!([2]));
    drop(slot2);
    heavy.success();
    forced.success();
    assert_eq!(f.grants(), [(heavy.pid(), 2), (forced.pid(), 2)]);
}

#[allow(unsafe_code)]
mod platform {
    use super::{Command, File};
    use std::os::unix::process::CommandExt;

    pub(super) fn session_on_exec(command: &mut Command, tty: File) {
        // SAFETY: pre_exec has no safe registration equivalent; the closure owns
        // its tty and calls only async-signal-safe setsid/ioctl wrappers after fork.
        unsafe {
            command.pre_exec(move || {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(&tty)?;
                Ok(())
            });
        }
    }
}

#[test]
fn leftover_boat_snapshot_key_warns_and_changes_nothing() {
    let f = Fixture::boat_fixture();
    let config = f.dir.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replace(
            "large_max_memory_gib = 8",
            "large_max_memory_gib = 8\nsnapshot = \"eon-warm\"",
        ),
    )
    .unwrap();
    let out = f
        .boat_command(&["slot", "run", "--", "nix", "build", ".#default"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            ": warning: slots.boat.snapshot is removed and ignored; remote runs start fresh \
             from Eon's pinned qualified image"
        ),
        "{stderr}"
    );
    assert!(!f.dir.path().join("local-ran").exists());
    let new = fs::read_to_string(f.dir.path().join("provider/new-args")).unwrap();
    assert!(
        new.contains(&format!("--from {PINNED} --no-snapshots")),
        "{new}"
    );
}

// The slot route seam (eon-eon-agent-lr8sx.40.4): a fake route in the test host.

/// The test host's fake source (`test-host/main.rs`).
const FAKE_SOURCE: &str = r#"{"a":[1,{"b":"é\n\"q\""}],"n":18446744073709551615,"z":{"y":true}}"#;

/// Adds `slots.local_runner = ['sh', <script>]` running `body` before the handoff.
fn local_runner(f: &Fixture, body: &str) {
    let runner = f.dir.path().join("runner.sh");
    fs::write(&runner, format!("set -eu\n{body}\nexec \"$@\"\n")).unwrap();
    let config = f.dir.path().join("config.toml");
    let text = fs::read_to_string(&config).unwrap().replace(
        "[slots]\n",
        &format!("[slots]\nlocal_runner = ['sh', '{}']\n", runner.display()),
    );
    fs::write(config, text).unwrap();
}

fn fake_route(f: &Fixture, mode: &str, args: &[&str]) -> Output {
    f.rota(args)
        .env("ROTA_TEST_ROUTE", "fake")
        .env("ROTA_TEST_ROUTE_MODE", mode)
        .env("ROTA_TEST_ROUTE_LOG", f.dir.path().join("route.log"))
        .output()
        .unwrap()
}

fn route_calls(f: &Fixture) -> Vec<String> {
    fs::read_to_string(f.dir.path().join("route.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn fake_route_receives_its_source_byte_exact_on_both_local_paths() {
    let source = |call: &str| format!("{call} {FAKE_SOURCE}");
    for delegated in [false, true] {
        let f = Fixture::new(1);
        if delegated {
            local_runner(&f, "");
        }
        let out = fake_route(&f, "", &["slot", "run", "--", "touch", "ran"]);
        assert!(out.status.success(), "{out:?}");
        assert!(f.dir.path().join("ran").exists());
        let expected = if delegated {
            // Fallback check, the runner through the route, then the check in the
            // `_slot-local-exec` process after the source crossed the handoff file.
            [
                source("verify host"),
                source("run_local host"),
                source("verify exec"),
            ]
        } else {
            [
                source("verify host"),
                source("verify host"),
                source("run_local host"),
            ]
        };
        assert_eq!(route_calls(&f), expected, "delegated={delegated}");
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("rota slot: where=local reason=fake-returned")
        );
    }
}

#[test]
fn a_null_source_is_refused_before_anything_runs() {
    for delegated in [false, true] {
        let f = Fixture::new(1);
        if delegated {
            local_runner(&f, "touch runner-called");
        }
        let out = fake_route(&f, "null", &["slot", "run", "--", "touch", "ran"]);
        assert_eq!(out.status.code(), Some(125), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("route-source-null"));
        assert!(!f.dir.path().join("ran").exists());
        assert!(!f.dir.path().join("runner-called").exists());
        assert_eq!(route_calls(&f), Vec::<String>::new());
    }
}

#[test]
fn a_null_source_is_refused_whatever_the_route_returns() {
    let f = Fixture::new(1);
    let out = fake_route(&f, "null-exit", &["slot", "run", "--", "touch", "ran"]);
    assert_eq!(out.status.code(), Some(125), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("route-source-null"));
    let exits: Vec<_> = f
        .events()
        .into_iter()
        .filter(|e| e["event"] == "exit")
        .collect();
    assert_eq!(exits.len(), 1);
    assert_eq!(exits[0]["exit_code"], 125);
}

#[test]
fn standalone_rota_refuses_a_delegated_source() {
    // The runner hands the host's `_slot-local-exec` payload to standalone `rota`, which
    // has no route to check the source with.
    let f = Fixture::new(1);
    local_runner(
        &f,
        &format!("shift\nexec '{}' \"$@\"", env!("CARGO_BIN_EXE_rota")),
    );
    let out = fake_route(&f, "", &["slot", "run", "--", "touch", "ran"]);
    assert_eq!(out.status.code(), Some(125), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("local source needs the host's slot route"),
        "{out:?}"
    );
    assert!(!f.dir.path().join("ran").exists());
}

#[test]
fn a_route_that_returns_without_granting_does_not_hold_the_queue() {
    let f = Fixture::new(1);
    let out = fake_route(&f, "ungranted", &["slot", "run", "--", "touch", "ran"]);
    assert!(out.status.success(), "{out:?}");
    assert!(f.dir.path().join("ran").exists());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("rota slot: where=local reason=fake-returned")
    );
}

#[test]
fn start_errors_exit_125_only_for_a_routed_fallback() {
    let missing = "/nonexistent/rota-route-test-command";
    {
        // Each fixture holds the suite's case lock: end this one before the next.
        let f = Fixture::new(1);
        let standalone = f
            .program(env!("CARGO_BIN_EXE_rota"), &["slot", "run", "--", missing])
            .output()
            .unwrap();
        assert_eq!(
            standalone.status.code(),
            Some(1),
            "no route: {standalone:?}"
        );
    }
    for (mode, code) in [("declined", 1), ("kept", 125), ("", 125)] {
        let f = Fixture::new(1);
        let out = fake_route(&f, mode, &["slot", "run", "--", missing]);
        assert_eq!(out.status.code(), Some(code), "{mode}: {out:?}");
    }
}

#[test]
fn a_routed_run_refuses_an_open_state_directory() {
    let f = Fixture::new(1);
    local_runner(&f, "touch runner-called");
    let state = f.dir.path().join("state");
    fs::create_dir_all(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    let out = f.output(&["slot", "run", "--", "touch", "ran"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsafe-slot-state-directory"));
    assert!(!f.dir.path().join("runner-called").exists());
    assert!(!f.dir.path().join("ran").exists());
}

#[test]
fn slot_delegate_reinvokes_the_host_without_a_prefix() {
    let f = Fixture::new(1);
    local_runner(&f, "printf '%s\\n' \"$@\" > argv");
    let out = f.output(&["slot", "run", "--", "sh", "-c", "exit 7"]);
    assert_eq!(out.status.code(), Some(7), "{out:?}");
    let argv = fs::read_to_string(f.dir.path().join("argv")).unwrap();
    let argv: Vec<_> = argv.lines().collect();
    assert_eq!(argv[0], env!("CARGO_BIN_EXE_rota-test-host"));
    assert_eq!(argv[1], "_slot-local-exec");
    assert_eq!(argv[3..], ["--", "sh", "-c", "exit 7"]);
}

#[test]
fn an_old_delegate_name_does_not_nest() {
    // The runner hands the inner command only the removed EON_SLOT_DELEGATE name. The
    // inner `slot run` must queue and delegate again rather than run as nested.
    let f = Fixture::new(1);
    local_runner(
        &f,
        "echo call >> runner-calls\nexport EON_SLOT_DELEGATE=\"$ROTA_SLOT_DELEGATE\"\nunset ROTA_SLOT_DELEGATE",
    );
    let host = env!("CARGO_BIN_EXE_rota-test-host");
    let out = f.output(&["slot", "run", "--", host, "slot", "run", "--", "true"]);
    assert!(out.status.success(), "{out:?}");
    let calls = fs::read_to_string(f.dir.path().join("runner-calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "the inner run delegated again");
    let queued = f.events().iter().filter(|e| e["event"] == "queue").count();
    assert_eq!(queued, 2, "the inner run queued");
}
