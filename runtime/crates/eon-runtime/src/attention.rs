//! Agent attention (EON-C31): polls the program status of every pane and
//! popup terminal off the request path and derives what needs input, what has
//! finished and what has been seen. Orbit owns the records; this owns their
//! meaning.

use super::{
    sessions::{self, ProgramStatusRead, RunningSession},
    workspace::is_directory_picker_session,
};
use eon_workspace_protocol::v19::{
    self as eonw, Action, AttentionKind, AttentionRecord, AttentionState as State, Sidebar,
    Snapshot,
};
use orbit_protocol::management::{ProgramKind, ProgramRecord, ProgramState, ProgramStatus};
use std::{
    cmp::Reverse,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// At most 2 Hz.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const POLL_DEADLINE: Duration = Duration::from_millis(250);
const STALE_AFTER: Duration = Duration::from_secs(3);
const AGENT_ENTRY: &str = "agent";

/// A pane or popup terminal, in workspace order: panes column by column, then
/// popups.
pub(super) struct Target {
    tab: usize,
    position: usize,
    /// The pane id, or the popup instance.
    id: String,
    session: String,
    agent: bool,
}

/// The pane and popup terminals, never a directory picker, and the session of
/// the focused one: the shown popup, else the selected pane, of the active tab.
pub(super) fn targets(snapshot: &Snapshot) -> (Vec<Target>, Option<String>) {
    let mut targets = Vec::new();
    let mut focus = None;
    for (index, tab) in snapshot.tabs.iter().enumerate() {
        let active = tab.id == snapshot.active_tab;
        let panes = tab.panes().map(|pane| {
            let focused =
                tab.selected_popup.is_none() && tab.selected_pane.as_ref() == Some(&pane.id);
            (&pane.id, &pane.session, false, focused)
        });
        let popups = tab.popups.iter().map(|popup| {
            let focused = tab.selected_popup.as_ref() == Some(&popup.id);
            (
                &popup.id,
                &popup.session,
                popup.entry == AGENT_ENTRY,
                focused,
            )
        });
        let mut position = 0;
        for (id, session, agent, focused) in panes.chain(popups) {
            if active && focused {
                focus = Some(session.clone());
            }
            if is_directory_picker_session(session) {
                continue;
            }
            targets.push(Target {
                tab: index,
                position,
                id: id.clone(),
                session: session.clone(),
                agent,
            });
            position += 1;
        }
    }
    (targets, focus)
}

/// Record text without control characters, which Orbit already rejects
/// (ORB-C15) and EONW refuses to carry.
fn plain(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

#[derive(Clone, Copy)]
struct Raise {
    at: Instant,
    seen: bool,
}

struct Record {
    record: ProgramRecord,
    /// Fixed on first sight of this change sequence.
    changed_at: Instant,
    raise: Option<Raise>,
}

impl Record {
    /// Its normalized state, and the time its order key uses: when it entered
    /// `blocked`, else its raise, else its change.
    fn normalized(&self) -> (State, Instant) {
        let unseen = self.raise.filter(|raise| !raise.seen);
        match (self.record.state, unseen) {
            (ProgramState::Blocked, _) => (State::NeedsInput, self.changed_at),
            (ProgramState::Error, Some(raise)) => (State::Error, raise.at),
            (ProgramState::Done | ProgramState::Idle, Some(raise)) => (State::Finished, raise.at),
            (ProgramState::Working, _) => (State::Working, self.changed_at),
            _ => (State::Idle, self.changed_at),
        }
    }
}

/// One terminal's last good reply.
struct Terminal {
    session: String,
    run: String,
    update: u64,
    count: u16,
    /// The terminal title, which the attention carries as `name`.
    name: String,
    records: Vec<Record>,
    /// Records the reply omitted for size, kept while Orbit still holds them
    /// so their change times and seen raises survive until they return.
    omitted: Vec<Record>,
    replied: Instant,
}

impl Terminal {
    /// The record of the highest-priority state; ties go to the most recent
    /// change. Its order time comes with it.
    fn most_urgent(&self) -> Option<(&Record, State, Instant)> {
        self.records
            .iter()
            .map(|record| {
                let (state, at) = record.normalized();
                (record, state, at)
            })
            .min_by_key(|&(record, state, _)| (state, Reverse(record.record.changed)))
    }

    fn see(&mut self) {
        for raise in self
            .records
            .iter_mut()
            .chain(&mut self.omitted)
            .filter_map(|record| record.raise.as_mut())
        {
            raise.seen = true;
        }
    }
}

/// What one terminal shows.
struct Shown<'a> {
    state: State,
    /// The terminal title; empty for unavailable.
    name: &'a str,
    /// The most urgent record; none for unavailable.
    record: Option<&'a ProgramRecord>,
    /// That record's fixed change time.
    changed_at: Option<Instant>,
    stale: bool,
    others: u16,
}

/// A queue entry's total order: class, fixed time, tab and terminal position.
pub(super) type Key = (State, Instant, usize, usize);

pub(super) struct Attention {
    terminals: Vec<Terminal>,
    focus: Option<String>,
    attached: bool,
    /// The order key of the last jump target.
    cursor: Option<Key>,
    next_poll: Instant,
    sidebar: Sidebar,
    /// One instant and its wall-clock Unix milliseconds, which turn every
    /// fixed change time into the same `changed_at` in each snapshot.
    epoch: (Instant, u64),
}

impl Attention {
    /// `enabled` is `[sidebar] enabled`; without it the sidebar is Off.
    pub(super) fn new(enabled: bool) -> Self {
        let now = Instant::now();
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        Attention {
            terminals: Vec::new(),
            focus: None,
            attached: false,
            cursor: None,
            next_poll: now,
            sidebar: if enabled { Sidebar::Open } else { Sidebar::Off },
            epoch: (now, wall),
        }
    }

    /// Sends due requests and reads replies without blocking. A miss, a Busy
    /// refusal or a bad reply keeps the last good result.
    pub(super) fn poll(
        &mut self,
        targets: &[Target],
        sessions: &mut [RunningSession],
        now: Instant,
    ) {
        let due = now >= self.next_poll;
        if due {
            self.next_poll = now + POLL_INTERVAL;
        }
        for session in sessions {
            if !targets.iter().any(|target| target.session == session.id) {
                continue;
            }
            if due {
                let _ = sessions::send_program_status(session, now + POLL_DEADLINE);
            }
            if let Ok(ProgramStatusRead::Reply(status)) = sessions::read_program_status(session) {
                self.apply(targets, &session.id, session.run_id(), status, now);
            }
        }
    }

    /// Drops the attention of removed targets and of ended or replaced runs.
    pub(super) fn retain(&mut self, targets: &[Target], sessions: &[RunningSession]) {
        self.terminals.retain(|terminal| {
            targets
                .iter()
                .any(|target| target.session == terminal.session)
                && sessions.iter().any(|session| {
                    session.id == terminal.session
                        && !session.is_finished()
                        && session.run_id() == terminal.run
                })
        });
    }

    /// Takes one good reply for `session`'s run `run`.
    fn apply(
        &mut self,
        targets: &[Target],
        session: &str,
        run: &str,
        status: ProgramStatus,
        now: Instant,
    ) {
        let index = self
            .terminals
            .iter()
            .position(|terminal| terminal.session == session);
        let before = self.entry(targets, session);
        let (records, omitted) = match index.map(|index| &mut self.terminals[index]) {
            Some(terminal) if terminal.run == run => {
                if status.update < terminal.update {
                    return;
                }
                (
                    std::mem::take(&mut terminal.records),
                    std::mem::take(&mut terminal.omitted),
                )
            }
            _ => Default::default(),
        };
        let known = |id: &str| {
            records
                .iter()
                .chain(&omitted)
                .find(|record| record.record.id == id)
        };
        let fresh: Vec<Record> = status
            .records
            .into_iter()
            .map(|record| {
                let cached = known(&record.id);
                if let Some(cached) =
                    cached.filter(|cached| cached.record.changed == record.changed)
                {
                    return Record {
                        changed_at: cached.changed_at,
                        raise: cached.raise,
                        record,
                    };
                }
                let changed_at = now
                    .checked_sub(Duration::from_millis(record.changed_age_ms))
                    .unwrap_or(now);
                let own = Some(Raise {
                    at: changed_at,
                    seen: false,
                });
                let raise = match (record.state, record.previous) {
                    (ProgramState::Done | ProgramState::Error, _)
                    | (ProgramState::Idle, Some(ProgramState::Working)) => own,
                    (ProgramState::Idle, Some(ProgramState::Done)) => cached
                        .filter(|cached| cached.record.state == ProgramState::Done)
                        .and_then(|cached| cached.raise)
                        .or(own),
                    _ => None,
                };
                Record {
                    record,
                    changed_at,
                    raise,
                }
            })
            .collect();
        // The reply names each omitted record by its creation sequence, so a
        // record that is gone is forgotten and one still held is kept.
        let omitted = records
            .into_iter()
            .chain(omitted)
            .filter(|record| status.omitted.contains(&record.record.created))
            .take(status.omitted.len())
            .collect();
        let terminal = Terminal {
            session: session.into(),
            run: run.into(),
            update: status.update,
            count: status.count,
            name: status.terminal_title,
            records: fresh,
            omitted,
            replied: now,
        };
        match index {
            Some(index) => self.terminals[index] = terminal,
            None => self.terminals.push(terminal),
        }
        if self.attached && self.focus.as_deref() == Some(session) {
            self.see(session);
        }
        // A new entry of a higher class than the cursor's restarts the walk
        // at the front of the queue.
        let after = self.entry(targets, session);
        if let (Some(cursor), Some(after)) = (self.cursor, after)
            && after.0 < cursor.0
            && before != Some(after)
        {
            self.cursor = None;
        }
    }

    /// Follows focus and attachment. A focus move by any other means than a
    /// queue jump resets the cursor. While a Venus is attached, focus arriving
    /// on a terminal, or a Venus attaching to it, sees its raises.
    pub(super) fn observe(&mut self, focus: Option<String>, attached: bool) {
        let moved = focus != self.focus;
        if moved {
            self.cursor = None;
        }
        if attached
            && (moved || !self.attached)
            && let Some(session) = &focus
        {
            self.see(session);
        }
        self.focus = focus;
        self.attached = attached;
    }

    /// Follows an accepted attention action, given the snapshot it left:
    /// `ToggleSidebar` flips visibility, and `FocusAttention` follows focus,
    /// moves the cursor to `jump` for a queue jump, and sees the target's
    /// raises when it has focus, also when it had focus already.
    pub(super) fn handled(
        &mut self,
        action: &Action,
        jump: Option<Key>,
        snapshot: &Snapshot,
        attached: bool,
    ) {
        match action {
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::FocusAttention { target, .. } => {
                let (targets, focus) = targets(snapshot);
                self.observe(focus.clone(), attached);
                if jump.is_some() {
                    self.cursor = jump;
                }
                if let Some(session) = targets
                    .iter()
                    .find(|item| item.id == *target)
                    .map(|item| &item.session)
                    && attached
                    && focus.as_ref() == Some(session)
                {
                    self.see(session);
                }
            }
            _ => {}
        }
    }

    /// Sees every raise cached for `session`, as focusing it does, also when
    /// it already has focus. Needs input is never cleared by viewing.
    fn see(&mut self, session: &str) {
        if let Some(terminal) = self
            .terminals
            .iter_mut()
            .find(|terminal| terminal.session == session)
        {
            terminal.see();
        }
    }

    fn shown(&self, target: &Target, now: Instant) -> Option<Shown<'_>> {
        let terminal = self
            .terminals
            .iter()
            .find(|terminal| terminal.session == target.session);
        match terminal.and_then(|terminal| Some((terminal, terminal.most_urgent()?))) {
            Some((terminal, (record, state, _))) => Some(Shown {
                state,
                name: &terminal.name,
                record: Some(&record.record),
                changed_at: Some(record.changed_at),
                stale: now.duration_since(terminal.replied) > STALE_AFTER,
                others: terminal.count.saturating_sub(1),
            }),
            None => target.agent.then_some(Shown {
                state: State::Unavailable,
                name: "",
                record: None,
                changed_at: None,
                stale: false,
                others: 0,
            }),
        }
    }

    /// A tab's highest-priority state; a stale terminal counts with its last
    /// state.
    fn rollup(&self, targets: &[Target], tab: usize, now: Instant) -> Option<State> {
        targets
            .iter()
            .filter(|target| target.tab == tab)
            .filter_map(|target| Some(self.shown(target, now)?.state))
            .min()
    }

    /// A terminal's queue entry: needs input, an unseen error or finished.
    fn entry(&self, targets: &[Target], session: &str) -> Option<Key> {
        let target = targets.iter().find(|target| target.session == session)?;
        let terminal = self
            .terminals
            .iter()
            .find(|terminal| terminal.session == session)?;
        let (_, state, at) = terminal.most_urgent()?;
        (state <= State::Finished).then_some((state, at, target.tab, target.position))
    }

    /// `NextAttention` (`forward`) or `PreviousAttention`: the entry after or
    /// before the cursor, wrapping, or the front of the queue after a reset;
    /// none for an empty queue. Its key becomes the cursor once the jump is
    /// accepted.
    fn next<'a>(&self, targets: &'a [Target], forward: bool) -> Option<(Key, &'a Target)> {
        let mut queue: Vec<(Key, &Target)> = targets
            .iter()
            .filter_map(|target| Some((self.entry(targets, &target.session)?, target)))
            .collect();
        queue.sort_by_key(|&(key, _)| key);
        match self.cursor {
            None => queue.first(),
            Some(cursor) if forward => queue
                .iter()
                .find(|(key, _)| *key > cursor)
                .or(queue.first()),
            Some(cursor) => queue
                .iter()
                .rev()
                .find(|(key, _)| *key < cursor)
                .or(queue.last()),
        }
        .copied()
    }

    /// The `FocusAttention` a queue jump performs, with the cursor it moves
    /// to; none for an empty queue.
    pub(super) fn jump(&self, snapshot: &Snapshot, forward: bool) -> Option<(Action, Key)> {
        let (targets, _) = targets(snapshot);
        let (key, target) = self.next(&targets, forward)?;
        let action = Action::FocusAttention {
            tab: snapshot.tabs[target.tab].id.clone(),
            target: target.id.clone(),
        };
        Some((action, key))
    }

    /// Sets each terminal's attention, each tab's rollup and the sidebar,
    /// after following the focus `snapshot` has: a reply to a focus change
    /// already shows the newly focused terminal's raises as seen.
    pub(super) fn present(&mut self, snapshot: &mut Snapshot, attached: bool, now: Instant) {
        let (targets, focus) = targets(snapshot);
        self.observe(focus, attached);
        let attention = |id: &str| {
            let target = targets.iter().find(|target| target.id == id)?;
            let shown = self.shown(target, now)?;
            Some(eonw::Attention {
                state: shown.state,
                record: shown.record.map(|record| AttentionRecord {
                    kind: record.kind.map(|kind| match kind {
                        ProgramKind::Permission => AttentionKind::Permission,
                        ProgramKind::Question => AttentionKind::Question,
                        ProgramKind::Auth => AttentionKind::Auth,
                    }),
                    progress: record.progress,
                    app: plain(&record.app),
                    name: plain(shown.name),
                    title: plain(&record.title),
                    message: plain(&record.message),
                    changed_at: self.wall(shown.changed_at.unwrap_or(now)),
                    stale: shown.stale,
                    others: shown.others,
                }),
            })
        };
        for (index, tab) in snapshot.tabs.iter_mut().enumerate() {
            for pane in tab.columns.iter_mut().flat_map(|column| &mut column.panes) {
                pane.attention = attention(&pane.id);
            }
            for popup in &mut tab.popups {
                popup.attention = attention(&popup.id);
            }
            tab.attention_rollup = self.rollup(&targets, index, now);
        }
        snapshot.sidebar = self.sidebar;
    }

    /// The wall-clock Unix milliseconds of `at`.
    fn wall(&self, at: Instant) -> u64 {
        let (instant, wall) = self.epoch;
        match at.checked_duration_since(instant) {
            Some(after) => wall.saturating_add(after.as_millis() as u64),
            None => wall.saturating_sub(instant.duration_since(at).as_millis() as u64),
        }
    }

    /// `ToggleSidebar`: flips Open and Closed; Off stays.
    fn toggle_sidebar(&mut self) {
        self.sidebar = match self.sidebar {
            Sidebar::Open => Sidebar::Closed,
            Sidebar::Closed => Sidebar::Open,
            Sidebar::Off => Sidebar::Off,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProgramState::{Blocked, Done, Error, Idle, Working};
    use eon_workspace_protocol::v19::{Column, Pane, Popup, PopupGeometry, Tab};
    use std::{fs, thread};

    fn record(
        id: &str,
        state: ProgramState,
        previous: Option<ProgramState>,
        changed: u64,
    ) -> ProgramRecord {
        ProgramRecord {
            id: id.into(),
            state,
            kind: None,
            progress: None,
            app: "claude-code".into(),
            title: String::new(),
            message: String::new(),
            updated: changed,
            created: changed,
            changed,
            previous,
            updated_age_ms: 0,
            changed_age_ms: 0,
        }
    }

    fn status(update: u64, records: Vec<ProgramRecord>) -> ProgramStatus {
        ProgramStatus {
            count: records.len() as u16,
            update,
            terminal_title: String::new(),
            omitted: Vec::new(),
            records,
        }
    }

    /// A tab of one column of `panes`, then `popups` as (entry, session);
    /// each terminal is named by its session.
    fn tab(id: &str, panes: &[&str], popups: &[(&str, &str)], shown: Option<&str>) -> Tab {
        Tab {
            id: id.into(),
            directory: b"/".to_vec(),
            pending: false,
            selected_pane: panes.first().map(|session| format!("pane-{session}")),
            selected_popup: shown.map(|session| format!("popup-{session}")),
            columns: vec![Column {
                panes: panes
                    .iter()
                    .map(|session| Pane {
                        id: format!("pane-{session}"),
                        session: (*session).into(),
                        endpoint: Vec::new(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: format!("pane-{}", panes[0]),
            }],
            popups: popups
                .iter()
                .map(|(entry, session)| Popup {
                    id: format!("popup-{session}"),
                    entry: (*entry).into(),
                    session: (*session).into(),
                    endpoint: Vec::new(),
                    attention: None,
                })
                .collect(),
            attention_rollup: None,
        }
    }

    fn snapshot(active: &str, tabs: Vec<Tab>) -> Snapshot {
        Snapshot {
            active_tab: active.into(),
            geometry: PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: Vec::new(),
            tabs,
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Off,
        }
    }

    /// A queue jump the supervisor accepted.
    fn step<'a>(
        attention: &mut Attention,
        targets: &'a [Target],
        forward: bool,
    ) -> Option<&'a Target> {
        let (key, target) = attention.next(targets, forward)?;
        attention.cursor = Some(key);
        Some(target)
    }

    fn state(
        attention: &Attention,
        targets: &[Target],
        session: &str,
        now: Instant,
    ) -> Option<State> {
        let target = targets
            .iter()
            .find(|target| target.session == session)
            .unwrap();
        Some(attention.shown(target, now)?.state)
    }

    #[test]
    fn targets_skip_pickers_and_name_the_focused_terminal() {
        let tabs = vec![
            tab(
                "t1",
                &["s1", "s2"],
                &[("project", "directory-picker-1")],
                None,
            ),
            tab("t2", &["s3"], &[("agent", "s4")], Some("s4")),
        ];
        let (found, focus) = targets(&snapshot("t2", tabs.clone()));
        let order: Vec<_> = found
            .iter()
            .map(|target| {
                (
                    target.tab,
                    target.position,
                    target.id.as_str(),
                    target.agent,
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                (0, 0, "pane-s1", false),
                (0, 1, "pane-s2", false),
                (1, 0, "pane-s3", false),
                (1, 1, "popup-s4", true),
            ]
        );
        assert_eq!(focus.as_deref(), Some("s4"));
        assert_eq!(targets(&snapshot("t1", tabs)).1.as_deref(), Some("s1"));
    }

    #[test]
    fn raises_follow_the_changes_eon_c31_names() {
        let (targets, _) = targets(&snapshot("t", vec![tab("t", &["s"], &[], None)]));
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let at = |attention: &mut Attention, update: u64, records: Vec<ProgramRecord>| {
            let now = base + Duration::from_secs(update);
            attention.apply(&targets, "s", "run", status(update, records), now);
            state(attention, &targets, "s", now)
        };
        let secs = |secs| Some(base + Duration::from_secs(secs));

        // Created in done; an idle from that done keeps its raise, which
        // orders the queue, while the row shows the idle's own change.
        assert_eq!(
            at(&mut attention, 1, vec![record("a", Done, None, 1)]),
            Some(State::Finished)
        );
        assert_eq!(
            at(&mut attention, 2, vec![record("a", Idle, Some(Done), 2)]),
            Some(State::Finished)
        );
        assert_eq!(attention.entry(&targets, "s").map(|key| key.1), secs(1));
        assert_eq!(
            attention.shown(&targets[0], base).unwrap().changed_at,
            secs(2)
        );
        attention.see("s");
        // A same-state re-report keeps the seen raise.
        let mut repeat = record("a", Idle, Some(Done), 2);
        repeat.updated = 3;
        assert_eq!(at(&mut attention, 3, vec![repeat]), Some(State::Idle));
        // working -> idle raises; blocked -> idle and error -> idle do not.
        assert_eq!(
            at(&mut attention, 4, vec![record("a", Working, Some(Idle), 4)]),
            Some(State::Working)
        );
        assert_eq!(
            at(&mut attention, 5, vec![record("a", Idle, Some(Working), 5)]),
            Some(State::Finished)
        );
        attention.see("s");
        assert_eq!(
            at(&mut attention, 6, vec![record("a", Blocked, Some(Idle), 6)]),
            Some(State::NeedsInput)
        );
        assert_eq!(
            at(&mut attention, 7, vec![record("a", Idle, Some(Blocked), 7)]),
            Some(State::Idle)
        );
        assert_eq!(
            at(&mut attention, 8, vec![record("a", Error, Some(Idle), 8)]),
            Some(State::Error)
        );
        assert_eq!(
            at(&mut attention, 9, vec![record("a", Idle, Some(Error), 9)]),
            Some(State::Idle)
        );
        // Created idle never raises; an idle from a done never seen raises at
        // its own change.
        assert_eq!(
            at(&mut attention, 10, vec![record("b", Idle, None, 10)]),
            Some(State::Idle)
        );
        assert_eq!(
            at(&mut attention, 11, vec![record("c", Idle, Some(Done), 11)]),
            Some(State::Finished)
        );
        assert_eq!(attention.entry(&targets, "s").map(|key| key.1), secs(11));
    }

    #[test]
    fn clears_drop_unseen_raises_and_a_restarted_provider_raises_anew() {
        let (targets, _) = targets(&snapshot("t", vec![tab("t", &["s"], &[], None)]));
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let apply = |attention: &mut Attention, update: u64, records| {
            let now = base + Duration::from_secs(update);
            attention.apply(&targets, "s", "run", status(update, records), now);
        };

        // An unseen finish vanishes with its record.
        apply(&mut attention, 1, vec![record("a", Done, None, 1)]);
        assert_eq!(
            state(&attention, &targets, "s", base),
            Some(State::Finished)
        );
        apply(&mut attention, 2, Vec::new());
        assert_eq!(state(&attention, &targets, "s", base), None);

        // A provider that clears and reports again raises again, though its
        // earlier finish was seen.
        apply(&mut attention, 3, vec![record("a", Done, None, 3)]);
        attention.see("s");
        apply(&mut attention, 5, vec![record("a", Done, None, 5)]);
        assert_eq!(
            state(&attention, &targets, "s", base),
            Some(State::Finished)
        );

        // Within one terminal, a tie in state goes to the most recent change.
        apply(
            &mut attention,
            7,
            vec![record("x", Blocked, None, 6), record("y", Blocked, None, 7)],
        );
        let shown = attention.shown(&targets[0], base).unwrap();
        assert_eq!(shown.record.unwrap().id, "y");
        assert_eq!(
            attention.entry(&targets, "s").map(|key| key.1),
            shown.changed_at
        );
    }

    #[test]
    fn identity_fixes_change_times_and_discards_older_replies_until_a_new_run() {
        let (targets, _) = targets(&snapshot("t", vec![tab("t", &["s"], &[], None)]));
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let done = |age| ProgramRecord {
            changed_age_ms: age,
            ..record("a", Done, Some(Working), 5)
        };
        let changed_at =
            |attention: &Attention| attention.shown(&targets[0], base).unwrap().changed_at;
        attention.apply(
            &targets,
            "s",
            "run",
            status(5, vec![done(1000)]),
            base + Duration::from_secs(10),
        );
        let fixed = Some(base + Duration::from_secs(9));
        assert_eq!(changed_at(&attention), fixed);

        // An out-of-order reply is discarded.
        attention.apply(
            &targets,
            "s",
            "run",
            status(4, vec![record("a", Working, None, 4)]),
            base,
        );
        assert_eq!(
            state(&attention, &targets, "s", base),
            Some(State::Finished)
        );
        // Polls never move the fixed time, whatever the reported age.
        attention.apply(
            &targets,
            "s",
            "run",
            status(6, vec![done(1600)]),
            base + Duration::from_millis(10_500),
        );
        assert_eq!(changed_at(&attention), fixed);

        // A record omitted for size keeps its time, and focus sees it while
        // it is omitted.
        let crowded = ProgramStatus {
            count: 2,
            update: 7,
            terminal_title: String::new(),
            omitted: vec![5],
            records: vec![record("b", Idle, None, 7)],
        };
        attention.apply(
            &targets,
            "s",
            "run",
            crowded,
            base + Duration::from_secs(11),
        );
        attention.see("s");
        attention.apply(
            &targets,
            "s",
            "run",
            status(8, vec![done(5000), record("b", Idle, None, 7)]),
            base + Duration::from_secs(12),
        );
        assert_eq!(state(&attention, &targets, "s", base), Some(State::Idle));
        let a = |attention: &Attention| attention.terminals[0].records[0].changed_at;
        assert_eq!(Some(a(&attention)), fixed);

        // Records seen and then cleared while `a` stays omitted are
        // forgotten, and `a` keeps its seen raise however many go.
        let churn = 9..9 + 2 * 256;
        for update in churn.clone() {
            let churning = ProgramStatus {
                count: 2,
                update,
                terminal_title: String::new(),
                omitted: vec![5],
                records: vec![record(&format!("r{update}"), Done, Some(Working), update)],
            };
            attention.apply(
                &targets,
                "s",
                "run",
                churning,
                base + Duration::from_secs(13),
            );
            attention.see("s");
        }
        assert_eq!(attention.terminals[0].omitted.len(), 1);
        attention.apply(
            &targets,
            "s",
            "run",
            status(churn.end, vec![done(5000)]),
            base + Duration::from_secs(13),
        );
        assert_eq!(state(&attention, &targets, "s", base), Some(State::Idle));

        // A new run starts empty, so its done is a new, unseen raise.
        attention.apply(
            &targets,
            "s",
            "run-2",
            status(1, vec![done(0)]),
            base + Duration::from_secs(14),
        );
        let shown = attention.shown(&targets[0], base).unwrap();
        assert_eq!(shown.state, State::Finished);
        assert_eq!(shown.changed_at, Some(base + Duration::from_secs(14)));
    }

    #[test]
    fn stale_results_keep_their_state_and_only_agent_popups_start_unavailable() {
        let shown_snapshot = snapshot("t", vec![tab("t", &["s"], &[("agent", "a")], None)]);
        let (targets, _) = targets(&shown_snapshot);
        let mut attention = Attention::new(true);
        let base = Instant::now();
        assert_eq!(state(&attention, &targets, "s", base), None);
        assert_eq!(
            state(&attention, &targets, "a", base),
            Some(State::Unavailable)
        );

        let working = ProgramStatus {
            count: 3,
            terminal_title: "eon-9a 🪐".into(),
            ..status(1, vec![record("w", Working, None, 1)])
        };
        attention.apply(&targets, "s", "run", working.clone(), base);
        let shown = attention.shown(&targets[0], base + STALE_AFTER).unwrap();
        assert!(!shown.stale);
        assert_eq!(shown.others, 2);
        let late = base + STALE_AFTER + Duration::from_millis(100);
        let shown = attention.shown(&targets[0], late).unwrap();
        assert!(shown.stale);
        assert_eq!(shown.state, State::Working);
        // The terminal title passes through unchanged as the name, which a
        // stale terminal keeps and the next reply replaces.
        let name = |attention: &mut Attention| {
            let mut presented = shown_snapshot.clone();
            attention.present(&mut presented, false, late);
            let pane = &presented.tabs[0].columns[0].panes[0];
            let record = pane.attention.as_ref().unwrap().record.as_ref().unwrap();
            (record.name.clone(), record.stale)
        };
        assert_eq!(name(&mut attention), ("eon-9a 🪐".into(), true));
        attention.apply(&targets, "s", "run", working.clone(), late);
        assert!(!attention.shown(&targets[0], late).unwrap().stale);
        let renamed = ProgramStatus {
            terminal_title: "eon-9b".into(),
            ..working
        };
        attention.apply(&targets, "s", "run", renamed, late);
        assert_eq!(name(&mut attention), ("eon-9b".into(), false));

        attention.apply(&targets, "a", "run", status(1, Vec::new()), base);
        assert_eq!(
            state(&attention, &targets, "a", base),
            Some(State::Unavailable)
        );
        attention.apply(
            &targets,
            "a",
            "run",
            status(2, vec![record("", Working, None, 2)]),
            base,
        );
        assert_eq!(state(&attention, &targets, "a", base), Some(State::Working));
    }

    #[test]
    fn seen_needs_an_attached_venus_and_the_exact_focused_terminal() {
        let (targets, _) = targets(&snapshot("t", vec![tab("t", &["s1", "s2"], &[], None)]));
        let mut attention = Attention::new(true);
        let now = Instant::now();
        let check = |attention: &Attention, session| state(attention, &targets, session, now);
        let apply = |attention: &mut Attention, session, update, records| {
            attention.apply(&targets, session, "run", status(update, records), now);
        };

        apply(&mut attention, "s1", 1, vec![record("a", Done, None, 1)]);
        attention.observe(Some("s1".into()), false);
        assert_eq!(
            check(&attention, "s1"),
            Some(State::Finished),
            "seen while detached"
        );
        attention.observe(Some("s1".into()), true);
        assert_eq!(
            check(&attention, "s1"),
            Some(State::Idle),
            "attach sees the focused terminal"
        );

        // A raise arriving while its terminal is focused is seen at once; one
        // elsewhere is not.
        apply(
            &mut attention,
            "s1",
            2,
            vec![record("a", Error, Some(Done), 2)],
        );
        assert_eq!(check(&attention, "s1"), Some(State::Idle));
        apply(&mut attention, "s2", 1, vec![record("b", Done, None, 1)]);
        assert_eq!(check(&attention, "s2"), Some(State::Finished));

        // Focus arriving sees it; needs input survives viewing.
        apply(
            &mut attention,
            "s1",
            3,
            vec![record("a", Blocked, Some(Error), 3)],
        );
        attention.observe(Some("s2".into()), true);
        assert_eq!(check(&attention, "s2"), Some(State::Idle));
        attention.observe(Some("s1".into()), true);
        assert_eq!(check(&attention, "s1"), Some(State::NeedsInput));

        // FocusAttention sees its target's raises.
        apply(
            &mut attention,
            "s2",
            2,
            vec![record("b", Error, Some(Done), 2)],
        );
        assert_eq!(check(&attention, "s2"), Some(State::Error));
        attention.see("s2");
        assert_eq!(check(&attention, "s2"), Some(State::Idle));

        // After detaching nothing becomes seen.
        attention.observe(Some("s1".into()), false);
        apply(
            &mut attention,
            "s1",
            4,
            vec![record("a", Error, Some(Blocked), 4)],
        );
        assert_eq!(check(&attention, "s1"), Some(State::Error));
    }

    #[test]
    fn a_focus_reply_already_shows_the_focused_finish_as_seen() {
        let before = snapshot("t", vec![tab("t", &["s1", "s2"], &[], None)]);
        let (targets, focus) = targets(&before);
        let mut attention = Attention::new(true);
        let now = Instant::now();
        attention.observe(focus, true);
        attention.apply(
            &targets,
            "s2",
            "run",
            status(1, vec![record("a", Done, None, 1)]),
            now,
        );
        let mut after = before.clone();
        after.tabs[0].selected_pane = Some("pane-s2".into());
        after.tabs[0].columns[0].expanded = "pane-s2".into();
        attention.present(&mut after, true, now);
        let pane = &after.tabs[0].columns[0].panes[1];
        assert_eq!(pane.attention.as_ref().unwrap().state, State::Idle);
        assert_eq!(after.tabs[0].attention_rollup, Some(State::Idle));
        assert_eq!(after.sidebar, Sidebar::Open);
    }

    #[test]
    fn the_queue_walks_its_total_order_and_its_cursor_resets() {
        let (targets, _) = targets(&snapshot(
            "t1",
            vec![
                tab("t1", &["s1", "s2"], &[], None),
                tab("t2", &["s3"], &[("agent", "s4")], None),
            ],
        ));
        let base = Instant::now();
        let mut attention = Attention::new(true);
        assert!(
            step(&mut attention, &targets, true).is_none(),
            "an empty queue moves nothing"
        );
        let apply = |attention: &mut Attention, session, secs, record: ProgramRecord| {
            let update = record.changed;
            attention.apply(
                &targets,
                session,
                "run",
                status(update, vec![record]),
                base + Duration::from_secs(secs),
            );
        };
        apply(&mut attention, "s3", 0, record("c", Done, None, 1));
        apply(&mut attention, "s1", 1, record("a", Done, None, 1));
        apply(&mut attention, "s2", 2, record("b", Blocked, None, 1));
        apply(&mut attention, "s4", 3, record("d", Error, None, 1));
        let walk = |attention: &mut Attention, forward| {
            step(attention, &targets, forward)
                .map(|target| target.session.clone())
                .unwrap()
        };

        // Class, then time; wrapping both ways.
        let order: Vec<_> = (0..5).map(|_| walk(&mut attention, true)).collect();
        assert_eq!(order, ["s2", "s4", "s3", "s1", "s2"]);
        assert_eq!(walk(&mut attention, false), "s1");
        assert_eq!(walk(&mut attention, false), "s3");

        // The cursor outlives its entry.
        attention.see("s3");
        assert_eq!(walk(&mut attention, true), "s1");
        assert_eq!(walk(&mut attention, true), "s2");
        assert_eq!(walk(&mut attention, true), "s4");

        // Any other focus move restarts at the front.
        attention.observe(Some("s3".into()), true);
        assert_eq!(walk(&mut attention, true), "s2");

        // A queue jump focuses its target as FocusAttention, keeps its
        // cursor though focus moved, and sees the target.
        let jumped = snapshot(
            "t2",
            vec![
                tab("t1", &["s1", "s2"], &[], None),
                tab("t2", &["s3"], &[("agent", "s4")], Some("s4")),
            ],
        );
        let (action, key) = attention.jump(&jumped, true).unwrap();
        assert_eq!(
            action,
            Action::FocusAttention {
                tab: "t2".into(),
                target: "popup-s4".into(),
            }
        );
        attention.handled(&action, Some(key), &jumped, true);
        assert_eq!(state(&attention, &targets, "s4", base), Some(State::Idle));
        assert_eq!(walk(&mut attention, true), "s1");

        // A new entry of a higher class than the cursor's restarts at the
        // front; one of the cursor's class keeps the cursor.
        apply(&mut attention, "s3", 4, record("c", Done, Some(Working), 3));
        apply(
            &mut attention,
            "s4",
            5,
            record("d", Blocked, Some(Error), 2),
        );
        assert_eq!(walk(&mut attention, true), "s2");
        apply(&mut attention, "s1", 6, record("a", Blocked, Some(Done), 2));
        assert_eq!(walk(&mut attention, true), "s4");

        assert_eq!(attention.rollup(&targets, 0, base), Some(State::NeedsInput));
        assert_eq!(attention.rollup(&targets, 1, base), Some(State::NeedsInput));
        attention.apply(&targets, "s4", "run", status(3, Vec::new()), base);
        assert_eq!(attention.rollup(&targets, 1, base), Some(State::Finished));

        assert_eq!(attention.sidebar, Sidebar::Open);
        attention.handled(&Action::ToggleSidebar, None, &jumped, true);
        assert_eq!(attention.sidebar, Sidebar::Closed);
        attention.toggle_sidebar();
        assert_eq!(attention.sidebar, Sidebar::Open);
        attention.sidebar = Sidebar::Off;
        attention.toggle_sidebar();
        assert_eq!(attention.sidebar, Sidebar::Off);
    }

    #[test]
    fn equal_times_order_by_tab_then_terminal_position() {
        let (targets, _) = targets(&snapshot(
            "t1",
            vec![
                tab("t1", &["s1", "s2"], &[], None),
                tab("t2", &["s3"], &[], None),
            ],
        ));
        let now = Instant::now();
        let mut attention = Attention::new(true);
        for session in ["s3", "s2", "s1"] {
            attention.apply(
                &targets,
                session,
                "run",
                status(1, vec![record("a", Done, None, 1)]),
                now,
            );
        }
        let order: Vec<_> = (0..3)
            .map(|_| {
                step(&mut attention, &targets, true)
                    .unwrap()
                    .session
                    .clone()
            })
            .collect();
        assert_eq!(order, ["s1", "s2", "s3"]);
    }

    /// Polls until `done` holds, as the supervisor loop does, checking that a
    /// poll never waits on a peer.
    fn poll_until(
        attention: &mut Attention,
        targets: &[Target],
        sessions: &mut [RunningSession],
        mut done: impl FnMut(&Attention) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(attention) {
            assert!(Instant::now() < deadline, "the poller never got there");
            let start = Instant::now();
            attention.poll(targets, sessions, start);
            attention.retain(targets, sessions);
            assert!(
                start.elapsed() < Duration::from_millis(50),
                "a poll waited on a peer"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn the_poller_reads_real_orbit_terminals_without_waiting_on_a_silent_one() {
        let roots = [
            crate::supervisor::temporary_directory(),
            crate::supervisor::temporary_directory(),
        ];
        let mut sessions: Vec<_> = roots
            .iter()
            .map(|root| {
                crate::sessions::tests::leased_orbit(
                    root,
                    "printf '\\033]7501;state=working:app=claude-code\\007'; sleep 4; \
                     printf '\\033]7501;state=done:app=claude-code\\007'; exec sleep 30",
                )
            })
            .collect();
        sessions[1].id = "session-2".into();
        let (targets, _) = targets(&snapshot(
            "t",
            vec![tab("t", &["session-1", "session-2"], &[], None)],
        ));
        let mut attention = Attention::new(true);
        let current =
            |attention: &Attention, session| state(attention, &targets, session, Instant::now());
        poll_until(&mut attention, &targets, &mut sessions, |attention| {
            ["session-1", "session-2"].map(|session| current(attention, session))
                == [Some(State::Working); 2]
        });

        // A silent peer only goes stale, keeping its state, while the other
        // is still polled at no more than 2 Hz.
        let silent = crate::sessions::tests::orbit_pid(&sessions[1]);
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(unsafe { libc::kill(silent, libc::SIGSTOP) }, 0);
        let start = Instant::now();
        let mut replies = Vec::new();
        poll_until(&mut attention, &targets, &mut sessions, |attention| {
            let replied = attention
                .terminals
                .iter()
                .find(|terminal| terminal.session == "session-1")
                .unwrap()
                .replied;
            if replies.last() != Some(&replied) {
                replies.push(replied);
            }
            start.elapsed() > STALE_AFTER + Duration::from_millis(500)
        });
        let window = start.elapsed().as_millis() as usize;
        assert!(
            replies.len() <= window / 500 + 2,
            "{} replies in {window} ms",
            replies.len()
        );
        assert!(
            replies.len() >= 4,
            "only {} replies in {window} ms",
            replies.len()
        );
        let now = Instant::now();
        let silent_shown = attention.shown(&targets[1], now).unwrap();
        assert!(silent_shown.stale);
        assert_eq!(silent_shown.state, State::Working);
        assert!(!attention.shown(&targets[0], now).unwrap().stale);
        poll_until(&mut attention, &targets, &mut sessions, |attention| {
            current(attention, "session-1") == Some(State::Finished)
        });
        let shown = attention.shown(&targets[0], Instant::now()).unwrap();
        assert_eq!(shown.record.unwrap().app, "claude-code");

        // A replaced run drops its attention until its own reply.
        attention.apply(
            &targets,
            "session-1",
            "other-run",
            status(u64::MAX, vec![record("a", Done, None, 1)]),
            Instant::now(),
        );
        assert_eq!(current(&attention, "session-1"), Some(State::Finished));
        attention.retain(&targets, &sessions);
        assert_eq!(current(&attention, "session-1"), None);
        poll_until(&mut attention, &targets, &mut sessions, |attention| {
            current(attention, "session-1") == Some(State::Finished)
        });

        // Ended runs and removed targets drop theirs.
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(unsafe { libc::kill(silent, libc::SIGCONT) }, 0);
        crate::sessions::stop_managed_sessions(&mut sessions[..1], Duration::from_secs(5)).unwrap();
        attention.retain(&targets, &sessions);
        assert_eq!(current(&attention, "session-1"), None);
        assert_eq!(current(&attention, "session-2"), Some(State::Working));
        attention.retain(&[], &sessions);
        assert_eq!(current(&attention, "session-2"), None);
        crate::sessions::stop_managed_sessions(&mut sessions[1..], Duration::from_secs(5)).unwrap();
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }
}
