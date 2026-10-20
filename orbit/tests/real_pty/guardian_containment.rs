use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::{
    ffi::OsStr,
    fs,
    os::unix::process::ExitStatusExt,
    path::Path,
    time::{Duration, Instant},
};

/// The host's bound on the worker's answer to one fact or at startup.
const DECISION_DEADLINE: f64 = 2.0;

/// One field of a process, such as pgid, ppid or tpgid.
fn field(name: &str, pid: i32) -> String {
    let listed = std::process::Command::new("ps")
        .arg("-o")
        .arg(format!("{name}="))
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .expect("ps");
    String::from_utf8_lossy(&listed.stdout).trim().to_string()
}

/// Kills what a failed probe leaves, including a group leader's group.
struct Leftovers(Vec<i32>);

impl Drop for Leftovers {
    fn drop(&mut self) {
        for &pid in &self.0 {
            if alive(pid) {
                let target = if field("pgid", pid) == pid.to_string() {
                    -pid
                } else {
                    pid
                };
                // SAFETY: sending a signal has no memory effects.
                unsafe { libc::kill(target, libc::SIGKILL) };
            }
        }
    }
}

/// Waits up to two seconds for a condition to hold.
fn within_two_seconds(holds: impl Fn() -> bool) -> bool {
    (0..200).any(|_| {
        if holds() {
            return true;
        }
        sleep(0.01);
        false
    })
}

/// Waits up to two seconds for a file to hold something.
fn written(path: &Path) -> bool {
    within_two_seconds(|| fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0))
}

