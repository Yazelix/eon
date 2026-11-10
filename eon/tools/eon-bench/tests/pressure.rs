//! The pressure worker as the bench runs it: a separate process that stops on
//! SIGTERM well before its deadline, reports its work and leaves no files.

use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use std::{
    io::{BufRead, BufReader},
    process::Stdio,
};

fn monotonic_ns() -> u64 {
    // SAFETY: timespec is plain data that clock_gettime fills.
    let mut time = unsafe { std::mem::zeroed::<libc::timespec>() };
    // SAFETY: time is a valid timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut time) };
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

/// The bench closes every window with SIGTERM and a 2 s grace; a worker that
/// ignored it, lost its report or kept its file would leak pressure.
#[test]
fn workers_stop_on_sigterm_and_report_their_work() {
    let out = std::env::temp_dir().join(format!("eon-bench-pressure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for (kind, amount) in [
        ("cpu", "1"),
        ("sequential", "2097152"),
        ("metadata", "4096"),
    ] {
        let report = out.join(format!("{kind}.json"));
        let deadline = monotonic_ns() + 60_000_000_000;
        let mut worker = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
            .args(["pressure-worker", kind, &deadline.to_string(), amount])
            .arg(&out)
            .arg(&report)
            .spawn()
            .unwrap();
        thread::sleep(Duration::from_millis(300));
        // SAFETY: kill has no memory arguments; the child is not yet reaped.
        unsafe { libc::kill(worker.id() as i32, libc::SIGTERM) };
        let stopping = Instant::now();
        let status = loop {
            if let Some(status) = worker.try_wait().unwrap() {
                break status;
            }
            if stopping.elapsed() > Duration::from_secs(2) {
                let _ = worker.kill();
                panic!("{kind} worker ignored SIGTERM");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{kind}: {status}");
        let text = std::fs::read_to_string(&report).unwrap();
        assert!(text.contains(&format!("\"kind\": \"{kind}\"")), "{text}");
        assert!(!text.contains("\"work\": 0,"), "{text}");
        std::fs::remove_file(&report).unwrap();
    }
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    std::fs::remove_dir(&out).unwrap();
}

/// The environment variable that makes `driver_death_parent` act as a driver.
#[cfg(target_os = "linux")]
const PARENT: &str = "EON_BENCH_PRESSURE_PARENT";

/// A stand-in driver for `workers_die_with_their_driver_and_leave_no_file`: it
/// starts one sequential writer the way the bench does, prints its PID and
/// waits to be killed. Without the variable it does nothing.
#[cfg(target_os = "linux")]
#[test]
fn driver_death_parent() {
    let Some(out) = std::env::var_os(PARENT) else {
        return;
    };
    let out = std::path::PathBuf::from(out);
    let mut worker = eon_bench::pressure::spawn(
        &[
            env!("CARGO_BIN_EXE_eon-bench").into(),
            "pressure-worker".into(),
        ],
        "sequential",
        monotonic_ns() + 60_000_000_000,
        64 << 20,
        &out,
        &out.join("report.json"),
    )
    .unwrap();
    println!("worker {}", worker.id());
    thread::sleep(Duration::from_secs(60));
    // Reached only when the test never killed this driver.
    worker.kill().unwrap();
    worker.wait().unwrap();
}

/// The start time field of /proc/PID/stat, or None once the PID is gone.
#[cfg(target_os = "linux")]
fn started(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    // A zombie has already released its files.
    (fields[0] != "Z").then(|| fields[19].to_owned())
}

/// When the driver dies, the kernel SIGKILLs its workers, skipping every
/// cleanup; the sequential writer's allocation must still not survive it.
#[cfg(target_os = "linux")]
#[test]
fn workers_die_with_their_driver_and_leave_no_file() {
    let out = std::env::temp_dir().join(format!("eon-bench-driver-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "driver_death_parent", "--nocapture"])
        .env(PARENT, &out)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pid: u32 = BufReader::new(parent.stdout.take().unwrap())
        .lines()
        .map_while(std::result::Result::ok)
        .find_map(|line| line.strip_prefix("worker ")?.parse().ok())
        .unwrap();
    let identity = started(pid).unwrap();
    // Wait until the writer holds an allocation: an open file in `out` with data.
    let writing = Instant::now();
    let held = loop {
        let held = std::fs::read_dir(format!("/proc/{pid}/fd"))
            .into_iter()
            .flatten()
            .flatten()
            .find(|fd| {
                std::fs::read_link(fd.path()).is_ok_and(|target| target.starts_with(&out))
                    && std::fs::metadata(fd.path()).is_ok_and(|file| file.len() >= 1 << 20)
            });
        if let Some(fd) = held {
            break fd.path();
        }
        assert!(
            writing.elapsed() < Duration::from_secs(10),
            "the writer never wrote"
        );
        thread::sleep(Duration::from_millis(20));
    };
    // The file it writes has no name, even while it writes.
    assert!(
        std::fs::read_link(&held)
            .unwrap()
            .to_string_lossy()
            .ends_with("(deleted)")
    );
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    parent.kill().unwrap();
    parent.wait().unwrap();
    let killed = Instant::now();
    while started(pid).as_ref() == Some(&identity) {
        assert!(
            killed.elapsed() < Duration::from_secs(5),
            "the worker outlived its driver"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    std::fs::remove_dir(&out).unwrap();
}
