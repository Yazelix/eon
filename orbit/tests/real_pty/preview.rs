use crate::support::*;
use orbit_protocol::session::{
    FailureCode, PreviewOutcome, ServerMessage,
    VerticalDirection::{self, Down, Up},
};

/// Returns revision, direction and None (terminal routed) or (cols, edge, rows).
fn vertical_preview(peer: &mut Peer) -> (u64, VerticalDirection, Option<(u16, bool, usize)>) {
    match peer.receive() {
        ServerMessage::VerticalPreview(preview) => (
            preview.frame_revision,
            preview.direction,
            match preview.outcome {
                PreviewOutcome::TerminalRouted => None,
                PreviewOutcome::Viewport {
                    cols,
                    edge_reached,
                    rows,
                } => Some((cols, edge_reached, rows.len())),
            },
        ),
        other => panic!("expected a vertical preview, received {}", brief(&other)),
    }
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "preview-gate");
    let shell = script(&directory, "PREVIEW_SHELL", "preview-shell");
    let mut server =
        Server::start(&directory, &shell, &[("ORBIT_PREVIEW_GATE", gate.path())]);
    let mut peer = Peer::attach(&server.path, 10.0);
    peer.frame();
    gate.release();
    let ready = peer.frame_titled("PREVIEW READY");
    let revision = ready.revision;
    assert_eq!(ready.scroll_position.rows_from_live, 0);

    // Up previews one full viewport of older rows; down is already at the live edge.
    peer.send(&preview(revision, Up));
    assert_eq!(vertical_preview(&mut peer), (revision, Up, Some((80, false, 24))));
    peer.send(&preview(revision, Down));
    assert_eq!(vertical_preview(&mut peer), (revision, Down, Some((80, true, 0))));
    peer.send(&preview(revision - 1, Up)); // A lagging presented revision resolves.
    assert_eq!(vertical_preview(&mut peer), (revision, Up, Some((80, false, 24))));
    peer.send(&preview(revision + 1, Up)); // A future revision fails without change.
    peer.failure(FailureCode::InvalidInput);
    peer.quiet(0.3, "a preview published a frame or other message");

    // Synchronized output defers one preview until release; a second one
    // fails. Quiet windows stay short so the held section ends well before
    // the one-second watchdog.
    gate.release();
    peer.quiet(0.15, "held output");
    peer.send(&preview(revision, Up));
    peer.quiet(0.15, "a deferred preview");
    peer.send(&preview(revision, Down));
    peer.failure(FailureCode::InvalidInput);
    // Mouse tracking enabled inside the held update routes a new request at
    // once, while the deferred one keeps waiting. The update's first output
    // advanced authority, though its frame is held.
    gate.release();
    peer.quiet(0.15, "held output");
    peer.send(&preview(revision, Down));
    assert_eq!(vertical_preview(&mut peer), (revision + 1, Down, None));
    peer.quiet(0.15, "the deferred preview");
    // Release publishes one frame, then resolves the deferred preview with
    // routing evaluated again.
    gate.release();
    let held = peer.frame_titled("PREVIEW HELD");
    let held_revision = held.revision;
    assert!(held_revision == revision + 1 && held.scroll_position.rows_from_live == 0);
    assert_eq!(vertical_preview(&mut peer), (held_revision, Up, None));

    // Mouse tracking keeps routing the wheel to the terminal.
    gate.release();
    let routed_revision = peer.frame_titled("PREVIEW ROUTED").revision;
    peer.send(&preview(routed_revision, Up));
    assert_eq!(vertical_preview(&mut peer), (routed_revision, Up, None));

    gate.release();
    peer.exited(7);
    drop(peer);

    server.passed(7, 10.0);
}
