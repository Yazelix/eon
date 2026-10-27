use crate::support::*;
use orbit_protocol::session::{ClientMessage, ServerMessage, SurfaceSize};
use std::{
    fs,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
};

// The child names its terminal, then asks for the text area, cell and
// character sizes (CSI 14t, 16t, 18t and 19t) and turns on in-band size
// reports (mode 2048): before any client size, after one, and after it
// switches itself to 132 columns (DECCOLM). A title follows the first round
// and the last, after mode 2048 is off again.
const SIZE_SCRIPT: &str = r#"stty -echo -icanon
tty > "$ORBIT_TTY"
# Asks the terminal one question and records its answer, or TIMEOUT.
query() {
  printf '%s' "$2"
  if IFS= read -r -s -t 5 -d t reply; then
    printf '%s %q\n' "$1" "$reply" >> "$ORBIT_REPLIES"
  else
    printf '%s TIMEOUT\n' "$1" >> "$ORBIT_REPLIES"
  fi
}
asks() {
  query "$1-text" $'\033[14t'
  query "$1-cell" $'\033[16t'
  query "$1-chars" $'\033[18t'
  query "$1-screen" $'\033[19t'
  query "$1-inband" $'\033[?2048h'
  printf '\033[?2048l'
}
asks start
printf '\033]0;STARTED\007'
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
asks sized
printf '\033[?40h\033[?3h'
asks switched
printf '\033]0;ASKED\007'
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
exit 7
"#;

/// The PTY's size as its child reads it: rows, columns and pixel fields.
fn window_size(terminal: &Path) -> [u16; 4] {
    let opened = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY)
        .open(terminal)
        .expect("open the child's terminal");
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes one winsize into `size`.
    let read = unsafe { libc::ioctl(opened.as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    assert_eq!(read, 0, "TIOCGWINSZ");
    [size.ws_row, size.ws_col, size.ws_xpixel, size.ws_ypixel]
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "gate");
    let tty = directory.join("tty");
    let answers = directory.join("replies");
    let shell = executable(
        &directory,
        "size-shell.sh",
        &format!("#!{}\n{SIZE_SCRIPT}", bash().display()),
    );
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_GATE", gate.path()),
            ("ORBIT_TTY", &tty),
            ("ORBIT_REPLIES", &answers),
        ],
    );
    // The client attaches without a size; the title shows that Orbit has
    // read the round's last request, which turns mode 2048 off.
    let mut client = Peer::attach(&server.path, 10.0);
    client.frame_titled("STARTED");
    let got = replies(&answers);
    // Before any client size, the session is 80x24 and no cell size is
    // known: the pixel fields and answers hold zero pixels.
    let terminal = fs::read_to_string(&tty).expect("tty");
    let terminal = Path::new(terminal.trim_end());
    assert_eq!(window_size(terminal), [24, 80, 0, 0]);
    assert_eq!(got["start-text"], "$'\\E[4;0;0'", "{got:?}");
    assert_eq!(got["start-cell"], "$'\\E[6;0;0'", "{got:?}");
    assert_eq!(got["start-chars"], "$'\\E[8;24;80'", "{got:?}");
    assert_eq!(got["start-screen"], "$'\\E[9;24;80'", "{got:?}");
    assert_eq!(got["start-inband"], "$'\\E[48;24;80;0;0'", "{got:?}");

    // A surface of 50x20 cells of 9x17 pixels inside padding.
    client.send(&ClientMessage::Resize(SurfaceSize {
        cols: 50,
        rows: 20,
        screen_width: 455,
        screen_height: 349,
        cell_width: 9,
        cell_height: 17,
        padding_top: 4,
        padding_bottom: 5,
        padding_left: 2,
        padding_right: 3,
    }));
    client.expect(ServerMessage::Accepted);
    // The pixel fields and the answers hold the text area, 450x340, not the
    // padded surface.
    assert_eq!(window_size(terminal), [20, 50, 450, 340]);
    gate.release();
    client.frame_titled("ASKED");
    let got = replies(&answers);
    // The child's own switch to 132 columns resizes libghostty's grid but
    // not the PTY, and the answers keep the PTY's size.
    assert_eq!(window_size(terminal), [20, 50, 450, 340]);
    for round in ["sized", "switched"] {
        for (asked, answer) in [
            ("text", "4;340;450"),
            ("cell", "6;17;9"),
            ("chars", "8;20;50"),
            ("screen", "9;20;50"),
            ("inband", "48;20;50;340;450"),
        ] {
            let answer = format!("$'\\E[{answer}'");
            assert_eq!(got[&format!("{round}-{asked}")], answer, "{got:?}");
        }
    }
    gate.release();
    client.exited(7);
    drop(client);

    server.passed(7, 10.0);
}
