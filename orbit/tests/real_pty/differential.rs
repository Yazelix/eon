//! The differential check: the old Rust Orbit, at
//! `OLD_ORBIT_BIN`, and Orbit at `ORBIT_BIN` run the same child and the same
//! client messages in steps, and the check fails at the first step whose
//! messages differ. After each step both streams are left to settle, so a
//! step compares everything the servers sent for it, with two
//! normalizations and nothing else:
//!
//! - A run of consecutive frames, or of metadata messages, counts as its
//!   last one. How many come between two other messages depends on how the
//!   PTY reads split the child's output, and a queued frame or metadata
//!   message may be replaced by a later one (ORB-C4, ORB-C7, ORB-C3).
//! - A revision counts as its relation to the one compared before it in the
//!   same stream: 0 for the first, 1 for the same, 2 for a later one. Its
//!   value counts PTY turns, which the same split decides. Each stream must
//!   still be ordered: revisions never go back, and frames always advance.

use crate::support::*;
use orbit_protocol::{
    Frame,
    session::{
        ClientMessage, KeyAction, Modifiers, MouseAction, MouseButton, ScrollOutcome,
        SelectionAction, SelectionPosition, ServerMessage, SurfaceSize, VerticalDirection,
        WheelOutcome,
    },
};
use std::{env, ffi::OsStr};

/// The ANSI palette option both servers take, as in the throughput runs.
const PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";

/// How long the client must hear nothing for a step to have settled; the
/// observer, written to in the same turns, then gets a shorter wait.
const QUIET: f64 = 0.3;
const OBSERVER_QUIET: f64 = 0.1;

enum Act {
    Release,
    /// A request built from the last presented frame revision.
    Send(fn(u64) -> ClientMessage),
}

enum Until {
    /// At least one message, then quiet.
    Answer,
    Frame,
    Copied,
    /// A frame with this title, then quiet.
    Title(&'static str),
    Exited,
}

impl Until {
    fn reached(&self, message: &ServerMessage) -> bool {
        match self {
            Self::Answer => true,
            Self::Frame => matches!(message, ServerMessage::Frame(_)),
            Self::Copied => matches!(message, ServerMessage::CopiedText { .. }),
            Self::Title(title) => {
                matches!(message, ServerMessage::Frame(frame) if frame.title == *title)
            }
            Self::Exited => exited(message),
        }
    }
}

/// The center of a 10x18-pixel cell; `offset` moves it toward the cell's
/// left (negative) or right edge.
fn cell(col: u16, row: u16, offset: f32) -> SelectionPosition {
    position(
        f32::from(col) * 10.0 + 5.0 + offset,
        f32::from(row) * 18.0 + 9.0,
    )
}

fn select(action: SelectionAction) -> ClientMessage {
    ClientMessage::Selection(action)
}

fn begin(frame_revision: u64, position: SelectionPosition, time_ns: u64) -> ClientMessage {
    select(SelectionAction::Begin {
        frame_revision,
        position,
        time_ns,
        modifiers: Modifiers::empty(),
    })
}

fn finish(position: SelectionPosition) -> ClientMessage {
    select(SelectionAction::Finish {
        position,
        modifiers: Modifiers::empty(),
    })
}

/// The workload: output, input, resize, scroll and selection, then exit.
fn steps() -> Vec<(&'static str, Act, Until)> {
    use Act::*;
    use Until::*;
    vec![
        ("output", Release, Title("DIFF OUTPUT")),
        (
            "paste",
            Send(|_| paste(b"hello")),
            Title("DIFF PASTE hello"),
        ),
        (
            "key press",
            Send(|_| key('x', KeyAction::Press)),
            Title("DIFF KEY x"),
        ),
        (
            "key release",
            Send(|_| key('x', KeyAction::Release)),
            Answer,
        ),
        (
            "resize",
            Send(|_| {
                ClientMessage::Resize(SurfaceSize {
                    cols: 100,
                    rows: 30,
                    screen_width: 1000,
                    screen_height: 540,
                    cell_width: 10,
                    cell_height: 18,
                    padding_top: 0,
                    padding_bottom: 0,
                    padding_left: 0,
                    padding_right: 0,
                })
            }),
            Until::Frame,
        ),
        ("size output", Release, Title("DIFF SIZE")),
        ("scroll up", Send(|revision| scroll(revision, -10)), Answer),
        (
            "preview up",
            Send(|revision| preview(revision, VerticalDirection::Up)),
            Answer,
        ),
        (
            "wheel down",
            Send(|_| mouse(MouseAction::Press, Some(MouseButton::Five))),
            Answer,
        ),
        ("scroll down", Send(|revision| scroll(revision, 4)), Answer),
        (
            "future scroll",
            Send(|revision| scroll(revision + 1, -3)),
            Answer,
        ),
        (
            "future preview",
            Send(|revision| preview(revision + 1, VerticalDirection::Down)),
            Answer,
        ),
        (
            "return to live",
            Send(|_| ClientMessage::ReturnToLive),
            Answer,
        ),
        ("selection output", Release, Title("DIFF SELECT")),
        (
            "begin",
            Send(|revision| begin(revision, cell(0, 0, -4.0), 1_000_000_000)),
            Until::Frame,
        ),
        (
            "update",
            Send(|_| {
                select(SelectionAction::Update {
                    position: cell(2, 0, 0.0),
                    modifiers: Modifiers::empty(),
                })
            }),
            Until::Frame,
        ),
        ("finish", Send(|_| finish(cell(4, 0, 4.0))), Copied),
        ("copy", Send(|_| select(SelectionAction::Copy)), Answer),
        ("output under selection", Release, Title("DIFF KEPT")),
        ("cancel", Send(|_| select(SelectionAction::Cancel)), Until::Frame),
        (
            "click",
            Send(|revision| begin(revision, cell(19, 0, 0.0), 5_000_000_000)),
            Until::Frame,
        ),
        ("release", Send(|_| finish(cell(19, 0, 0.0))), Until::Frame),
        (
            "double click",
            Send(|revision| begin(revision, cell(19, 0, 0.0), 5_100_000_000)),
            Until::Frame,
        ),
        ("double release", Send(|_| finish(cell(19, 0, 0.0))), Copied),
        ("exit", Release, Exited),
    ]
}

/// The frame a message carries.
fn frame(message: &ServerMessage) -> Option<&Frame> {
    match message {
        ServerMessage::Frame(frame)
        | ServerMessage::WheelOutcome(WheelOutcome::Viewport { frame, .. })
        | ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { frame, .. }) => Some(frame),
        _ => None,
    }
}

