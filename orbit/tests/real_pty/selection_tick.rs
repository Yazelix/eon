use crate::support::*;
use orbit_protocol::session::{
    ClientMessage, ClipboardLocation, FailureCode, Modifiers, ScrollOutcome, SelectionAction,
    SelectionPosition, ServerMessage,
};
use std::time::Instant;

fn begin(frame_revision: u64, position: SelectionPosition) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Begin {
        frame_revision,
        position,
        time_ns: 1_000_000_000,
        modifiers: Modifiers::empty(),
    })
}

fn finish(position: SelectionPosition) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Finish {
        position,
        modifiers: Modifiers::empty(),
    })
}

fn tick(position: SelectionPosition) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::AutoscrollUp { position })
}

/// Column 0 of the top row.
const TOP: SelectionPosition = SelectionPosition { x: 5.0, y: 9.0 };

fn finished(client: &mut Peer) -> u64 {
    match client.receive() {
        ServerMessage::SelectionFinished { frame_revision } => frame_revision,
        other => panic!("expected SelectionFinished, received {}", brief(&other)),
    }
}

/// Reads frames until a scroll outcome; returns its requested and applied rows.
fn outcome(client: &mut Peer) -> (i16, i16) {
    loop {
        match client.receive() {
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                requested_rows,
                applied_rows,
                ..
            }) => return (requested_rows, applied_rows),
            ServerMessage::Frame(_) => {}
            other => panic!("expected a scroll outcome, received {}", brief(&other)),
        }
    }
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "tick-gate");
    let held = directory.join("held");
    let shell = script(&directory, "TICK_SHELL", "selection-tick-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_TICK_GATE", gate.path()),
            ("ORBIT_HELD_FILE", &held),
        ],
    );
    let mut client = Peer::attach(&server.path, 4.0);
    let revision = client.frame_titled("TICK READY").revision;

    // A tick needs a held host gesture and a top-row position.
    client.send(&tick(TOP));
    client.failure(FailureCode::InvalidInput);
    client.send(&begin(revision, position(1.0, 5.0 * 18.0 + 9.0))); // Row 5: "line 43".
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&tick(position(5.0, 3.0 * 18.0 + 9.0)));
    client.failure(FailureCode::InvalidInput);

    // During a synchronized update one tick waits and a second fails.
    gate.release();
    let deadline = Instant::now() + seconds(10.0);
    while !held.exists() {
        assert!(
            Instant::now() < deadline,
            "the child never opened its update"
        );
        sleep(0.02);
    }
    client.send(&tick(TOP));
    client.quiet(0.3, "a tick moved during a synchronized update");
    client.send(&tick(TOP));
    client.failure(FailureCode::InvalidInput);
    gate.release();
    let mut moved = vec![outcome(&mut client)];

    // Ticks extend the same selection through more than one viewport of
    // history and stop at the oldest row.
    for _ in 0..40 {
        client.send(&tick(TOP));
        moved.push(outcome(&mut client));
    }
    assert!(moved.iter().all(|&(requested, _)| requested == -1), "{moved:?}");
    let applied: Vec<i16> = moved.iter().map(|&(_, rows)| rows).collect();
    let mut expected = vec![-1; 37];
    expected.extend([0; 4]);
    assert_eq!(applied, expected);

    client.send(&finish(position(1.0, 9.0)));
    let named = finished(&mut client);
    let released = client.frame();
    assert_eq!(released.revision, named);
    assert!(selected(&released));
    // From the oldest row down to the press at the start of line 43.
    let text = match client.receive() {
        ServerMessage::CopiedText {
            location: ClipboardLocation::Selection,
            text,
        } => text,
        other => panic!("expected a selection copy, received {}", brief(&other)),
    };
    let lines: Vec<String> = (1..43).map(|n| format!("line {n:02}")).collect();
    assert_eq!(text, lines.join("\n"));

    // A commit toward live clears the presented selection, though rows of it
    // stay in view; so does a return to live from two rows above live.
    client.send(&scroll(named, 35));
    let (_, applied, moved) = client.scroll_outcome();
    assert_eq!((applied, moved.scroll_position.rows_from_live), (35, 2));
    assert!(!selected(&moved));
    let row = 10.0 * 18.0 + 9.0; // "line 46", still in view at live.
    client.send(&begin(moved.revision, position(1.0, row)));
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&finish(position(39.0, row)));
    let named = finished(&mut client);
    assert!(selected(&client.frame()));
    client.expect(ServerMessage::CopiedText {
        location: ClipboardLocation::Selection,
        text: "line".into(),
    });
    client.send(&ClientMessage::ReturnToLive);
    let live = client.frame();
    assert_eq!(
        (live.revision, live.scroll_position.rows_from_live),
        (named + 1, 0)
    );
    assert!(!selected(&live));

    gate.release();
    client.exited(6);
    drop(client);

    server.passed(6, 10.0);
}
