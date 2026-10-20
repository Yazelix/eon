use crate::support::*;

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "reopen-gate");
    let shell = script(&directory, "REOPEN_SHELL", "pty-reopen-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_REOPEN_GATE", gate.path())]);
    let mut client = Peer::attach(&server.path, 10.0);
    client.frame();
    // The child lets go of its terminal and takes it back later: the session
    // survives, and its later output reaches the client.
    gate.release();
    client.frame_titled("BACK");
    gate.release();
    client.exited(4);
    drop(client);

    server.passed(4, 10.0);
}
