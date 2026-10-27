use crate::support::*;
use orbit_protocol::{
    ImageLayer, PlacementState,
    session::{ClientMessage, ServerMessage, SurfaceSize},
};
use std::{fs, path::Path};

// The child waits for the client's size, then plays the stream Orbit wrote
// to $ORBIT_STREAM and names its terminal.
const KITTY_SCRIPT: &str = r#"stty -echo -icanon
printf '\033]0;STARTED\007'
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
printf '%s\033]0;PLACED\007' "$(< "$ORBIT_STREAM")"
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
exit 0
"#;

/// Standard base64 of `bytes`, which the Kitty protocol carries.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let word = chunk
            .iter()
            .enumerate()
            .fold(0u32, |word, (index, byte)| word | u32::from(*byte) << (16 - 8 * index));
        for index in 0..4 {
            text.push(if index <= chunk.len() {
                char::from(DIGITS[(word >> (18 - 6 * index) & 63) as usize])
            } else {
                '='
            });
        }
    }
    text
}

/// Yazi's Kitty stream for a 30x40 RGB image in 3x2 placeholder cells at
/// column 2, then a 1x1 image sent through the file medium, whose
/// placeholder cell in row 3 has nothing to show.
fn stream(pixel_file: &Path) -> String {
    let image = base64(&[0xff; 30 * 40 * 3]);
    let path = base64(pixel_file.as_os_str().as_encoded_bytes());
    format!(
        "\x1b_Ga=T,U=1,f=24,s=30,v=40,i=7,c=3,r=2,q=2;{image}\x1b\\\
         \x1b[38;5;7m\x1b[2;3H\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{10EEEE}\
         \x1b[3;3H\u{10EEEE}\u{30D}\u{305}\u{10EEEE}\u{10EEEE}\
         \x1b_Ga=T,U=1,t=f,f=24,s=1,v=1,i=9,c=1,r=1,q=2;{path}\x1b\\\
         \x1b[38;5;9m\x1b[4;3H\u{10EEEE}\u{305}\u{305}\x1b[0m"
    )
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "gate");
    let pixel_file = directory.join("pixel");
    fs::write(&pixel_file, [0xff; 3]).expect("pixel file");
    let stream_file = directory.join("stream");
    fs::write(&stream_file, stream(&pixel_file)).expect("stream");
    let shell = executable(
        &directory,
        "kitty-shell.sh",
        &format!("#!{}\n{KITTY_SCRIPT}", bash().display()),
    );
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_KITTY_GRAPHICS", Path::new("1")),
            ("ORBIT_GATE", gate.path()),
            ("ORBIT_STREAM", &stream_file),
        ],
    );
    let mut client = Peer::attach(&server.path, 10.0);
    client.frame_titled("STARTED");
    client.send(&ClientMessage::Resize(SurfaceSize {
        cols: 20,
        rows: 6,
        screen_width: 200,
        screen_height: 120,
        cell_width: 10,
        cell_height: 20,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    }));
    client.expect(ServerMessage::Accepted);
    gate.release();
    let frame = client.frame_titled("PLACED");
    let graphics = frame.graphics.as_ref().expect("Kitty graphics state");
    assert_eq!((graphics.cell_width, graphics.cell_height), (10, 20));
    assert!(!graphics.placements_omitted);
    // One pending entry per placeholder row, cut from the image in cells of
    // the client's size; the file-medium image was never stored.
    let entries: Vec<_> = graphics
        .placements
        .iter()
        .map(|entry| {
            (
                entry.layer,
                (entry.col, entry.row, entry.grid_cols, entry.grid_rows),
                (entry.width, entry.height),
                (entry.source.y, entry.source.height),
                entry.state,
            )
        })
        .collect();
    let row = |row, source_y| {
        (
            ImageLayer::BelowText,
            (2, row, 3, 1),
            (30, 20),
            (source_y, 20),
            PlacementState::Pending,
        )
    };
    assert_eq!(entries, [row(1, 0), row(2, 20)], "{graphics:?}");
    assert_eq!(graphics.placements[0].resource, graphics.placements[1].resource);
    // No placeholder codepoint reaches the client.
    let placeholders = frame.rows[1..3]
        .iter()
        .flat_map(|row| &row.cells[2..5])
        .chain([&frame.rows[3].cells[2]]);
    assert!(
        placeholders.clone().all(|cell| cell.text.is_empty()),
        "{:?}",
        placeholders.collect::<Vec<_>>()
    );
    gate.release();
    client.exited(0);
    drop(client);

    server.passed(0, 10.0);
}
