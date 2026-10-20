use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage};
use std::{fs, process::Command};

/// The sockets a process holds: from /proc on Linux, from lsof elsewhere.
fn socket_count(pid: i32) -> usize {
    if !cfg!(target_os = "linux") {
        let listed = Command::new("lsof")
            .args(["-a", "-U", "-p", &pid.to_string(), "-Ff"])
            .output()
            .expect("lsof");
        return String::from_utf8_lossy(&listed.stdout)
            .split_whitespace()
            .filter(|line| line.starts_with('f'))
            .count();
    }
    let descriptors = format!("/proc/{pid}/fd");
    let Ok(entries) = fs::read_dir(&descriptors) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries {
        let Ok(entry) = entry else { return 0 };
        let Ok(target) = fs::read_link(entry.path()) else {
            return 0;
        };
        count += usize::from(target.to_string_lossy().starts_with("socket:["));
    }
    count
}

/// Sockets held by the server process and its Bend worker, wherever they live.
fn server_socket_count(server: i32) -> usize {
    std::iter::once(server)
        .chain(children(server))
        .map(socket_count)
        .sum()
}

pub fn run() {
    for mode in ["never", "detach", "silent"] {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "release");
        let shell = script(&directory, "EXIT_SHELL", "detached-exit-shell");
        let mut server =
            Server::start(&directory, &shell, &[("ORBIT_EXIT_GATE", gate.path())]);
        let mut silent_peer = None;
        if mode == "detach" {
            let mut peer = Peer::connect(&server.path, 4.0);
            peer.send(&ClientMessage::Hello);
            peer.expect(ServerMessage::Attached);
            peer.frame();
            peer.shutdown_write();
            peer.expect_closed(); // Bend has detached this client.
        } else if mode == "silent" {
            let before = (0..200)
                .find_map(|_| {
                    let count = server_socket_count(server.pid());
                    if count >= 1 {
                        return Some(count); // The listener.
                    }
                    sleep(0.02);
                    None
                })
                .expect("server did not open its listener");
            silent_peer = Some(Peer::connect(&server.path, 4.0));
            assert!(
                (0..200).any(|_| {
                    if server_socket_count(server.pid()) > before {
                        return true; // The server accepted the still-silent peer.
                    }
                    sleep(0.02);
                    false
                }),
                "server did not accept the silent peer"
            );
        }

        gate.release();
        server.passed(7, 10.0);
        if let Some(mut silent_peer) = silent_peer {
            silent_peer.expect_closed();
        }
    }
}
