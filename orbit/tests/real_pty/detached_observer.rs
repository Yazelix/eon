use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::time::Instant;

fn metadata_with(observer: &mut Peer, title: &str) {
    while observer.metadata().title != title {}
}

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let gate = Gate::new(&directory, "title-gate");
    let shell = script(&directory, "DETACHED_OBSERVER_SHELL", "detached-observer-shell");
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

    // No interactive client has attached yet.
    let mut observer = Peer::connect(&server.path, 4.0);
    observer.send(&ClientMessage::ObserveMetadata(None));
    observer.expect(ServerMessage::ObservingMetadata(Default::default()));
    observer.metadata();

    let mut contender = Peer::connect(&server.path, 4.0);
    contender.send(&ClientMessage::ObserveMetadata(None));
    contender.expect(ServerMessage::Busy); // Observer slot is full.
    drop(contender);

    gate.release();
    metadata_with(&mut observer, "DETACHED ONE");

    let mut client = Peer::attach(&server.path, 4.0);
    client.frame_with("DETACHED ONE");
    drop(client);
    // Give Bend time to see the close, so the next title arrives detached.
    sleep(0.3);
    gate.release();
    metadata_with(&mut observer, "DETACHED TWO");

    let started = Instant::now();
    gate.release();
    metadata_with(&mut observer, "DETACHED HELD");
    let held = started.elapsed().as_secs_f64();
    assert!(held >= 0.5, "held metadata arrived after {held:.3} s");

    let mut client = Peer::attach(&server.path, 4.0);
    client.frame_with("DETACHED HELD");
    client.send(&paste(b"x"));
    client.expect(ServerMessage::Accepted);
    client.exited(7);
    drop(client);
    observer.expect(ServerMessage::Exited { code: 7 });
    drop(observer);

    assert_eq!(server.passed(7, 10.0), pid);
}
