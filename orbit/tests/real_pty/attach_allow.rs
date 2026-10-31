use crate::management_lease::{Manager, Reply, leased as lease, own_name};
use crate::management_record::suffixed;
use crate::support::*;
use orbit_protocol::{
    management,
    session::{ClientMessage, KeyAction, ServerMessage},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

const MANAGED: [&str; 4] = ["--management-v4", "session-4", "run-4", "component-4"];

/// A child that appends each line it reads to `$ORBIT_LOG`, so a pasted word
/// and Enter that reach it leave a line there.
fn logging_run(directory: &Path, options: &[&str]) -> (Server, PathBuf) {
    let log = directory.join("log");
    let shell = executable(
        directory,
        "child.sh",
        "#!/bin/sh\nstty -echo\n\
         while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$ORBIT_LOG\"; done\n",
    );
    let server = Server::launch(
        directory,
        &[shell.as_os_str()],
        &[("ORBIT_LOG", &log)],
        &[],
        options,
    );
    (server, log)
}

fn logged(log: &Path) -> String {
    fs::read_to_string(log).unwrap_or_default()
}

/// Waits up to five seconds for the log to hold exactly these lines.
fn until_logged(log: &Path, lines: &[&str]) {
    let expected: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let deadline = Instant::now() + seconds(5.0);
    while logged(log) != expected {
        assert!(
            Instant::now() < deadline,
            "log {:?}, expected {expected:?}",
            logged(log)
        );
        sleep(0.02);
    }
}

/// Pastes a word and presses Enter as an attached client.
fn type_line(peer: &mut Peer, word: &str) {
    for message in [
        paste(word.as_bytes()),
        enter(KeyAction::Press),
        enter(KeyAction::Release),
    ] {
        peer.send(&message);
    }
}

/// eon-ahzkt's probe: a peer sends Hello and, once attached, pastes `word`
/// and presses Enter. Returns the answer and the peer.
fn probe(path: &Path, word: &str) -> (ServerMessage, Peer) {
    let mut peer = Peer::connect(path, 4.0);
    peer.send(&ClientMessage::Hello);
    let answer = peer.receive();
    if answer == ServerMessage::Attached {
        type_line(&mut peer, word);
    }
    (answer, peer)
}

/// A peer this run must turn away: its Hello is answered Busy and the
/// connection closes, so the word it would paste reaches no shell (ORB-C3).
fn refused(path: &Path, log: &Path, word: &str, when: &str) {
    let (answer, mut peer) = probe(path, word);
    if answer != ServerMessage::Busy {
        let deadline = Instant::now() + seconds(2.0);
        while !logged(log).contains(word) && Instant::now() < deadline {
            sleep(0.02);
        }
        panic!(
            "{when}: a Hello was answered {} and its paste {} the shell",
            brief(&answer),
            if logged(log).contains(word) {
                "reached"
            } else {
                "did not reach"
            }
        );
    }
    let (data, _) = peer.read_until_closed(true);
    assert!(data.is_empty(), "{when}: data after Busy");
}

pub fn run() {
    // Without management, the first peer is admitted and its paste reaches
    // the shell, as eon-ahzkt's probe found.
    {
        let directory = TempDir::new();
        let (mut server, log) = logging_run(&directory, &[]);
        let (answer, peer) = probe(&server.path, "first-come");
        assert_eq!(
            answer,
            ServerMessage::Attached,
            "an unmanaged run turned its first peer away"
        );
        until_logged(&log, &["first-come"]);
        drop(peer);
        signal(server.pid(), libc::SIGTERM);
        server.communicate(10.0).assert_silent(0);
    }

    // With management, only the process the current lease holder named is
    // admitted, matched by its process ID and start identity.
    let directory = TempDir::new();
    let (mut server, log) = logging_run(&directory, &MANAGED);
    let record = suffixed(&server.path, ".record");
    wait_for(&record);
    let path = server.path.clone();
    refused(&path, &log, "no-lease", "before any lease");

    let mut first = lease(&record);
    refused(
        &path,
        &log,
        "unnamed",
        "under a lease that named no process",
    );
    let (own_pid, own_start) = own_name();
    let allowed = Reply::Message(management::ServerMessage::Allowed);
    assert_eq!(first.allow(own_pid, own_start + 1), allowed);
    refused(
        &path,
        &log,
        "other-start",
        "with this process ID under another start identity",
    );
    let child = server.child as u32;
    assert_eq!(first.allow(child, process_start(child)), allowed);
    refused(&path, &log, "other-process", "with another process named");

    // The name belongs to the lease that set it.
    first.allow_self();
    drop(first);
    // Orbit has seen that lease end once a new management connection is
    // admitted to negotiate rather than answered Busy; it holds no lease.
    let deadline = Instant::now() + seconds(2.0);
    let negotiating = loop {
        let mut next = Manager::new(&record);
        next.connect();
        next.set_timeout(0.2);
        match next.read() {
            Reply::Timeout => break next,
            Reply::Message(management::ServerMessage::Busy) if Instant::now() < deadline => {
                sleep(0.02);
            }
            other => panic!("expected a negotiating connection, received {other:?}"),
        }
    };
    refused(
        &path,
        &log,
        "ended-lease",
        "with no lease after the naming one ended",
    );
    drop(negotiating);
    let mut second = lease(&record);
    refused(
        &path,
        &log,
        "earlier-lease",
        "under a new lease after the naming one ended",
    );

    // The named process attaches, and renaming leaves its attachment alone.
    second.allow_self();
    let mut client = Peer::attach(&path, 4.0);
    client.frame();
    type_line(&mut client, "named");
    until_logged(&log, &["named"]);
    assert_eq!(second.allow(own_pid, own_start + 1), allowed);
    type_line(&mut client, "renamed");
    until_logged(&log, &["named", "renamed"]);

    match second.send(&management::ClientMessage::Stop) {
        Reply::Message(management::ServerMessage::Stopped(_)) => {}
        other => panic!("expected Stopped, received {other:?}"),
    }
    drop(second);
    drop(client);
    server.communicate(10.0).assert_silent(0);
    // No refused peer's word reached the shell.
    assert_eq!(logged(&log), "named\nrenamed\n");
}
