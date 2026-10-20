use crate::support::*;
use orbit_protocol::session::{
    ClientMessage,
    ClipboardLocation::{self, Selection, Standard},
    FailureCode, Modifiers, SelectionAction, SelectionPosition, ServerMessage,
};
use std::fs;

const NO_MODIFIERS: Modifiers = Modifiers::empty();

/// The center of a 10x18-pixel cell, as surface coordinates.
fn cell(col: u16, row: u16) -> SelectionPosition {
    position(f32::from(col) * 10.0 + 5.0, f32::from(row) * 18.0 + 9.0)
}

/// Near the left edge of a cell, where a press starts selecting it.
fn left(col: u16, row: u16) -> SelectionPosition {
    position(f32::from(col) * 10.0 + 1.0, f32::from(row) * 18.0 + 9.0)
}

/// Past the middle of a cell, where a drag selects through it.
fn right(col: u16, row: u16) -> SelectionPosition {
    position(f32::from(col) * 10.0 + 9.0, f32::from(row) * 18.0 + 9.0)
}

fn begin_at(
    frame_revision: u64,
    position: SelectionPosition,
    modifiers: Modifiers,
    time_ns: u64,
) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Begin {
        frame_revision,
        position,
        time_ns,
        modifiers,
    })
}

fn begin(frame_revision: u64, position: SelectionPosition) -> ClientMessage {
    begin_at(frame_revision, position, NO_MODIFIERS, 1_000_000_000)
}

fn update(position: SelectionPosition) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Update {
        position,
        modifiers: NO_MODIFIERS,
    })
}

fn finish(position: SelectionPosition, modifiers: Modifiers) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Finish {
        position,
        modifiers,
    })
}

const COPY: ClientMessage = ClientMessage::Selection(SelectionAction::Copy);
const CANCEL: ClientMessage = ClientMessage::Selection(SelectionAction::Cancel);

fn finished(client: &mut Peer) -> u64 {
    match client.receive() {
        ServerMessage::SelectionFinished { frame_revision } => frame_revision,
        other => panic!("expected SelectionFinished, received {}", brief(&other)),
    }
}

fn copied(client: &mut Peer, location: ClipboardLocation, text: &str) {
    client.expect(ServerMessage::CopiedText {
        location,
        text: text.into(),
    });
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "select-gate");
    let pointer_file = directory.join("pointer");
    let shell = script(&directory, "SELECT_SHELL", "selection-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_SELECT_GATE", gate.path()),
            ("ORBIT_POINTER_FILE", &pointer_file),
        ],
    );
    let mut client = Peer::attach(&server.path, 4.0);
    let mut revision = client.frame_titled("SELECT READY").revision;

    // Nothing to copy, no sequence to update, and no future press.
    client.send(&COPY);
    client.failure(FailureCode::InvalidInput);
    client.send(&update(cell(1, 0)));
    client.failure(FailureCode::InvalidInput);
    client.send(&begin(revision + 1000, cell(0, 0)));
    client.failure(FailureCode::InvalidInput);

    // Without mouse tracking the sequence selects on the host.
    client.send(&begin(revision, left(0, 0)));
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&begin(revision, left(0, 0)));
    client.failure(FailureCode::InvalidInput); // A press during a sequence.
    client.send(&update(cell(2, 0)));
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&finish(right(4, 0), NO_MODIFIERS));
    let named = finished(&mut client);
    assert_eq!(client.frame().revision, named);
    copied(&mut client, Selection, "HELLO");
    client.send(&COPY);
    copied(&mut client, Standard, "HELLO");

    // Cancel clears the presented selection but keeps the copy.
    client.send(&CANCEL);
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&COPY);
    copied(&mut client, Standard, "HELLO");
    client.send(&update(cell(1, 0)));
    client.failure(FailureCode::InvalidInput);

    // The old Orbit's click ladder: a word, a space-delimited span, then the
    // logical line, each frozen on release.
    let line = "HELLO WORLD alpha:beta/gamma";
    for (time_ns, expected) in [
        (5_000_000_000, None),
        (5_100_000_000, Some("beta/gamma")),
        (5_200_000_000, Some("alpha:beta/gamma")),
        (5_300_000_000, Some(line)),
        (5_400_000_000, Some(line)),
    ] {
        client.send(&begin_at(revision, cell(19, 0), NO_MODIFIERS, time_ns));
        client.expect(ServerMessage::Accepted);
        client.frame();
        client.send(&finish(cell(19, 0), NO_MODIFIERS));
        revision = finished(&mut client);
        assert_eq!(client.frame().revision, revision);
        if let Some(expected) = expected {
            copied(&mut client, Selection, expected);
        }
    }

    // Output that enables mouse tracking ends an active host sequence.
    client.send(&begin_at(revision, left(0, 0), NO_MODIFIERS, 9_000_000_000));
    client.expect(ServerMessage::Accepted);
    client.frame();
    gate.release();
    revision = client.frame_titled("TRACKING").revision;
    client.send(&update(cell(3, 0)));
    client.failure(FailureCode::InvalidInput);

    // Under mouse tracking the sequence goes to the terminal as mouse input,
    // and Finish names the current revision.
    client.send(&begin(revision, cell(0, 0)));
    client.expect(ServerMessage::Accepted);
    client.send(&update(cell(2, 0)));
    client.expect(ServerMessage::Accepted);
    client.send(&finish(cell(0, 0), NO_MODIFIERS));
    assert_eq!(finished(&mut client), revision);
    client.frame_titled("POINTER READ");
    assert_eq!(
        fs::read_to_string(&pointer_file).expect("pointer"),
        "$'\\E[<0;1;1' $'\\E[<0;1;1'\n"
    );

    // Shift selects on the host even while the terminal tracks the mouse.
    client.send(&begin_at(revision, left(6, 0), Modifiers::SHIFT, 1_000_000_000));
    client.expect(ServerMessage::Accepted);
    client.frame();
    client.send(&finish(right(10, 0), Modifiers::SHIFT));
    let named = finished(&mut client);
    assert_eq!(client.frame().revision, named);
    copied(&mut client, Selection, "WORLD");
    // Leaving ends the attachment before the next client arrives.
    client.shutdown_write();
    client.drain();
    drop(client);

    // A new attachment starts without the old copy.
    let mut client = Peer::attach(&server.path, 4.0);
    client.frame();
    client.send(&COPY);
    client.failure(FailureCode::InvalidInput);
    gate.release();
    client.exited(3);
    drop(client);

    server.passed(3, 10.0);
}
