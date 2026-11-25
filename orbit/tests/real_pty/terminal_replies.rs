use crate::support::*;
use std::collections::HashMap;
use std::time::Instant;

/// The bytes the child recorded under `name`.
fn reply(got: &HashMap<String, String>, name: &str) -> Vec<u8> {
    let quoted = got
        .get(name)
        .unwrap_or_else(|| panic!("no reply named {name}: {got:?}"));
    reply_bytes(quoted)
}

/// Asserts that `name` recorded exactly `parts`, joined, and nothing else.
fn exactly(got: &HashMap<String, String>, name: &str, parts: &[&[u8]]) {
    let (actual, expected) = (reply(got, name), parts.concat());
    assert!(
        actual == expected,
        "{name}: the child read\n  {}\nnot\n  {}",
        actual.escape_ascii(),
        expected.escape_ascii(),
    );
}

/// The hostile corpus of terminal-replies-shell.sh, recorded in `phase`:
/// every reply a program can trigger, each case read through a device
/// attributes answer `da`. No case returns program-chosen text except the
/// Kitty clipboard id, which libghostty reduces to letters, digits and
/// `-_+.`, at most 512 of them; nothing carries a control byte, whitespace,
/// quotes, shell syntax or the end of a bracketed paste (SECURITY-MODEL
/// finding 14, upstream question eon-z7ghx.4).
fn corpus(got: &HashMap<String, String>, phase: &str, da: &[u8], clipboard_write: &[u8]) {
    let case = |name: &str| format!("{phase}-{name}");
    exactly(got, &case("baseline"), &[da]);
    // libghostty leaves title reports off: a set title never returns.
    exactly(got, &case("title"), &[da]);
    // DECRQSS answers from typed state; requests it cannot hold are dropped.
    exactly(
        got,
        &case("decrqss"),
        &[b"\x1bP1$r0m\x1b\\", b"\x1bP0$r\x1b\\", da],
    );
    // XTGETTCAP answers known keys from a fixed table; Orbit sets no name.
    exactly(got, &case("tcap"), &[b"\x1bP1+r436F=323536\x1b\\", da]);
    // Colours return as numbers, never as the specification text.
    exactly(
        got,
        &case("colors"),
        &[
            b"\x1b]10;rgb:1212/3434/5656\x07",
            b"\x1b]4;1;rgb:cccc/6666/6666\x1b\\",
            b"\x1b]11;rgb:0000/0000/0000\x07",
            b"\x1b]12;rgb:1212/3434/5656\x07",
            b"\x1b]21;foreground=rgb:12/34/56\x1b\\",
            b"\x1b]21;background=rgb:00/00/00\x07",
            da,
        ],
    );
    // Fixed text and numbers, secondary and tertiary device attributes
    // included; answerback, colour scheme, checksum and the working
    // directory get no answer.
    exactly(
        got,
        &case("status"),
        &[
            b"\x1b[>1;0;0c",
            b"\x1bP!|00000000\x1b\\",
            b"\x1bP>|libghostty\x1b\\",
            b"\x1b[0n",
            b"\x1b[?31337;0$y",
            b"\x1b[?0u",
            da,
        ],
    );
    // OSC 52 reads get nothing: Orbit installs no clipboard read callback.
    let long_id = [b'A'; 512];
    exactly(
        got,
        &case("clipboard"),
        &[
            b"\x1b]5522;type=read:status=EPERM:id=xechoIFSPWNED-260mh15shx\x1b\\",
            b"\x1b]5522;type=read:status=EPERM:id=",
            &long_id,
            b"\x07\x1b]5522;type=write:status=",
            clipboard_write,
            b":id=xechoIFSPWNED-260mh15shx\x1b\\",
            da,
        ],
    );
    // Kitty graphics messages are fixed text; Glyph Protocol is off.
    exactly(got, &case("graphics"), &[b"\x1b_Gi=31;OK\x1b\\", da]);
    // libghostty's C API ignores Kitty drag and drop (OSC 72): no reply.
    exactly(got, &case("dnd"), &[da]);
    exactly(got, &case("stray"), &[]);
}

pub fn run() {
    let directory = TempDir::new();
    let gate = Gate::new(&directory, "reply-gate");
    let answers = directory.join("replies");
    let shell = script(&directory, "REPLY_SHELL", "terminal-replies-shell");
    let mut server = Server::start(
        &directory,
        &shell,
        &[
            ("ORBIT_REPLY_GATE", gate.path()),
            ("ORBIT_REPLIES", &answers),
        ],
    );
    // The terminal answers the child's queries while no client is attached:
    // Orbit, not a client, owns terminal replies.
    let deadline = Instant::now() + seconds(20.0);
    while !answers.exists() || !replies(&answers).contains_key("detached-stray") {
        assert!(
            Instant::now() < deadline,
            "the detached queries got no answer line"
        );
        assert!(server.alive(), "server exited before the child asked");
        sleep(0.05);
    }
    let got = replies(&answers);
    exactly(&got, "detached", &[b"\x1b[2;1R"]);
    // Device attributes name a VT100-family terminal: ESC [ ? digits and
    // semicolons c. Every case below expects these exact bytes last.
    let da = reply(&got, "detached-baseline");
    let attributes = da
        .strip_prefix(b"\x1b[?")
        .and_then(|rest| rest.strip_suffix(b"c"))
        .unwrap_or_else(|| panic!("{}", da.escape_ascii()));
    assert!(
        !attributes.is_empty()
            && attributes
                .iter()
                .all(|byte| byte.is_ascii_digit() || *byte == b';'),
        "{}",
        da.escape_ascii()
    );
    // Program status reports, a valid one holding shell syntax and one
    // rejected for its control bytes, are never answered, and the probe
    // after them gets only its fixed answer, before the device attributes
    // query that follows it (ORB-C15).
    exactly(
        &got,
        "probe",
        &[
            b"\x1b]7501;?:states=idle,working,done,blocked,error\
              :kinds=permission,question,auth\x1b\\",
            &da,
        ],
    );
    // Bend refuses clipboard writes while no client is attached.
    corpus(&got, "detached", &da, b"EPERM");

    let mut client = Peer::attach(&server.path, 10.0);
    gate.release();
    client.frame_titled("REPLIED");
    let got = replies(&answers);
    exactly(&got, "position", &[b"\x1b[2;1R"]);
    exactly(&got, "attributes", &[&da]);
    // An attached client takes clipboard writes; an empty one is unsupported.
    corpus(&got, "attached", &da, b"ENOSYS");
    gate.release();
    client.exited(6);
    drop(client);

    server.passed(6, 10.0);
}
