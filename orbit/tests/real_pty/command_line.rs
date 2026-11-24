use crate::support::*;
use orbit_protocol::{
    Dimensions, Rgb,
    session::{ClientMessage, ServerMessage, SurfaceSize},
};
use std::{ffi::OsStr, fs, path::Path};

const PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";

fn palette() -> Vec<Rgb> {
    PALETTE
        .split(',')
        .map(|color| {
            let value = u32::from_str_radix(color, 16).expect("a hex color");
            Rgb {
                r: (value >> 16) as u8,
                g: (value >> 8) as u8,
                b: value as u8,
            }
        })
        .collect()
}

pub fn run() {
    let directory = TempDir::new();
    let marker = directory.join("args");
    let gate = Gate::new(&directory, "exit-gate");
    // Eon's baseline harness passes a palette, a multi-line script and
    // positional arguments; the child runs as given, with no shell between.
    let text = "printf \"%s|\" \"$@\" >\"$1\"\nIFS= read -r _ < \"$ORBIT_GATE\"\nexit 4\n";
    let mut server = Server::launch(
        &directory,
        &[
            OsStr::new("/bin/sh"),
            OsStr::new("-c"),
            OsStr::new(text),
            OsStr::new("sh"),
            marker.as_os_str(),
            OsStr::new("two words"),
            OsStr::new("it's"),
        ],
        &[("ORBIT_GATE", gate.path())],
        &[],
        &["--ansi-palette-v1", PALETTE],
    );
    let mut client = Peer::connect(&server.path, 10.0);
    client.send(&ClientMessage::Hello);
    client.expect(ServerMessage::Attached);
    // The palette replaces the default ANSI colors and no others.
    let frame = client.frame();
    assert_eq!(frame.colors.palette[..16], palette()[..]);
    assert!(frame.colors.palette[16..].iter().any(|color| *color != Rgb::BLACK));
    gate.release();
    client.exited(4);
    drop(client);

    // The server exits with the child's code and prints nothing.
    server.passed(4, 10.0);
    assert_eq!(
        fs::read_to_string(&marker).expect("arguments"),
        format!("{}|two words|it's|", marker.display())
    );

    default_colors();
}

/// The PTY captures replies separately from the client's canonical frames.
const COLOR_SCRIPT: &str = r#"set -eu
stty raw -echo
ask() {
    printf '\033]10;?\033\\\033]11;?\007\033[6n'
    IFS= read -r -t 5 -d R reply
    printf '%sR' "$reply" > "$REPORT/answer"
    mv "$REPORT/answer" "$REPORT/$1"
    IFS= read -r _ < "$GATE"
}
ask initial
printf '\033]11;#808080\033\\'
ask background
printf '\033]10;rgb:12/34/56\007'
ask foreground
ask detached
printf '\033]111\033\\'
ask reset-background
printf '\033]110\007'
ask reset-foreground
"#;

fn expect_colors(directory: &Path, phase: &str, foreground: &str, background: &str) {
    let reply = directory.join(phase);
    wait_for(&reply);
    assert_eq!(
        fs::read_to_string(reply).expect("color replies"),
        format!("\x1b]10;rgb:{foreground}\x1b\\\x1b]11;rgb:{background}\x07\x1b[1;1R"),
        "{phase}"
    );
}

fn default_colors() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "gate");
    let mut server = Server::launch(
        &directory,
        &[bash().as_os_str(), OsStr::new("-c"), OsStr::new(COLOR_SCRIPT)],
        &[("GATE", gate.path()), ("REPORT", &directory)],
        &[&directory.join("initial")],
        &["--ansi-palette-v1", PALETTE, "--default-colors-v1", "d5abc3,172b3f"],
    );
    let rgb = |[r, g, b]: [u8; 3]| Rgb { r, g, b };
    let initial_fg = rgb([0xd5, 0xab, 0xc3]);
    let initial_bg = rgb([0x17, 0x2b, 0x3f]);
    let foreground = rgb([0x12, 0x34, 0x56]);
    let background = rgb([0x80; 3]);
    let assert_frame = |peer: &mut Peer, expected| loop {
        let frame = peer.frame();
        if (frame.colors.foreground, frame.colors.background) == expected {
            break frame;
        }
    };

    expect_colors(&directory, "initial", "d5d5/abab/c3c3", "1717/2b2b/3f3f");
    let mut peer = Peer::attach(&server.path, 5.0);
    let initial = peer.frame();
    assert_eq!((initial.colors.foreground, initial.colors.background), (initial_fg, initial_bg));
    assert_eq!(initial.colors.palette[..16], palette()[..]);

    gate.release();
    expect_colors(&directory, "background", "d5d5/abab/c3c3", "8080/8080/8080");
    let updated = assert_frame(&mut peer, (initial_fg, background));
    assert!(updated.revision > initial.revision);
    assert_eq!(updated.colors.palette, initial.colors.palette);

    gate.release();
    expect_colors(&directory, "foreground", "1212/3434/5656", "8080/8080/8080");
    let updated = assert_frame(&mut peer, (foreground, background));
    assert_eq!(updated.colors.palette, initial.colors.palette);

    peer.send(&ClientMessage::Resize(SurfaceSize {
        cols: 50,
        rows: 20,
        screen_width: 500,
        screen_height: 360,
        cell_width: 10,
        cell_height: 18,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    }));
    let mut accepted = false;
    loop {
        match peer.receive() {
            ServerMessage::Accepted => accepted = true,
            ServerMessage::Frame(frame) if frame.dimensions == (Dimensions { cols: 50, rows: 20 }) => {
                assert!(accepted);
                assert_eq!((frame.colors.foreground, frame.colors.background), (foreground, background));
                break;
            }
            ServerMessage::Frame(_) => {}
            message => panic!("unexpected resize response: {message:?}"),
        }
    }
    drop(peer);
    gate.release();
    expect_colors(&directory, "detached", "1212/3434/5656", "8080/8080/8080");

    let mut peer = Peer::attach(&server.path, 5.0);
    let frame = peer.frame();
    assert_eq!((frame.colors.foreground, frame.colors.background), (foreground, background));
    assert_eq!(frame.dimensions, Dimensions { cols: 50, rows: 20 });
    for (phase, foreground_reply, background_reply, colors) in [
        ("reset-background", "1212/3434/5656", "1717/2b2b/3f3f", (foreground, initial_bg)),
        ("reset-foreground", "d5d5/abab/c3c3", "1717/2b2b/3f3f", (initial_fg, initial_bg)),
    ] {
        gate.release();
        expect_colors(&directory, phase, foreground_reply, background_reply);
        let frame = assert_frame(&mut peer, colors);
        assert_eq!(frame.colors.palette, initial.colors.palette);
    }
    gate.release();
    peer.exited(0);
    drop(peer);
    server.passed(0, 5.0);
}
