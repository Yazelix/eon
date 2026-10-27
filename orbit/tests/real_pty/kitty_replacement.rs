use crate::kitty::*;

pub fn run() {
    let large = big(2);
    let shown = placeholders(7, 2, 1, 3, 2);
    let replacement = Image { seed: 1, ..SMALL };
    let late = Image { seed: 3, ..SMALL };
    let kitty = Kitty::start(&[
        format!("{}{}", transmit(7, SMALL, 3, 2), step(&shown, "SHOWN")),
        format!("{}{}", transmit(7, replacement, 3, 2), step("", "REPLACED")),
        format!("{}{}", transmit(7, large, 3, 2), step("", "BIG")),
        format!("{}{}", transmit(7, late, 3, 2), step("", "SWAPPED")),
    ]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut observer = kitty.observe();
    let mut ledger = Ledger::default();

    kitty.next();
    ledger.settle(&mut client, "SHOWN");
    let [(first, _), (same, _)] = ledger.entries()[..] else {
        panic!("expected two entries, found {:?}", ledger.entries());
    };
    assert_eq!(first, same);
    assert_eq!(ledger.pixels(first), SMALL.pixels());

    // New pixels under the same id are a new resource with a greater
    // number; the old one retires once a frame without it is written.
    kitty.next();
    ledger.until(&mut client, |ledger| {
        ledger.settled("REPLACED") && ledger.retired.contains(&first)
    });
    let [(second, _), _] = ledger.entries()[..] else {
        panic!("expected two entries, found {:?}", ledger.entries());
    };
    assert!(second > first);
    assert_eq!(ledger.pixels(second), replacement.pixels());

    // A replacement while its resource is still in transfer: the client
    // stops reading once the big image is announced, and the child replaces
    // it again.
    kitty.next();
    ledger.until(&mut client, |ledger| {
        ledger.transferring.is_some_and(|resource| resource > second)
    });
    let big = ledger.transferring.expect("the big image in transfer");
    kitty.next();
    heard(&mut observer, "SWAPPED");
    drop(observer);
    let swapped = ledger.titled(&mut client, "SWAPPED");
    assert_eq!(
        ledger.transferring,
        Some(big),
        "the replacement arrived during the transfer"
    );
    let [(last, state), _] = entries(&swapped)[..] else {
        panic!("expected two entries, found {:?}", entries(&swapped));
    };
    assert!(last > big && state == orbit_protocol::PlacementState::Pending);
    // The transfer finishes from its copy, then the resource retires and
    // the replacement follows.
    ledger.until(&mut client, |ledger| {
        ledger.settled("SWAPPED") && ledger.retired.contains(&big)
    });
    assert!(ledger.pixels(big) == large.pixels(), "the big pixels");
    assert_eq!(ledger.pixels(last), late.pixels());
    assert_eq!(ledger.entries()[0].0, last);
    let at = |note| ledger.notes.iter().position(|seen| *seen == note);
    assert!(at(Note::Retired(big)) < at(Note::Announced(last)));
    assert_eq!(ledger.notes.len(), 7, "{:?}", ledger.notes);

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
