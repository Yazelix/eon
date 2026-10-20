use crate::support::*;
use orbit_protocol::{
    Frame,
    session::{
        FailureCode, KeyAction,
        MouseAction::{Motion, Press, Release},
        MouseButton::{Five, Four, Left},
        ServerMessage, WheelOutcome,
    },
};

/// Reads a WheelOutcome::Viewport that applied these rows; returns its frame.
fn wheel(peer: &mut Peer, rows: i8) -> Frame {
    match peer.receive() {
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows,
            frame,
        }) if applied_rows == rows => *frame,
        other => panic!("expected a wheel outcome of {rows}, received {}", brief(&other)),
    }
}

fn hex_of(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "mouse-gate");
    let shell = script(&directory, "MOUSE_SHELL", "mouse-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_MOUSE_GATE", gate.path())]);
    let mut client = Peer::attach(&server.path, 10.0);
    client.receive();
    gate.release();
    let free = client.frame_titled("MOUSE FREE");
    assert_eq!(free.scroll_position.rows_from_live, 0);

    // Without mouse tracking a click writes nothing to the child.
    client.send(&mouse(Press, Some(Left)));
    client.expect(ServerMessage::Accepted);
    client.send(&mouse(Release, Some(Left)));
    client.expect(ServerMessage::Accepted);
    client.quiet(0.2, "an untracked click published a change");

    // The wheel moves the viewport one row, clamped at live output, and each
    // outcome carries the frame at the next revision.
    client.send(&mouse(Press, Some(Four)));
    let up = wheel(&mut client, -1);
    assert_eq!(
        (up.revision, up.scroll_position.rows_from_live),
        (free.revision + 1, 1)
    );
    client.send(&mouse(Press, Some(Five)));
    let down = wheel(&mut client, 1);
    assert_eq!(
        (down.revision, down.scroll_position.rows_from_live),
        (up.revision + 1, 0)
    );
    client.send(&mouse(Press, Some(Five)));
    let still = wheel(&mut client, 0);
    assert_eq!(
        (still.revision, still.scroll_position.rows_from_live),
        (down.revision + 1, 0)
    );
    client.quiet(0.2, "a wheel published a separate frame");

    // Synchronized output fails the wheel at once rather than defer it.
    gate.release();
    client.quiet(0.15, "held output");
    client.send(&mouse(Press, Some(Four)));
    client.failure(FailureCode::Terminal);
    gate.release();
    let released = client.frame_titled("MOUSE HELD");
    assert_eq!(released.scroll_position.rows_from_live, 0);

    // Under SGR mouse tracking, untracked motion writes nothing, and a click
    // and the wheel reach the child; the wheel answers TerminalRouted and
    // leaves the viewport.
    gate.release();
    client.frame_titled("MOUSE TRACKED");
    client.send(&mouse(Motion, None));
    client.expect(ServerMessage::Accepted);
    client.send(&mouse(Press, Some(Left)));
    client.expect(ServerMessage::Accepted);
    client.send(&mouse(Release, Some(Left)));
    client.expect(ServerMessage::Accepted);
    client.send(&mouse(Press, Some(Four)));
    client.expect(ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted));
    client.send(&key('a', KeyAction::Press));
    client.expect(ServerMessage::Accepted);
    let tracked = hex_of("\x1b[<0;3;2M\x1b[<0;3;2m\x1b[<64;3;2M");
    let reported = client.frame_titled(&format!("TRACKED {tracked}"));
    assert_eq!(reported.scroll_position.rows_from_live, 0);
    gate.release();

    // On the alternate screen, mode 1007 turns the wheel into an arrow key.
    client.frame_titled("MOUSE ALTERNATE");
    client.send(&mouse(Press, Some(Five)));
    client.expect(ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted));
    client.send(&key('a', KeyAction::Press));
    client.expect(ServerMessage::Accepted);
    client.frame_titled(&format!("ALTERNATE {}", hex_of("\x1b[B")));
    gate.release();
    client.exited(5);
    drop(client);

    server.passed(5, 10.0);
}
