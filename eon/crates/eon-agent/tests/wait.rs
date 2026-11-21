#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! Bounded waiting and the exit statuses (EON-C40 clause 7): `--wait` retries admission until it
//! admits or its deadline passes, a hold exits 75, and a refusal waiting cannot change exits 69 at
//! once. No test asserts elapsed time: they wait for decision records, stderr lines and the
//! reader's own call script, and every bound below is a watchdog. How many attempts fit in a wait
//! is proved on a fake clock in `wait::tests`.

use fs2::FileExt;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// How long any wait for a waiter may take before the test fails: a watchdog, not a speed bound.
const WATCHDOG: Duration = Duration::from_secs(120);

struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Waits up to [`WATCHDOG`] for `done`, failing the test with `what` after that.
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    while !done() {
        assert!(Instant::now() < deadline, "watchdog: waited for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

/// The configuration: one backend `w` on account `acct`, admitted while its week has at least
/// `min` percent left. `top` adds top-level keys and `backend` keys to the backend.
fn config_text(root: &str, min: u32, top: &str, backend: &str) -> String {
    format!(
        "state_file = '{root}/state/anchor'\nlog_file = '{root}/log/decisions.jsonl'\n\
         wait_retry_seconds = 1\n{top}\n\
         [[backends]]\nname = 'w'\naccount = 'acct'\nroles = ['implement']\nguard = ['true']\n\
         {backend}\n\
         [backends.commands]\n\
         medium = ['sh', '-c', 'echo launched >> \"$0\"', '{root}/launched', '{{prompt}}']\n\
         high = ['sh', '-c', 'echo launched >> \"$0\"', '{root}/launched', '{{prompt}}']\n\
         [backends.quota]\ncommand = ['sh', '{root}/reader.sh']\nparser = 'remaining_json'\n\
         refresh = ['sh', '-c', 'echo refresh >> \"$0\"', '{root}/refreshes']\n\
         [backends.limits.implement.min_remaining]\nweek = {min}\n\
         [backends.run]\nstdin = 'brief'\nefforts = ['high']\ndefault_effort = 'high'\n\
         [backends.run.commands]\nimplement = ['sh', '-c', 'echo ran >> \"$0\"', '{root}/ran']\n"
    )
}

fn root() -> Root {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    let root = Root { _dir: dir, path };
    fs::write(root.path.join("brief"), "brief text\n").unwrap();
    // The reader counts its calls, then sleeps for the seconds in `delay` if that exists. It
    // prints the week from `week` and the observation from `at`, or from `at2` once its call
    // count reaches the number in `switch`.
    let p = root.p("");
    fs::write(
        root.path.join("reader.sh"),
        format!(
            "n=$(cat '{p}calls' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{p}calls'\n\
             if [ -e '{p}delay' ]; then sleep \"$(cat '{p}delay')\"; fi\n\
             at=$(cat '{p}at')\n\
             if [ -e '{p}switch' ] && [ $n -ge $(cat '{p}switch') ]; then at=$(cat '{p}at2'); fi\n\
             printf '{{\"week\":%s,\"observed_at\":%s}}' \"$(cat '{p}week')\" \"$at\"\n"
        ),
    )
    .unwrap();
    // A stand-in systemd-run: the start-up probe passes and the run's command runs directly.
    let bin = root.path.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    let script = bin.join("systemd-run");
    fs::write(
        &script,
        "#!/bin/sh\ncase \"$*\" in *--unit=*) ;; *) exit 0 ;; esac\n\
         while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    root.put("week", "80");
    root.put("at", &(now_ms() - 2000).to_string());
    root.config(40, "", "");
    root
}

impl Root {
    fn p(&self, name: &str) -> String {
        self.path.join(name).to_str().unwrap().to_owned()
    }

    /// Replaces `name` atomically, so a reader never sees it half written.
    fn put(&self, name: &str, value: &str) {
        let temp = self.path.join(format!(".{name}.tmp"));
        fs::write(&temp, value).unwrap();
        fs::rename(&temp, self.path.join(name)).unwrap();
    }

    fn config(&self, min: u32, top: &str, backend: &str) {
        self.put("config.toml", &config_text(&self.p(""), min, top, backend));
    }

    fn command(&self, config: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rota"));
        command
            .env("ROTA_CONFIG", self.path.join(config))
            .env(
                "PATH",
                format!("{}:{}", self.p("fake-bin"), std::env::var("PATH").unwrap()),
            )
            .current_dir(&self.path)
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn rota(&self, args: &[&str]) -> Output {
        self.command("config.toml", args).output().unwrap()
    }

    /// Starts `rota` in the background with its stderr in the file `err-NAME`.
    fn spawn(&self, name: &str, config: &str, args: &[&str]) -> Waiter {
        let stderr = fs::File::create(self.path.join(format!("err-{name}"))).unwrap();
        let child = self
            .command(config, args)
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap();
        Waiter {
            child: Some(child),
            stderr: self.path.join(format!("err-{name}")),
        }
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path.join(name)).unwrap_or_default()
    }

    fn count(&self, name: &str) -> usize {
        self.read(name).lines().count()
    }

    fn calls(&self) -> u64 {
        self.read("calls").trim().parse().unwrap_or(0)
    }

    /// The complete decision records: one still being written has no newline yet.
    fn records(&self) -> Vec<serde_json::Value> {
        self.read("log/decisions.jsonl")
            .split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn spent(&self) -> Option<u64> {
        let text = fs::read_to_string(self.path.join("state/admissions.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["accounts"]["acct"]["observed_at"].as_u64()
    }

    /// Holds the admission lock until the returned file is dropped.
    fn hold_lock(&self) -> fs::File {
        fs::create_dir_all(self.path.join("state")).unwrap();
        let lock = fs::File::create(self.path.join("state/anchor.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        lock
    }
}

/// A background `rota`, killed by its exact PID if the test ends before it does.
struct Waiter {
    child: Option<Child>,
    stderr: PathBuf,
}

impl Waiter {
    fn stderr(&self) -> String {
        fs::read_to_string(&self.stderr).unwrap_or_default()
    }

    fn wait_for_stderr(&self, needle: &str) {
        wait_until(&format!("{needle:?} on stderr"), || {
            self.stderr().contains(needle)
        });
    }

    /// Its exit, within the watchdog; the stdout and stderr it printed.
    fn finish(mut self) -> (Option<i32>, String, String) {
        let mut child = self.child.take().unwrap();
        let deadline = Instant::now() + WATCHDOG;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("watchdog: waiter still running; stderr:\n{}", self.stderr());
            }
            thread::sleep(Duration::from_millis(20));
        };
        let mut stdout = String::new();
        if let Some(mut out) = child.stdout.take() {
            std::io::Read::read_to_string(&mut out, &mut stdout).unwrap();
        }
        (status.code(), stdout, self.stderr())
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn launch_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["launch"];
    args.extend_from_slice(extra);
    args.push("brief");
    args
}

fn run_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["run", "--brief", "brief", "--name", "t", "--timeout", "30"];
    args.extend_from_slice(extra);
    args
}

/// Joins a FIFO writer within the watchdog: a writer whose reader never comes fails the test
/// instead of blocking it.
fn join_feed(writer: thread::JoinHandle<()>) {
    wait_until("the waiter to read its configuration", || {
        writer.is_finished()
    });
    writer.join().unwrap();
}

/// The last line of `stderr`.
fn last_line(stderr: &str) -> &str {
    stderr.lines().last().unwrap_or("")
}

#[test]
fn records_skip_an_unfinished_tail() {
    let root = root();
    fs::create_dir_all(root.path.join("log")).unwrap();
    // A writer paused halfway through its second record.
    fs::write(
        root.path.join("log/decisions.jsonl"),
        "{\"decision\":\"pick\"}\n{\"decision\":\"REF",
    )
    .unwrap();
    let records = root.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["decision"], "pick");
}

#[test]
fn a_waiter_admits_once_the_reading_drops_under_the_limit() {
    for run in [false, true] {
        let root = root();
        root.put("week", "10");
        let args = |extra: &[&'static str]| {
            if run {
                run_args(extra)
            } else {
                launch_args(extra)
            }
        };
        let waiter = root.spawn("w", "config.toml", &args(&["--wait", "10m"]));
        waiter.wait_for_stderr("held: no backend available, refusing; retrying for up to 10m");
        assert!(root.records().iter().all(|r| r["decision"] == "REFUSAL"));
        root.put("week", "80");
        let (code, stdout, stderr) = waiter.finish();
        let records = root.records();
        assert!(records.len() >= 2, "{records:?}");
        assert_eq!(records.last().unwrap()["decision"], "pick");
        assert!(stdout.starts_with("pick: w\n"), "{stdout}");
        assert!(
            root.spent().is_some(),
            "the admission spends its observation"
        );
        if run {
            // The fake systemd-run starts no scope; the admission itself is what is proved.
            assert!(code != Some(75) && code != Some(69), "{code:?}: {stderr}");
        } else {
            assert_eq!(code, Some(0), "{stderr}");
            assert_eq!(root.count("launched"), 1);
        }
    }
}

#[test]
fn a_hold_exits_75_at_the_deadline() {
    // How many attempts fit, and when the held line prints, are proved on a fake clock: here
    // only the outcome is checked, with a first attempt that may or may not cross the deadline.
    // A slow one (its reader sleeps 3 s) crosses it, and its hold is final.
    for (run, slow) in [(false, false), (true, false), (false, true), (true, true)] {
        let root = root();
        root.put("week", "10");
        if slow {
            root.put("delay", "3");
        }
        let extra = ["--wait", "2s"];
        let args = if run {
            run_args(&extra)
        } else {
            launch_args(&extra)
        };
        let (code, _, stderr) = root.spawn("w", "config.toml", &args).finish();
        assert_eq!(code, Some(75), "{stderr}");
        assert_eq!(
            last_line(&stderr),
            "rota: no backend available, refusing; still held after waiting 2s"
        );
        if slow {
            assert_eq!(root.calls(), 1, "no attempt starts after the deadline");
            assert!(!stderr.contains("held:"), "{stderr}");
        }
        assert!(root.spent().is_none() && root.count("launched") == 0 && root.count("ran") == 0);
    }
}

#[test]
fn without_wait_a_hold_exits_75_at_once() {
    for run in [false, true] {
        let root = root();
        root.put("week", "10");
        let output = root.rota(&if run { run_args(&[]) } else { launch_args(&[]) });
        assert_eq!(output.status.code(), Some(75), "{output:?}");
        let stderr = text(&output.stderr);
        assert!(!stderr.contains("held:"), "{stderr}");
        assert_eq!(last_line(&stderr), "rota: no backend available, refusing");
        assert_eq!(root.records().len(), 1);
        assert_eq!(root.calls(), 1);
    }
}

#[test]
fn a_refusal_waiting_cannot_change_exits_69_at_once() {
    let root = root();
    let disabled = "enabled = false";
    let excluded = format!("excluded_roots = ['{}']", root.p(""));
    let cases: Vec<(&str, Vec<&str>, &str)> = vec![
        (
            "",
            launch_args(&["--project", "nope"]),
            "unknown project nope",
        ),
        (
            "",
            launch_args(&["--only", "absent"]),
            "--only absent is not allowed",
        ),
        (
            "",
            launch_args(&["--role", "review"]),
            "no backend serves role review",
        ),
        (
            "",
            launch_args(&["--add-dir", "."]),
            "w: the high command takes no --add-dir",
        ),
        (
            disabled,
            launch_args(&[]),
            "no backend can take this launch: w: disabled",
        ),
        (
            &excluded,
            launch_args(&[]),
            "w: folder belongs to an excluded account",
        ),
        ("", run_args(&["--project", "nope"]), "unknown project nope"),
        (
            "",
            run_args(&["--only", "absent"]),
            "--only absent is not a configured backend",
        ),
        (
            "",
            run_args(&["--role", "review"]),
            "no backend has a headless command",
        ),
        (
            "",
            run_args(&["--effort", "max"]),
            "no backend has a headless command",
        ),
        (
            "",
            run_args(&["--only", "w", "--effort", "max"]),
            "w: effort max is not in efforts",
        ),
        (
            disabled,
            run_args(&[]),
            "no backend can take this run: w: disabled",
        ),
        (
            &excluded,
            run_args(&[]),
            "w: folder belongs to an excluded account",
        ),
    ];
    for (backend, args, reason) in cases {
        root.config(40, "", backend);
        let mut args = args;
        args.splice(1..1, ["--wait", "1h"]);
        // Under the watchdog: a refusal that regressed to a hold would wait the full hour.
        let (code, _, stderr) = root.spawn("r", "config.toml", &args).finish();
        assert_eq!(code, Some(69), "{args:?}: {stderr}");
        assert!(stderr.contains(reason), "{args:?}: {stderr}");
        assert!(!stderr.contains("held:"), "{args:?}: {stderr}");
        assert!(
            root.records().is_empty(),
            "{args:?}: nothing was read or recorded"
        );
        assert_eq!(root.calls(), 0, "{args:?}");
    }
}

#[test]
fn a_sleeping_waiter_blocks_no_admission() {
    let root = root();
    root.put("week", "10");
    // W reads its configuration from a FIFO: each attempt starts by reading it, before it takes
    // the admission lock, so W cannot start another attempt until the test writes one.
    let fifo = root.path.join("fifo.toml");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let content = root.read("config.toml");
    let feed = |content: String| {
        let fifo = fifo.clone();
        thread::spawn(move || fs::write(fifo, content).unwrap())
    };
    let w = root.spawn("w", "fifo.toml", &launch_args(&["--wait", "1h"]));
    join_feed(feed(content.clone()));
    // The held line prints only after the attempt returned and dropped its lock.
    w.wait_for_stderr("held: no backend available, refusing");
    assert_eq!(root.records().len(), 1);
    let l = root.rota(&launch_args(&[]));
    assert_eq!(l.status.code(), Some(75), "{l:?}");
    let stderr = text(&l.stderr);
    assert!(
        stderr.contains("no backend available, refusing"),
        "{stderr}"
    );
    assert!(!stderr.contains("is selecting a backend"), "{stderr}");
    assert_eq!(root.records().len(), 2);
    // W still waits: its next attempt runs when the test lets it, and holds.
    let next = feed(content);
    wait_until("W's second attempt", || root.records().len() == 3);
    join_feed(next);
    assert_eq!(root.count("launched"), 0);
    drop(w);
}

#[test]
fn two_waiters_share_no_observation() {
    let root = root();
    root.put("week", "10");
    let first = (now_ms() - 3000).to_string();
    root.put("at", &first);
    let a = root.spawn("a", "config.toml", &launch_args(&["--wait", "10m"]));
    let b = root.spawn("b", "config.toml", &launch_args(&["--wait", "10m"]));
    a.wait_for_stderr("held:");
    b.wait_for_stderr("held:");
    // One fresh observation under the limit: exactly one waiter admits on it.
    root.put("week", "80");
    wait_until("one admission", || root.count("launched") == 1);
    wait_until("the other waiter holding on the spent observation", || {
        root.records().iter().any(|r| {
            r["decision"] == "REFUSAL" && r.to_string().contains("already admitted a launch")
        })
    });
    assert_eq!(root.count("launched"), 1);
    assert_eq!(root.spent(), Some(first.parse().unwrap()));
    // A newer observation admits the other.
    let second = (now_ms() - 1000).to_string();
    root.put("at", &second);
    let (code_a, _, err_a) = a.finish();
    let (code_b, _, err_b) = b.finish();
    assert_eq!((code_a, code_b), (Some(0), Some(0)), "{err_a}\n{err_b}");
    assert_eq!(root.count("launched"), 2);
    assert_eq!(root.spent(), Some(second.parse().unwrap()));
}

#[test]
fn a_held_lock_through_the_deadline_starts_nothing() {
    for run in [false, true] {
        let root = root();
        let _lock = root.hold_lock();
        let extra = ["--wait", "2s"];
        let args = if run {
            run_args(&extra)
        } else {
            launch_args(&extra)
        };
        let (code, _, stderr) = root.spawn("w", "config.toml", &args).finish();
        assert_eq!(code, Some(75), "{stderr}");
        assert_eq!(
            last_line(&stderr),
            "rota: another rota command is selecting a backend; still held after waiting 2s"
        );
        assert_eq!(root.calls(), 0, "no reading starts");
        assert!(root.records().is_empty());
        assert!(root.count("launched") == 0 && root.count("ran") == 0);
        assert!(root.spent().is_none());
    }
}

#[test]
fn a_waiter_rides_a_busy_admission_lock() {
    let root = root();
    let lock = root.hold_lock();
    let busy = root.rota(&launch_args(&[]));
    assert_eq!(busy.status.code(), Some(75), "{busy:?}");
    let w = root.spawn("w", "config.toml", &launch_args(&["--wait", "10m"]));
    w.wait_for_stderr("held: another rota command is selecting a backend");
    drop(lock);
    let (code, stdout, stderr) = w.finish();
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.starts_with("pick: w\n"));
    assert_eq!(root.count("launched"), 1);
}

#[test]
fn a_waiter_follows_a_tightened_limit() {
    let has = |root: &Root, needle: &str| {
        root.records()
            .iter()
            .any(|r| r.to_string().contains(needle))
    };
    for run in [false, true] {
        let root = root();
        root.put("week", "30");
        // Each attempt sleeps 1 s in its reader, after loading its configuration, so a write can
        // land between the two; the order below holds whenever it lands.
        root.put("delay", "1");
        let args = if run {
            run_args(&["--wait", "10m"])
        } else {
            launch_args(&["--wait", "10m"])
        };
        let w = root.spawn("w", "config.toml", &args);
        w.wait_for_stderr("held:");
        // The limit tightens while the reading stays held. One waiter's attempts run in sequence,
        // so once an attempt has recorded the new limit every later attempt loads it too.
        root.config(70, "", "");
        wait_until("an attempt under the new limit", || {
            has(&root, "week left 30% (min_remaining >= 70%)")
        });
        // A reading that passes only the old limit is refused.
        root.put("week", "50");
        wait_until("the new reading under the new limit", || {
            has(&root, "week left 50% (min_remaining >= 70%)")
        });
        let records = root.records();
        assert!(
            records
                .iter()
                .filter(|r| r.to_string().contains("week left 50%"))
                .all(|r| r["decision"] == "REFUSAL"),
            "{records:?}"
        );
        assert!(root.spent().is_none() && root.count("launched") == 0 && root.count("ran") == 0);
        root.put("week", "80");
        let (code, stdout, stderr) = w.finish();
        assert!(stdout.starts_with("pick: w\n"), "{stdout}{stderr}");
        assert!(root.spent().is_some());
        if !run {
            assert_eq!(code, Some(0), "{stderr}");
            assert_eq!(root.count("launched"), 1);
        }
    }
    // A configuration that turns invalid mid-wait ends the wait with 2.
    let root = root();
    root.put("week", "10");
    let w = root.spawn("w", "config.toml", &launch_args(&["--wait", "10m"]));
    w.wait_for_stderr("held:");
    root.put("config.toml", "not = [valid");
    let (code, _, stderr) = w.finish();
    assert_eq!(code, Some(2), "{stderr}");
}

#[test]
fn a_waiter_refreshes_once() {
    let root = root();
    let (first, second) = (now_ms() - 3000, now_ms() - 1000);
    root.put("at", &first.to_string());
    root.put("at2", &second.to_string());
    root.put("switch", "5");
    fs::create_dir_all(root.path.join("state")).unwrap();
    fs::write(
        root.path.join("state/admissions.json"),
        format!("{{\"version\":1,\"accounts\":{{\"acct\":{{\"observed_at\":{first}}}}}}}\n"),
    )
    .unwrap();
    // Attempt 1 reads, refreshes and reads again (calls 1-2); attempts 2 and 3 read once each
    // without refreshing (calls 3-4); attempt 4 reads the newer observation (call 5) and admits.
    let (code, _, stderr) = root
        .spawn("w", "config.toml", &launch_args(&["--wait", "10m"]))
        .finish();
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(root.calls(), 5);
    assert_eq!(root.count("refreshes"), 1);
    assert_eq!(root.spent(), Some(second));
    assert_eq!(root.count("launched"), 1);
}