/// Attaches an interactive client and a metadata observer, each past the
/// messages that open its role.
fn peers(path: &Path) -> (Peer, Peer) {
    let mut client = Peer::attach(path, 10.0);
    client.frame();
    let mut observer = Peer::connect(path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    observer.metadata();
    (client, observer)
}

/// The peer's connection ends, at EOF or a reset, with nothing more to read:
/// the old Orbit sends a peer nothing when it fails.
fn ended(peer: &mut Peer, who: &str) {
    let (data, _) = peer.read_until_closed(true);
    assert!(
        data.is_empty(),
        "{who} read {} bytes before its connection ended: {:?}",
        data.len(),
        split(&data).0.iter().map(brief).collect::<Vec<_>>()
    );
}

/// A session whose child records its PID; the host is its parent.
struct Probe {
    directory: TempDir,
    server: Server,
    child: i32,
}

impl Probe {
    fn start(variable: &str, name: &str) -> Self {
        let directory = TempDir::new();
        let shell = script(&directory, variable, name);
        Self::launch(directory, &shell)
    }

    fn launch(directory: TempDir, shell: &Path) -> Self {
        let child_file = directory.join("child.pid");
        let job_file = directory.join("job.pid");
        let server = Server::launch(
            &directory,
            &[shell.as_os_str()],
            &[
                ("ORBIT_CHILD_PID_FILE", &child_file),
                ("ORBIT_JOB_PID_FILE", &job_file),
            ],
            &[],
            &[],
        );
        assert!(written(&child_file), "the child never wrote its PID");
        let child = read_pid(&child_file);
        assert_eq!(field("ppid", child), server.pid().to_string());
        Self {
            directory,
            server,
            child,
        }
    }

    /// The Bend worker: the host's child other than the PTY child.
    fn worker(&self) -> i32 {
        children(self.server.pid())
            .into_iter()
            .rfind(|&pid| pid != self.child)
            .expect("the Bend worker")
    }

    /// Stops the worker with SIGSTOP, then gives the host a fact to ask it
    /// about: a peer's Hello. Returns the peer and when the Hello was sent.
    fn stall(&self, worker: i32) -> (Peer, Instant) {
        signal(worker, libc::SIGSTOP);
        let mut peer = Peer::connect(&self.server.path, 10.0);
        peer.send(&ClientMessage::Hello);
        (peer, Instant::now())
    }

    /// The host exits with this code, the socket pathname is gone, the child
    /// is gone, and Bend never reported a stop.
    fn finish(mut self, code: i32) -> Finished {
        let finished = self.server.communicate(8.0);
        assert_eq!(finished.code(), Some(code), "{:?}", finished.status);
        assert!(!self.server.path.exists(), "the socket pathname remains");
        assert!(
            !alive(self.child),
            "PTY child survived guardian cleanup: {}",
            self.child
        );
        assert!(
            !String::from_utf8_lossy(&finished.stdout).contains("STOPPED"),
            "Bend reported STOPPED during forced exit"
        );
        finished
    }
}

/// Asserts that the host failed naming a stalled worker.
fn assert_stalled(finished: &Finished) {
    let message = String::from_utf8_lossy(&finished.stderr);
    assert!(
        finished.code() == Some(1) && message == "orbit: worker stalled: no answer within 2 s\n",
        "status {:?}, stderr {message:?}",
        finished.status
    );
}

/// Runs a copy of the host with a worker that never answers, a script that
/// records its PID and then runs `then`. Returns how the host finished and how
/// long it ran, once the session never opened and the worker is gone.
fn silent_worker(then: &str) -> (Finished, Duration) {
    let directory = TempDir::new();
    let pid_file = directory.join("worker.pid");
    executable(
        &directory,
        "yazelix-orbit-worker",
        &format!(
            "#!/bin/sh\necho $$ > '{}'\n{then}\nexec sleep 60\n",
            pid_file.display()
        ),
    );
    let begun = Instant::now();
    let finished = run_unopened(&directory);
    let ran = begun.elapsed();
    let worker = read_pid(&pid_file);
    assert!(!alive(worker), "the silent worker survived: {worker}");
    (finished, ran)
}

pub fn run() {
    // A fatal Bend exit: the child ignores SIGHUP, so cleanup must escalate.
    let directory = TempDir::new();
    let path = directory.join("orbit.sock");
    let bash = bash();
    let line = server_command(
        &path,
        &[
            bash.as_os_str(),
            OsStr::new("-c"),
            OsStr::new("trap '' HUP; while :; do sleep 1; done"),
        ],
        &[],
    );
    let fault = run_command(&line, &[("ORBIT_GUARDIAN_FAULT", Path::new("1"))], 8.0);
    let output = String::from_utf8_lossy(&fault.stdout);
    let fault_pid: i32 = output
        .lines()
        .find_map(|line| line.strip_prefix("FAULT pid="))
        .and_then(|pid| pid.parse().ok())
        .expect("the fault fixture names the child");
    let _leftovers = Leftovers(vec![fault_pid]);
    assert_eq!(fault.code(), Some(1), "{:?}", fault.status);
    assert!(
        String::from_utf8_lossy(&fault.stderr)
            .lines()
            .any(|line| line == "guardian fault fixture"),
        "{:?}",
        String::from_utf8_lossy(&fault.stderr)
    );
    assert!(!path.exists(), "the socket pathname remains");
    assert!(
        !alive(fault_pid),
        "PTY child survived Bend fatal exit: {fault_pid}"
    );
    assert!(
        !output.contains("STOPPED"),
        "Bend reported STOPPED after fatal exit"
    );

    // A killed worker, with an interactive client and a metadata observer
    // attached: the host contains the child, each peer's connection ends
    // with nothing more, and the host exits with the worker's status.
    let probe = Probe::start("IGNORE_HUP_SHELL", "ignore-hup-shell");
    let _leftovers = Leftovers(vec![probe.child]);
    let (mut client, mut observer) = peers(&probe.server.path);
    signal(probe.worker(), libc::SIGKILL);
    ended(&mut client, "the client");
    ended(&mut observer, "the observer");
    probe.finish(137);

    // A killed host, with both peers attached, cleans up nothing itself, as
    // the old Orbit cannot: the kernel ends each peer's connection and
    // hangs up the PTY, so a child that exits on SIGHUP is gone, the worker
    // dies with the host, and the socket pathname stays until the next start
    // reclaims it.
    let directory = TempDir::new();
    let shell = executable(
        &directory,
        "hangup-shell",
        "#!/bin/sh\necho $$ > \"$ORBIT_CHILD_PID_FILE\"\nwhile :; do sleep 1; done\n",
    );
    let mut probe = Probe::launch(directory, &shell);
    let worker = probe.worker();
    let _leftovers = Leftovers(vec![probe.child, worker]);
    let (mut client, mut observer) = peers(&probe.server.path);
    signal(probe.server.pid(), libc::SIGKILL);
    ended(&mut client, "the client");
    ended(&mut observer, "the observer");
    assert!(
        within_two_seconds(|| !alive(probe.child)),
        "the child survived the host's SIGKILL: {}",
        probe.child
    );
    assert!(
        within_two_seconds(|| !alive(worker)),
        "the worker survived the host's SIGKILL: {worker}"
    );
    let killed = probe.server.communicate(8.0);
    assert_eq!(killed.status.signal(), Some(libc::SIGKILL));
    assert!(probe.server.path.exists(), "the socket pathname is gone");
    let next_pid = probe.directory.join("next.pid");
    let next = Server::start(
        &probe.directory,
        &shell,
        &[("ORBIT_CHILD_PID_FILE", &next_pid)],
    );
    let _leftovers = Leftovers(vec![next.child]);
    Peer::attach(&next.path, 10.0).frame();
    drop(next);

    // A stalled worker, stopped before a fact, is treated as dead once the
    // decision deadline passes: the host contains the child, kills the
    // worker, and fails naming the stall.
    let probe = Probe::start("IGNORE_HUP_SHELL", "ignore-hup-shell");
    let worker = probe.worker();
    let _leftovers = Leftovers(vec![probe.child, worker]);
    let (_peer, _) = probe.stall(worker);
    assert_stalled(&probe.finish(1));
    assert!(!alive(worker), "the stalled worker survived: {worker}");

    // A termination signal ends the wait for a stalled worker: the orderly
    // stop reaps the child long before the deadline would end the wait.
    let probe = Probe::start("IGNORE_HUP_SHELL", "ignore-hup-shell");
    let worker = probe.worker();
    let _leftovers = Leftovers(vec![probe.child, worker]);
    let (_peer, asked) = probe.stall(worker);
    sleep(0.3);
    signal(probe.server.pid(), libc::SIGTERM);
    while alive(probe.child) && asked.elapsed() < seconds(DECISION_DEADLINE) {
        sleep(0.01);
    }
    assert!(
        !alive(probe.child),
        "the stop waited for the stalled worker's deadline"
    );
    probe.finish(0);
    assert!(!alive(worker), "the stalled worker survived: {worker}");

    // The same deadline bounds the startup exchange: the host gives up on a
    // worker that never names its schema before it binds the socket or starts
    // the child, and a termination signal during that wait stops it at once,
    // with nothing to stop yet.
    let (stalled, _) = silent_worker("");
    assert_stalled(&stalled);
    let (terminated, ran) = silent_worker("kill -TERM $PPID");
    terminated.assert_silent(0);
    assert!(
        ran < seconds(DECISION_DEADLINE),
        "the signal waited {ran:?} for the silent worker"
    );

    // A termination signal is an orderly stop: once the child is reaped, the
    // host exits 0, as the old Orbit does (ORB-C12).
    let probe = Probe::start("IGNORE_HUP_SHELL", "ignore-hup-shell");
    let _leftovers = Leftovers(vec![probe.child]);
    signal(probe.server.pid(), libc::SIGTERM);
    probe.finish(0);

    // The stop also reaches the terminal's foreground process group: a job
    // the shell runs in its own group, ignoring SIGHUP, is killed with it.
    let probe = Probe::start("FOREGROUND_SHELL", "foreground-job-shell");
    let job_file = probe.directory.join("job.pid");
    let job = if written(&job_file) {
        read_pid(&job_file)
    } else {
        panic!("the job never wrote its PID")
    };
    let _leftovers = Leftovers(vec![probe.child, job]);
    let job_group = field("pgid", job);
    assert_ne!(job_group, field("pgid", probe.child));
    assert_eq!(field("tpgid", probe.child), job_group);
    signal(probe.server.pid(), libc::SIGTERM);
    probe.finish(0);
    let deadline = Instant::now() + seconds(1.0);
    while alive(job) && Instant::now() < deadline {
        sleep(0.01);
    }
    assert!(!alive(job), "foreground job survived shutdown: {job}");
}
