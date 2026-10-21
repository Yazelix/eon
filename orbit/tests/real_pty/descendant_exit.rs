use crate::management_record::{suffixed, tombstone};
use crate::support::*;
use orbit_protocol::management::{ProcessOutcome, TerminationReason};
use std::{fs, path::Path, time::Instant};

const MANAGED: [&str; 4] = ["--management-v1", "session-1", "run-1", "component-1"];

/// Kills a descendant a failed check leaves behind.
struct Descendant(i32);

impl Drop for Descendant {
    fn drop(&mut self) {
        if alive(self.0) {
            // SAFETY: sending a signal has no memory effects.
            unsafe { libc::kill(self.0, libc::SIGKILL) };
        }
    }
}

/// Waits up to five seconds for the descendant to record its PID.
fn descendant(path: &Path) -> Descendant {
    let deadline = Instant::now() + seconds(5.0);
    while !fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0) {
        assert!(Instant::now() < deadline, "the descendant never wrote its PID");
        sleep(0.01);
    }
    Descendant(read_pid(path))
}

/// Runs a child script that starts a HUP-ignoring descendant holding the PTY
/// and exits 3 once the gate opens; the server must end with the child,
/// within `bound` seconds of the gate, rather than with the descendant.
fn exits_with_child(
    directory: &Path,
    text: &str,
    options: &[&str],
    bound: f64,
) -> (Server, Descendant) {
    let gate = Gate::new(directory, "exit-gate");
    let job_file = directory.join("job.pid");
    let shell = executable(directory, "child.sh", text);
    let mut server = Server::launch(
        directory,
        &[shell.as_os_str()],
        &[
            ("ORBIT_GATE", gate.path()),
            ("ORBIT_JOB_PID_FILE", &job_file),
        ],
        &[],
        options,
    );
    let job = descendant(&job_file);
    assert!(alive(job.0), "the descendant is not running");
    gate.release();
    let started = Instant::now();
    let finished = server.communicate(bound + 5.0);
    assert!(
        started.elapsed() < seconds(bound),
        "the server ended {:?} after the child's exit",
        started.elapsed()
    );
    server.check_passed(&finished, 3);
    (server, job)
}

pub fn run() {
    // A writing descendant: a HUP-ignoring job keeps
    // writing to the PTY after the child exits. The server still ends with
    // the child's status, and the job fails once the PTY master closes.
    let directory = TempDir::new();
    let (_server, job) = exits_with_child(
        &directory,
        "#!/bin/sh\n\
         sh -c 'echo $$ > \"$ORBIT_JOB_PID_FILE\"; trap \"\" HUP; \
         while printf orbit; do sleep 0.05; done' &\n\
         read -r _ < \"$ORBIT_GATE\"\n\
         exit 3\n",
        &[],
        3.0,
    );
    let deadline = Instant::now() + seconds(3.0);
    while alive(job.0) {
        assert!(
            Instant::now() < deadline,
            "the writing descendant outlived the server: {}",
            job.0
        );
        sleep(0.01);
    }

    // An idle HUP-ignoring job that holds the PTY open: a managed run ends
    // promptly with the child's code and writes the natural tombstone.
    let directory = TempDir::new();
    let (server, _job) = exits_with_child(
        &directory,
        "#!/bin/sh\n\
         trap '' HUP\n\
         sleep 30 &\n\
         echo $! > \"$ORBIT_JOB_PID_FILE\"\n\
         read -r _ < \"$ORBIT_GATE\"\n\
         exit 3\n",
        &MANAGED,
        1.0,
    );
    let tombstone = tombstone(&suffixed(&server.path, ".record"));
    assert_eq!(
        (tombstone.reason, tombstone.outcome),
        (TerminationReason::NaturalExit, ProcessOutcome::ExitCode(3)),
        "{tombstone:?}"
    );
    assert!(!suffixed(&server.path, ".management").exists());
}
