use crate::support::*;
use orbit_protocol::session::{ClientMessage, FailureCode, ServerMessage};
use std::time::Instant;

// Each idle tick queues one Failure of about 84 bytes; this many leave the
// stalled client's queue between the room a frame needs (about 8.4 MB) and
// the bound (about 12.6 MB) at which Bend detaches it.
const STALLED_REQUESTS: usize = 120_000;

/// Returns revision, title and rows_from_live of the next message, a frame.
fn frame(peer: &mut Peer) -> (u64, String, u64) {
    let current = peer.frame();
    (
        current.revision,
        current.title,
        current.scroll_position.rows_from_live,
    )
}

fn frame_titled(peer: &mut Peer, title: &str) -> (u64, u64) {
    let current = peer.frame_titled(title);
    (current.revision, current.scroll_position.rows_from_live)
}

/// Returns requested, applied, frame revision and rows_from_live.
fn outcome(peer: &mut Peer) -> (i16, i16, u64, u64) {
    let (requested, applied, frame) = peer.scroll_outcome();
    (
        requested,
        applied,
        frame.revision,
        frame.scroll_position.rows_from_live,
    )
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "live-gate");
    let pasted = directory.join("pasted");
    let shell = script(&directory, "LIVE_SHELL", "return-to-live-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_LIVE_GATE", gate.path()),
            ("ORBIT_LIVE_PASTED", &pasted),
        ],
    );
    let mut peer = Peer::attach(&server.path, 10.0);
    peer.receive();
    gate.release();
    let (mut revision, rows_from_live) = frame_titled(&mut peer, "LIVE READY");
    assert_eq!(rows_from_live, 0);

    // Two bounded commits reach beyond the 1,024-row relative limit.
    peer.send(&scroll(revision, -1024));
    assert_eq!(outcome(&mut peer), (-1024, -1024, revision + 1, 1024));
    peer.send(&scroll(revision + 1, -1024));
    let (_, applied, published, deep) = outcome(&mut peer);
    revision = published;
    assert!(applied < 0 && deep == (1024 - i64::from(applied)) as u64 && deep > 1024, "{deep}");

    // One return reaches live output from any depth and publishes one
    // complete frame at the next revision.
    peer.send(&ClientMessage::ReturnToLive);
    assert_eq!(frame(&mut peer), (revision + 1, "LIVE READY".into(), 0));
    peer.quiet(0.2, "a return to live published twice");
    revision += 1;

    // Synchronized output defers one return until release; a second request
    // fails. Quiet windows stay short so the held section ends well before
    // the one-second watchdog.
    peer.send(&scroll(revision, -10));
    assert_eq!(outcome(&mut peer), (-10, -10, revision + 1, 10));
    gate.release();
    peer.quiet(0.15, "held output");
    peer.send(&ClientMessage::ReturnToLive);
    peer.quiet(0.15, "a deferred return to live");
    peer.send(&ClientMessage::ReturnToLive);
    peer.failure(FailureCode::InvalidInput);
    gate.release();
    loop {
        let (published, _, rows_from_live) = frame(&mut peer);
        assert!(published > revision);
        revision = published;
        if rows_from_live == 0 {
            break;
        }
    }
    peer.quiet(0.2, "a resolved return to live published twice");

    // Mouse tracking owns scrolling: the return fails and moves nothing.
    peer.send(&scroll(revision, -8));
    assert_eq!(outcome(&mut peer), (-8, -8, revision + 1, 8));
    gate.release();
    let (_, rows_from_live) = frame_titled(&mut peer, "LIVE ROUTED");
    assert_eq!(rows_from_live, 8);
    peer.send(&ClientMessage::ReturnToLive);
    peer.failure(FailureCode::InvalidInput);
    peer.quiet(0.2, "a terminal-owned return to live published");

    // So does the alternate screen, even without wheel routing.
    gate.release();
    frame_titled(&mut peer, "LIVE ALTERNATE");
    peer.send(&ClientMessage::ReturnToLive);
    peer.failure(FailureCode::InvalidInput);
    peer.quiet(0.2, "an alternate-screen return to live published");

    gate.release();
    let (primary, rows_from_live) = frame_titled(&mut peer, "LIVE PRIMARY");
    revision = primary;
    assert_eq!(rows_from_live, 8);
    peer.send(&scroll(revision, -20));
    assert_eq!(outcome(&mut peer), (-20, -20, revision + 1, 28));
    revision += 1;

    // A client that stops reading lets its queue fill past the room a frame
    // needs: the return then fails with Failure(Terminal) before the viewport
    // moves, and the client stays attached. The server reads the flood as
    // fast as it answers each request, which under load takes longer than the
    // usual ten seconds.
    peer.set_timeout(60.0);
    let mut flood = encode(&idle_tick()).repeat(STALLED_REQUESTS);
    flood.extend(encode(&ClientMessage::ReturnToLive));
    flood.extend(encode(&paste(b"x")));
    peer.send_bytes(&flood);
    peer.set_timeout(10.0);
    let deadline = Instant::now() + seconds(60.0);
    while !pasted.exists() {
        assert!(Instant::now() < deadline, "the paste never reached the child");
        assert!(server.alive(), "server exited under pressure");
        sleep(0.05);
    }
    for _ in 0..STALLED_REQUESTS {
        peer.failure(FailureCode::InvalidInput);
    }
    peer.failure(FailureCode::Terminal);
    peer.expect(ServerMessage::Accepted);
    peer.send(&scroll(revision, -1));
    assert_eq!(outcome(&mut peer), (-1, -1, revision + 1, 29));
    peer.send(&ClientMessage::ReturnToLive);
    assert_eq!(frame(&mut peer), (revision + 2, "LIVE PRIMARY".into(), 0));

    gate.release();
    peer.exited(7);
    drop(peer);

    server.passed(7, 10.0);
}
