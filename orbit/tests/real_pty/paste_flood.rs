use crate::support::*;
use orbit_protocol::session::{FailureCode, ServerMessage};

// Two pastes that together exceed the PTY input queue (1 MiB plus 16 bytes)
// while the child reads nothing, then one that the child echoes as it reads.
const STALLED_LINES: usize = 7_500;
const ECHOED_LINES: usize = 2_500;

/// Eighty columns of output per pasted line.
fn line() -> Vec<u8> {
    let mut line = vec![b'x'; 79];
    line.push(b'\n');
    line
}

/// Reads frames until one has the title, counting the Accepted messages.
fn until_title(peer: &mut Peer, title: &str, mut accepted: usize) -> usize {
    loop {
        match peer.receive() {
            ServerMessage::Accepted => accepted += 1,
            ServerMessage::Frame(frame) if frame.title == title => return accepted,
            ServerMessage::Frame(_) => {}
            other => panic!("expected Accepted or a frame, received {}", brief(&other)),
        }
    }
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "paste-gate");
    let shell = script(&directory, "PASTE_SHELL", "paste-flood-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_PASTE_GATE", gate.path())]);
    // While the child reads nothing, input waits in a bounded queue and the
    // session keeps serving; a paste the queue cannot hold fails with
    // Failure(Terminal) and ends only that attachment.
    let mut stalled = Peer::attach(&server.path, 20.0);
    // Echo must be off before the first paste reaches the terminal.
    stalled.frame_titled("PASTE READY");
    stalled.send(&paste(&line().repeat(STALLED_LINES)));
    stalled.expect(ServerMessage::Accepted);
    stalled.send(&paste(&line().repeat(STALLED_LINES)));
    stalled.failure(FailureCode::Terminal);
    stalled.expect_closed();
    drop(stalled);

    // The server still answers, and the queued input reaches the child in
    // order once it reads, while its output keeps flowing to the client.
    let mut client = Peer::attach(&server.path, 20.0);
    client.frame();
    client.send(&paste(b"MARK\n"));
    gate.release();
    let accepted = until_title(&mut client, &format!("MARK {}", STALLED_LINES + 1), 0);
    // A paste the child echoes while it reads cannot stop the session.
    let mut echoed = line().repeat(ECHOED_LINES);
    echoed.extend_from_slice(b"END\n");
    client.send(&paste(&echoed));
    let expected = STALLED_LINES + 1 + ECHOED_LINES + 1;
    let accepted = until_title(&mut client, &format!("LINES {expected}"), accepted);
    assert_eq!(accepted, 2);
    gate.release();
    client.exited(5);
    drop(client);

    server.passed(5, 10.0);
}
