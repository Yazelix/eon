use crate::support::*;
use orbit_protocol::session::{ClientMessage, FocusEvent, ServerMessage, SurfaceSize};
use std::{fs, net::Shutdown};

const FOCUS_SCRIPT: &str = r#"stty -echo
printf '\033[?1004h\033]0;FOCUS READY\007'
IFS= read -r -n 3 focus
test "$focus" = $'\033[I' || exit 45
IFS= read -r _ < "$ORBIT_GATE"
printf '\033]0;AFTER FOCUS\007\n'
exit 7
"#;

const RESIZE_SCRIPT: &str = r#"stty -echo
trap 'IFS= read -r _ < "$ORBIT_GATE"; printf "\033]0;AFTER RESIZE\007\n"; exit 7' WINCH
printf '\033]0;RESIZE READY\007'
while :; do sleep 0.1; done
"#;

pub fn run() {
    for reporting in [true, false] {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "release");
        let received = directory.join("focus");
        let mode = if reporting { "h" } else { "l" };
        let shell = executable(
            &directory,
            "focus-out.sh",
            &format!(
                "#!{}\nstty -echo -icanon\nprintf '\\033[?1004{mode}\\033]0;FOCUS OUT READY\\007'\n\
                 focus=''\nIFS= read -r -t 8 -N 3 focus || :\n\
                 printf '%s' \"$focus\" >\"$ORBIT_RECEIVED.tmp\"\n\
                 mv \"$ORBIT_RECEIVED.tmp\" \"$ORBIT_RECEIVED\"\n\
                 IFS= read -r _ <\"$ORBIT_GATE\"\nexit 7\n",
                bash().display()
            ),
        );
        let mut server = Server::start(
            &directory,
            &shell,
            &[("ORBIT_GATE", gate.path()), ("ORBIT_RECEIVED", &received)],
        );
        let mut peer = Peer::attach(&server.path, 4.0);
        peer.frame_with("FOCUS OUT READY");
        // Both requests and the full close must precede Orbit's first reply attempt.
        signal(server.pid(), libc::SIGSTOP);
        let mut status = 0;
        // SAFETY: this is our child; the status pointer is valid and WUNTRACED waits for its stop.
        assert_eq!(
            unsafe { libc::waitpid(server.pid(), &mut status, libc::WUNTRACED) },
            server.pid()
        );
        assert!(libc::WIFSTOPPED(status));
        peer.send(&ClientMessage::ReturnToLive);
        peer.send(&ClientMessage::Focus(FocusEvent::Lost));
        peer.stream.shutdown(Shutdown::Both).unwrap();
        drop(peer);
        signal(server.pid(), libc::SIGCONT);
        wait_for(&received);
        assert_eq!(
            fs::read(&received).unwrap(),
            if reporting { &b"\x1b[O"[..] } else { b"" },
            "focus-out before full detach, reporting={reporting}"
        );
        let reopened = Peer::attach(&server.path, 4.0);
        drop(reopened);
        gate.release();
        server.passed(7, 10.0);
    }

    let resize = ClientMessage::Resize(SurfaceSize {
        cols: 101,
        rows: 30,
        screen_width: 1010,
        screen_height: 540,
        cell_width: 10,
        cell_height: 18,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    });
    let cases = [
        ("FOCUS", ClientMessage::Focus(FocusEvent::Gained), FOCUS_SCRIPT),
        ("RESIZE", resize, RESIZE_SCRIPT),
    ];
    for (name, request, text) in cases {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "release");
        let shell = executable(
            &directory,
            "test-shell.sh",
            &format!("#!{}\n{text}", bash().display()),
        );
        let mut server = Server::start(&directory, &shell, &[("ORBIT_GATE", gate.path())]);
        let mut peer = Peer::attach(&server.path, 4.0);
        peer.frame_with(&format!("{name} READY"));

        peer.send(&request);
        peer.expect(ServerMessage::Accepted);
        if name == "RESIZE" {
            peer.frame();
        }
        peer.shutdown_write();
        peer.expect_open(0.2, &format!("{name} detached early"));
        gate.release();
        let mut saw_output = false;
        loop {
            match peer.receive() {
                ServerMessage::Exited { code } => {
                    assert_eq!(code, 7);
                    break;
                }
                ServerMessage::Frame(frame) => {
                    saw_output |= mentions(&frame, &format!("AFTER {name}"));
                }
                other => panic!("expected a frame or Exited, received {}", brief(&other)),
            }
        }
        assert!(saw_output, "{name}");
        drop(peer);

        server.passed(7, 10.0);
    }
}
