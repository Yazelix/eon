use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::{io::Write, os::unix::net::UnixStream};

pub fn run() {
    let directory = TempDir::new();
    let pid_file = directory.join("child.pid");
    let ready_gate = Gate::new(&directory, "ready");
    let gate = Gate::new(&directory, "release");
    let output_file = directory.join("output-ready");
    let shell = script(&directory, "RECONNECT_SHELL", "three-attach-shell");
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[
            ("ORBIT_PID_FILE", &pid_file),
            ("ORBIT_READY_GATE", ready_gate.path()),
            ("ORBIT_RECONNECT_GATE", gate.path()),
            ("ORBIT_OUTPUT_FILE", &output_file),
        ],
        &[&pid_file],
        &[],
    );

    let mut incomplete = Peer::connect(&server.path, 4.0);
    incomplete.send_bytes(b"ORBS");
    incomplete.shutdown_write();
    {
        let mut gone = UnixStream::connect(&server.path).expect("connect");
        gone.write_all(&encode(&ClientMessage::Hello)).expect("send");
    }
    ready_gate.release();
    incomplete.expect_closed(); // Bend discards the partial Hello.
    drop(incomplete);

    let mut incomplete = Peer::attach(&server.path, 4.0);
    incomplete.frame(); // Initial frame
    incomplete.send_bytes(b"ORBS");
    incomplete.shutdown_write();
    incomplete.drain(); // An incomplete request closes only this attachment.
    drop(incomplete);

    let pid = read_pid(&pid_file);
    let mut previous_revision = 0;
    let mut last = None;
    for attempt in 0..3 {
        assert!(alive(pid), "PTY child died before reconnect");
        let mut peer = Peer::attach(&server.path, 4.0);
        let mut current = peer.frame();
        let marker = if attempt < 2 {
            "RECONNECT UPDATED"
        } else {
            "FIRST INPUT"
        };
        if attempt == 0 {
            assert!(!mentions(&current, marker));
        } else if !mentions(&current, marker) {
            current = peer.frame_with(marker);
        }
        // Output while detached advances authority; with none since the last
        // frame, the reattach frame repeats it, revision included (ORB-C4).
        match attempt {
            0 => {}
            1 => assert!(current.revision > previous_revision),
            _ => assert_eq!(Some(&current), last.as_ref()),
        }
        previous_revision = current.revision;

        match attempt {
            0 => {
                peer.shutdown_write();
                peer.expect_closed(); // Bend processed the detach.
            }
            1 => {
                peer.send(&paste(b"x"));
                peer.expect(ServerMessage::Accepted);
                let input = peer.frame_with("FIRST INPUT");
                previous_revision = input.revision;
                last = Some(input);
                peer.shutdown_write();
                let mut contender = Peer::connect(&server.path, 4.0);
                contender.send(&ClientMessage::Hello);
                contender.expect(ServerMessage::Busy); // Still attached.
                // Closing the read side later must free the attachment.
            }
            _ => {
                peer.send(&paste(b"y"));
                peer.shutdown_write();
                peer.expect(ServerMessage::Accepted);
                // Output after the repeated frame gets a later revision.
                assert!(peer.frame().revision > previous_revision);
                peer.exited(7);
            }
        }
        drop(peer);

        if attempt == 0 {
            gate.release();
            let printed = (0..200).any(|_| {
                if output_file.exists() {
                    return true;
                }
                assert!(server.alive(), "server exited before detached output");
                sleep(0.02);
                false
            });
            assert!(printed, "shell did not print while detached");
        }
    }

    assert_eq!(server.passed(7, 10.0), pid);
}
