use crate::kitty::size;
use crate::management_lease::{Manager, Reply, leased};
use crate::management_record::{suffixed, tombstone};
use crate::support::*;
use orbit_protocol::{
    management::{
        self, ClientMessage, LeaseInput, ProcessOutcome, Refusal, Screen, ServerMessage,
        TerminationReason, VisibleText,
    },
    session,
};
use std::{fs, io::Write, path::PathBuf, time::Instant};

const MANAGED: [&str; 4] = ["--management-v4", "session-5", "run-5", "component-5"];

/// A managed run whose bash child, in raw mode without echo, runs each
/// command line the check sends through a FIFO while `cat` copies the
/// terminal's input to a file, with a lease on it.
pub struct Driven {
    // Fields drop in order: the server stops before its directory goes.
    pub server: Server,
    pub lease: Manager,
    commands: Gate,
    input: PathBuf,
    directory: TempDir,
}

impl Driven {
    pub fn start() -> Self {
        let directory = TempDir::new();
        let commands = Gate::new(&directory, "commands");
        let input = directory.join("input");
        let shell = executable(
            &directory,
            "child.sh",
            &format!(
                "#!{}\nstty raw -echo\nexec 3<>\"$ORBIT_COMMANDS\" 4<&0\n\
                 cat <&4 > \"$ORBIT_INPUT\" & cat_pid=$!\nexec 4<&-\n\
                 while IFS= read -r line <&3; do eval \"$line\"; done\n",
                bash().display()
            ),
        );
        let server = Server::launch(
            &directory,
            &[shell.as_os_str()],
            &[("ORBIT_COMMANDS", commands.path()), ("ORBIT_INPUT", &input)],
            &[],
            &MANAGED,
        );
        let record = suffixed(&server.path, ".record");
        wait_for(&record);
        wait_for(&input);
        Self {
            server,
            lease: leased(&record),
            commands,
            input,
            directory,
        }
    }

    /// Has the child run one command line.
    pub fn command(&self, line: &str) {
        let mut fifo = fs::OpenOptions::new()
            .write(true)
            .open(self.commands.path())
            .expect("open commands");
        fifo.write_all(format!("{line}\n").as_bytes())
            .expect("command");
    }

    /// Names this process and attaches it as the interactive client, past
    /// its first frame.
    pub fn attach(&mut self) -> Peer {
        self.lease.allow_self();
        let mut client = Peer::attach(&self.server.path, 4.0);
        client.frame();
        client
    }

    fn read(&mut self) -> VisibleText {
        visible(self.lease.send(&ClientMessage::ReadVisible))
    }

    /// Reads until the text holds `needle`.
    fn until_visible(&mut self, needle: &str) -> VisibleText {
        let deadline = Instant::now() + seconds(10.0);
        loop {
            let text = self.read();
            if text.text.contains(needle) {
                return text;
            }
            assert!(Instant::now() < deadline, "{needle:?} never appeared");
            sleep(0.02);
        }
    }

    /// Reads until the terminal's title is `title`, which the check's
    /// commands set last, so everything before it has been read.
    pub fn until_titled(&mut self, title: &str) -> VisibleText {
        let deadline = Instant::now() + seconds(10.0);
        loop {
            let text = self.read();
            if text.title == title {
                return text;
            }
            assert!(Instant::now() < deadline, "the title {title:?} never came");
            sleep(0.02);
        }
    }

    /// Everything the terminal's input carried to the child.
    pub fn input(&self) -> Vec<u8> {
        fs::read(&self.input).expect("input")
    }

    /// The input after the first `from` bytes, once it holds `len` bytes and
    /// nothing more arrives for 0.2 seconds.
    pub fn settled_input(&self, from: usize, len: usize) -> Vec<u8> {
        let deadline = Instant::now() + seconds(10.0);
        let mut seen = 0;
        loop {
            let input = self.input();
            if input.len() >= from + len && input.len() == seen {
                return input[from..].to_vec();
            }
            assert!(
                Instant::now() < deadline,
                "input stopped at {} bytes",
                input.len()
            );
            seen = input.len();
            sleep(0.2);
        }
    }

    /// Stops the run through the lease: the lease hears Stopped and the
    /// server exits 0.
    pub fn stop(mut self) {
        let reply = self.lease.send(&ClientMessage::Stop);
        assert!(
            matches!(reply, Reply::Message(ServerMessage::Stopped(_))),
            "{reply:?}"
        );
        self.server.communicate(10.0).assert_silent(0);
    }
}

