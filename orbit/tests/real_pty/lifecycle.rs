use crate::support::*;
use orbit_protocol::session::ClientMessage;
use std::{ffi::OsStr, io::Read, path::Path, time::Instant};

/// A child that reads one byte, changes its title and directory, and exits 7.
const OBSERVED_SCRIPT: &str = r#"IFS= read -r -n 1 input; printf "\033]0;ORBIT CHANGED\007\033]7;file:///tmp/orbit\007\nINPUT:%s\n" "$input"; exit 7"#;

/// Reads and discards what arrives for half a second, as a relay client does
/// after its input ends, or until the stream ends.
fn linger(peer: &mut Peer) {
    let deadline = Instant::now() + seconds(0.5);
    let mut chunk = vec![0; 1 << 16];
    while let Some(left) = deadline.checked_duration_since(Instant::now())
        && !left.is_zero()
    {
        // macOS refuses the option once the server has closed the connection.
        if peer.stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match peer.stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break, // The deadline passed, or the stream ended.
        }
    }
}

/// Connects, sends Hello and waits for the server's first message.
fn interactive(path: &Path) -> Peer {
    let mut peer = Peer::connect(path, 8.0);
    peer.send(&ClientMessage::Hello);
    peer.receive();
    peer
}

/// Sends a paste, ends the input and lingers.
fn paste_and_leave(mut peer: Peer) {
    peer.send(&paste(b"x"));
    peer.shutdown_write();
    linger(&mut peer);
}

pub fn run() {
    // Orbit's command line: a bad one, or a command that cannot
    // start, fails with status 1 and a message, and leaves no socket pathname.
    let binary = std::env::var_os("ORBIT_BIN").expect("ORBIT_BIN");
    let usage = run_command(&[binary, "client".into()], &[], 5.0);
    assert_eq!(usage.code(), Some(1), "{:?}", usage.status);
    assert!(
        String::from_utf8_lossy(&usage.stderr)
            .lines()
            .any(|line| line.starts_with("orbit: usage: yazelix-orbit serve")),
        "{:?}",
        String::from_utf8_lossy(&usage.stderr)
    );
    let directory = TempDir::new();
    let path = directory.join("orbit.sock");
    let missing = run_command(
        &server_command(&path, &[OsStr::new("/does/not/exist")], &[]),
        &[],
        5.0,
    );
    assert_eq!(missing.code(), Some(1), "{:?}", missing.status);
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .lines()
            .any(|line| line.starts_with("orbit: ")),
        "{:?}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(!path.exists());

    // The server exits with its child's code; a child killed by a signal has
    // code 1, as in the old Orbit.
    let bash = bash();
    for (expected, text) in [(0, "exit 0"), (3, "exit 3"), (1, "kill -TERM $$")] {
        let line = server_command(
            &path,
            &[bash.as_os_str(), OsStr::new("-c"), OsStr::new(text)],
            &[],
        );
        run_command(&line, &[], 5.0).assert_silent(expected);
        assert!(!path.exists());
    }
    drop(directory);

    // A second Hello while a client is attached leaves the session running.
    let directory = TempDir::new();
    let mut server = Server::launch(
        &directory,
        &[bash.as_os_str(), OsStr::new("-c"), OsStr::new(OBSERVED_SCRIPT)],
        &[],
        &[],
        &[],
    );
    let client = interactive(&server.path);
    let mut contender = Peer::connect(&server.path, 2.0);
    contender.send(&ClientMessage::Hello);
    contender.shutdown_write();
    linger(&mut contender);
    drop(contender);
    assert!(server.alive());
    paste_and_leave(client);
    server.passed(7, 10.0);
    drop(directory);

    // One interactive client beside a metadata observer that never reads,
    // while the child changes its title a thousand times.
    let directory = TempDir::new();
    let shell = script(&directory, "PRESSURE_SHELL", "pressure-shell");
    let mut server = Server::start(&directory, &shell, &[]);
    let client = interactive(&server.path);
    let mut observer = Peer::connect(&server.path, 8.0);
    observer.send(&ClientMessage::ObserveMetadata);
    paste_and_leave(client);
    server.passed(7, 10.0);
    drop(observer);
}
