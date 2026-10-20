use crate::support::*;
use orbit_protocol::session::{
    FailureCode, PreviewOutcome, ScrollOutcome, ServerMessage,
    VerticalDirection::{self, Down},
};

/// Requested and applied rows, frame revision, rows_from_live and the next
/// window's (edge, rows).
type Outcome = (i16, i16, u64, u64, (bool, usize));

/// The next window's edge and row count, from an 80-column viewport.
fn window(next: &PreviewOutcome) -> (bool, usize) {
    match next {
        PreviewOutcome::Viewport {
            cols,
            edge_reached,
            rows,
        } => {
            assert_eq!(*cols, 80);
            (*edge_reached, rows.len())
        }
        PreviewOutcome::TerminalRouted => panic!("a terminal-routed window"),
    }
}

fn parse_outcome(message: ServerMessage) -> Outcome {
    match message {
        ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows,
            applied_rows,
            frame,
            next,
        }) => (
            requested_rows,
            applied_rows,
            frame.revision,
            frame.scroll_position.rows_from_live,
            window(&next),
        ),
        other => panic!("expected a scroll outcome, received {}", brief(&other)),
    }
}

fn outcome(peer: &mut Peer) -> Outcome {
    parse_outcome(peer.receive())
}

fn vertical_preview(peer: &mut Peer) -> (u64, VerticalDirection, (bool, usize)) {
    match peer.receive() {
        ServerMessage::VerticalPreview(preview) => (
            preview.frame_revision,
            preview.direction,
            window(&preview.outcome),
        ),
        other => panic!("expected a vertical preview, received {}", brief(&other)),
    }
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "scroll-gate");
    let shell = script(&directory, "SCROLL_SHELL", "scroll-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_SCROLL_GATE", gate.path())]);
    let mut peer = Peer::attach(&server.path, 10.0);
    peer.receive();
    gate.release();
    let ready = peer.frame_titled("SCROLL READY");
    let mut revision = ready.revision;
    assert_eq!(ready.scroll_position.rows_from_live, 0);

    // One commit moves the authoritative viewport, advances the revision once
    // and carries the frame and the next window in its direction.
    peer.send(&scroll(revision, -10));
    assert_eq!(outcome(&mut peer), (-10, -10, revision + 1, 10, (false, 24)));
    peer.quiet(0.2, "a scroll published a separate frame");
    // A lagging presented revision resolves against current authority.
    peer.send(&scroll(revision, 4));
    assert_eq!(outcome(&mut peer), (4, 4, revision + 2, 6, (true, 6)));
    revision += 2;

    // Movement clamps at both history edges.
    peer.send(&scroll(revision, -1024));
    let (requested, applied, published, top, next) = outcome(&mut peer);
    revision = published;
    assert!(requested == -1024 && -1024 < applied && applied < 0);
    assert_eq!(top, (6 - i64::from(applied)) as u64);
    assert_eq!(next, (true, 0));
    peer.send(&scroll(revision, -5));
    assert_eq!(outcome(&mut peer), (-5, 0, revision + 1, top, (true, 0)));
    revision += 1;
    peer.send(&scroll(revision, 1024));
    assert_eq!(outcome(&mut peer), (1024, top as i16, revision + 1, 0, (true, 0)));
    revision += 1;

    // A future revision fails before any change.
    peer.send(&scroll(revision + 1, -3));
    peer.failure(FailureCode::InvalidInput);
    peer.quiet(0.2, "a future scroll changed the viewport");

    // Synchronized output defers one commit until release; a second one
    // fails. Quiet windows stay short so the held section ends well before
    // the one-second watchdog.
    gate.release();
    peer.quiet(0.15, "held output");
    peer.send(&scroll(revision, -3));
    peer.quiet(0.15, "a deferred scroll");
    peer.send(&scroll(revision, -2));
    peer.failure(FailureCode::InvalidInput);
    gate.release();
    let mut frame_published = None;
    let (requested, applied, rows_from_live) = loop {
        match peer.receive() {
            ServerMessage::Frame(frame) => frame_published = Some(frame.revision),
            message => {
                let (requested, applied, published, rows_from_live, _) = parse_outcome(message);
                revision = published;
                break (requested, applied, rows_from_live);
            }
        }
    };
    assert_eq!((requested, applied, rows_from_live), (-3, -3, 3));
    assert!(frame_published.is_none_or(|published| published < revision));

    // Mouse tracking owns the wheel: the terminal-owned answer moves nothing.
    gate.release();
    revision = peer.frame_titled("SCROLL ROUTED").revision;
    peer.send(&scroll(revision, -3));
    peer.expect(ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned {
        requested_rows: -3,
    }));
    peer.quiet(0.2, "a terminal-owned scroll changed the viewport");

    gate.release();
    let free = peer.frame_titled("SCROLL FREE");
    revision = free.revision;
    assert_eq!(free.scroll_position.rows_from_live, 3);

    // With the client not reading, queued scroll outcomes, which nothing
    // replaces, fill the room the next outcome needs; from then on each
    // commit fails before the viewport moves.
    let requested = 150;
    peer.send_bytes(&encode(&scroll(revision, -1)).repeat(requested));
    sleep(1.5); // The server answers every commit before the client reads.
    let mut moved = 0;
    loop {
        match peer.receive() {
            ServerMessage::Failure(failure) => {
                assert_eq!(failure.code, FailureCode::Terminal);
                break;
            }
            message => {
                moved += 1;
                let (_, applied, published, rows_from_live, _) = parse_outcome(message);
                assert_eq!(
                    (applied, published, rows_from_live),
                    (-1, revision + moved as u64, 3 + moved as u64)
                );
            }
        }
    }
    assert!(0 < moved && moved < requested - 1, "{moved}");
    for _ in 0..requested - moved - 1 {
        peer.failure(FailureCode::Terminal);
    }
    revision += moved as u64;
    peer.send(&preview(revision, Down));
    let (previewed, direction, _) = vertical_preview(&mut peer);
    assert_eq!((previewed, direction), (revision, Down));
    peer.send(&scroll(revision, -1));
    let (requested, applied, published, rows_from_live, _) = outcome(&mut peer);
    assert_eq!(
        (requested, applied, published, rows_from_live),
        (-1, -1, revision + 1, 3 + moved as u64 + 1)
    );

    peer.send(&paste(b"x"));
    peer.expect(ServerMessage::Accepted);
    peer.exited(7);
    drop(peer);

    server.passed(7, 10.0);
}