/// The revision a message carries.
fn revision(message: &mut ServerMessage) -> Option<&mut u64> {
    match message {
        ServerMessage::Frame(frame)
        | ServerMessage::WheelOutcome(WheelOutcome::Viewport { frame, .. })
        | ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { frame, .. }) => {
            Some(&mut frame.revision)
        }
        ServerMessage::SelectionFinished { frame_revision } => Some(frame_revision),
        ServerMessage::VerticalPreview(preview) => Some(&mut preview.frame_revision),
        ServerMessage::Metadata(metadata) => Some(&mut metadata.revision),
        _ => None,
    }
}

/// One peer's messages, step by step.
type Stream = Vec<Vec<ServerMessage>>;

/// How the server and its child ended.
#[derive(Debug, PartialEq)]
struct Ending {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    child_alive: bool,
    socket_remains: bool,
}

struct Trace {
    client: Stream,
    observer: Stream,
    ending: Ending,
}

/// Messages up to and including the first that `done` accepts.
fn read_until(peer: &mut Peer, done: impl Fn(&ServerMessage) -> bool) -> Vec<ServerMessage> {
    let mut messages = Vec::new();
    loop {
        let message = peer.receive();
        let last = done(&message);
        messages.push(message);
        if last {
            return messages;
        }
    }
}

fn exited(message: &ServerMessage) -> bool {
    matches!(message, ServerMessage::Exited { .. })
}

