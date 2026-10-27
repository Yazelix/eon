use crate::management_lease::{Manager, Reply};
use crate::management_record::{live, suffixed, tombstone};
use crate::support::*;
use orbit_protocol::management::{
    ClientMessage, LiveIdentity, ProcessOutcome, ServerMessage, TerminationReason,
};
use std::path::PathBuf;

/// A managed run whose traced child the escalation cannot reap: the child's
/// zombie stays visible only to the check until the check waits on it.
struct Held {
    _directory: TempDir,
    server: Server,
    record: PathBuf,
    identity: LiveIdentity,
    lease: Manager,
}

/// Starts a run, traces its child and sends Stop through the lease; the
/// escalation's two seconds pass with no answer, and the run, its live record
/// and its lease remain.
fn held() -> Held {
    let directory = TempDir::new();
    let shell = executable(&directory, "child.sh", "#!/bin/sh\nwhile :; do sleep 1; done\n");
    let server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[],
        &[],
        &["--management-v3", "session-3", "run-3", "component-3"],
    );
    let record = suffixed(&server.path, ".record");
    wait_for(&record);
    let identity = live(&record);
    // SAFETY: PTRACE_SEIZE neither stops nor signals the child.
    let seized = unsafe { libc::ptrace(libc::PTRACE_SEIZE, server.child, 0, 0) };
    assert_eq!(seized, 0, "seize: {}", std::io::Error::last_os_error());
    let mut lease = Manager::new(&record);
    assert_eq!(lease.acquire(), Reply::Message(ServerMessage::Lease(identity.clone())));
    assert_eq!(lease.send(&ClientMessage::Stop), Reply::Timeout);
    sleep(2.5);
    assert_eq!(live(&record), identity);
    Held {
        _directory: directory,
        server,
        record,
        identity,
        lease,
    }
}

/// Waits on the traced child until it has died, which releases it to Orbit.
fn release(child: i32) {
    loop {
        let mut status = 0;
        // SAFETY: status is writable; the traced child is waited on by PID.
        let waited = unsafe { libc::waitpid(child, &mut status, libc::__WALL) };
        assert_eq!(waited, child, "wait: {}", std::io::Error::last_os_error());
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            return;
        }
    }
}

pub fn run() {
    // Once the killed child is released, Orbit reaps it and completes the
    // Stop.
    let Held {
        _directory,
        mut server,
        record,
        identity,
        mut lease,
    } = held();
    release(server.child);
    let killed = (
        TerminationReason::ExplicitStop,
        ProcessOutcome::Signal(libc::SIGKILL),
    );
    match lease.read() {
        Reply::Message(ServerMessage::Stopped(stopped)) => {
            assert_eq!((stopped.reason, stopped.outcome), killed, "{stopped:?}")
        }
        other => panic!("expected Stopped, received {other:?}"),
    }
    drop(lease);
    server.communicate(10.0).assert_silent(0);
    let tombstone = tombstone(&record);
    assert_eq!((tombstone.reason, tombstone.outcome), killed, "{tombstone:?}");
    assert_eq!(tombstone.identity, identity);
    assert!(!server.path.exists());

    // A termination signal during the wait keeps ORB-C12's bound: the child
    // is still not reaped, so Orbit exits 128 plus the signal.
    let mut waiting = held();
    signal(waiting.server.pid(), libc::SIGTERM);
    let finished = waiting.server.communicate(10.0);
    release(waiting.server.child);
    assert_eq!(finished.code(), Some(128 + libc::SIGTERM));
    assert!(!waiting.record.exists() && !waiting.server.path.exists());
}
