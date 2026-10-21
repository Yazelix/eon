use crate::support::*;
use orbit_protocol::session::{ClientMessage, FailureCode, FocusEvent, ServerMessage};

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let gate = Gate::new(&directory, "title-gate");
    let shell = script(&directory, "LATE_OBSERVER_SHELL", "late-observer-shell");
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[
            ("ORBIT_PID_FILE", &pid_file),
            ("ORBIT_TITLE_GATE", gate.path()),
        ],
        &[&pid_file],
        &[],
    );
    let pid = read_pid(&pid_file);

    let mut active = Peer::attach(&server.path, 4.0);
    active.frame();

    let mut contender = Peer::connect(&server.path, 4.0);
    contender.send(&ClientMessage::Hello);
    contender.expect(ServerMessage::Busy);
    drop(contender);

    let mut silent = Peer::connect(&server.path, 4.0);
    silent.send_bytes(b"ORBS"); // Partial role header must not park Bend.
    active.send(&ClientMessage::Focus(FocusEvent::Gained)); // Focus must still reach the active client.
    active.expect(ServerMessage::Accepted);
    gate.release();
    active.frame_with("LATE ONE");
    // PTY output arrived before the pending role expired.
    silent.expect_nothing_yet("pending role expired before PTY output");
    silent.expect_closed(); // Pending role expires within one second.
    drop(silent);

    let mut observer = Peer::connect(&server.path, 4.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    let first = observer.metadata();
    assert_eq!(first.title, "LATE ONE");

    let mut contender = Peer::connect(&server.path, 4.0);
    contender.send(&ClientMessage::ObserveMetadata);
    contender.expect(ServerMessage::Busy); // Observer slot is full.
    drop(contender);

    gate.release();
    active.frame_with("LATE TWO");
    let second = observer.metadata();
    assert_eq!(second.title, "LATE TWO");
    assert!(second.revision > first.revision);
    drop(observer);

    let mut observer = Peer::connect(&server.path, 4.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    let current = observer.metadata();
    assert_eq!(
        (current.revision, current.title.as_str()),
        (second.revision, "LATE TWO")
    );

    // An observer may send nothing: its message is refused with
    // Failure(Protocol) and it is closed (ORB-C3),
    // while the client and the child go on; its slot is free at once.
    observer.send(&ClientMessage::Focus(FocusEvent::Gained));
    observer.failure(FailureCode::Protocol);
    observer.expect_closed();
    drop(observer);
    let mut observer = Peer::connect(&server.path, 4.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    assert_eq!(observer.metadata().title, "LATE TWO");

    active.send(&paste(b"x"));
    active.expect(ServerMessage::Accepted);
    active.exited(7);
    observer.expect(ServerMessage::Exited { code: 7 });
    drop(observer);
    drop(active);

    assert_eq!(server.passed(7, 10.0), pid);
}
