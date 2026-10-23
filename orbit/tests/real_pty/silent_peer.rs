use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::path::Path;

// Bend's pending-peer bound (src/main.bend `bounds`), plus a margin for load.
const PEER_BOUND: f64 = 1.0;
const MARGIN: f64 = 2.0;

/// A connection that arrives while another peer negotiates hears Busy at
/// once and is closed (ORB-C3).
fn turned_away(path: &Path) {
    let mut crowded = Peer::connect(path, PEER_BOUND + MARGIN);
    crowded.expect(ServerMessage::Busy);
    crowded.expect_closed();
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "release");
    let shell = script(&directory, "EXIT_SHELL", "detached-exit-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_EXIT_GATE", gate.path())]);
    // While no client is attached, a peer that never sends its role holds the
    // negotiation slot only for Bend's bound, and a connection behind it is
    // turned away; a client then attaches to the same child.
    let mut silent = Peer::connect(&server.path, PEER_BOUND + MARGIN);
    sleep(0.2);
    turned_away(&server.path);
    // The silent peer was closed without an answer.
    silent.expect_closed();
    let mut client = Peer::connect(&server.path, PEER_BOUND + MARGIN);
    client.send(&ClientMessage::Hello(None));
    client.expect(ServerMessage::Attached(Default::default()));
    client.frame();
    // The same holds while a client is attached, which keeps its child.
    let mut silent = Peer::connect(&server.path, PEER_BOUND + MARGIN);
    sleep(0.2);
    turned_away(&server.path);
    silent.expect_closed();
    gate.release();
    client.exited(7);
    drop(client);

    server.passed(7, 10.0);
}
