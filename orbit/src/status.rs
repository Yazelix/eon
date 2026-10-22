//! The program status records of ORB-C15 (docs/CONTRACTS.md): what each
//! program in the terminal last reported with OSC 7501. libghostty parses
//! the reports and enforces the specification's limits and discard rules,
//! and answers the support probe. The host keeps one record per id, and
//! Bend's `StatusLifetime` decides when a PTY turn drops the working and
//! blocked ones (src/status.bend).

use libghostty_vt::terminal::{
    ProgramStatus, ProgramStatusKind, ProgramStatusState, SemanticPrompt, SemanticPromptKind,
    SemanticPromptPromptKind,
};
use std::{collections::HashMap, time::Instant};

/// The most records a terminal keeps; a new record past it evicts the one
/// updated longest ago.
const MAX_RECORDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

/// One accepted report, its text disarmed.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "management reads records from eon-agent-sidebar-yx8.5 on"
    )
)]
struct Report {
    id: Vec<u8>,
    state: State,
    kind: ProgramStatusKind,
    progress: Option<u8>,
    app: Vec<u8>,
    title: String,
    message: String,
}

/// What a PTY turn's output says about program status, in stream order.
enum Event {
    Report(Report),
    /// Clears the record with this id and every record beneath it; an empty
    /// id clears every record.
    Clear(Vec<u8>),
    /// A primary prompt start: OSC 133 A, or N or P without another prompt
    /// kind.
    Mark,
}

/// The events of the current PTY turn, which libghostty's callbacks collect
/// while the host feeds the turn's output. Their memory grows with the turn's
/// output, which is at most 512 KiB, and is freed when the turn is applied or
/// the next one starts.
#[derive(Default)]
pub(crate) struct Turn {
    events: Vec<Event>,
}

impl Turn {
    /// Collects a report that libghostty accepted.
    pub(crate) fn report(&mut self, report: ProgramStatus<'_>) {
        let state = match report.state() {
            Ok(ProgramStatusState::Clear) => {
                self.events.push(Event::Clear(report.id().to_vec()));
                return;
            }
            Ok(ProgramStatusState::Idle) => State::Idle,
            Ok(ProgramStatusState::Working) => State::Working,
            Ok(ProgramStatusState::Done) => State::Done,
            Ok(ProgramStatusState::Blocked) => State::Blocked,
            Ok(ProgramStatusState::Error) => State::Error,
            // A state added after this version of libghostty.
            Ok(_) | Err(_) => return,
        };
        self.events.push(Event::Report(Report {
            id: report.id().to_vec(),
            state,
            kind: report.kind(),
            progress: report.progress(),
            app: report.app().to_vec(),
            title: disarmed(report.title()),
            message: disarmed(report.message()),
        }));
    }

    /// Collects a primary prompt start; adjacent ones count once.
    pub(crate) fn prompt(&mut self, event: SemanticPrompt<'_>) {
        if matches!(event.kind(), Ok(SemanticPromptKind::PromptStart))
            && matches!(event.prompt_kind(), Ok(SemanticPromptPromptKind::Primary))
            && !matches!(self.events.last(), Some(Event::Mark))
        {
            self.events.push(Event::Mark);
        }
    }

    /// Whether the turn carried a prompt mark.
    pub(crate) fn marks(&self) -> bool {
        self.events.iter().any(|event| matches!(event, Event::Mark))
    }
}

/// One record and its neutral facts. Orbit derives no attention meaning.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "management reads records from eon-agent-sidebar-yx8.5 on"
    )
)]
struct Record {
    report: Report,
    /// The update sequence of its last report.
    updated: u64,
    /// The update sequence at which its state last changed; creating the
    /// record counts as a change.
    changed: u64,
    /// The state before that change, or None for a new record.
    previous: Option<State>,
    updated_at: Instant,
    changed_at: Instant,
}

/// A terminal's program status records.
#[derive(Default)]
pub(crate) struct Records {
    records: HashMap<Vec<u8>, Record>,
    /// Increases by one on each accepted report and clear, and on each
    /// lifetime drop that removes a record, so a reader sees removals.
    update: u64,
}

impl Records {
    /// Carries out Bend's `StatusLifetime` for a turn: with `edge`, the
    /// foreground return drops the working and blocked records before any of
    /// the turn's output; then the turn's reports and clears apply in stream
    /// order, and with `marks` each prompt mark drops the working and blocked
    /// records that exist at that point.
    pub(crate) fn apply(&mut self, turn: Turn, edge: bool, marks: bool) {
        let now = Instant::now();
        if edge {
            self.drop_active();
        }
        for event in turn.events {
            match event {
                Event::Report(report) => self.put(report, now),
                Event::Clear(id) => self.clear(&id),
                Event::Mark if marks => self.drop_active(),
                Event::Mark => {}
            }
        }
    }

