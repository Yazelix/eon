use crate::kitty::*;
use orbit_protocol::PlacementState::{Pending, Ready};

pub fn run() {
    let large = big(6);
    // Image 7 is big; 8, 9 and 10 are small. The first step shows 7, 9 and
    // 10; the second, while the client does not read, erases 9, deletes 10
    // and erases its cells, and shows 8.
    let kitty = Kitty::start(&[
        format!(
            "{}{}{}{}{}",
            transmit(7, large, 3, 2),
            transmit(8, Image { seed: 1, ..SMALL }, 3, 2),
            transmit(9, Image { seed: 2, ..SMALL }, 3, 2),
            transmit(10, Image { seed: 3, ..SMALL }, 3, 2),
            step(
                &format!(
                    "{}{}{}",
                    placeholders(7, 0, 0, 3, 2),
                    placeholders(9, 5, 0, 3, 2),
                    placeholders(10, 10, 0, 3, 2)
                ),
                "SHOWN"
            )
        ),
        step(
            &format!(
                "{}{}{}{}",
                erase(5, 0, 3, 2),
                delete(10),
                erase(10, 0, 3, 2),
                placeholders(8, 15, 0, 3, 2)
            ),
            "SWAPPED",
        ),
    ]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut observer = kitty.observe();
    let mut ledger = Ledger::default();

    // While the big image is in transfer, the other two wait pending.
    kitty.next();
    let shown = ledger.titled(&mut client, "SHOWN");
    let [(big, _), _, (_, Pending), _, (deleted, Pending), _] = entries(&shown)[..] else {
        panic!("expected six entries, found {:?}", entries(&shown));
    };
    ledger.until(&mut client, |ledger| ledger.transferring == Some(big));
    kitty.next();
    heard(&mut observer, "SWAPPED");
    drop(observer);
    let swapped = ledger.titled(&mut client, "SWAPPED");
    assert_eq!(ledger.transferring, Some(big), "the client held the transfer");
    let [(_, Pending), _, (late, Pending), _] = entries(&swapped)[..] else {
        panic!("expected four pending entries, found {:?}", entries(&swapped));
    };
    assert!(late > deleted);

    // Once room returns, only the image a current frame references starts:
    // neither the erased image, still stored, nor the deleted one.
    ledger.settle(&mut client, "SWAPPED");
    assert_eq!(
        ledger.entries(),
        [(big, Ready), (big, Ready), (late, Ready), (late, Ready)]
    );
    assert!(ledger.pixels(big) == large.pixels(), "the big pixels");
    assert_eq!(ledger.pixels(late), Image { seed: 1, ..SMALL }.pixels());
    assert_eq!(ledger.notes, [Note::Announced(big), Note::Announced(late)]);

    kitty.next();
    ledger.exited(&mut client, 0);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
