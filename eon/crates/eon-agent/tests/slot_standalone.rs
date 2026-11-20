#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! Standalone `rota` has no slot route: the keys only a route acts on (`[slots.boat]`,
//! `slots.local_runner`) warn that they need `eon agent`, and every command runs on a local
//! slot. The suites with a route live in `test-host/`.

use fs2::FileExt;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, PoisonError};

const BOTH: &str = "rota: warning: [slots.boat] and slots.local_runner need eon agent; rota runs every slot command locally\n";
const BOAT: &str =
    "rota: warning: [slots.boat] needs eon agent; rota runs every slot command locally\n";
const RUNNER: &str =
    "rota: warning: slots.local_runner needs eon agent; rota runs every slot command locally\n";

/// One case at a time: each writes executables it then runs, and a sibling thread's fork
/// while one is open for writing makes its exec fail with ETXTBSY under libtest threads.
static CASE: Mutex<()> = Mutex::new(());

struct Root {
    dir: tempfile::TempDir,
    _case: MutexGuard<'static, ()>,
}

impl Root {
    fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// A scratch root whose `[slots]` table holds `slots`; `boat` on PATH and the runner record
/// any call, `nix` records the local run. It holds the case lock until dropped.
fn root(slots: &str) -> Root {
    let case = CASE.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.display();
    fs::create_dir_all(root.join("fake")).unwrap();
    for (name, body) in [
        ("boat", "touch \"$ROOT/boat-called\"\nexit 1\n"),
        ("nix", "touch \"$ROOT/local-ran\"\n"),
        ("runner", "touch \"$ROOT/runner-called\"\nexec \"$@\"\n"),
    ] {
        fs::write(root.join("fake").join(name), format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(
            root.join("fake").join(name),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fs::create_dir_all(root.join("state/slots")).unwrap();
    // Boat's switch: a route would send `nix build` to Boat.
    fs::write(root.join("state/slots/boat-on"), "enabled").unwrap();
    let slots = slots.replace("<ROOT>", &path.to_string());
    fs::write(
        root.join("config.toml"),
        format!(
            r#"
state_file = '{path}/state/last-pick'
log_file = '{path}/state/decisions.jsonl'

[slots]
count = 1
{slots}

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
    Root { dir, _case: case }
}

fn rota(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rota"))
        .current_dir(root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("fake").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("ROOT", root)
        .env("ROTA_CONFIG", root.join("config.toml"))
        .env("ROTA_SLOT_POLL_MS", "50")
        .env_remove("ROTA_SLOT")
        .env_remove("ROTA_SLOT_TOKEN")
        .env_remove("ROTA_SLOT_DELEGATE")
        .args(args)
        .output()
        .unwrap()
}

const BOAT_KEYS: &str = "[slots.boat]\nallow_shapes = ['nix-gate']";
const RUNNER_KEY: &str = "local_runner = ['<ROOT>/fake/runner']";

/// The warning lines of a run's stderr (the slot line follows them).
fn warnings(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|line| line.contains(": warning: "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn list(root: &Path) -> (Value, String) {
    let output = rota(root, &["slot", "ls", "--json"]);
    assert!(output.status.success(), "{output:?}");
    (
        serde_json::from_slice(&output.stdout).unwrap(),
        warnings(&output.stderr),
    )
}

#[test]
fn route_keys_warn_and_every_command_runs_locally() {
    let dir = root(&format!("{RUNNER_KEY}\n{BOAT_KEYS}"));
    let root = dir.path();
    let output = rota(root, &["slot", "run", "--", "nix", "build", ".#default"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(warnings(&output.stderr), BOTH);
    assert!(root.join("local-ran").exists());
    assert!(!root.join("boat-called").exists());
    assert!(!root.join("runner-called").exists());
    let log = fs::read_to_string(root.join("state/slots/log.jsonl")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["queue", "grant", "exit"]
    );
    for event in &events {
        assert_eq!(event["where"], "local");
        assert!(event["fallback_reason"].is_null());
    }
    let (listing, stderr) = list(root);
    assert_eq!(stderr, BOTH, "slot ls warns too");
    let keys: Vec<_> = listing.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["holders", "waiters"]);
}

#[test]
fn each_route_key_warns_alone() {
    for (slots, warning) in [(BOAT_KEYS, BOAT), (RUNNER_KEY, RUNNER)] {
        let dir = root(slots);
        let output = rota(
            dir.path(),
            &["slot", "run", "--", "nix", "build", ".#default"],
        );
        assert!(output.status.success(), "{slots}: {output:?}");
        assert_eq!(warnings(&output.stderr), warning);
        assert!(dir.path().join("local-ran").exists());
        assert!(!dir.path().join("runner-called").exists());
    }
}

#[test]
fn absent_or_inert_route_keys_stay_silent() {
    for slots in [
        "",
        "[slots.boat]\nallow_shapes = []\nlarge_max_memory_gib = 8",
    ] {
        let dir = root(slots);
        let output = rota(
            dir.path(),
            &["slot", "run", "--", "nix", "build", ".#default"],
        );
        assert!(output.status.success(), "{slots}: {output:?}");
        assert_eq!(warnings(&output.stderr), "", "{slots}");
        assert_eq!(list(dir.path()).1, "", "{slots}");
    }
}

#[test]
fn a_remote_ticket_from_a_shared_queue_lists_as_remote() {
    let dir = root("");
    let queue = dir.path().join("state/slots/queue");
    fs::create_dir_all(&queue).unwrap();
    let pid = std::process::id();
    let path = queue.join(format!("1-{pid}.json"));
    fs::write(
        &path,
        serde_json::json!({
            "queued_ns": 1, "pid": pid, "class": "run", "cwd": null,
            "argv": ["nix", "build"], "front": false, "quiet_exempt": false, "remote": true
        })
        .to_string(),
    )
    .unwrap();
    // A live ticket is one its owner holds locked.
    let owner = fs::File::open(&path).unwrap();
    owner.lock_exclusive().unwrap();
    let (listing, _) = list(dir.path());
    let waiters = listing["waiters"].as_array().unwrap();
    assert_eq!(waiters.len(), 1, "{listing}");
    assert_eq!(waiters[0]["where"], "remote");
}

#[test]
fn standalone_local_exec_refuses_a_source() {
    // Only a host's route can check a prepared source; standalone `rota` refuses it.
    let dir = root("");
    let handoff = dir.path().join("handoff");
    fs::create_dir(&handoff).unwrap();
    fs::set_permissions(&handoff, fs::Permissions::from_mode(0o700)).unwrap();
    let ran = dir.path().join("ran");
    let argv = ["touch".to_owned(), ran.display().to_string()];
    let payload = handoff.join("command.json");
    fs::write(
        &payload,
        serde_json::json!({
            "argv": argv, "cwd": dir.path(), "source": {"a": 1}, "nesting_token_hash": "x"
        })
        .to_string(),
    )
    .unwrap();
    fs::set_permissions(&payload, fs::Permissions::from_mode(0o600)).unwrap();
    let output = rota(
        dir.path(),
        &[
            "_slot-local-exec",
            payload.to_str().unwrap(),
            "--",
            &argv[0],
            &argv[1],
        ],
    );
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("local source needs the host's slot route"),
        "{output:?}"
    );
    assert!(!ran.exists());
}
