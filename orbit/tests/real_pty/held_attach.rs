use crate::support::*;
use orbit_protocol::{CellWidth, Screen};
use std::{path::Path, time::Instant};

/// Waits for the child's mark, then a little longer so the server has read
/// the output written before it.
fn marked(path: &Path) -> Instant {
    wait_for(path);
    let seen = Instant::now();
    sleep(0.3);
    seen
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "held-gate");
    let shell = script(&directory, "HELD_ATTACH_SHELL", "held-attach-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[("ORBIT_GATE", gate.path()), ("ORBIT_MARK", &directory.join("mark"))],
    );

    // A client that attaches while an update is held gets its first frame
    // only at the release, which sets the title.
    gate.release();
    marked(&directory.join("mark.held"));
    let mut client = Peer::attach(&server.path, 4.0);
    gate.release();
    assert_eq!(client.frame().title, "RELEASED");
    drop(client);

    // While an update is never released, the watchdog bounds the wait: no
    // frame for 0.3 s after attaching, then one within 2 s of the mark.
    gate.release();
    let held = marked(&directory.join("mark.watchdog"));
    let mut client = Peer::attach(&server.path, 4.0);
    client.quiet_then(0.3, "an attachment frame published during a held update", 4.0);
    let watchdog = client.frame();
    let elapsed = held.elapsed().as_secs_f64();
    assert!(elapsed < 2.0, "{elapsed}");
    assert_eq!(watchdog.title, "WATCHDOG");

    // Reattaching to a rich alternate screen gives exactly the frame the
    // last client saw.
    gate.release();
    let alternate = client.frame_titled("ALTERNATE");
    assert_eq!(alternate.screen, Screen::Alternate);
    let cells = &alternate.rows[0].cells;
    assert!(cells[0].style.bold);
    assert_eq!(cells[0].hyperlink, "https://example.test");
    assert_eq!(cells[5].width, CellWidth::Wide);
    assert_eq!(cells[8].text, "e\u{301}");
    assert_eq!(
        alternate.cursor.viewport.map(|cursor| (cursor.x, cursor.y)),
        Some((4, 2))
    );
    drop(client);
    let mut client = Peer::attach(&server.path, 4.0);
    assert_eq!(client.frame(), alternate);

    gate.release();
    client.exited(4);
    drop(client);
    server.passed(4, 10.0);
}
