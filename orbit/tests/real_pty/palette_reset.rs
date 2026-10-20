use crate::support::*;
use orbit_protocol::{
    Rgb,
    session::{ClientMessage, ServerMessage},
};
use std::ffi::OsStr;

const PALETTE: &str = "000102,101112,202122,303132,404142,505152,606162,707172,808182,909192,a0a1a2,b0b1b2,c0c1c2,d0d1d2,e0e1e2,f0f1f2";

/// The supplied colors, then xterm's 256-color defaults: a 6x6x6 cube and
/// a 24-step gray ramp.
fn expected() -> Vec<Rgb> {
    let hex = |color: &str| {
        let value = u32::from_str_radix(color, 16).expect("a hex color");
        Rgb {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        }
    };
    let level = |step: usize| if step == 0 { 0 } else { 55 + 40 * step as u8 };
    let cube = (0..216).map(|index| Rgb {
        r: level(index / 36),
        g: level(index / 6 % 6),
        b: level(index % 6),
    });
    let gray = (0..24).map(|step| {
        let value = 8 + 10 * step as u8;
        Rgb {
            r: value,
            g: value,
            b: value,
        }
    });
    PALETTE.split(',').map(hex).chain(cube).chain(gray).collect()
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "reset-gate");
    // The child overrides two ANSI colors and one above them, then resets
    // the whole palette with an OSC 104 that names no index.
    let text = concat!(
        "printf '\\033]4;0;rgb:01/02/03;1;rgb:04/05/06;200;rgb:07/08/09\\033\\\\'\n",
        "printf '\\033]2;OVERRIDDEN\\033\\\\'\n",
        "IFS= read -r _ < \"$ORBIT_GATE\"\n",
        "printf '\\033]104\\033\\\\\\033]2;RESET\\033\\\\'\n",
        "IFS= read -r _ < \"$ORBIT_GATE\"\n",
        "exit 5\n",
    );
    let mut server = Server::launch(
        &directory,
        &[OsStr::new("/bin/sh"), OsStr::new("-c"), OsStr::new(text)],
        &[("ORBIT_GATE", gate.path())],
        &[],
        &["--ansi-palette-v1", PALETTE],
    );
    let mut client = Peer::connect(&server.path, 10.0);
    client.send(&ClientMessage::Hello);
    client.expect(ServerMessage::Attached);
    let defaults = expected();

    // The supplied palette replaces the 16 ANSI colors; the other 240 keep
    // their defaults exactly.
    let frame = client.frame_titled("OVERRIDDEN");
    let overridden = &frame.colors.palette;
    assert_eq!(overridden[0], Rgb { r: 1, g: 2, b: 3 });
    assert_eq!(overridden[1], Rgb { r: 4, g: 5, b: 6 });
    assert_eq!(overridden[200], Rgb { r: 7, g: 8, b: 9 });
    assert_eq!(overridden[2..200], defaults[2..200]);
    assert_eq!(overridden[201..], defaults[201..]);

    // A bare OSC 104 restores every color: the supplied 16 and the rest.
    gate.release();
    let frame = client.frame_titled("RESET");
    assert_eq!(frame.colors.palette[..], defaults[..]);

    gate.release();
    client.exited(5);
    drop(client);
    server.passed(5, 10.0);
}