/// Runs the workload against the server at `binary`.
fn record(binary: &OsStr) -> Trace {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "diff-gate");
    let shell = script(&directory, "DIFF_SHELL", "differential-shell");
    let path = directory.join("orbit.sock");
    let mut line = server_command(&path, &[shell.as_os_str()], &["--ansi-palette-v1", PALETTE]);
    line[0] = binary.into();
    let mut server = Server::open(path, &line, &[("ORBIT_DIFF_GATE", gate.path())], &[]);
    let mut client = Peer::connect(&server.path, 10.0);
    client.send(&ClientMessage::Hello);
    let mut sent = read_until(&mut client, |message| frame(message).is_some());
    sent.extend(client.settle(QUIET));
    let mut sent = vec![sent];
    let mut observer = Peer::connect(&server.path, 10.0);
    observer.send(&ClientMessage::ObserveMetadata);
    let mut observed = read_until(&mut observer, |message| {
        matches!(message, ServerMessage::Metadata(_))
    });
    observed.extend(observer.settle(OBSERVER_QUIET));
    let mut observed = vec![observed];
    for (_, act, until) in steps() {
        let presented = sent
            .iter()
            .flatten()
            .rev()
            .find_map(frame)
            .expect("a presented frame")
            .revision;
        match act {
            Act::Release => gate.release(),
            Act::Send(request) => client.send(&request(presented)),
        }
        let mut messages = read_until(&mut client, |message| until.reached(message));
        let mut metadata = Vec::new();
        match until {
            Until::Exited => {
                client.expect_closed();
                metadata = read_until(&mut observer, exited);
                observer.expect_closed();
            }
            Until::Title(title) => {
                metadata = read_until(
                    &mut observer,
                    |message| matches!(message, ServerMessage::Metadata(metadata) if metadata.title == title),
                );
            }
            _ => {}
        }
        if !messages.last().is_some_and(exited) {
            messages.extend(client.settle(QUIET));
            metadata.extend(observer.settle(OBSERVER_QUIET));
        }
        sent.push(messages);
        observed.push(metadata);
    }
    let finished = server.communicate(10.0);
    Trace {
        client: sent,
        observer: observed,
        ending: Ending {
            code: finished.code(),
            stdout: finished.stdout,
            stderr: finished.stderr,
            child_alive: alive(server.child),
            socket_remains: server.path.exists(),
        },
    }
}

/// The stream as the comparison sees it, step by step, after checking its
/// order.
fn normalize(name: &str, stream: &Stream) -> Stream {
    let mut stream = stream.clone();
    let (mut latest, mut latest_frame) = (None, None);
    for message in stream.iter_mut().flatten() {
        let carries_frame = frame(message).is_some();
        let Some(&mut value) = revision(message) else {
            continue;
        };
        assert!(
            latest.is_none_or(|latest| value >= latest),
            "{name}: revision {value} after {latest:?}"
        );
        if carries_frame {
            assert!(
                latest_frame.is_none_or(|latest| value > latest),
                "{name}: frame revision {value} after frame revision {latest_frame:?}"
            );
            latest_frame = Some(value);
        }
        latest = Some(value);
    }
    let mut previous = None;
    for step in &mut stream {
        let mut kept: Vec<ServerMessage> = Vec::new();
        for message in step.drain(..) {
            if matches!(
                (kept.last(), &message),
                (Some(ServerMessage::Frame(_)), ServerMessage::Frame(_))
                    | (Some(ServerMessage::Metadata(_)), ServerMessage::Metadata(_))
            ) {
                kept.pop();
            }
            kept.push(message);
        }
        for value in kept.iter_mut().filter_map(revision) {
            let relation = match previous {
                None => 0,
                Some(previous) if previous == *value => 1,
                Some(_) => 2,
            };
            previous = Some(*value);
            *value = relation;
        }
        *step = kept;
    }
    stream
}

