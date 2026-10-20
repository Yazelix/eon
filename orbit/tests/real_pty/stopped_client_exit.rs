use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::time::Instant;

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let gate = Gate::new(&directory, "output-gate");
    let exit_mark = directory.join("child-exiting");
    let shell = script(&directory, "STOPPED_CLIENT_SHELL", "stopped-client-exit-shell");
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[
            ("ORBIT_PID_FILE", &pid_file),
            ("ORBIT_OUTPUT_GATE", gate.path()),
            ("ORBIT_EXIT_MARK", &exit_mark),
        ],
        &[&pid_file],
        &[],
    );
    let pid = read_pid(&pid_file);

    let mut client = Peer::connect_with_receive_buffer(&server.path, 4096, 4.0);
    client.send(&ClientMessage::Hello);
    client.expect(ServerMessage::Attached);
    client.frame();
    // The client reads nothing more while the child floods and exits.
    gate.release();
    let deadline = Instant::now() + seconds(30.0);
    while !exit_mark.exists() {
        assert!(server.alive(), "server exited before the child");
        assert!(Instant::now() < deadline, "child did not finish its output");
        sleep(0.01);
    }
    let exiting = Instant::now();
    let finished = server.communicate(10.0);
    let elapsed = exiting.elapsed().as_secs_f64();
    // Bend gives the stopped client its one-second finish bound.
    assert!(
        (0.8..=4.0).contains(&elapsed),
        "server finished {elapsed:.3} s after child exit"
    );
    drop(client);

    assert_eq!(server.check_passed(&finished, 7), pid);
}
