use crate::{kitty::*, support::*};

pub fn run() {
    let large = big(0);
    let kitty = Kitty::start(&[format!(
        "{}{}{}",
        transmit(7, large, 3, 2),
        transmit(8, SMALL, 3, 2),
        step(
            &format!("{}{}", placeholders(7, 2, 1, 3, 2), placeholders(8, 10, 1, 3, 2)),
            "SHOWN"
        )
    )]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    let mut ledger = Ledger::default();

    // The child exits while the client holds the big image's transfer.
    kitty.next();
    ledger.until(&mut client, |ledger| ledger.transferring.is_some());
    let big = ledger.transferring.expect("the big image in transfer");
    kitty.next();
    // Orbit reaps the child just before its resource queue drops what it
    // has not started writing.
    let child = kitty.server.child;
    for _ in 0..500 {
        if !alive(child) {
            break;
        }
        sleep(0.01);
    }
    assert!(!alive(child), "Orbit did not reap its child");

    // Orbit finishes the chunk it is writing, drops the rest and the small
    // image, which never started, and Exited ends the stream.
    ledger.exited(&mut client, 0);
    assert_eq!(ledger.transferring, Some(big), "the transfer was cut short");
    assert_eq!(ledger.notes, [Note::Announced(big)]);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
