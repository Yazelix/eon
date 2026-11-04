use crate::management_lease::{Manager, Reply, failure, leased};
use crate::management_record::suffixed;
use crate::support::*;
use crate::visible_text::Driven;
use orbit_protocol::{
    management::{
        self, ClientMessage, FailureCode, LeaseInput, MAX_INPUT_PASTE_BYTES, Refusal, ServerMessage,
    },
    session::{self, KeyAction},
};
use std::{thread, time::Instant};

fn input(run: &mut Driven, input: LeaseInput) -> Reply {
    quiet_input(run, input, 0)
}

fn quiet_input(run: &mut Driven, input: LeaseInput, quiet_ms: u16) -> Reply {
    run.lease.send(&ClientMessage::Input { input, quiet_ms })
}

fn pasted(run: &mut Driven, text: &str) -> Reply {
    input(run, LeaseInput::Paste(text.into()))
}

/// A paste of `text` with a 1.5 s quiet window.
fn quiet_pasted(run: &mut Driven, text: &str) -> Reply {
    quiet_input(run, LeaseInput::Paste(text.into()), 1500)
}

fn written(bracketed: bool) -> Reply {
    Reply::Message(ServerMessage::Input { bracketed })
}

fn refused(refusal: Refusal) -> Reply {
    Reply::Message(ServerMessage::Refused(refusal))
}

fn encoded(message: &ClientMessage) -> Vec<u8> {
    management::encode_client_message(message).expect("encodable request")
}

