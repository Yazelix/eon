use crate::management_lease::leased;
use crate::management_record::{suffixed, tombstone};
use crate::support::*;
use orbit_protocol::{
    management::{ProcessOutcome, TerminationReason},
    session::{ClientMessage, ServerMessage},
};
use std::time::Instant;

const MANAGED: [&str; 4] = ["--management-v4", "session-1", "run-1", "component-1"];

fn observe(path: &std::path::Path) -> Peer {
    let mut observer = Peer::connect(path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    observer.metadata();
    observer
}

pub fn run() {
    // A termination signal reaps the child, tells the observer its code (1
    // for the hangup) before the explicit tombstone, and closes the
    // interactive client without Exited.
    {
        let directory = TempDir::new();
        let shell = executable(&directory, "child.sh", "#!/bin/sh\nwhile :; do sleep 1; done\n");
        let mut server = Server::launch(&directory, &[shell.as_os_str()], &[], &[], &MANAGED);
        let record_path = suffixed(&server.path, ".record");
        wait_for(&record_path);
        let mut lease = leased(&record_path);
        lease.allow_self();
        let mut client = Peer::attach(&server.path, 10.0);
        client.frame();
        let mut observer = observe(&server.path);

        signal(server.pid(), libc::SIGTERM);
        observer.expect(ServerMessage::Exited { code: 1 });
        observer.expect_closed();
        let (data, _) = client.read_until_closed(true);
        let (messages, _) = split(&data);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, ServerMessage::Exited { .. })),
            "the interactive client heard Exited after a termination signal"
        );
        server.communicate(30.0).assert_silent(0);
        assert!(!alive(server.child), "the child outlived the server");
        assert!(!server.path.exists(), "the socket pathname remains");
        let tombstone = tombstone(&record_path);
        assert_eq!(
            (tombstone.reason, tombstone.outcome),
            (
                TerminationReason::ExplicitStop,
                ProcessOutcome::Signal(libc::SIGHUP)
            ),
            "{tombstone:?}"
        );
    }

    // An observer that stops reading until its socket is full, during a
    // thousand title changes, and reads again only after the child is reaped
    // still receives the latest title and Exited within the finish bound.
    {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "slow-gate");
        let shell = script(&directory, "SLOW_SHELL", "slow-observer-shell");
        let mut server =
            Server::start(&directory, &shell, &[("ORBIT_SLOW_GATE", gate.path())]);
        let mut observer = observe(&server.path);
        gate.release();
        sleep(2.0);

        // The script prints a line under the same title, then exits.
        gate.release();
        sleep(0.2);
        gate.release();
        let deadline = Instant::now() + seconds(5.0);
        while alive(server.child) {
            assert!(Instant::now() < deadline, "the child was not reaped");
            sleep(0.005);
        }
        let reaped = Instant::now();
        sleep(0.2);
        let mut latest = String::new();
        loop {
            match observer.receive() {
                ServerMessage::Metadata(metadata) => latest = metadata.title,
                ServerMessage::Exited { code } => {
                    assert_eq!(code, 7, "exit code");
                    break;
                }
                other => panic!("expected metadata or Exited, received {}", brief(&other)),
            }
        }
        assert_eq!(latest, "SLOW LAST");
        assert!(
            reaped.elapsed() < seconds(1.0),
            "Exited arrived {:?} after the reap",
            reaped.elapsed()
        );
        observer.expect_closed();
        server.passed(7, 10.0);
    }
}
