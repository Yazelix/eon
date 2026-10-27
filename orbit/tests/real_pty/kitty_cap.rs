use crate::kitty::*;
use orbit_protocol::PlacementState::{Pending, Ready};

/// Kitty images 1 to 1,025, one pixel each.
const IMAGES: u32 = 1025;

/// One frame shows 256 images, the most a frame references: one cell
/// each on a 32x8 screen.
const SCREEN: u32 = 256;

fn pixel(id: u32) -> Image {
    Image {
        width: 1,
        height: 1,
        channels: 3,
        seed: id as usize,
    }
}

/// A cleared screen showing images `ids`, one cell each.
fn screen(ids: impl Iterator<Item = u32>, title: &str) -> String {
    let cells: String = ids
        .enumerate()
        .map(|(cell, id)| placeholders(id, cell as u32 % 32, cell as u32 / 32, 1, 1))
        .collect();
    step(&format!("\x1b[H\x1b[2J{cells}"), title)
}

pub fn run() {
    let stored: String = (1..=IMAGES).map(|id| transmit(id, pixel(id), 1, 1)).collect();
    let mut steps = vec![stored + &step("", "STORED")];
    for page in 0..4 {
        steps.push(screen(
            page * SCREEN + 1..=(page + 1) * SCREEN,
            &format!("PAGE {page}"),
        ));
    }
    steps.push(screen(IMAGES..=IMAGES, "OVER"));
    steps.push(step(&delete(1), "FREED"));
    let kitty = Kitty::start(&steps);
    let mut client = kitty.attach(size(32, 8, 10, 20));
    let mut ledger = Ledger::default();
    kitty.next();
    ledger.titled(&mut client, "STORED");

    // Four pages send 1,024 resources, which all stay stored, so none
    // retires.
    for page in 0..4 {
        kitty.next();
        let shown = ledger.settle(&mut client, &format!("PAGE {page}"));
        assert_eq!(entries(&shown).len(), SCREEN as usize);
        assert!(!shown.graphics.as_ref().is_some_and(|graphics| graphics.placements_omitted));
    }
    assert_eq!(ledger.announced.len(), 1024);
    let first = *ledger.announced.keys().next().expect("a resource");

    // The 1,025th waits pending while the cap holds ...
    kitty.next();
    let over = ledger.titled(&mut client, "OVER");
    let [(last, Pending)] = entries(&over)[..] else {
        panic!("expected one pending entry, found {:?}", entries(&over));
    };
    client.quiet(1.0, "a resource past the cap started");

    // ... and starts once deleting image 1 retires its resource.
    kitty.next();
    ledger.settle(&mut client, "FREED");
    assert_eq!(ledger.entries(), [(last, Ready)]);
    assert_eq!(
        ledger.notes[1024..],
        [Note::Retired(first), Note::Announced(last)]
    );
    assert_eq!(ledger.pixels(last), pixel(IMAGES).pixels());

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
