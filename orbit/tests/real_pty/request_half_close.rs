use crate::support::*;
use orbit_protocol::session::{ClientMessage, FocusEvent, ServerMessage, SurfaceSize};

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
