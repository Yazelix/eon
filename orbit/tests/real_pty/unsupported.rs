use crate::support::*;
use orbit_protocol::session::{ClientMessage, FailureCode, FocusEvent, ServerMessage};
use std::path::Path;

/// A canonical message sent with another ORBS version.
fn with_version(message: &ClientMessage, version: u16) -> Vec<u8> {
    let mut bytes = encode(message);
    bytes[4..6].copy_from_slice(&version.to_le_bytes());
    bytes
}

fn first_message(path: &Path, data: &[u8]) -> Peer {
    let mut peer = Peer::connect(path, 4.0);
    peer.send_bytes(data);
    peer
}

/// Bad first messages end only their peer.
fn refusals(path: &Path) {
    // An unsupported version is dropped silently.
    first_message(path, &with_version(&ClientMessage::Hello, 14)).expect_closed();
    let mut magic = encode(&ClientMessage::Hello);
    magic[..4].copy_from_slice(b"ORBX");
    let mut peer = first_message(path, &magic);
    peer.failure(FailureCode::Protocol);
    peer.expect_closed();
    // Focus before a role.
    let mut peer = first_message(path, &encode(&ClientMessage::Focus(FocusEvent::Gained)));
    peer.failure(FailureCode::Protocol);
    peer.expect_closed();
}

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let shell = script(&directory, "UNSUPPORTED_SHELL", "unsupported-shell");
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[("ORBIT_PID_FILE", &pid_file)],
        &[&pid_file],
        &[],
    );
    let pid = read_pid(&pid_file);

    refusals(&server.path);

    let mut first = Peer::attach(&server.path, 4.0);
    first.frame_with("");
    refusals(&server.path); // The same answers while a client is attached.
    // A request that fails keeps the client.
    first.send(&idle_tick());
    first.failure(FailureCode::InvalidInput);
    first.send(&paste(b"a")); // Paste reaches the same child.
    first.expect(ServerMessage::Accepted);
    first.frame_with("FIRST a");
    // A repeated role fails as a protocol error and ends only this attachment.
    first.send(&ClientMessage::Hello);
    first.failure(FailureCode::Protocol);
    first.expect_closed();
    drop(first);

    // A request that fails to decode ends only its attachment.
    // The encoder refuses zero scroll rows, so the check zeroes
    // the rows of a canonical scroll.
    let mut malformed = Peer::attach(&server.path, 4.0);
    malformed.frame_with("FIRST a");
    let mut zero_rows = encode(&scroll(1, 1));
    let length = zero_rows.len();
    zero_rows[length - 2..].copy_from_slice(&0i16.to_le_bytes());
    malformed.send_bytes(&zero_rows);
    malformed.failure(FailureCode::Protocol);
    malformed.expect_closed();
    drop(malformed);
    let mut changed = Peer::attach(&server.path, 4.0);
    changed.frame_with("FIRST a");
    changed.send_bytes(&with_version(&ClientMessage::ReturnToLive, 14));
    changed.expect_closed(); // A changed version is closed silently.
    drop(changed);

    let mut second = Peer::attach(&server.path, 4.0);
    second.frame_with("FIRST a");
    assert!(alive(pid));
    second.send(&paste(b"b"));
    second.expect(ServerMessage::Accepted);
    second.exited(7);
    drop(second);

    assert_eq!(server.passed(7, 10.0), pid);
}