fn visible(reply: Reply) -> VisibleText {
    match reply {
        Reply::Message(ServerMessage::Visible(text)) => text,
        other => panic!("expected Visible, received {other:?}"),
    }
}

pub fn run() {
    reads_the_live_area();
    long_text_keeps_its_last_rows();
    held_updates_delay_the_read();
    a_read_ends_with_the_run();
    reads_keep_up_with_a_flood();
    requests_wait_for_the_whole_answer();
    a_holder_that_stops_reading_loses_the_lease();
    a_stop_finishes_the_answer_first();
}

/// Fills a 400 by 250 terminal with rows whose live area is cut to 1 MiB, an
/// answer far larger than a socket's buffer, and returns the rows it keeps.
fn fill(run: &mut Driven) -> Vec<String> {
    let mut client = run.attach();
    client.send(&size(400, 250, 8, 16));
    client.expect(session::ServerMessage::Accepted);
    drop(client);
    let cell = format!("e{}", "\u{301}".repeat(5));
    let rows: Vec<String> = (0..250)
        .map(|i| format!("{i:04}{}", cell.repeat(396)))
        .collect();
    let file = run.directory.join("rows");
    fs::write(&file, rows.join("\r\n")).expect("rows");
    run.command(&format!(
        "cat '{}'; printf '\\033]0;cut\\007'",
        file.display()
    ));
    run.until_titled("cut");
    rows[10..].to_vec()
}

/// A read returns the active screen's live area, primary or alternate, as one
/// trimmed line per row, soft wraps as line breaks, wide and combining text
/// kept, with the title and directory an observer hears and the revision of
/// the client's latest frame. It needs no attachment, ignores and keeps a
/// client's scroll position, sends that client nothing and writes nothing to
/// the PTY.
fn reads_the_live_area() {
    let mut run = Driven::start();
    let wide = "w".repeat(100);
    run.command(&format!(
        "printf '\\033]0;read title\\007\\033]7;file://host/tmp/read\\007\
         plain   \\r\\n漢字 e\u{301}\\r\\n{wide}\\r\\nend'"
    ));
    let text = run.until_visible("end");
    let mut rows = vec!["plain", "漢字 e\u{301}", &wide[..80], &wide[80..], "end"];
    rows.resize(24, "");
    assert_eq!(text.text, rows.join("\n"));
    assert_eq!(
        (text.screen, text.rows, text.columns),
        (Screen::Primary, 24, 80)
    );
    assert_eq!((text.omitted_rows, text.lossy), (0, true));
    let mut observer = Peer::connect(&run.server.path, 4.0);
    observer.send(&session::ClientMessage::ObserveMetadata);
    observer.expect(session::ServerMessage::ObservingMetadata);
    let metadata = observer.metadata();
    assert_eq!(text.title, metadata.title);
    assert_eq!(text.directory, metadata.working_directory.as_bytes());
    assert_eq!(text.title, "read title");
    assert!(
        metadata.working_directory.contains("/tmp/read"),
        "{metadata:?}"
    );

    // The alternate screen reads as such, and the primary returns intact.
    run.command("printf '\\033[?1049h\\033[Halternate'");
    let alternate = run.until_visible("alternate");
    assert_eq!(alternate.screen, Screen::Alternate);
    assert_eq!(alternate.text, format!("alternate{}", "\n".repeat(23)));
    run.command("printf '\\033[?1049l'");
    let deadline = Instant::now() + seconds(10.0);
    while run.read().screen != Screen::Primary {
        assert!(
            Instant::now() < deadline,
            "the primary screen never returned"
        );
        sleep(0.02);
    }
    assert_eq!(run.read().text, rows.join("\n"));

    // A client scrolled into history leaves the read on the live area.
    let mut client = run.attach();
    run.command(
        "for i in $(seq 0 39); do printf '\\r\\nh%d' $i; done; printf '\\033]0;history\\007'",
    );
    let frame = client.frame_titled("history");
    let read = run.read();
    assert_eq!(
        read.revision, frame.revision,
        "the read's revision is not the latest frame's"
    );
    let live: Vec<String> = (16..40).map(|i| format!("h{i}")).collect();
    assert_eq!(read.text, live.join("\n"));
    client.send(&scroll(frame.revision, -10));
    let (_, applied, scrolled) = client.scroll_outcome();
    assert_eq!(applied, -10);
    assert_eq!(run.read().text, live.join("\n"));
    client.quiet(0.3, "a read sent the client");
    client.send(&scroll(scrolled.revision, 10));
    assert_eq!(
        client.scroll_outcome().1,
        10,
        "a read moved the client's viewport"
    );
    assert!(run.input().is_empty(), "a read wrote to the PTY");
    drop(client);
    drop(observer);
    run.stop();
}

