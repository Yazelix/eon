use crate::support::*;
use orbit_protocol::session::{
    ClientMessage,
    ClipboardLocation::{self, Primary, Selection, Standard},
    ServerMessage,
};

/// Reads messages until a frame with this title; returns the clipboard writes
/// that came before it.
fn clipboard_until_frame(client: &mut Peer, title: &str) -> Vec<(ClipboardLocation, String)> {
    let mut writes = Vec::new();
    loop {
        match client.receive() {
            ServerMessage::ClipboardWrite { location, text } => writes.push((location, text)),
            ServerMessage::Frame(frame) if frame.title == title => return writes,
            ServerMessage::Frame(_) => {}
            other => panic!("expected a write or a frame, received {}", brief(&other)),
        }
    }
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "clipboard-gate");
    let shell = script(&directory, "CLIPBOARD_SHELL", "clipboard-shell");
    let mut server =
        Server::start(&directory, &shell, &[("ORBIT_CLIPBOARD_GATE", gate.path())]);
    let mut observer = Peer::connect(&server.path, 4.0);
    observer.send(&ClientMessage::ObserveMetadata);
    observer.expect(ServerMessage::ObservingMetadata);
    observer.metadata();

    // The observer hears the title that follows the detached write, so the
    // server has read that write before the client attaches.
    gate.release();
    while observer.metadata().title != "DETACHED WROTE" {}

    let mut client = Peer::attach(&server.path, 4.0);
    client.frame();
    gate.release();
    // Each write precedes the frame of its read; the clear and the detached
    // write never arrive.
    let writes = clipboard_until_frame(&mut client, "COPIED");
    assert_eq!(
        writes,
        [(Standard, "copied".into()), (Primary, "primary".into())]
    );

    gate.release();
    let held = loop {
        match client.receive() {
            ServerMessage::Frame(frame) => assert_ne!(frame.title, "HELD"),
            other => break other,
        }
    };
    assert_eq!(
        held,
        ServerMessage::ClipboardWrite {
            location: Selection,
            text: "held".into()
        }
    );
    client.quiet(0.3, "a held update published a frame");
    gate.release();
    assert_eq!(clipboard_until_frame(&mut client, "HELD"), []);

    // The overflowing read ends the attachment without any write.
    gate.release();
    let (data, _) = client.read_until_closed(true);
    let (messages, _) = split(&data);
    for message in &messages {
        assert!(
            !matches!(message, ServerMessage::ClipboardWrite { .. }),
            "{}",
            brief(message)
        );
    }
    drop(client);

    // The child kept running; a new client hears no replayed write.
    let mut client = Peer::attach(&server.path, 4.0);
    client.frame();
    gate.release();
    client.frame_titled("AFTER OVERFLOW");
    gate.release();
    client.exited(5);
    drop(client);
    drop(observer);

    server.passed(5, 10.0);
}
