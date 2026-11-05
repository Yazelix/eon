use crate::kitty::size;
use crate::management_lease::{Reply, leased};
use crate::management_record::suffixed;
use crate::support::*;
use crate::visible_text::Driven;
use orbit_protocol::{
    management::{self, ClientMessage, HistoryText, Refusal, Screen, ServerMessage},
    session::{self, KeyAction},
};
use std::{
    fs,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

impl Driven {
    fn history(&mut self) -> HistoryText {
        history(self.lease.send(&ClientMessage::ReadHistory))
    }

    /// Reads history until the terminal's title is `title`, which the check's
    /// commands set last, so everything before it has been read.
    fn history_titled(&mut self, title: &str) -> HistoryText {
        let deadline = Instant::now() + seconds(20.0);
        loop {
            let text = self.history();
            if text.title == title {
                return text;
            }
            assert!(Instant::now() < deadline, "the title {title:?} never came");
            sleep(0.05);
        }
    }

    /// Resizes the terminal to 200 by 50 and fills the 16 MiB history: 16,000
    /// lines, each a five-digit number and `cell` to the full width, so the
    /// history budget evicts the earliest. Returns the lines written.
    fn fill_history(&mut self, cell: &str) -> Vec<String> {
        let mut client = self.attach();
        client.send(&size(200, 50, 8, 16));
        client.expect(session::ServerMessage::Accepted);
        drop(client);
        let lines: Vec<String> = (0..16_000)
            .map(|i| format!("{i:05}{}", cell.repeat(195)))
            .collect();
        let file = self.directory.join("history");
        fs::write(&file, lines.join("\r\n")).expect("history");
        self.command(&format!(
            "cat '{}'; printf '\\033]0;filled\\007'",
            file.display()
        ));
        self.until_titled("filled");
        lines
    }
}

fn history(reply: Reply) -> HistoryText {
    match reply {
        Reply::Message(ServerMessage::History(text)) => text,
        other => panic!("expected History, received {other:?}"),
    }
}

pub fn run() {
    reads_history_then_the_live_area();
    held_updates_delay_the_history();
    a_full_history_is_read_whole_and_in_time();
    a_holder_that_stops_reading_history_loses_the_lease();
    echo_elsewhere_goes_on_during_full_reads();
}

/// A history read returns the retained history and then the live area,
/// oldest first, one line per logical line: a soft-wrapped line joined,
/// trailing blanks trimmed, wide and combining text kept, with the title and
/// directory and the revision of the client's latest frame. It ignores and
/// keeps a client's scroll position, sends that client nothing and writes
/// nothing to the PTY. The alternate screen reads its own live area alone,
/// and the primary history returns with it.
fn reads_history_then_the_live_area() {
    let mut run = Driven::start();
    let mut client = run.attach();
    let wide = "w".repeat(100);
    run.command(&format!(
        "for i in $(seq 0 59); do printf 'h%d   \\r\\n' $i; done; \
         printf '\\033]7;file://host/tmp/history\\007{wide}\\r\\n漢字 e\u{301}\\r\\nend\
         \\033]0;history\\007'"
    ));
    let frame = client.frame_titled("history");
    let text = run.history();
    let mut lines: Vec<String> = (0..60).map(|i| format!("h{i}")).collect();
    lines.extend([wide.clone(), "漢字 e\u{301}".into(), "end".into()]);
    assert_eq!(text.text, lines.join("\n"));
    assert_eq!(
        (text.screen, text.rows, text.columns),
        (Screen::Primary, 24, 80)
    );
    assert_eq!(
        (text.omitted_lines, text.cut_bytes, text.lossy),
        (0, 0, true)
    );
    assert_eq!(text.title, "history");
    assert!(
        String::from_utf8_lossy(&text.directory).contains("/tmp/history"),
        "{:?}",
        text.directory
    );
    assert_eq!(
        text.revision, frame.revision,
        "the read's revision is not the latest frame's"
    );

    // A client scrolled into history leaves the read whole and in place.
    client.send(&scroll(frame.revision, -10));
    let (_, applied, scrolled) = client.scroll_outcome();
    assert_eq!(applied, -10);
    assert_eq!(run.history().text, lines.join("\n"));
    client.quiet(0.3, "a history read sent the client");
    client.send(&scroll(scrolled.revision, 10));
    assert_eq!(
        client.scroll_outcome().1,
        10,
        "a history read moved the client's viewport"
    );
    assert!(run.input().is_empty(), "a history read wrote to the PTY");
    drop(client);

    run.command("printf '\\033[?1049h\\033[Halternate\\033]0;alternate\\007'");
    let alternate = run.history_titled("alternate");
    assert_eq!(alternate.screen, Screen::Alternate);
    assert_eq!(alternate.text, "alternate");
    run.command("printf '\\033[?1049l\\033]0;primary\\007'");
    let primary = run.history_titled("primary");
    assert_eq!(primary.screen, Screen::Primary);
    assert_eq!(primary.text, lines.join("\n"));
    run.stop();
}

/// A history read during a held synchronized update waits for its release
/// and sees the whole update; a request sent meanwhile is in flight beside it
/// and refused.
fn held_updates_delay_the_history() {
    let mut run = Driven::start();
    run.command("printf 'before\\r\\n\\033[?2026hpartial'");
    sleep(0.3);
    run.lease.set_timeout(0.3);
    assert_eq!(
        run.lease.send(&ClientMessage::ReadHistory),
        Reply::Timeout,
        "a history read answered during a held update"
    );
    assert_eq!(
        run.lease.send(&ClientMessage::ReadVisible),
        Reply::Message(ServerMessage::Refused(Refusal::InvalidRequest)),
        "a read during a held history read"
    );
    run.command("printf ' complete\\033[?2026l'");
    run.lease.set_timeout(3.0);
    let text = history(run.lease.read());
    assert_eq!(text.text, "before\npartial complete");
    run.stop();
}

/// A history the 16 MiB budget fills with four-byte characters at 200
/// columns reads whole within the 8 MiB bound and well within the 2 s a lease
/// holder waits: the text is the newest lines written, contiguous, and the
/// earliest written are evicted, not omitted.
fn a_full_history_is_read_whole_and_in_time() {
    let mut run = Driven::start();
    let lines = run.fill_history("\u{1d11e}");
    let asked = Instant::now();
    let text = run.history();
    let took = asked.elapsed();
    let first = lines.len() - text.text.lines().count();
    eprintln!(
        "full history: {} bytes, {} lines, {} omitted, {} evicted, read in {took:?}",
        text.text.len(),
        lines.len() - first,
        text.omitted_lines,
        first - text.omitted_lines as usize,
    );
    assert!(took < seconds(2.0), "a full history read took {took:?}");
    assert!(text.text.len() <= management::MAX_HISTORY_TEXT_BYTES);
    assert!(first > 0, "the budget evicted nothing");
    assert_eq!((text.omitted_lines, text.cut_bytes), (0, 0));
    assert!(
        text.text == lines[first..].join("\n"),
        "the history is not the newest lines written"
    );
    assert_eq!((text.rows, text.columns), (50, 200));
    run.stop();
}

/// A holder that leaves a full-budget history answer of 2 to 4 MB unread
/// loses the lease within the one-second bound, while the PTY and the
/// attached client go on: the child sets the title only once the answer
/// begins to arrive, and that title reaches the client within the bound.
fn a_holder_that_stops_reading_history_loses_the_lease() {
    let mut run = Driven::start();
    run.fill_history("é");
    let size = run.history().text.len();
    eprintln!("unread history answer: {size} bytes");
    assert!(
        (2_000_000..=4_000_000).contains(&size),
        "the answer is {size} bytes"
    );
    let mut client = run.attach();
    run.lease.post(&ClientMessage::ReadHistory);
    let asked = Instant::now();
    run.lease.await_answer();
    let answered = asked.elapsed();
    run.command("printf '\\033]0;unblocked\\007'");
    client.frame_titled("unblocked");
    let titled = asked.elapsed();
    assert!(
        titled < seconds(1.0),
        "the unread history held the PTY or the client for {titled:?}; \
         the answer began after {answered:?}"
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

/// Keystroke echo in another terminal goes on while one terminal answers
/// full history reads back to back: no echo waits as long as the slowest
/// read, and no read takes 2 s, the time a lease holder waits. Separate Orbit
/// runs share only the machine, so this bounds their contention. The check
/// prints the idle and loaded echo times and the read times, which bound how
/// long a read holds its own terminal's owner loop.
fn echo_elsewhere_goes_on_during_full_reads() {
    let mut run = Driven::start();
    run.fill_history("\u{1d11e}");
    let directory = TempDir::new();
    let shell = executable(
        &directory,
        "echo.sh",
        &format!(
            "#!{}\nstty raw -echo\nn=0\n\
             while IFS= read -r -n1 _; do n=$((n+1)); printf '\\033]0;echo %d\\007' $n; done\n",
            bash().display()
        ),
    );
    let echo = Server::start(&directory, &shell, &[]);
    let mut client = Peer::attach(&echo.path, 4.0);
    client.frame();
    let mut typed = 0;
    let mut echoes = |count: usize| -> Vec<Duration> {
        (0..count)
            .map(|_| {
                typed += 1;
                let sent = Instant::now();
                client.send(&key('a', KeyAction::Press));
                client.expect(session::ServerMessage::Accepted);
                client.frame_titled(&format!("echo {typed}"));
                let took = sent.elapsed();
                client.send(&key('a', KeyAction::Release));
                client.expect(session::ServerMessage::Accepted);
                sleep(0.02);
                took
            })
            .collect()
    };
    let idle = echoes(20);

    let (stop, stopped) = mpsc::channel::<()>();
    let reader = thread::spawn(move || {
        let mut reads = Vec::new();
        while stopped.try_recv().is_err() {
            let asked = Instant::now();
            let text = run.history();
            reads.push(asked.elapsed());
            assert!(!text.text.is_empty());
        }
        run.stop();
        reads
    });
    sleep(0.2);
    let loaded = echoes(40);
    stop.send(()).expect("stop the reads");
    let reads = reader.join().expect("the reads");

    let slowest = |times: &[Duration]| times.iter().max().copied().unwrap_or_default();
    let median = |times: &[Duration]| {
        let mut sorted = times.to_vec();
        sorted.sort();
        sorted[sorted.len() / 2]
    };
    eprintln!(
        "echo idle: median {:?}, max {:?}; during {} full reads (median {:?}, max {:?}): \
         median {:?}, max {:?}; load {}",
        median(&idle),
        slowest(&idle),
        reads.len(),
        median(&reads),
        slowest(&reads),
        median(&loaded),
        slowest(&loaded),
        fs::read_to_string("/proc/loadavg").unwrap_or_default().trim(),
    );
    assert!(reads.len() >= 2, "only {} reads ran", reads.len());
    assert!(
        slowest(&reads) < seconds(2.0),
        "a full read took {:?}",
        slowest(&reads)
    );
    assert!(
        slowest(&loaded) < slowest(&reads),
        "echo stalled for {:?} beside reads of at most {:?}",
        slowest(&loaded),
        slowest(&reads)
    );
    drop(client);
}
