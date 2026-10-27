use crate::{kitty::*, support::*};
use orbit_protocol::{
    Frame, ImageLayer, PlacementState,
    session::{PixelFormat, RESOURCE_CHUNK_BYTES, ResourceAnnounce, ServerMessage},
};
use std::{fs, path::Path};

/// The image Yazi sends: 300x400 RGB pixels, two chunks on the wire.
const IMAGE: Image = Image {
    width: 300,
    height: 400,
    channels: 3,
    seed: 0,
};

/// Yazi's Kitty stream for the image as Kitty image 7, then, in a
/// synchronized update so that the first frame shows them all, its 3x2
/// placeholder cells at column 2 and a 1x1 image sent through the file
/// medium, whose placeholder cell in row 3 has nothing to show.
fn stream(pixel_file: &Path) -> String {
    let transmission = transmit(7, IMAGE, 3, 2);
    let path = base64(pixel_file.as_os_str().as_encoded_bytes());
    let placed = format!(
        "\x1b[38;5;7m\x1b[2;3H\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{10EEEE}\
         \x1b[3;3H\u{10EEEE}\u{30D}\u{305}\u{10EEEE}\u{10EEEE}\
         \x1b_Ga=T,U=1,t=f,f=24,s=1,v=1,i=9,c=1,r=1,q=2;{path}\x1b\\\
         \x1b[38;5;9m\x1b[4;3H\u{10EEEE}\u{305}\u{305}\x1b[0m"
    );
    format!("{transmission}{}", step(&placed, "PLACED"))
}

pub fn run() {
    let directory = TempDir::new();
    let pixel_file = directory.join("pixel");
    fs::write(&pixel_file, [0xff; 3]).expect("pixel file");
    let kitty = Kitty::start(&[stream(&pixel_file)]);
    let mut client = kitty.attach(size(20, 6, 10, 20));
    kitty.next();
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
        IMAGE.width,
        IMAGE.height,
    )
    .expect("within the limits");
    assert_eq!(announce, expected);
    let sizes: Vec<_> = chunks
        .iter()
        .map(|chunk| (chunk.resource, chunk.index, chunk.bytes.len()))
        .collect();
    let last = (IMAGE.width * IMAGE.height * 3 - RESOURCE_CHUNK_BYTES) as usize;
    assert_eq!(
        sizes,
        [
            (resource, 0, RESOURCE_CHUNK_BYTES as usize),
            (resource, 1, last)
        ]
    );
    let pixels: Vec<u8> = chunks.into_iter().flat_map(|chunk| chunk.bytes).collect();
    assert!(pixels == IMAGE.pixels());
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
    kitty.next();
    client.exited(0);
    drop(client);
    let mut server = kitty.server;
    server.passed(0, 10.0);
}
