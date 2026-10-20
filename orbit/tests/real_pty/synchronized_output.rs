use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::time::Instant;

fn held(peer: &mut Peer, timeout: f64, what: &str) {
    peer.quiet_then(timeout, &format!("{what} published during a held update"), 4.0);
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "sync-gate");
    let shell = script(&directory, "SYNC_SHELL", "synchronized-output-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_SYNC_GATE", gate.path())]);
    let mut active = Peer::attach(&server.path, 4.0);
    let mut observer = Peer::connect(&server.path, 4.0);
    let revision = active.frame().revision;
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    assert_eq!(observer.metadata().title, "");

    // A synchronized update split across PTY reads stays held while input is
    // still accepted.
    gate.release();
    held(&mut active, 0.5, "a frame");
    active.send(&paste(b"x")); // Paste reaches the PTY while held.
    active.expect(ServerMessage::Accepted);
    held(&mut active, 0.3, "a frame");
    held(&mut observer, 0.1, "metadata");
    gate.release();
    let complete = active.frame();
    assert!(complete.revision > revision);
    assert!(mentions(&complete, "SYNC COMPLETE") && !mentions(&complete, "SYNC PARTIAL"));
    assert_eq!(observer.metadata().title, "SYNC COMPLETE");
    held(&mut active, 0.3, "a second frame");

    // A missing release is bounded by the one-second watchdog.
    gate.release();
    let started = Instant::now();
    held(&mut active, 0.7, "a frame");
    let released = active.frame();
    let elapsed = started.elapsed().as_secs_f64();
    assert!((0.8..3.0).contains(&elapsed), "{elapsed}");
    assert!(released.revision > complete.revision);
    assert!(mentions(&released, "SYNC WATCHDOG"));
    assert_eq!(observer.metadata().title, "SYNC WATCHDOG");

    // Child exit releases a held update before Exited.
    gate.release();
    let last = active.frame();
    assert!(last.revision > released.revision);
    assert!(mentions(&last, "SYNC EXIT"));
    active.expect(ServerMessage::Exited { code: 7 });
    assert_eq!(observer.metadata().title, "SYNC EXIT");
    observer.expect(ServerMessage::Exited { code: 7 });
    drop(observer);
    drop(active);

    server.passed(7, 10.0);
}
