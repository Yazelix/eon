#![cfg(target_os = "linux")]

use fs2::FileExt;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
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

    fn rota(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rota"));
        command
            .current_dir(self.dir.path())
            .env("ROTA_CONFIG", self.dir.path().join("config.toml"))
            .env("ROTA_SLOT_POLL_MS", "50")
            .env_remove("ROTA_SLOT")
            .env_remove("ROTA_SLOT_TOKEN")
            .args(args);
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
            unsafe { libc::kill(self.0, libc::SIGTERM) };
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
    assert_eq!(f.list(), serde_json::json!({"holders": [], "waiters": []}));
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
    let _file = if unsafe { libc::geteuid() } != 0 {
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
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
    assert!(fd >= 0);
    let mut events = unsafe { File::from_raw_fd(fd) };
    assert!(unsafe { libc::inotify_add_watch(fd, path.as_ptr(), libc::IN_OPEN) } >= 0);
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
