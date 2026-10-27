use crate::{kitty::*, support::*};
use orbit_protocol::{
    Frame, PlacementState,
    session::{ScrollOutcome, ServerMessage},
};

/// An entry's resource, viewport cell and drawn pixel size.
type Geometry = (u64, (i32, i32), (u32, u32));

/// Each entry's geometry; every entry is ready.
fn geometry(frame: &Frame) -> Vec<Geometry> {
    let graphics = frame.graphics.as_ref().expect("Kitty graphics state");
    graphics
        .placements
        .iter()
        .map(|entry| {
            assert_eq!(entry.state, PlacementState::Ready);
            (entry.resource, (entry.col, entry.row), (entry.width, entry.height))
        })
        .collect()
}

/// Sends a scroll of `rows` and reads its outcome's frame.
fn scrolled(client: &mut Peer, ledger: &mut Ledger, revision: u64, rows: i16) -> Frame {
    client.send(&scroll(revision, rows));
    loop {
        if let ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            applied_rows,
            frame,
            ..
        }) = ledger.read(client)
        {
            assert_eq!(applied_rows, rows);
            return *frame;
        }
    }
}

pub fn run() {
    let kitty = Kitty::start(&[
        format!(
            "{}{}",
            transmit(7, SMALL, 3, 2),
            step(&placeholders(7, 2, 3, 3, 2), "SHOWN")
        ),
        step("\x1b[6;1H\n\n", "SCROLLED"),
        step("\n\n\n\n", "GONE"),
    ]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut ledger = Ledger::default();

    kitty.next();
    let shown = ledger.settle(&mut client, "SHOWN");
    let resource = ledger.entries()[0].0;
    let at = |row, (width, height)| (resource, (2, row), (width, height));
    assert_eq!(geometry(&shown), [at(3, (30, 20)), at(4, (30, 20))]);

    // A new cell size changes the geometry; the resource stays sent.
    client.send(&size(20, 6, 12, 24));
    ledger.until(&mut client, |ledger| {
        ledger
            .frame
            .as_ref()
            .and_then(|frame| frame.graphics.as_ref())
            .is_some_and(|graphics| graphics.cell_width == 12)
    });
    let resized = ledger.frame.clone().expect("frame");
    let graphics = resized.graphics.as_ref().expect("Kitty graphics state");
    assert_eq!(graphics.cell_height, 24);
    let at = |row| at(row, (36, 24));
    assert_eq!(geometry(&resized), [at(3), at(4)]);

    // Output that scrolls the screen moves the entries up with their cells,
    // and off the viewport they leave the frame.
    kitty.next();
    let moved = ledger.titled(&mut client, "SCROLLED");
    assert_eq!(geometry(&moved), [at(1), at(2)]);
    kitty.next();
    let gone = ledger.titled(&mut client, "GONE");
    assert_eq!(geometry(&gone), []);

    // A viewport scrolled into history is a live frame with its
    // placements, and returning to the bottom leaves them out again.
    let history = scrolled(&mut client, &mut ledger, gone.revision, -4);
    assert_eq!(geometry(&history), [at(1), at(2)]);
    let live = scrolled(&mut client, &mut ledger, history.revision, 4);
    assert_eq!(live.scroll_position.rows_from_live, 0);
    assert_eq!(geometry(&live), []);
    let history = scrolled(&mut client, &mut ledger, live.revision, -5);
    assert_eq!(geometry(&history), [at(2), at(3)]);
    assert_eq!(ledger.notes, [Note::Announced(resource)]);

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