    /// A report replaces its record. A same-state report keeps the record's
    /// change sequence and previous state.
    fn put(&mut self, report: Report, now: Instant) {
        self.update += 1;
        let update = self.update;
        let (changed, previous, changed_at) = match self.records.remove(&report.id) {
            Some(old) if old.report.state == report.state => {
                (old.changed, old.previous, old.changed_at)
            }
            Some(old) => (update, Some(old.report.state), now),
            None => (update, None, now),
        };
        if self.records.len() == MAX_RECORDS
            && let Some(oldest) = self
                .records
                .iter()
                .min_by_key(|(_, record)| record.updated)
                .map(|(id, _)| id.clone())
        {
            self.records.remove(&oldest);
        }
        self.records.insert(
            report.id.clone(),
            Record {
                report,
                updated: update,
                changed,
                previous,
                updated_at: now,
                changed_at,
            },
        );
    }

    fn clear(&mut self, id: &[u8]) {
        self.update += 1;
        self.records.retain(|key, _| !beneath(key, id));
    }

    /// Drops the working and blocked records, at a prompt mark or a
    /// foreground return.
    fn drop_active(&mut self) {
        let before = self.records.len();
        self.records
            .retain(|_, record| !matches!(record.report.state, State::Working | State::Blocked));
        if self.records.len() < before {
            self.update += 1;
        }
    }

    /// The app a record shows: its own, or else that of its nearest ancestor
    /// that has one, up to the root record.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "management reads records from eon-agent-sidebar-yx8.5 on"
        )
    )]
    fn app(&self, mut id: &[u8]) -> &[u8] {
        loop {
            if let Some(record) = self.records.get(id)
                && !record.report.app.is_empty()
            {
                return &record.report.app;
            }
            if id.is_empty() {
                return &[];
            }
            id = id
                .iter()
                .rposition(|&byte| byte == b'/')
                .map_or(&[], |slash| &id[..slash]);
        }
    }
}

/// Whether the record `key` is `id` or beneath it; every record is beneath
/// the empty id.
fn beneath(key: &[u8], id: &[u8]) -> bool {
    id.is_empty()
        || key
            .strip_prefix(id)
            .is_some_and(|rest| rest.first().is_none_or(|&byte| byte == b'/'))
}

/// Program text without the invisible characters that could disguise it
/// outside the terminal, such as bidi overrides and zero-width spaces.
fn disarmed(text: &[u8]) -> String {
    String::from_utf8_lossy(text)
        .chars()
        .filter(|&character| !invisible(character))
        .collect()
}

