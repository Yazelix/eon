use crate::support::*;
use orbit_protocol::{
    Rgb,
    session::{ClientMessage, ServerMessage},
};
use std::{ffi::OsStr, fs};

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
}
