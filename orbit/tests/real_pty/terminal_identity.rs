use crate::support::*;
use orbit_protocol::Frame;
use std::{env, fs, path::PathBuf};

/// The package's compiled terminfo, `share/terminfo` beside the `bin` that
/// holds `ORBIT_BIN`.
fn package_terminfo() -> PathBuf {
    let binary = PathBuf::from(env::var_os("ORBIT_BIN").expect("ORBIT_BIN"));
    let terminfo = binary
        .parent()
        .and_then(|bin| bin.parent())
        .expect("ORBIT_BIN has a package directory")
        .join("share/terminfo");
    assert!(
        terminfo.is_dir(),
        "the package's terminfo is missing at {}",
        terminfo.display()
    );
    terminfo
}

fn row_text(frame: &Frame, index: usize) -> String {
    frame.rows[index]
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_string()
}

pub fn run() {
    let terminfo = package_terminfo();
    let directory = TempDir::new();
    // Only the package's entry may resolve `eon`: no home or TERMINFO
    // database supplies one.
    let empty = directory.join("no-terminfo");
    fs::create_dir(&empty).expect("empty terminfo directory");
    let gate = Gate::new(&directory, "identity-gate");
    let shell = script(&directory, "IDENTITY_SHELL", "terminal-identity-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_IDENTITY_GATE", gate.path()),
            ("TERMINFO_DIRS", &terminfo),
            ("TERMINFO", &empty),
            ("HOME", &directory),
        ],
    );
    let mut peer = Peer::attach(&server.path, 10.0);

    // The child sees Orbit's terminal identity, and only the package's
    // terminfo directory.
    let identity = format!(
        "TERM=eon COLORTERM=truecolor TERMINFO_DIRS={}",
        terminfo.display()
    );
    let before = loop {
        let frame = peer.frame_titled(&identity);
        if (0..frame.rows.len()).any(|row| row_text(&frame, row) == "BEFORE CLEAR 3") {
            break frame;
        }
    };
    assert_eq!(row_text(&before, 0), "BEFORE CLEAR 1");

    // `clear` finds the entry, exits 0, and the next frame is blank but for
    // what the child printed after it.
    gate.release();
    let cleared = peer.frame_titled("CLEARED");
    assert_eq!(row_text(&cleared, 0), "CLEARED 0");
    for row in 1..cleared.rows.len() {
        assert_eq!(row_text(&cleared, row), "", "row {row} after clear");
    }

    gate.release();
    peer.exited(0);
    drop(peer);
    server.passed(0, 10.0);
}
