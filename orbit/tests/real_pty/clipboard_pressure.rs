use crate::support::*;
use orbit_protocol::session::{ClientMessage, ClipboardLocation, ServerMessage};

/// The length of each write's text.
const TEXT_BYTES: usize = 524_286;
const WRITES: usize = 40;

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "clipboard-gate");
    let shell = script(&directory, "CLIPBOARD_PRESSURE_SHELL", "clipboard-pressure-shell");
    let mut server =
        Server::start(&directory, &shell, &[("ORBIT_CLIPBOARD_GATE", gate.path())]);
    let mut observer = Peer::connect(&server.path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    observer.metadata();

    // The client reads nothing after its first frame, so ordered writes,
    // which no frame replaces, fill its output queue.
    let mut client = Peer::connect_with_receive_buffer(&server.path, 4096, 10.0);
    client.send(&ClientMessage::Hello);
    client.expect(ServerMessage::Attached);
    client.frame();
    for write in 1..=WRITES {
        gate.release();
        let title = format!("W{write}");
        while observer.metadata().title != title {}
    }

    // The queue's bound ended the attachment; the client received only whole
    // writes, fewer than the child sent.
    let (data, _) = client.read_until_closed(true);
    let (messages, _) = split(&data);
    let mut received = 0;
    for message in &messages {
        match message {
            ServerMessage::ClipboardWrite { location, text } => {
                assert_eq!(*location, ClipboardLocation::Standard);
                assert!(text.len() == TEXT_BYTES && text.bytes().all(|byte| byte == b'x'));
                received += 1;
            }
            ServerMessage::Frame(_) => {}
            other => panic!("expected a write or a frame, received {}", brief(other)),
        }
    }
    assert!(received < WRITES, "the client took all {received} writes");
    drop(client);

    // The child kept running, and a new client hears none of the writes.
    let mut client = Peer::attach(&server.path, 10.0);
    client.frame();
    gate.release();
    client.frame_titled("AFTER PRESSURE");
    gate.release();
    client.exited(6);
    drop(client);
    drop(observer);

    server.passed(6, 10.0);
}
