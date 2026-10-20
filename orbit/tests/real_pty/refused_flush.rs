use crate::support::*;
use orbit_protocol::session::{Failure, FailureCode, ServerMessage};

pub fn run() {
    let mut line = vec![b'x'; 79];
    line.push(b'\n');
    // Two of these overflow the PTY input queue.
    let mut pastes = encode(&paste(&line.repeat(7_500)));
    pastes.extend_from_within(..);

    let directory = TempDir::new();
    let gate = Gate::new(&directory, "flush-gate");
    let shell = script(&directory, "FLUSH_SHELL", "refused-flush-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_FLUSH_GATE", gate.path())]);
    // A refused client reads every message queued before its Failure, whole,
    // and then the Failure, before the server closes it.
    let mut client = Peer::attach(&server.path, 20.0);
    client.frame_titled("FLUSH READY");
    gate.release();
    sleep(0.5); // Frames pile up unread.
    client.send_bytes(&pastes);
    let (data, _) = client.read_until_closed(false);
    let (messages, cut) = split(&data);
    assert_eq!(cut, 0, "a message was cut: {cut} bytes left");
    let count = messages.len();
    assert!(count >= 2, "{count} messages");
    assert_eq!(messages[count - 2], ServerMessage::Accepted, "{}", brief(&messages[count - 2]));
    assert!(
        matches!(
            &messages[count - 1],
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                ..
            })
        ),
        "{}",
        brief(&messages[count - 1])
    );
    for message in &messages[..count - 2] {
        assert!(
            matches!(message, ServerMessage::Frame(_) | ServerMessage::Accepted),
            "{}",
            brief(message)
        );
    }
    drop(client);

    // The child kept running.
    let mut client = Peer::attach(&server.path, 4.0);
    client.frame();
    gate.release();
    client.exited(4);
    drop(client);

    server.passed(4, 10.0);
}
