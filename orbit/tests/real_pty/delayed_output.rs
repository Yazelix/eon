use crate::support::*;
use orbit_protocol::session::{ClientMessage, FocusEvent, ServerMessage};

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "release");
    let shell = script(&directory, "DELAY_SHELL", "delayed-output-shell");
    let mut server = Server::start(&directory, &shell, &[("ORBIT_DELAY_GATE", gate.path())]);
    let mut peer = Peer::attach(&server.path, 4.0);
    let initial = peer.frame();

    gate.release();
    // No client request has been sent; the frame from before the gate may
    // still come first.
    let delayed = peer.frame_titled("DELAYED");
    assert!(delayed.revision > initial.revision);

    peer.send(&ClientMessage::Focus(FocusEvent::Gained));
    peer.expect(ServerMessage::Accepted); // Enabled Focus writes no child output.
    peer.send(&paste(b"x"));
    peer.expect(ServerMessage::Accepted);
    peer.frame_with("FIRST INPUT");

    for data in [b"y", b"z"] {
        peer.send(&paste(data));
    }
    peer.shutdown_write();
    for _ in 0..2 {
        loop {
            match peer.receive() {
                ServerMessage::Accepted => break,
                ServerMessage::Frame(_) => {} // Output from x may still be queued.
                other => panic!("expected Accepted or a frame, received {}", brief(&other)),
            }
        }
    }
    let mut saw_final = false;
    loop {
        match peer.receive() {
            ServerMessage::Exited { code } => {
                assert_eq!(code, 7);
                break;
            }
            ServerMessage::Frame(frame) => saw_final |= mentions(&frame, "FINAL INPUT"),
            other => panic!("expected a frame or Exited, received {}", brief(&other)),
        }
    }
    assert!(saw_final);
    drop(peer);

    server.passed(7, 10.0);
}
