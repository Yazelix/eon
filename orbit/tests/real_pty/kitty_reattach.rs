use crate::{kitty::*, support::*};
use orbit_protocol::{PlacementState, session::ServerMessage};

pub fn run() {
    let large = big(4);
    let kitty = Kitty::start(&[format!(
        "{}{}{}",
        transmit(7, SMALL, 3, 2),
        transmit(8, large, 3, 2),
        step(
            &format!("{}{}", placeholders(7, 2, 1, 3, 2), placeholders(8, 10, 1, 3, 2)),
            "SHOWN"
        )
    )]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut ledger = Ledger::default();

    // The first client takes the small image and leaves while the big one
    // is in transfer.
    kitty.next();
    ledger.until(&mut client, |ledger| {
        ledger.notes.len() == 2 && ledger.transferring.is_some()
    });
    let small = ledger.entries()[0].0;
    assert!(ledger.delivered(small));
    drop(client);

    // Each new attachment starts with nothing sent: its first frame lists
    // every placement pending, and both images arrive again from storage.
    for round in 0..2 {
        let mut client = Peer::attach(&kitty.server.path, 10.0);
        let mut ledger = Ledger::default();
        let ServerMessage::Frame(first) = ledger.read(&mut client) else {
            panic!("expected the attachment's first frame");
        };
        let pending = entries(&first);
        assert_eq!(pending.len(), 4, "{pending:?}");
        assert!(
            pending
                .iter()
                .all(|(_, state)| *state == PlacementState::Pending)
        );
        ledger.settle(&mut client, "SHOWN");
        let [small, _, big, _] = ledger.entries()[..] else {
            panic!("expected four entries, found {:?}", ledger.entries());
        };
        assert_eq!(ledger.pixels(small.0), SMALL.pixels());
        assert!(ledger.pixels(big.0) == large.pixels(), "the big pixels");
        assert_eq!(
            ledger.notes,
            [Note::Announced(small.0), Note::Announced(big.0)]
        );
        if round == 1 {
            kitty.next();
            ledger.exited(&mut client, 0);
        }
    }
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