/// A live area over 1 MiB keeps its last whole rows that fit and counts the
/// rest: 250 rows of 400 cells, each an `e` with five combining accents, are
/// 4,360 bytes a row, so 240 rows fit and 10 are omitted.
fn long_text_keeps_its_last_rows() {
    let mut run = Driven::start();
    let kept = fill(&mut run);
    let text = run.read();
    assert_eq!((text.rows, text.columns), (250, 400));
    assert_eq!(text.omitted_rows, 10);
    assert!(text.text.len() <= management::MAX_VISIBLE_TEXT_BYTES);
    assert!(text.text == kept.join("\n"), "the kept rows differ");
    run.stop();
}

/// A request sent while a large answer is still being written is in flight
/// beside it: it is refused after the whole answer, and nothing of it runs.
fn requests_wait_for_the_whole_answer() {
    let mut run = Driven::start();
    let kept = fill(&mut run);
    let before = run.input().len();
    run.lease.post(&ClientMessage::ReadVisible);
    // The answer outgrows the socket's buffer, so it waits, partly written,
    // until the holder reads it.
    sleep(0.2);
    run.lease
        .post(&ClientMessage::Input(LeaseInput::Paste("in flight".into())));
    sleep(0.2);
    let text = visible(run.lease.read());
    assert!(text.text == kept.join("\n"), "the answer differs");
    assert_eq!(
        run.lease.read(),
        Reply::Message(ServerMessage::Refused(Refusal::InvalidRequest)),
        "a request sent during an answer"
    );
    sleep(0.3);
    assert_eq!(
        run.input().len(),
        before,
        "a request sent during an answer was written"
    );
    let reply = run.lease.send(&ClientMessage::Status);
    assert!(
        matches!(reply, Reply::Message(ServerMessage::Status(_))),
        "the lease ended: {reply:?}"
    );
    run.stop();
}

/// A holder that stops reading a large answer loses the connection, and the
/// lease with it, within the one-second bound, while the PTY and the attached
/// client go on; another connection then takes the lease.
fn a_holder_that_stops_reading_loses_the_lease() {
    let mut run = Driven::start();
    fill(&mut run);
    let mut client = run.attach();
    run.lease.post(&ClientMessage::ReadVisible);
    let asked = Instant::now();
    run.command("printf '\\033]0;unblocked\\007'");
    client.frame_titled("unblocked");
    assert!(
        asked.elapsed() < seconds(1.0),
        "the unread answer held the PTY or the client for {:?}",
        asked.elapsed()
    );
    let record = suffixed(&run.server.path, ".record");
    let next = leased(&record);
    let lost = asked.elapsed();
    assert!(lost >= seconds(0.9), "the lease went after {lost:?}");
    assert!(lost < seconds(2.5), "the lease stayed for {lost:?}");
    assert_eq!(
        run.lease.read(),
        Reply::Closed,
        "the slow holder's connection"
    );
    drop(client);
    run.lease = next;
    run.stop();
}

/// A termination signal while a large answer is partly written finishes that
/// answer before the lease hears Stopped.
fn a_stop_finishes_the_answer_first() {
    let mut run = Driven::start();
    let kept = fill(&mut run);
    run.lease.post(&ClientMessage::ReadVisible);
    sleep(0.2);
    signal(run.server.pid(), libc::SIGTERM);
    let text = visible(run.lease.read());
    assert!(text.text == kept.join("\n"), "the answer differs");
    let reply = run.lease.read();
    assert!(
        matches!(reply, Reply::Message(ServerMessage::Stopped(_))),
        "{reply:?}"
    );
    run.server.communicate(10.0).assert_silent(0);
}

