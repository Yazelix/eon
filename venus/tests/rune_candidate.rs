//! The private Rune candidate entry refuses to start without a Wayland display
//! (eon-yp6vg.7.7, lead ruling 2026-11-20), before any event loop exists.
#![forbid(unsafe_code)]

use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

/// Waits for `child` up to `limit`. A child still running then is killed and reaped, and
/// `None` is returned. The limit only bounds a hang; how long a correct child takes is
/// not part of any verdict.
fn wait_bounded(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            return None;
        }
        sleep(Duration::from_millis(20));
    }
}

fn read_all(pipe: Option<impl Read>) -> String {
    let mut text = String::new();
    pipe.unwrap().read_to_string(&mut text).unwrap();
    text
}

#[test]
#[cfg(target_os = "linux")]
fn the_candidate_refuses_without_wayland_even_with_an_x11_display_set() {
    // `:93` names no display here: a broken refusal must not reach a real X server.
    let mut child = Command::new(env!("CARGO_BIN_EXE_venus-rune-candidate"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env("DISPLAY", ":93")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = wait_bounded(&mut child, Duration::from_secs(60));
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let status = status.unwrap_or_else(|| panic!("the refusal hung; stderr: {stderr}"));
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("needs a native Wayland display"),
        "{stderr}"
    );
    assert!(stdout.is_empty(), "nothing was drawn: {stdout}");
}

#[test]
#[cfg(unix)]
fn a_hung_child_is_killed_and_reaped() {
    let mut child = Command::new("sleep")
        .arg("600")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    assert_eq!(wait_bounded(&mut child, Duration::from_millis(200)), None);
    assert!(
        child.try_wait().unwrap().is_some(),
        "the killed child has been reaped"
    );
}
