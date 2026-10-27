use crate::support::*;
use orbit_protocol::{
    Frame, ImageLayer, PlacementState,
    session::{
        ClientMessage, PixelFormat, RESOURCE_CHUNK_BYTES, ResourceAnnounce, ServerMessage,
        SurfaceSize,
    },
};
use std::{fs, path::Path};

// The child waits for the client's size, then plays the stream Orbit wrote
// to $ORBIT_STREAM and names its terminal, ending the synchronized update
// the stream began.
const KITTY_SCRIPT: &str = r#"stty -echo -icanon
printf '\033]0;STARTED\007'
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
printf '%s\033]0;PLACED\007\033[?2026l' "$(< "$ORBIT_STREAM")"
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

/// The image Yazi sends: 300x400 RGB pixels, two chunks on the wire.
const WIDTH: u32 = 300;
const HEIGHT: u32 = 400;

fn pixel(index: usize) -> u8 {
    (index % 251) as u8
}

/// Yazi's Kitty stream for a 300x400 RGB image, in Kitty chunks of 4,096
/// base64 bytes, then, in a synchronized update so that the first frame
/// shows them all, its 3x2 placeholder cells at column 2 and a 1x1 image
/// sent through the file medium, whose placeholder cell in row 3 has nothing
/// to show.
fn stream(pixel_file: &Path) -> String {
    let pixels: Vec<u8> = (0..(WIDTH * HEIGHT * 3) as usize).map(pixel).collect();
    let image = base64(&pixels);
    let chunks: Vec<_> = image.as_bytes().chunks(4096).collect();
    let mut transmission = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let keys = if index == 0 {
            format!("a=T,U=1,f=24,s={WIDTH},v={HEIGHT},i=7,c=3,r=2,")
        } else {
            String::new()
        };
        let more = u8::from(index + 1 < chunks.len());
        let chunk = std::str::from_utf8(chunk).expect("base64");
        transmission.push_str(&format!("\x1b_G{keys}q=2,m={more};{chunk}\x1b\\"));
    }
    let path = base64(pixel_file.as_os_str().as_encoded_bytes());
    format!(
        "{transmission}\x1b[?2026h\
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
    // Frames and resource messages arrive in one stream, until a frame
    // shows the placed image ready.
    let mut placed: Option<Frame> = None;
    let mut announced = Vec::new();
    let mut chunks = Vec::new();
    let ready = loop {
        match client.receive() {
            ServerMessage::Frame(frame) => {
                let ready = frame.graphics.as_ref().is_some_and(|graphics| {
                    !graphics.placements.is_empty()
                        && graphics
                            .placements
                            .iter()
                            .all(|entry| entry.state == PlacementState::Ready)
                });
                if ready {
                    break *frame;
                }
                if placed.is_none() && frame.title == "PLACED" {
                    placed = Some(*frame);
                }
            }
            ServerMessage::ResourceAnnounce(announce) => announced.push(announce),
            ServerMessage::ResourceChunk(chunk) => chunks.push(chunk),
            other => panic!("expected a frame or a resource, received {}", brief(&other)),
        }
    };
    let frame = placed.expect("a frame of the placed image before it is ready");
    let graphics = frame.graphics.as_ref().expect("Kitty graphics state");
    assert_eq!((graphics.cell_width, graphics.cell_height), (10, 20));
    assert!(!graphics.placements_omitted);
    // One pending entry per placeholder row, cut from the image in cells of
    // the client's size; the file-medium image was never stored.
    let entries = |frame: &Frame| -> Vec<_> {
        frame
            .graphics
            .as_ref()
            .expect("Kitty graphics state")
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
            .collect()
    };
    let row = |row, source_y, state| {
        (
            ImageLayer::BelowText,
            (2, row, 3, 1),
            (30, 20),
            (source_y, 200),
            state,
        )
    };
    let pending = PlacementState::Pending;
    assert_eq!(
        entries(&frame),
        [row(1, 0, pending), row(2, 200, pending)],
        "{graphics:?}"
    );
    let resource = graphics.placements[0].resource;
    assert_eq!(graphics.placements[1].resource, resource);
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

    // The image arrives once, as one announcement and its chunks in order,
    // each 262,144 bytes but the last; then, with no more output from the
    // child, a frame at a later revision shows its entries ready.
    let [announce] = announced[..] else {
        panic!("expected one announcement, received {announced:?}");
    };
    let expected = ResourceAnnounce::new(
        resource,
        announce.generation,
        PixelFormat::Rgb,
        WIDTH,
        HEIGHT,
    )
    .expect("within the limits");
    assert_eq!(announce, expected);
    let sizes: Vec<_> = chunks
        .iter()
        .map(|chunk| (chunk.resource, chunk.index, chunk.bytes.len()))
        .collect();
    let last = (WIDTH * HEIGHT * 3 - RESOURCE_CHUNK_BYTES) as usize;
    assert_eq!(
        sizes,
        [
            (resource, 0, RESOURCE_CHUNK_BYTES as usize),
            (resource, 1, last)
        ]
    );
    let pixels: Vec<u8> = chunks.into_iter().flat_map(|chunk| chunk.bytes).collect();
    assert!(
        pixels
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == pixel(index))
    );
    let ready_state = PlacementState::Ready;
    assert_eq!(
        entries(&ready),
        [row(1, 0, ready_state), row(2, 200, ready_state)]
    );
    assert_eq!(
        ready
            .graphics
            .as_ref()
            .map(|graphics| graphics.placements[0].resource),
        Some(resource)
    );
    assert!(ready.revision > frame.revision && ready.title == "PLACED");
    gate.release();
    client.exited(0);
    drop(client);

    server.passed(0, 10.0);
}