/// A request of `message`'s kind with this payload instead of its own.
fn reframed(message: &ClientMessage, payload: &[u8]) -> Vec<u8> {
    let mut bytes = encoded(message)[..management::HEADER_BYTES].to_vec();
    bytes[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}

/// Reads the client's messages until a frame has this title.
fn until_titled(client: &mut Peer, title: &str) -> orbit_protocol::Frame {
    loop {
        match client.receive() {
            session::ServerMessage::Frame(frame) if frame.title == title => return *frame,
            session::ServerMessage::Frame(_) | session::ServerMessage::Accepted => {}
            other => panic!("expected a frame or Accepted, received {}", brief(&other)),
        }
    }
}

/// Sends an attached client's Enter press and release.
fn press_enter(client: &mut Peer) {
    client.send(&enter(KeyAction::Press));
    client.send(&enter(KeyAction::Release));
}

pub fn run() {
    input_is_encoded_as_a_client_would_send_it();
    a_lease_paste_is_never_interleaved();
    refused_requests_keep_the_lease();
    input_waits_out_a_clients_typing();
    typing_counts_once_it_reaches_the_pty();
}

/// Lease input reaches the PTY exactly as an attached client's would, without
/// an attachment and without touching one: a paste bracketed only when the
/// program enabled bracketed paste, and Enter with its release once the
/// keyboard mode reports releases. A newline without bracketed paste and a
/// control character are refused and write nothing.
fn input_is_encoded_as_a_client_would_send_it() {
    let mut run = Driven::start();
    assert_eq!(pasted(&mut run, "tab\there"), written(false));
    for (text, refusal) in [
        ("line\nfeed", Refusal::UnbracketedNewline),
        ("carriage\rreturn", Refusal::UnbracketedNewline),
        ("escape\x1b[A", Refusal::InvalidInput),
        ("bell\x07", Refusal::InvalidInput),
        ("delete\x7f", Refusal::InvalidInput),
        ("next\u{85}line", Refusal::InvalidInput),
    ] {
        assert_eq!(pasted(&mut run, text), refused(refusal), "{text:?}");
    }
    assert_eq!(input(&mut run, LeaseInput::Enter), written(false));
    assert_eq!(
        run.settled_input(0, 9),
        b"tab\there\r",
        "a refused paste wrote"
    );

    let mut client = run.attach();
    client.send(&paste(b"tab\there"));
    press_enter(&mut client);
    assert_eq!(
        run.settled_input(9, 9),
        b"tab\there\r",
        "a client's input differs"
    );

    // With bracketed paste, and a client scrolled into history.
    run.command(
        "printf '\\033[?2004h'; for i in $(seq 40); do printf 'h\\r\\n'; done; \
         printf '\\033]0;bracketed\\007'",
    );
    let frame = until_titled(&mut client, "bracketed");
    client.send(&scroll(frame.revision, -10));
    let (_, applied, scrolled) = client.scroll_outcome();
    assert_eq!(applied, -10);
    let from = run.input().len();
    assert_eq!(pasted(&mut run, "one\ntwo"), written(true));
    assert_eq!(input(&mut run, LeaseInput::Enter), written(false));
    let leased = run.settled_input(from, 1);
    client.quiet(0.3, "lease input sent the client something");
    client.send(&scroll(scrolled.revision, 10));
    assert_eq!(
        client.scroll_outcome().1,
        10,
        "lease input moved the client's viewport"
    );
    let from = run.input().len();
    client.send(&paste(b"one\ntwo"));
    press_enter(&mut client);
    assert_eq!(
        run.settled_input(from, leased.len()),
        leased,
        "a client's input differs"
    );
    assert!(
        leased.starts_with(b"\x1b[200~") && leased.ends_with(b"\x1b[201~\r"),
        "{leased:?}"
    );

    // With key releases reported, Enter writes its press and release.
    run.command("printf '\\033[>10u\\033]0;releases\\007'");
    until_titled(&mut client, "releases");
    let from = run.input().len();
    assert_eq!(input(&mut run, LeaseInput::Enter), written(false));
    let leased = run.settled_input(from, 1);
    let from = run.input().len();
    press_enter(&mut client);
    assert_eq!(
        run.settled_input(from, leased.len()),
        leased,
        "a client's Enter differs"
    );
    assert!(
        leased.windows(3).any(|bytes| bytes == b":3u"),
        "no release: {leased:?}"
    );
    drop(client);
    run.stop();
}

/// A 64 KiB lease paste reaches the PTY whole, between an attached client's
/// keys, never among them; once the PTY input queue cannot take another
/// whole one, the paste is refused with `InputFull`, and the lease stays.
fn a_lease_paste_is_never_interleaved() {
    let mut run = Driven::start();
    let mut client = run.attach();
    let text: String = (0..MAX_INPUT_PASTE_BYTES)
        .map(|i| char::from(b"0123456789abcdef"[i % 16]))
        .collect();
    let typist = thread::spawn(move || {
        for _ in 0..300 {
            client.send(&key('k', KeyAction::Press));
            sleep(0.001);
        }
        client
    });
    sleep(0.05);
    assert_eq!(pasted(&mut run, &text), written(false));
    let client = typist.join().expect("typist");
    let typed = run.settled_input(0, text.len() + 300);
    assert_eq!(typed.len(), text.len() + 300);
    let start = typed
        .iter()
        .position(|&byte| byte != b'k')
        .expect("the paste");
    let end = start + text.len();
    assert!(
        typed[start..end] == *text.as_bytes(),
        "the paste was interleaved"
    );
    assert!(
        typed[..start]
            .iter()
            .chain(&typed[end..])
            .all(|&byte| byte == b'k')
    );
    assert!(
        start > 0 && end < typed.len(),
        "the client typed only on one side of the paste ({start} keys before)"
    );

    // A child that reads nothing fills the queue.
    run.command("kill $cat_pid; printf '\\033]0;deaf\\007'");
    run.until_titled("deaf");
    let chunk = "x".repeat(MAX_INPUT_PASTE_BYTES);
    let mut accepted = 0;
    loop {
        match pasted(&mut run, &chunk) {
            reply if reply == written(false) && accepted < 40 => accepted += 1,
            reply => {
                assert_eq!(
                    reply,
                    refused(Refusal::InputFull),
                    "after {accepted} pastes"
                );
                break;
            }
        }
    }
    assert!(
        matches!(
            run.lease.send(&ClientMessage::Status),
            Reply::Message(ServerMessage::Status(_))
        ),
        "the lease ended at InputFull"
    );
    drop(client);
    run.stop();
}

/// A whole request that does not decode, or one pipelined behind another, is
/// refused under the lease, which stays, and a partial request after it is
/// kept and answered once whole. A request over its kind's bound ends the
/// lease and writes nothing, and one that does not decode before the lease is
/// refused and closes.
fn refused_requests_keep_the_lease() {
    let mut run = Driven::start();
    let record = suffixed(&run.server.path, ".record");
    let invalid = refused(Refusal::InvalidRequest);
    let allow = ClientMessage::AttachAllow {
        process_id: 1,
        process_start: 1,
    };
    let paste = ClientMessage::Input {
        input: LeaseInput::Paste("a".into()),
        quiet_ms: 0,
    };
    // A zero process ID, and an empty paste with a zero quiet window.
    assert_eq!(run.lease.send_bytes(&reframed(&allow, &[0; 12])), invalid);
    let tag = encoded(&paste)[management::HEADER_BYTES];
    assert_eq!(run.lease.send_bytes(&reframed(&paste, &[tag, 0, 0])), invalid);
    let read = encoded(&ClientMessage::ReadVisible);
    let status = encoded(&ClientMessage::Status);
    let pipelined = [read.as_slice(), read.as_slice(), &status[..5]].concat();
    assert_eq!(run.lease.send_bytes(&pipelined), invalid);
    let reply = run.lease.send_bytes(&status[5..]);
    assert!(
        matches!(reply, Reply::Message(ServerMessage::Status(_))),
        "{reply:?}"
    );

    // Over the bound.
    assert!(
        management::encode_client_message(&ClientMessage::Input {
            input: LeaseInput::Paste("x".repeat(MAX_INPUT_PASTE_BYTES + 1)),
            quiet_ms: 0,
        })
        .is_err()
    );
    let mut over = reframed(&paste, &[]);
    let length = (management::MAX_INPUT_MESSAGE_BYTES - management::HEADER_BYTES + 1) as u32;
    over[8..12].copy_from_slice(&length.to_le_bytes());
    let reply = run.lease.send_bytes(&over);
    assert!(failure(&reply, FailureCode::InvalidRequest), "{reply:?}");
    assert_eq!(run.lease.read(), Reply::Closed);

    // Before the lease, once the refused connection has gone.
    let deadline = Instant::now() + seconds(2.0);
    loop {
        let mut early = Manager::new(&record);
        early.connect();
        let reply = early.send_bytes(&reframed(&allow, &[0; 12]));
        if reply == Reply::Message(ServerMessage::Busy) && Instant::now() < deadline {
            sleep(0.02);
            continue;
        }
        assert!(failure(&reply, FailureCode::InvalidRequest), "{reply:?}");
        assert_eq!(early.read(), Reply::Closed);
        break;
    }
    run.lease = leased(&record);
    assert!(run.input().is_empty(), "a refused request wrote to the PTY");
    run.stop();
}

/// Lease input with a quiet window is refused, and writes nothing, while an
/// attached client's key press or paste is younger than the window. Lease
/// input itself never counts, nor does a key release or a focus change that
/// writes to the PTY, and a zero window never refuses.
fn input_waits_out_a_clients_typing() {
    let mut run = Driven::start();
    let mut client = run.attach();
    let typing = refused(Refusal::Typing);
    assert_eq!(quiet_pasted(&mut run, "x"), written(false), "nobody typed");
    assert_eq!(
        quiet_pasted(&mut run, "y"),
        written(false),
        "lease input counted as typing"
    );
    assert_eq!(run.settled_input(0, 2), b"xy");

    client.send(&key('a', KeyAction::Press));
    assert_eq!(run.settled_input(2, 1), b"a");
    assert_eq!(quiet_pasted(&mut run, "z"), typing, "paste after a key press");
    assert_eq!(
        quiet_input(&mut run, LeaseInput::Enter, 1500),
        typing,
        "Enter after a key press"
    );
    assert_eq!(pasted(&mut run, "0"), written(false), "a zero window refused");
    assert_eq!(run.settled_input(3, 1), b"0");

    sleep(1.7);
    assert_eq!(quiet_pasted(&mut run, "z"), written(false), "after the window");
    client.send(&paste(b"p"));
    assert_eq!(run.settled_input(4, 2), b"zp");
    assert_eq!(quiet_pasted(&mut run, "q"), typing, "paste after a client paste");
    // A written sentinel shows that nothing refused arrived before it.
    assert_eq!(pasted(&mut run, "s"), written(false));
    assert_eq!(run.settled_input(0, 7), b"xya0zps", "a refused request wrote");

    // With key releases and focus changes reported, both write to the PTY.
    run.command("printf '\\033[>10u\\033[?1004h\\033]0;reports\\007'");
    until_titled(&mut client, "reports");
    sleep(1.7);
    let from = run.input().len();
    client.send(&key('a', KeyAction::Release));
    client.send(&session::ClientMessage::Focus(session::FocusEvent::Gained));
    let reported = run.settled_input(from, 4);
    assert!(
        reported.windows(3).any(|bytes| bytes == b":3u") && reported.ends_with(b"\x1b[I"),
        "{reported:?}"
    );
    assert_eq!(
        quiet_pasted(&mut run, "r"),
        written(false),
        "a release or focus change counted as typing"
    );
    drop(client);
    run.stop();
}

/// A client's typing that waits in Orbit's PTY queue, behind a child that
/// stopped reading, counts from when it reaches the PTY: lease input with a
/// quiet window is refused while it waits and within the window after it is
/// written.
fn typing_counts_once_it_reaches_the_pty() {
    let mut run = Driven::start();
    let mut client = run.attach();
    let typing = refused(Refusal::Typing);
    run.command("kill -STOP $cat_pid; printf '\\033]0;stopped\\007'");
    until_titled(&mut client, "stopped");
    // Far more than the PTY's own buffer, so most of it waits in Orbit.
    let text = vec![b'k'; 64 * 1024];
    client.send(&paste(&text));
    sleep(1.7);
    assert_eq!(quiet_pasted(&mut run, "x"), typing, "typing still queued");
    assert!(run.input().is_empty(), "the stopped child read");

    run.command("kill -CONT $cat_pid");
    assert_eq!(run.settled_input(0, text.len()), text, "the typing arrived");
    assert_eq!(
        quiet_pasted(&mut run, "x"),
        typing,
        "typing that just reached the PTY"
    );
    sleep(1.7);
    assert_eq!(quiet_pasted(&mut run, "x"), written(false), "after the window");
    assert_eq!(run.settled_input(text.len(), 1), b"x");
    drop(client);
    run.stop();
}
