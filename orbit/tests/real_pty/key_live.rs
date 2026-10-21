use crate::support::*;
use orbit_protocol::session::{KeyAction, ServerMessage};

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "key-gate");
    let shell = script(&directory, "KEY_SHELL", "key-live-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_KEY_GATE", gate.path())]);
    let mut client = Peer::attach(&server.path, 10.0);
    client.frame();
    gate.release();
    let ready = client.frame_titled("KEY READY");
    assert_eq!(ready.scroll_position.rows_from_live, 0);
    client.send(&scroll(ready.revision, -10));
    let (requested, applied, scrolled) = client.scroll_outcome();
    assert_eq!(
        (requested, applied, scrolled.scroll_position.rows_from_live),
        (-10, -10, 10)
    );

    // Input other than a key, and a key that writes nothing, leave the
    // viewport in history.
    client.send(&paste(b"x"));
    client.expect(ServerMessage::Accepted);
    client.quiet(0.3, "a paste moved the viewport");
    client.send(&key('a', KeyAction::Release));
    client.expect(ServerMessage::Accepted);
    client.quiet(0.3, "a key release moved the viewport");

    // A key that writes PTY bytes returns the viewport to live output in one
    // frame after Accepted.
    client.send(&key('a', KeyAction::Press));
    client.expect(ServerMessage::Accepted);
    let live = client.frame();
    assert_eq!(live.scroll_position.rows_from_live, 0);
    assert!(live.revision > scrolled.revision);
    let read = client.frame_titled("READ xa");
    assert_eq!(read.scroll_position.rows_from_live, 0);
    gate.release();
    client.exited(3);
    drop(client);

    server.passed(3, 10.0);
}