fn shorten(mut text: String) -> String {
    if text.len() > 1500 {
        let mut end = 1500;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

/// The first field in which two frames differ.
fn frame_difference(expected: &Frame, actual: &Frame) -> String {
    macro_rules! fields {
        ($($field:ident),*) => {$(
            if expected.$field != actual.$field {
                return format!(
                    "frame {}: old {:?}, new {:?}",
                    stringify!($field),
                    expected.$field,
                    actual.$field
                );
            }
        )*};
    }
    fields!(
        revision,
        dimensions,
        screen,
        title,
        working_directory,
        capabilities,
        colors,
        cursor,
        scroll_position
    );
    for (row, (expected, actual)) in expected.rows.iter().zip(&actual.rows).enumerate() {
        let cell = expected
            .cells
            .iter()
            .zip(&actual.cells)
            .position(|(expected, actual)| expected != actual);
        if let Some(cell) = cell {
            return format!(
                "frame row {row} cell {cell}: old {:?}, new {:?}",
                expected.cells[cell], actual.cells[cell]
            );
        }
        if expected != actual {
            return format!("frame row {row}: old {expected:?}, new {actual:?}");
        }
    }
    format!(
        "frame rows: old {}, new {}",
        expected.rows.len(),
        actual.rows.len()
    )
}

fn difference(expected: Option<&ServerMessage>, actual: Option<&ServerMessage>) -> String {
    if let (Some(expected), Some(actual)) = (expected, actual)
        && let (Some(expected_frame), Some(actual_frame)) = (frame(expected), frame(actual))
        && expected_frame != actual_frame
    {
        return shorten(frame_difference(expected_frame, actual_frame));
    }
    let show = |message: Option<&ServerMessage>| {
        message.map_or("nothing".into(), |message| shorten(format!("{message:?}")))
    };
    format!("old {}, new {}", show(expected), show(actual))
}

/// Panics at the first step whose normalized messages differ.
fn compare(peer: &str, names: &[&str], expected: &Stream, actual: &Stream) {
    assert_eq!(expected.len(), names.len(), "{peer}: old step count");
    assert_eq!(actual.len(), names.len(), "{peer}: new step count");
    for ((name, expected), actual) in names.iter().zip(expected).zip(actual) {
        if expected == actual {
            continue;
        }
        let index = expected
            .iter()
            .zip(actual)
            .position(|(expected, actual)| expected != actual)
            .unwrap_or(expected.len().min(actual.len()));
        panic!(
            "{peer} differs at step {name:?}, message {index} (revisions as 0 first, 1 same, \
             2 later): {}",
            difference(expected.get(index), actual.get(index))
        );
    }
}

pub fn run() {
    let old = env::var_os("OLD_ORBIT_BIN")
        .expect("the differential check runs the old Orbit at OLD_ORBIT_BIN");
    let new = env::var_os("ORBIT_BIN").expect("ORBIT_BIN");
    let expected = record(&old);
    let actual = record(&new);
    let names: Vec<&str> = ["attach"]
        .into_iter()
        .chain(steps().into_iter().map(|(name, _, _)| name))
        .collect();
    let (client, observer) = (
        normalize("old client", &expected.client),
        normalize("old observer", &expected.observer),
    );
    assert!(
        client[1]
            .iter()
            .filter_map(frame)
            .flat_map(|frame| &frame.rows)
            .flat_map(|row| &row.cells)
            .any(|cell| cell.hyperlink == "https://example.com/"),
        "the output step must retain the rich text in its compared frame"
    );
    let kinds =
        |messages: &[ServerMessage]| messages.iter().map(brief).collect::<Vec<_>>().join("; ");
    for ((name, sent), observed) in names.iter().zip(&client).zip(&observer) {
        eprintln!(
            "{name}: client {}; observer {}",
            kinds(sent),
            kinds(observed)
        );
    }
    compare(
        "the client stream",
        &names,
        &client,
        &normalize("new client", &actual.client),
    );
    compare(
        "the observer stream",
        &names,
        &observer,
        &normalize("new observer", &actual.observer),
    );
    assert_eq!(
        expected.ending, actual.ending,
        "the servers ended differently"
    );
    assert_eq!(
        expected.ending,
        Ending {
            code: Some(5),
            stdout: Vec::new(),
            stderr: Vec::new(),
            child_alive: false,
            socket_remains: false,
        }
    );
}

#[test]
fn multipart_steps_wait_for_the_final_reply() {
    use orbit_protocol::session::ClipboardLocation;

    let frame = yazelix_orbit::frame_after_vt(1, 1, b"x", 0, 1).unwrap();
    for (name, replies) in [
        (
            "resize",
            vec![ServerMessage::Accepted, ServerMessage::Frame(Box::new(frame.clone()))],
        ),
        (
            "finish",
            vec![
                ServerMessage::SelectionFinished { frame_revision: 1 },
                ServerMessage::Frame(Box::new(frame.clone())),
                ServerMessage::CopiedText {
                    location: ClipboardLocation::Selection,
                    text: "x".into(),
                },
            ],
        ),
    ] {
        let (_, _, until) = steps().into_iter().find(|step| step.0 == name).unwrap();
        assert_eq!(
            replies.iter().position(|message| until.reached(message)),
            Some(replies.len() - 1),
            "{name} stopped before its final reply"
        );
    }
}

#[test]
#[should_panic(expected = "new step count")]
fn comparison_rejects_a_missing_step() {
    compare("client", &["attach"], &vec![vec![ServerMessage::Attached]], &vec![]);
}