/// Unicode's format characters (Cf) and the invisible Khmer vowels, as
/// herdr 143bf5fd filters status text (src/app/tab_bar_status.rs).
fn invisible(character: char) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::Terminal;
    use std::{cell::RefCell, rc::Rc};

    const MARK: &str = "\x1b]133;A\x1b\\";

    /// A terminal whose program status events collect in a turn, as in
    /// `Orbit::spawn`.
    struct Fixture {
        terminal: Terminal<'static, 'static>,
        turn: Rc<RefCell<Turn>>,
        records: Records,
    }

    impl Fixture {
        fn new() -> Self {
            let turn = Rc::new(RefCell::new(Turn::default()));
            let (reports, prompts) = (Rc::clone(&turn), Rc::clone(&turn));
            let mut terminal = Terminal::new(80, 24).unwrap();
            terminal
                .on_program_status(move |_, report| reports.borrow_mut().report(report))
                .unwrap()
                .on_semantic_prompt(move |_, event| prompts.borrow_mut().prompt(event))
                .unwrap();
            Self {
                terminal,
                turn,
                records: Records::default(),
            }
        }

        /// Feeds one PTY turn's reads and applies it as Bend decides, with
        /// `edge` for its foreground sample; returns whether it marked.
        fn turn(&mut self, reads: &[&str], edge: bool) -> bool {
            for read in reads {
                self.terminal.vt_write(read.as_bytes());
            }
            let turn = self.turn.take();
            let marks = turn.marks();
            self.records.apply(turn, edge, marks);
            marks
        }

        fn record(&self, id: &str) -> &Record {
            &self.records.records[id.as_bytes()]
        }

        fn ids(&self) -> Vec<&str> {
            let mut ids: Vec<_> = self
                .records
                .records
                .keys()
                .map(|id| std::str::from_utf8(id).unwrap())
                .collect();
            ids.sort_unstable();
            ids
        }
    }

    fn report(pairs: &str) -> String {
        format!("\x1b]7501;{pairs}\x1b\\")
    }

    #[test]
    fn drops_apply_in_stream_order() {
        let mut fixture = Fixture::new();
        let first = [
            report("state=working:id=a"),
            report("state=done:id=d"),
            report("state=blocked:kind=permission:id=b"),
        ]
        .concat();
        assert!(!fixture.turn(&[&first], false));
        assert_eq!(fixture.ids(), ["a", "b", "d"]);
        assert_eq!(
            fixture.record("b").report.kind,
            ProgramStatusKind::Permission
        );

        // A foreground return drops the working and blocked records before
        // the turn's own report.
        assert!(!fixture.turn(&[&report("state=working:id=c")], true));
        assert_eq!(fixture.ids(), ["c", "d"]);

        // A prompt mark drops what exists at that point in the stream, even
        // within one read; a report after it survives.
        let marked = [
            report("state=working:id=e"),
            MARK.into(),
            report("state=working:id=f"),
        ];
        assert!(fixture.turn(&[&marked.concat()], false));
        assert_eq!(fixture.ids(), ["d", "f"]);
        // Three reports, one edge drop, one report, two reports and one mark
        // drop.
        assert_eq!(fixture.records.update, 8);

        // OSC 133 N starts a primary prompt too; a secondary prompt and the
        // other steps of a command do not mark.
        assert!(fixture.turn(&["\x1b]133;N\x1b\\"], false));
        assert_eq!(fixture.ids(), ["d"]);
        let unmarked = "\x1b]133;P;k=s\x1b\\\x1b]133;B\x1b\\\x1b]133;C\x1b\\\x1b]133;D;0\x1b\\";
        assert!(!fixture.turn(&[unmarked], false));
        // A drop that removes nothing is no update.
        assert!(fixture.turn(&[MARK], false));
        assert_eq!(fixture.records.update, 9);
    }

    #[test]
    fn records_keep_neutral_facts() {
        let mut fixture = Fixture::new();
        fixture.turn(&[&report("state=working:id=x:progress=40")], false);
        let created = fixture.record("x");
        assert_eq!(
            (created.updated, created.changed, created.previous),
            (1, 1, None)
        );
        let created_at = created.changed_at;
        assert_eq!(created.report.progress, Some(40));

        // A same-state report replaces the record but keeps its change.
        fixture.turn(&[&report("state=working:id=x")], false);
        let repeated = fixture.record("x");
        assert_eq!(
            (repeated.updated, repeated.changed, repeated.previous),
            (2, 1, None)
        );
        assert_eq!(repeated.changed_at, created_at);
        assert!(repeated.updated_at >= created_at);
        assert_eq!(repeated.report.progress, None);

        fixture.turn(&[&report("state=done:id=x")], false);
        let done = fixture.record("x");
        assert_eq!(
            (done.updated, done.changed, done.previous),
            (3, 3, Some(State::Working))
        );
    }

    #[test]
    fn clears_remove_subtrees_and_reset_removes_all() {
        let mut fixture = Fixture::new();
        // A report without an id is about the root record; an empty id is
        // invalid and discards its report, so the root stays done.
        let reports = [
            "state=done",
            "state=working:id=",
            "state=done:id=build",
            "state=done:id=build/test",
            "state=done:id=builder",
        ]
        .map(report)
        .concat();
        fixture.turn(&[&reports], false);
        assert_eq!(fixture.ids(), ["", "build", "build/test", "builder"]);
        assert_eq!(fixture.record("").report.state, State::Done);

        fixture.turn(&[&report("state=clear:id=build")], false);
        assert_eq!(fixture.ids(), ["", "builder"]);

        // RIS reaches the store as a clear of every record.
        fixture.turn(&["\x1bc"], false);
        assert_eq!(fixture.ids(), Vec::<&str>::new());
        assert_eq!(fixture.records.update, 6);
    }

    #[test]
    fn apps_inherit_and_text_is_disarmed() {
        let mut fixture = Fixture::new();
        // The title is "Build \u{202e}evil\u{200b}"; the message is "Plan".
        let reports = [
            report("state=working:app=deploy"),
            report("state=working:id=us-east:title=QnVpbGQg4oCuZXZpbOKAiw:msg=UGxhbg"),
            report("state=working:id=us-east/a:app=cargo"),
            report("state=working:id=orphan/b"),
        ];
        fixture.turn(&[&reports.concat()], false);
        let records = &fixture.records;
        assert_eq!(records.app(b"us-east"), b"deploy");
        assert_eq!(records.app(b"us-east/a"), b"cargo");
        // The ancestor need not exist itself.
        assert_eq!(records.app(b"orphan/b"), b"deploy");
        let shown = &fixture.record("us-east").report;
        assert_eq!(
            (shown.title.as_str(), shown.message.as_str()),
            ("Build evil", "Plan")
        );
        assert!(shown.app.is_empty());
    }

    #[test]
    fn eviction_keeps_the_most_recently_updated() {
        let mut fixture = Fixture::new();
        let reports: String = (0..MAX_RECORDS)
            .map(|n| report(&format!("state=idle:id=r{n}")))
            .collect();
        fixture.turn(&[&reports], false);
        fixture.turn(
            &[&report("state=idle:id=r0"), &report("state=idle:id=new")],
            false,
        );
        assert_eq!(fixture.records.records.len(), MAX_RECORDS);
        assert!(!fixture.records.records.contains_key(b"r1".as_slice()));
        assert!(fixture.records.records.contains_key(b"r0".as_slice()));
    }

    #[test]
    fn libghostty_enforces_the_report_rules() {
        let mut fixture = Fixture::new();
        // A report split across reads applies once whole.
        let split = report("state=working:id=split");
        let (head, tail) = split.split_at(9);
        fixture.turn(&[head, tail], false);
        // A sequence over 4096 bytes, an unknown state, a malformed base64
        // value and an invalid id each discard the whole report.
        let oversized = report(&format!("state=working:id=big:future={}", "x".repeat(4096)));
        let discarded = [
            oversized,
            report("state=dancing:id=unknown"),
            report("state=working:id=base64:msg=a"),
            report("state=working:id=a//b"),
        ];
        fixture.turn(&[&discarded.concat()], false);
        // An unknown key or a malformed pair is skipped, and an unknown kind
        // or an out-of-range progress reads as absent.
        fixture.turn(
            &[&report(
                "state=blocked:id=kept:future=1:junk:kind=dance:progress=101",
            )],
            false,
        );
        assert_eq!(fixture.ids(), ["kept", "split"]);
        let kept = &fixture.record("kept").report;
        assert_eq!((kept.kind, kept.progress), (ProgramStatusKind::None, None));
    }

    /// A real PTY child's reports and prompt marks reach the records through
    /// `Orbit`'s callbacks, and `Orbit` tells the child's own foreground group
    /// from a job's.
    #[test]
    fn a_pty_childs_reports_reach_the_records() {
        use crate::{
            boundary::Foreground,
            pty::{Orbit, PtyRead},
        };
        use std::{ffi::OsString, thread, time::Duration};

        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        // With job control, a foreground subshell takes the foreground and the
        // shell takes it back. Each phase waits on a line the test writes once
        // it has sampled that phase's group, so no phase can pass unseen.
        let script = "set -m; read x; \
             printf '\\033]7501;state=working:id=a\\007\\033]133;A\\007'; \
             printf '\\033]7501;state=working:id=b\\007'; \
             (read x); read x";
        let command = ["/bin/sh", "-c", script].map(OsString::from);
        let mut orbit = Orbit::spawn(&command, None, 80, 24).unwrap();
        orbit.start_turn();
        let mut data = [0; 8192];
        let mut phases = [Foreground::Child, Foreground::Other, Foreground::Child].into_iter();
        let mut phase = phases.next();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "never sampled {phase:?}");
            if phase == Some(orbit.foreground()) {
                assert!(orbit.writes.push(b"\n"));
                phase = phases.next();
            }
            orbit.flush_writes().unwrap();
            match orbit.read_into(&mut data).unwrap() {
                PtyRead::Closed => break,
                PtyRead::Bytes(_) => {}
                PtyRead::Blocked(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
        assert_eq!(phase, None);
        assert!(orbit.status_marks());
        orbit.apply_status(false, true);
        let ids: Vec<_> = orbit.status.records.keys().cloned().collect();
        assert_eq!(ids, [b"b".to_vec()]);
    }

    #[test]
    fn real_agent_reports_keep_their_records() {
        // The reports Claude Code 2.1.303 and Codex 0.163.0 sent for one
        // short prompt, recorded inside an isolated Orbit on 2026-10-21
        // (eon-agent-sidebar-yx8.4). Claude ends each with BEL, Codex with ST.
        let claude = ["idle", "working", "done"]
            .map(|state| format!("\x1b]7501;state={state}:app=claude-code\x07"));
        let codex = ["idle", "working", "idle"]
            .map(|state| format!("\x1b]7501;state={state}:app=codex\x1b\\"));
        for (reports, app, last, previous) in [
            (claude, "claude-code", State::Done, State::Working),
            (codex, "codex", State::Idle, State::Working),
        ] {
            let mut fixture = Fixture::new();
            for report in &reports {
                fixture.turn(&[report], false);
            }
            let root = fixture.record("");
            assert_eq!(fixture.ids(), [""]);
            assert_eq!(root.report.app, app.as_bytes());
            assert_eq!(
                (root.report.state, root.previous, root.changed),
                (last, Some(previous), 3)
            );
        }
    }
}
