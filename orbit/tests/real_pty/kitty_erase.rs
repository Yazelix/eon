use crate::kitty::*;
use orbit_protocol::PlacementState;

pub fn run() {
    let shown = placeholders(7, 2, 1, 3, 2);
    let kitty = Kitty::start(&[
        format!("{}{}", transmit(7, SMALL, 3, 2), step(&shown, "SHOWN")),
        step("\x1b[H\x1b[2J", "CLEARED"),
        step(&shown, "RESHOWN"),
        // Yazi's hide: spaces over the cells, then a delete of every
        // placement on screen.
        step(&format!("{}\x1b_Ga=d,d=A,q=2\x1b\\", erase(2, 1, 3, 2)), "HIDDEN"),
        step(&shown, "BACK"),
        step("\x1b_Ga=d,d=A,q=2\x1b\\", "KEPT"),
        step(&delete(7), "DELETED"),
    ]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut ledger = Ledger::default();

    kitty.next();
    ledger.settle(&mut client, "SHOWN");
    let resource = ledger.entries()[0].0;
    let ready = [(resource, PlacementState::Ready); 2];

    // Clearing the screen leaves the image stored: no entry, no retirement,
    // and printing the cells again shows the sent resource at once.
    for (title, expected) in [
        ("CLEARED", &[][..]),
        ("RESHOWN", &ready[..]),
        ("HIDDEN", &[][..]),
        ("BACK", &ready[..]),
        ("KEPT", &ready[..]),
    ] {
        kitty.next();
        let frame = ledger.titled(&mut client, title);
        assert_eq!(entries(&frame), expected, "{title}");
    }
    assert_eq!(ledger.notes, [Note::Announced(resource)]);

    // Deleting the image while its cells remain blanks them with no entry,
    // and the resource retires after that frame.
    kitty.next();
    let deleted = ledger.titled(&mut client, "DELETED");
    assert_eq!(entries(&deleted), []);
    let cells = deleted.rows[1..3].iter().flat_map(|row| &row.cells[2..5]);
    assert!(
        cells.clone().all(|cell| cell.text.is_empty()),
        "{:?}",
        cells.collect::<Vec<_>>()
    );
    ledger.until(&mut client, |ledger| ledger.retired.contains(&resource));
    assert_eq!(
        ledger.notes,
        [Note::Announced(resource), Note::Retired(resource)]
    );

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
