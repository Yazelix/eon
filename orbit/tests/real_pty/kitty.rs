//! What the Kitty graphics checks share: a child that plays prepared steps,
//! Yazi-shaped Kitty streams, and a ledger that reads frames and resource
//! messages as one stream and checks ORB-C16's invariants on every message.

use crate::support::*;
use orbit_protocol::{
    Frame, PlacementState,
    session::{
        ClientMessage, RESOURCE_CHUNK_BYTES, ResourceAnnounce, ScrollOutcome, ServerMessage,
        SurfaceSize,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

// The child names its terminal, then plays one step file from $ORBIT_STEPS
// for each release of $ORBIT_GATE, in name order, and exits 0 on the release
// after the last.
const STEPS_SCRIPT: &str = r#"stty -echo -icanon
printf '\033]0;STARTED\007'
for step in "$ORBIT_STEPS"/*; do
  until IFS= read -r _ < "$ORBIT_GATE"; do :; done
  printf '%s' "$(< "$step")"
done
until IFS= read -r _ < "$ORBIT_GATE"; do :; done
exit 0
"#;

/// Standard base64 of `bytes`, which the Kitty protocol carries.
pub fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let word = chunk.iter().enumerate().fold(0u32, |word, (index, byte)| {
            word | u32::from(*byte) << (16 - 8 * index)
        });
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

/// A test image: `width` by `height` pixels of `channels` bytes (3 for RGB,
/// 4 for RGBA), each byte from `seed` and its index, so that two seeds give
/// different pixels.
#[derive(Clone, Copy)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub seed: usize,
}

impl Image {
    pub fn pixels(&self) -> Vec<u8> {
        (0..(self.width * self.height * self.channels) as usize)
            .map(|index| ((index + self.seed) % 251) as u8)
            .collect()
    }
}

/// A 30x40 RGB image, one resource chunk.
pub const SMALL: Image = Image {
    width: 30,
    height: 40,
    channels: 3,
    seed: 0,
};

/// A 9,000,000-byte RGBA image, 35 chunks, whose transfer a client holds
/// by not reading. Orbit sets no send buffer on its socket, so it can write
/// no more than the client has read plus the kernel's default send buffer,
/// `net.core.wmem_default`, which must stay well below the image.
pub fn big(seed: usize) -> Image {
    if let Ok(text) = fs::read_to_string("/proc/sys/net/core/wmem_default") {
        let bytes: u32 = text.trim().parse().expect("wmem_default");
        assert!(
            bytes <= 2_000_000,
            "a default send buffer of {bytes} bytes can take most of the image"
        );
    }
    Image {
        width: 1500,
        height: 1500,
        channels: 4,
        seed,
    }
}

/// Yazi's transmission of `image` as Kitty image `id`, in Kitty chunks of
/// 4,096 base64 bytes, with a virtual placement of `cols` by `rows` cells
/// for Unicode placeholders.
pub fn transmit(id: u32, image: Image, cols: u32, rows: u32) -> String {
    let encoded = base64(&image.pixels());
    let chunks: Vec<_> = encoded.as_bytes().chunks(4096).collect();
    let format = image.channels * 8;
    let mut text = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let keys = if index == 0 {
            format!(
                "a=T,U=1,f={format},s={},v={},i={id},c={cols},r={rows},",
                image.width, image.height
            )
        } else {
            String::new()
        };
        let more = u8::from(index + 1 < chunks.len());
        let chunk = std::str::from_utf8(chunk).expect("base64");
        text.push_str(&format!("\x1b_G{keys}q=2,m={more};{chunk}\x1b\\"));
    }
    text
}

/// Kitty's diacritics for placeholder rows and columns 0 to 9.
const DIACRITICS: [char; 10] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}',
];

/// Unicode placeholder cells for Kitty image `id`, `cols` by `rows` with
/// their top-left corner at the zero-based `col` and `row`, as Yazi prints
/// them: the image id in a 24-bit foreground color, and each row's first
/// cell marked with its row and column 0.
pub fn placeholders(id: u32, col: u32, row: u32, cols: u32, rows: u32) -> String {
    let [_, red, green, blue] = id.to_be_bytes();
    let mut text = format!("\x1b[38;2;{red};{green};{blue}m");
    for line in 0..rows {
        text.push_str(&format!("\x1b[{};{}H", row + line + 1, col + 1));
        text.push('\u{10EEEE}');
        text.push(DIACRITICS[line as usize]);
        text.push(DIACRITICS[0]);
        for _ in 1..cols {
            text.push('\u{10EEEE}');
        }
    }
    text.push_str("\x1b[39m");
    text
}

/// Spaces over `cols` by `rows` cells from the zero-based `col` and `row`,
/// as Yazi hides a preview.
pub fn erase(col: u32, row: u32, cols: u32, rows: u32) -> String {
    (0..rows)
        .map(|line| {
            format!(
                "\x1b[{};{}H{}",
                row + line + 1,
                col + 1,
                " ".repeat(cols as usize)
            )
        })
        .collect()
}

/// Kitty's delete of image `id` and its pixels.
pub fn delete(id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
}

/// `body` in one synchronized update that ends by naming the terminal
/// `title`, so the first frame that carries the title shows all of it.
pub fn step(body: &str, title: &str) -> String {
    format!("\x1b[?2026h{body}\x1b]0;{title}\x07\x1b[?2026l")
}

/// A client size of `cols` by `rows` cells of `width` by `height` pixels,
/// without padding.
pub fn size(cols: u16, rows: u16, width: u32, height: u32) -> ClientMessage {
    ClientMessage::Resize(SurfaceSize {
        cols,
        rows,
        screen_width: u32::from(cols) * width,
        screen_height: u32::from(rows) * height,
        cell_width: width,
        cell_height: height,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    })
}

/// An Orbit whose child plays `steps`.
pub struct Kitty {
    // Fields drop in order: the server stops before its directory goes.
    pub server: Server,
    gate: Gate,
    _directory: TempDir,
}

impl Kitty {
    pub fn start(steps: &[String]) -> Self {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "gate");
        let folder = directory.join("steps");
        fs::create_dir(&folder).expect("steps directory");
        for (index, text) in steps.iter().enumerate() {
            fs::write(folder.join(format!("{index:03}")), text).expect("step");
        }
        let shell = executable(
            &directory,
            "kitty-shell.sh",
            &format!("#!{}\n{STEPS_SCRIPT}", bash().display()),
        );
        let server = Server::start(
            &directory,
            &shell,
            &[("ORBIT_GATE", gate.path()), ("ORBIT_STEPS", &folder)],
        );
        Self {
            server,
            gate,
            _directory: directory,
        }
    }

    /// Attaches the first client and gives it this size once the child
    /// runs.
    pub fn attach(&self, size: ClientMessage) -> Peer {
        let mut client = Peer::attach(&self.server.path, 10.0);
        client.frame_titled("STARTED");
        client.send(&size);
        client.expect(ServerMessage::Accepted);
        client
    }

    /// Lets the child play its next step, or exit after the last.
    pub fn next(&self) {
        self.gate.release();
    }

    /// A metadata observer, which hears each title as Orbit publishes the
    /// frame that carries it, without reading the client.
    pub fn observe(&self) -> Peer {
        let mut observer = Peer::connect(&self.server.path, 10.0);
        observer.send(&ClientMessage::ObserveMetadata);
        observer.expect(ServerMessage::ObservingMetadata);
        observer
    }
}

/// A resource message, in the order the client read it.
#[derive(Debug, PartialEq)]
pub enum Note {
    Announced(u64),
    Retired(u64),
}

/// One attachment's frames and resource messages, each checked as it
/// arrives against the invariants that docs/BEHAVIOR.md lists for these
/// checks.
#[derive(Default)]
pub struct Ledger {
    pub announced: BTreeMap<u64, (ResourceAnnounce, Vec<u8>)>,
    pub retired: BTreeSet<u64>,
    pub notes: Vec<Note>,
    /// The resource whose chunks are still due.
    pub transferring: Option<u64>,
    pub frame: Option<Frame>,
    /// Every number a frame has referenced.
    referenced: BTreeSet<u64>,
}

impl Ledger {
    /// Whether all of the resource's chunks have arrived.
    pub fn delivered(&self, resource: u64) -> bool {
        self.announced
            .get(&resource)
            .is_some_and(|(announce, bytes)| bytes.len() == announce.byte_length as usize)
    }

    pub fn pixels(&self, resource: u64) -> &[u8] {
        assert!(
            self.delivered(resource),
            "resource {resource} not delivered"
        );
        &self.announced[&resource].1
    }

    /// The last frame's entries: resource and state.
    pub fn entries(&self) -> Vec<(u64, PlacementState)> {
        self.frame.as_ref().map(entries).unwrap_or_default()
    }

    /// Whether the last frame has this title and every entry ready.
    pub fn settled(&self, title: &str) -> bool {
        self.frame.as_ref().is_some_and(|frame| {
            frame.title == title
                && entries(frame)
                    .iter()
                    .all(|(_, state)| *state == PlacementState::Ready)
        })
    }

    /// Reads and checks one message and returns it.
    pub fn read(&mut self, client: &mut Peer) -> ServerMessage {
        let Some(message) = client.receive_within() else {
            panic!(
                "timed out after {:?} with entries {:?}, transferring {:?} and {} notes",
                self.frame.as_ref().map(|frame| &frame.title),
                self.entries(),
                self.transferring,
                self.notes.len()
            );
        };
        match &message {
            ServerMessage::Frame(frame) => self.saw(frame),
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { frame, .. }) => self.saw(frame),
            ServerMessage::ResourceAnnounce(announce) => {
                assert!(
                    self.referenced.contains(&announce.resource),
                    "announced {} before a frame referenced it",
                    announce.resource
                );
                assert_eq!(self.transferring, None, "announced during a transfer");
                if let Some(last) = self.announced.keys().next_back() {
                    assert!(announce.resource > *last, "identities must increase");
                }
                self.announced
                    .insert(announce.resource, (*announce, Vec::new()));
                self.notes.push(Note::Announced(announce.resource));
                self.transferring = Some(announce.resource);
            }
            ServerMessage::ResourceChunk(chunk) => {
                assert_eq!(
                    Some(chunk.resource),
                    self.transferring,
                    "chunk outside its transfer"
                );
                let (announce, bytes) = self.announced.get_mut(&chunk.resource).expect("announced");
                assert_eq!(
                    chunk.index as usize,
                    bytes.len() / RESOURCE_CHUNK_BYTES as usize,
                    "chunk order"
                );
                let left = announce.byte_length as usize - bytes.len();
                assert_eq!(
                    chunk.bytes.len(),
                    left.min(RESOURCE_CHUNK_BYTES as usize),
                    "chunk size"
                );
                bytes.extend_from_slice(&chunk.bytes);
                if bytes.len() == announce.byte_length as usize {
                    self.transferring = None;
                }
            }
            ServerMessage::ResourceRetire { resource } => {
                assert!(
                    self.delivered(*resource),
                    "retired {resource} before its last chunk"
                );
                assert!(self.retired.insert(*resource), "retired {resource} twice");
                let frame = self.frame.as_ref().expect("a frame before a retirement");
                assert!(
                    entries(frame).iter().all(|(entry, _)| entry != resource),
                    "retired {resource} while the last frame references it"
                );
                self.notes.push(Note::Retired(*resource));
            }
            _ => {}
        }
        message
    }

    /// Reads until `done` holds after a message; anything other than frames,
    /// scroll outcomes, resource messages and a resize's Accepted fails.
    pub fn until(&mut self, client: &mut Peer, done: impl Fn(&Self) -> bool) {
        while !done(self) {
            match self.read(client) {
                ServerMessage::Frame(_)
                | ServerMessage::ScrollOutcome(_)
                | ServerMessage::ResourceAnnounce(_)
                | ServerMessage::ResourceChunk(_)
                | ServerMessage::ResourceRetire { .. }
                | ServerMessage::Accepted => {}
                other => panic!("unexpected {}", brief(&other)),
            }
        }
    }

    /// Reads frames and resource messages until Exited with this code, then
    /// expects the end of the stream.
    pub fn exited(&mut self, client: &mut Peer, code: i32) {
        loop {
            match self.read(client) {
                ServerMessage::Exited { code: received } => {
                    assert_eq!(received, code, "exit code");
                    client.expect_closed();
                    return;
                }
                ServerMessage::Frame(_)
                | ServerMessage::ResourceAnnounce(_)
                | ServerMessage::ResourceChunk(_)
                | ServerMessage::ResourceRetire { .. } => {}
                other => panic!("expected Exited, received {}", brief(&other)),
            }
        }
    }

    /// Reads the first frame with this title.
    pub fn titled(&mut self, client: &mut Peer, title: &str) -> Frame {
        self.until(client, |ledger| {
            ledger
                .frame
                .as_ref()
                .is_some_and(|frame| frame.title == title)
        });
        self.frame.clone().expect("frame")
    }

    /// Reads until a frame has this title and every entry ready.
    pub fn settle(&mut self, client: &mut Peer, title: &str) -> Frame {
        self.until(client, |ledger| ledger.settled(title));
        self.frame.clone().expect("frame")
    }

    fn saw(&mut self, frame: &Frame) {
        let graphics = frame.graphics.as_ref().expect("Kitty graphics state");
        let previous: BTreeSet<u64> = self
            .entries()
            .into_iter()
            .map(|(resource, _)| resource)
            .collect();
        for entry in &graphics.placements {
            let resource = entry.resource;
            assert!(
                !self.retired.contains(&resource),
                "a frame references retired {resource}"
            );
            if entry.state == PlacementState::Ready {
                assert!(
                    self.delivered(resource),
                    "{resource} ready before its last chunk"
                );
            }
            if self.referenced.contains(&resource) {
                assert!(
                    previous.contains(&resource) || self.announced.contains_key(&resource),
                    "{resource} left the frames unstarted and came back"
                );
            } else if let Some(last) = self.referenced.last() {
                assert!(resource > *last, "new number {resource} after {last}");
            }
        }
        self.referenced
            .extend(graphics.placements.iter().map(|entry| entry.resource));
        self.frame = Some(frame.clone());
    }
}

/// Reads metadata until this title arrives.
pub fn heard(observer: &mut Peer, title: &str) {
    while observer.metadata().title != title {}
}

/// A frame's entries: resource and state.
pub fn entries(frame: &Frame) -> Vec<(u64, PlacementState)> {
    frame
        .graphics
        .as_ref()
        .expect("Kitty graphics state")
        .placements
        .iter()
        .map(|entry| (entry.resource, entry.state))
        .collect()
}
