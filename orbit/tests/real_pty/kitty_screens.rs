use crate::kitty::*;
use orbit_protocol::PlacementState::{Pending, Ready};

const ALTERNATE: Image = Image {
    width: 20,
    height: 20,
    channels: 4,
    seed: 5,
};

pub fn run() {
    let primary = placeholders(7, 2, 1, 3, 2);
    let alternate = placeholders(8, 10, 1, 2, 1);
    let kitty = Kitty::start(&[
        format!("{}{}", transmit(7, SMALL, 3, 2), step(&primary, "PRIMARY")),
        // The primary screen's image id has nothing to show on the
        // alternate screen, whose storage is its own.
        format!(
            "\x1b[?1049h{}{}",
            transmit(8, ALTERNATE, 2, 1),
            step(&format!("{primary}{alternate}"), "ALTERNATE")
        ),
        step("\x1b[?1049l", "BACK"),
        step(&format!("\x1b[?1049h{alternate}"), "AGAIN"),
    ]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut ledger = Ledger::default();

    kitty.next();
    ledger.settle(&mut client, "PRIMARY");
    let first = ledger.entries()[0].0;

    kitty.next();
    let entered = ledger.titled(&mut client, "ALTERNATE");
    let [(second, Pending)] = entries(&entered)[..] else {
        panic!("expected one pending entry, found {:?}", entries(&entered));
    };
    assert!(second > first);
    ledger.settle(&mut client, "ALTERNATE");
    assert_eq!(ledger.pixels(second), ALTERNATE.pixels());

    // Each screen's storage keeps its images: returning to a screen
    // references its sent resource at once, and nothing is sent again or
    // retired.
    kitty.next();
    let back = ledger.titled(&mut client, "BACK");
    assert_eq!(entries(&back), [(first, Ready); 2]);
    kitty.next();
    let again = ledger.titled(&mut client, "AGAIN");
    assert_eq!(entries(&again), [(second, Ready)]);
    assert_eq!(
        ledger.notes,
        [Note::Announced(first), Note::Announced(second)]
    );

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