/// A read during a held synchronized update waits for its release, or the
/// watchdog's, and sees the whole update.
fn held_updates_delay_the_read() {
    let mut run = Driven::start();
    run.command("printf before");
    run.until_visible("before");
    run.command("printf '\\033[?2026h\\033[2J\\033[Hpartial'");
    sleep(0.2);
    run.lease.set_timeout(0.3);
    assert_eq!(
        run.lease.send(&ClientMessage::ReadVisible),
        Reply::Timeout,
        "a read answered during a held update"
    );
    // A request sent while the read waits is in flight beside it: it is
    // refused at once, and the read stays held, so it still sees the whole
    // update, well before the one-second watchdog.
    assert_eq!(
        run.lease.send(&ClientMessage::Status),
        Reply::Message(ServerMessage::Refused(Refusal::InvalidRequest)),
        "a request during a held read"
    );
    assert_eq!(
        run.lease.send(&ClientMessage::Stop),
        Reply::Message(ServerMessage::Refused(Refusal::InvalidRequest)),
        "a Stop during a held read"
    );
    run.command("printf ' complete\\033[?2026l'");
    run.lease.set_timeout(3.0);
    let text = visible(run.lease.read());
    assert!(
        text.text.starts_with("partial complete\n"),
        "{:?}",
        text.text
    );
    let reply = run.lease.send(&ClientMessage::Status);
    assert!(
        matches!(reply, Reply::Message(ServerMessage::Status(_))),
        "the lease ended: {reply:?}"
    );

    run.command("printf '\\033[?2026h\\033[2J\\033[Hwatchdog'");
    sleep(0.2);
    let asked = Instant::now();
    let text = visible(run.lease.send(&ClientMessage::ReadVisible));
    let waited = asked.elapsed().as_secs_f64();
    assert!((0.5..3.0).contains(&waited), "{waited}");
    assert!(text.text.starts_with("watchdog\n"), "{:?}", text.text);
    run.stop();
}

/// A read held for a synchronized update ends with the run, with no text,
/// whether a termination signal stops the run or the child exits; lease
/// input after the run ended reaches nothing.
fn a_read_ends_with_the_run() {
    for exits in [false, true] {
        let mut run = Driven::start();
        run.command("printf '\\033[?2026hheld'; read -r _ <&3; exit 3");
        sleep(0.3);
        run.lease.set_timeout(0.5);
        assert_eq!(run.lease.send(&ClientMessage::ReadVisible), Reply::Timeout);
        if exits {
            run.command("");
        } else {
            signal(run.server.pid(), libc::SIGTERM);
        }
        run.lease.set_timeout(5.0);
        let reply = run.lease.read();
        assert!(
            !matches!(reply, Reply::Message(ServerMessage::Visible(_))),
            "a held read was answered as the run ended: {reply:?}"
        );
        let record = suffixed(&run.server.path, ".record");
        run.server
            .communicate(10.0)
            .assert_silent(if exits { 3 } else { 0 });
        let ended = tombstone(&record);
        assert_eq!(
            (ended.reason, ended.outcome),
            if exits {
                (TerminationReason::NaturalExit, ProcessOutcome::ExitCode(3))
            } else {
                (
                    TerminationReason::ExplicitStop,
                    ProcessOutcome::Signal(libc::SIGHUP),
                )
            },
            "{ended:?}"
        );
        let reply = run.lease.send(&ClientMessage::Input(LeaseInput::Enter));
        assert_eq!(reply, Reply::Closed, "lease input after the run ended");
    }
}

/// Reads stay prompt and whole while the child floods the terminal: each
/// answers within a second with one line per row and no control character but
/// line feeds, revisions never go back, and the flood moves on between them.
fn reads_keep_up_with_a_flood() {
    let mut run = Driven::start();
    run.command("(i=0; while :; do printf 'flood %d\\r\\n' $i; i=$((i+1)); done) &");
    run.until_visible("flood");
    let mut revision = 0;
    let mut texts = Vec::new();
    for _ in 0..50 {
        let asked = Instant::now();
        let text = run.read();
        assert!(
            asked.elapsed() < seconds(1.0),
            "a read took {:?}",
            asked.elapsed()
        );
        assert_eq!(text.text.matches('\n').count(), 23, "{:?}", text.text);
        assert!(!text.text.chars().any(|c| c.is_control() && c != '\n'));
        assert!(text.revision >= revision, "the revision went back");
        revision = text.revision;
        texts.push(text.text);
        sleep(0.01);
    }
    assert_ne!(
        texts.first(),
        texts.last(),
        "the flood stalled during the reads"
    );
    run.stop();
}
