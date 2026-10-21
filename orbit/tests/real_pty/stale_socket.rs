use crate::support::*;
use std::{
    fs::{self, File},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
};

/// Leaves the pathname a server killed without cleanup leaves: a socket with
/// this mode that nobody listens on.
fn leave_socket(path: &Path, mode: u32) {
    let stale = UnixListener::bind(path).expect("bind");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    drop(stale);
}

/// Starts a server on the occupied pathname; it must refuse with a message
/// that contains `expected`.
fn refused(directory: &Path, path: &Path, expected: &str) {
    let shell = script(directory, "STALE_SHELL", "stale-socket-shell");
    let pid_file = directory.join("refused.pid");
    let gate = directory.join("unused-gate");
    let result = run_command(
        &server_command(path, &[shell.as_os_str()], &[]),
        &[("ORBIT_PID_FILE", &pid_file), ("ORBIT_STALE_GATE", &gate)],
        10.0,
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.starts_with("orbit: ") && stderr.contains(expected),
        "{stderr:?}"
    );
    assert!(!result.status.success(), "{:?}", result.status);
}

fn stale() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "stale-gate");
    let pid_file = directory.join("child.pid");
    let shell = script(&directory, "STALE_SHELL", "stale-socket-shell");
    leave_socket(&directory.join("orbit.sock"), 0o600);

    // A stale socket pathname no longer stops the next start (ORB-C1).
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[
            ("ORBIT_PID_FILE", &pid_file),
            ("ORBIT_STALE_GATE", gate.path()),
        ],
        &[&pid_file],
        &[],
    );
    let mut client = Peer::attach(&server.path, 4.0);
    client.frame();

    // A live server's pathname is refused with these messages, and
    // that server keeps it: one while another start holds the claim, one
    // once the claim is free.
    let claim = File::open(server.path.parent().expect("parent")).expect("open parent");
    claim.lock().expect("lock parent");
    refused(
        &directory,
        &server.path,
        &format!(
            "socket claim already in progress at {}",
            server.path.display()
        ),
    );
    drop(claim);
    refused(
        &directory,
        &server.path,
        &format!("server already listening at {}", server.path.display()),
    );
    assert!(server.path.exists());
    gate.release();
    client.exited(7);
    drop(client);

    server.passed(7, 10.0);
}

pub fn run() {
    stale();

    let directory = TempDir::new();
    // A pathname that is not a private socket is refused and left alone.
    let path = directory.join("orbit.sock");
    fs::write(&path, "not a socket\n").expect("write");
    refused(&directory, &path, "");
    assert_eq!(fs::read_to_string(&path).expect("read"), "not a socket\n");
    fs::remove_file(&path).expect("remove");
    leave_socket(&path, 0o644);
    refused(&directory, &path, "");
    assert!(path.exists());
}
