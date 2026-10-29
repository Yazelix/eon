//! Agent attention (EON-C31): polls the program status of every pane and
//! popup terminal off the request path and derives what needs input, what has
//! finished and what has been seen. Orbit owns the records; this owns their
//! meaning.

use super::{
    sessions::{self, ProgramStatusRead, RunningSession},
    workspace::is_directory_picker_session,
};
use eon_workspace_protocol::v25::{
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

/// A pane or popup terminal, in window order: workspace by workspace, tab by
/// tab, panes column by column, then popups.
pub(super) struct Target {
    workspace: usize,
    tab: usize,
    position: usize,
    /// The pane id, or the popup instance.
    id: String,
    session: String,
    agent: bool,
}

/// The pane and popup terminals of every workspace, given in window order,
/// never a directory picker, and the session of the focused one: the shown
/// popup, else the selected pane, of the shown workspace's active tab
/// (EON-C33).
pub(super) fn targets(workspaces: &[Snapshot]) -> (Vec<Target>, Option<String>) {
    let mut targets = Vec::new();
    let mut focus = None;
    for (workspace, snapshot) in workspaces.iter().enumerate() {
        let shown = snapshot.workspace == snapshot.active_workspace;
        for (index, tab) in snapshot.tabs.iter().enumerate() {
            let active = shown && tab.id == snapshot.active_tab;
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
                    workspace,
                    tab: index,
                    position,
                    id: id.clone(),
                    session: session.clone(),
                    agent,
                });
                position += 1;
            }
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
    /// The change sequence it raised at.
    changed: u64,
    seen: bool,
}

struct Record {
    record: ProgramRecord,
    /// Fixed on first sight of this change sequence.
    changed_at: Instant,
    raise: Option<Raise>,
}

impl Record {
    /// Its normalized state, given whether its terminal runs `background`
    /// processes, and the time its order key uses: when it entered `blocked`,
    /// else its raise, else its change. Background masks a raise without
    /// seeing it.
    fn normalized(&self, background: bool) -> (State, Instant) {
        let unseen = self.raise.filter(|raise| !raise.seen);
        match (self.record.state, unseen) {
            (ProgramState::Blocked, _) => (State::NeedsInput, self.changed_at),
            (ProgramState::Error, Some(raise)) => (State::Error, raise.at),
            (ProgramState::Done | ProgramState::Idle, _) if background => {
                (State::Background, self.changed_at)
            }
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
    /// Its background process count (ORB-C15).
    background: u8,
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
                let (state, at) = record.normalized(self.background > 0);
                (record, state, at)
            })
            .min_by_key(|&(record, state, _)| (state, Reverse(record.record.changed)))
    }

    /// A `working` record that changed after an unseen finish shows the
    /// program busy again, so that finish counts as seen. Errors stay.
    fn see_finishes_before_work(&mut self) {
        let Some(working) = self
            .records
            .iter()
            .filter(|record| record.record.state == ProgramState::Working)
            .map(|record| record.record.changed)
            .max()
        else {
            return;
        };
        for record in self.records.iter_mut().chain(&mut self.omitted) {
            if let (ProgramState::Done | ProgramState::Idle, Some(raise)) =
                (record.record.state, record.raise.as_mut())
                && raise.changed < working
            {
                raise.seen = true;
            }
        }
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

/// A queue entry's total order: class, fixed time, then workspace, tab and
/// terminal position.
pub(super) type Key = (State, Instant, usize, usize, usize);

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
        let fresh: Vec<Record> = status
            .records
            .into_iter()
            .map(|record| {
                // A cleared and re-reported record comes back with a new
                // creation sequence, so it is a new record and inherits nothing.
                let cached = records.iter().chain(&omitted).find(|cached| {
                    cached.record.id == record.id && cached.record.created == record.created
                });
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
                    changed: record.changed,
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
        let mut terminal = Terminal {
            session: session.into(),
            run: run.into(),
            update: status.update,
            count: status.count,
            background: status.background,
            name: status.terminal_title,
            records: fresh,
            omitted,
            replied: now,
        };
        terminal.see_finishes_before_work();
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

    /// Follows an accepted attention action, given the workspaces it left:
    /// `ToggleSidebar` flips visibility, and `FocusAttention` follows focus,
    /// moves the cursor to `jump` for a queue jump, and sees the target's
    /// raises when it has focus, also when it had focus already.
    pub(super) fn handled(
        &mut self,
        action: &Action,
        jump: Option<Key>,
        workspaces: &[Snapshot],
        attached: bool,
        direct_cross_workspace: bool,
    ) {
        match action {
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::FocusAttention { target, .. } => {
                if direct_cross_workspace {
                    self.cursor = None;
                }
                let (targets, focus) = targets(workspaces);
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

    /// The highest-priority state of a workspace, or of one of its tabs; a
    /// stale terminal counts with its last state.
    fn rollup(
        &self,
        targets: &[Target],
        workspace: usize,
        tab: Option<usize>,
        now: Instant,
    ) -> Option<State> {
        targets
            .iter()
            .filter(|target| target.workspace == workspace)
            .filter(|target| tab.is_none_or(|tab| target.tab == tab))
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
        (state <= State::Finished).then_some((
            state,
            at,
            target.workspace,
            target.tab,
            target.position,
        ))
    }

    fn queue<'a>(&self, targets: &'a [Target]) -> Vec<(Key, &'a Target)> {
        let mut queue: Vec<_> = targets
            .iter()
            .filter_map(|target| Some((self.entry(targets, &target.session)?, target)))
            .collect();
        queue.sort_by_key(|&(key, _)| key);
        queue
    }

    /// `NextAttention` (`forward`) or `PreviousAttention`: the entry after or
    /// before the cursor, wrapping, or the front of the queue after a reset;
    /// none for an empty queue. Its key becomes the cursor once the jump is
    /// accepted.
    fn next<'a>(&self, targets: &'a [Target], forward: bool) -> Option<(Key, &'a Target)> {
        let queue = self.queue(targets);
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

    /// The workspace and `FocusAttention` of a queue jump over `workspaces`,
    /// with the cursor it moves to; none for an empty queue.
    pub(super) fn jump(
        &self,
        workspaces: &[Snapshot],
        forward: bool,
    ) -> Option<(String, Action, Key)> {
        let (targets, _) = targets(workspaces);
        let (key, target) = self.next(&targets, forward)?;
        let workspace = &workspaces[target.workspace];
        let action = Action::FocusAttention {
            tab: workspace.tabs[target.tab].id.clone(),
            target: target.id.clone(),
        };
        Some((workspace.workspace.clone(), action, key))
    }

    /// Sets each terminal's attention, each tab's and workspace's rollup and
    /// the sidebar of `snapshot`, which projects one of `workspaces`, after
    /// following the focus the shown one has: a reply to a focus change
    /// already shows the newly focused terminal's raises as seen.
    pub(super) fn present(
        &mut self,
        snapshot: &mut Snapshot,
        workspaces: &[Snapshot],
        attached: bool,
        now: Instant,
    ) {
        let (targets, focus) = targets(workspaces);
        self.observe(focus, attached);
        let projected = workspaces
            .iter()
            .position(|workspace| workspace.workspace == snapshot.workspace);
        let attention = |id: &str| {
            let target = targets
                .iter()
                .find(|target| Some(target.workspace) == projected && target.id == id)?;
            self.attention(target, now)
        };
        for (index, tab) in snapshot.tabs.iter_mut().enumerate() {
            for pane in tab.columns.iter_mut().flat_map(|column| &mut column.panes) {
                pane.attention = attention(&pane.id);
            }
            for popup in &mut tab.popups {
                popup.attention = attention(&popup.id);
            }
            tab.attention_rollup =
                projected.and_then(|workspace| self.rollup(&targets, workspace, Some(index), now));
        }
        for info in &mut snapshot.workspaces {
            info.attention = workspaces
                .iter()
                .position(|workspace| workspace.workspace == info.id)
                .and_then(|index| self.rollup(&targets, index, None, now));
        }
        snapshot.sidebar = self.sidebar;
        snapshot.attention_queue = self
            .queue(&targets)
            .into_iter()
            .map(|(_, target)| {
                let workspace = &workspaces[target.workspace];
                eonw::QueueEntry {
                    workspace: workspace.workspace.clone(),
                    tab: workspace.tabs[target.tab].id.clone(),
                    target: target.id.clone(),
                    attention: self
                        .attention(target, now)
                        .expect("queued terminal has attention"),
                }
            })
            .collect();
    }

    fn attention(&self, target: &Target, now: Instant) -> Option<eonw::Attention> {
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
    use eon_workspace_protocol::v25::{Column, Pane, Popup, PopupGeometry, Tab};
    use std::{fs, slice, thread};

    /// A record created at its change `changed`.
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
            background: 0,
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
                        label: format!("pane-{session}"),
                        pinned_name: None,
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
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v25::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: tabs.len(),
                attention: None,
                recency: 0,
            }],
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
            attention_queue: Vec::new(),
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
        let (found, focus) = targets(&[snapshot("t2", tabs.clone())]);
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
        assert_eq!(targets(&[snapshot("t1", tabs)]).1.as_deref(), Some("s1"));
    }

    #[test]
    fn raises_follow_the_changes_eon_c31_names() {
        let (targets, _) = targets(&[snapshot("t", vec![tab("t", &["s"], &[], None)])]);
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let at = |attention: &mut Attention, update: u64, records: Vec<ProgramRecord>| {
            let now = base + Duration::from_secs(update);
            attention.apply(&targets, "s", "run", status(update, records), now);
            state(attention, &targets, "s", now)
        };
        let secs = |secs| Some(base + Duration::from_secs(secs));
        // One record, created at update 1, through its state changes.
        let a = |state, previous, changed| ProgramRecord {
            created: 1,
            ..record("a", state, previous, changed)
        };

        // Created in done; an idle from that done keeps its raise, which
        // orders the queue, while the row shows the idle's own change.
        assert_eq!(
            at(&mut attention, 1, vec![a(Done, None, 1)]),
            Some(State::Finished)
        );
        assert_eq!(
            at(&mut attention, 2, vec![a(Idle, Some(Done), 2)]),
            Some(State::Finished)
        );
        assert_eq!(attention.entry(&targets, "s").map(|key| key.1), secs(1));
        assert_eq!(
            attention.shown(&targets[0], base).unwrap().changed_at,
            secs(2)
        );
        attention.see("s");
        // A same-state re-report keeps the seen raise.
        let mut repeat = a(Idle, Some(Done), 2);
        repeat.updated = 3;
        assert_eq!(at(&mut attention, 3, vec![repeat]), Some(State::Idle));
        // working -> idle raises; blocked -> idle and error -> idle do not.
        assert_eq!(
            at(&mut attention, 4, vec![a(Working, Some(Idle), 4)]),
            Some(State::Working)
        );
        assert_eq!(
            at(&mut attention, 5, vec![a(Idle, Some(Working), 5)]),
            Some(State::Finished)
        );
        attention.see("s");
        assert_eq!(
            at(&mut attention, 6, vec![a(Blocked, Some(Idle), 6)]),
            Some(State::NeedsInput)
        );
        assert_eq!(
            at(&mut attention, 7, vec![a(Idle, Some(Blocked), 7)]),
            Some(State::Idle)
        );
        assert_eq!(
            at(&mut attention, 8, vec![a(Error, Some(Idle), 8)]),
            Some(State::Error)
        );
        assert_eq!(
            at(&mut attention, 9, vec![a(Idle, Some(Error), 9)]),
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
        let (targets, _) = targets(&[snapshot("t", vec![tab("t", &["s"], &[], None)])]);
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
        // So does one that clears and reports again within one poll and
        // arrives as an idle from its new done.
        attention.see("s");
        let again = ProgramRecord {
            created: 7,
            ..record("a", Idle, Some(Done), 8)
        };
        apply(&mut attention, 8, vec![again]);
        assert_eq!(
            state(&attention, &targets, "s", base),
            Some(State::Finished)
        );

        // Within one terminal, a tie in state goes to the most recent change.
        apply(
            &mut attention,
            11,
            vec![
                record("x", Blocked, None, 10),
                record("y", Blocked, None, 11),
            ],
        );
        let shown = attention.shown(&targets[0], base).unwrap();
        assert_eq!(shown.record.unwrap().id, "y");
        assert_eq!(
            attention.entry(&targets, "s").map(|key| key.1),
            shown.changed_at
        );
    }

    #[test]
    fn newer_work_sees_an_older_finish_but_never_an_error() {
        let (targets, _) = targets(&[snapshot("t", vec![tab("t", &["s"], &[], None)])]);
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let at = |attention: &mut Attention, update: u64, records: Vec<ProgramRecord>| {
            let now = base + Duration::from_secs(update);
            attention.apply(&targets, "s", "run", status(update, records), now);
            state(attention, &targets, "s", now)
        };
        // As Claude Code reports: its session's record has no id, and each
        // subagent's has its own, which goes working, then is cleared.
        let main = |state, previous, changed| ProgramRecord {
            created: 1,
            ..record("", state, previous, changed)
        };
        let agent = |created| record("a8196066adeaa6815", Working, None, created);

        // A finish after the work raises.
        assert_eq!(
            at(&mut attention, 2, vec![main(Working, None, 1), agent(2)]),
            Some(State::Working)
        );
        assert_eq!(
            at(&mut attention, 3, vec![main(Done, Some(Working), 3)]),
            Some(State::Finished)
        );
        // Work that changed after an unseen finish hides it, and the finish
        // stays seen once the work stops.
        assert_eq!(
            at(
                &mut attention,
                4,
                vec![main(Done, Some(Working), 3), agent(4)]
            ),
            Some(State::Working)
        );
        assert_eq!(
            at(&mut attention, 5, vec![main(Done, Some(Working), 3)]),
            Some(State::Idle)
        );
        // An older working record does not hide a newer finish.
        assert_eq!(
            at(
                &mut attention,
                7,
                vec![main(Done, Some(Working), 7), agent(6)]
            ),
            Some(State::Finished)
        );
        assert_eq!(
            attention.entry(&targets, "s").map(|key| key.1),
            Some(base + Duration::from_secs(7))
        );
        // Newer work never sees an error.
        assert_eq!(
            at(
                &mut attention,
                9,
                vec![main(Error, Some(Done), 8), agent(9)]
            ),
            Some(State::Error)
        );

        // An idle that keeps its done's raise compares at that done: first
        // seen beside work that changed between the two, it shows the work.
        let c = |state, previous, changed| ProgramRecord {
            created: 10,
            ..record("c", state, previous, changed)
        };
        let w = |state, previous, changed| ProgramRecord {
            created: 11,
            ..record("w", state, previous, changed)
        };
        assert_eq!(
            at(&mut attention, 10, vec![c(Done, None, 10)]),
            Some(State::Finished)
        );
        assert_eq!(
            at(
                &mut attention,
                12,
                vec![c(Idle, Some(Done), 12), w(Working, None, 11)]
            ),
            Some(State::Working)
        );
        // The working record's own finish raises, and an idle raised from
        // working, as Codex reports, is seen by newer work in turn.
        assert_eq!(
            at(
                &mut attention,
                13,
                vec![c(Idle, Some(Done), 12), w(Idle, Some(Working), 13)]
            ),
            Some(State::Finished)
        );
        assert_eq!(
            at(
                &mut attention,
                14,
                vec![
                    c(Idle, Some(Done), 12),
                    w(Idle, Some(Working), 13),
                    record("y", Working, None, 14)
                ]
            ),
            Some(State::Working)
        );
    }

    #[test]
    fn background_masks_finishes_and_idle_and_holds_their_raise() {
        let tabs = vec![tab("t", &["s1", "s2", "s3", "s4"], &[], None)];
        let (targets, _) = targets(&[snapshot("t", tabs)]);
        let mut attention = Attention::new(true);
        let base = Instant::now();
        let at = |attention: &mut Attention,
                  session: &str,
                  update: u64,
                  background: u8,
                  records: Vec<ProgramRecord>| {
            let now = base + Duration::from_secs(update);
            let reply = ProgramStatus {
                background,
                ..status(update, records)
            };
            attention.apply(&targets, session, "run", reply, now);
            state(attention, &targets, session, now)
        };
        // As Claude Code reports a background task: its session's record goes
        // done while the task runs, so it waits on that work, out of the queue.
        let main = |state, previous, changed| ProgramRecord {
            created: 1,
            ..record("", state, previous, changed)
        };
        assert_eq!(
            at(
                &mut attention,
                "s1",
                2,
                1,
                vec![main(Done, Some(Working), 2)]
            ),
            Some(State::Background)
        );
        assert_eq!(attention.entry(&targets, "s1"), None);
        // When the work ends, the held raise shows finished.
        assert_eq!(
            at(
                &mut attention,
                "s1",
                3,
                0,
                vec![main(Done, Some(Working), 2)]
            ),
            Some(State::Finished)
        );
        // An idle, raised or not, waits too; errors and needs input show.
        assert_eq!(
            at(&mut attention, "s1", 4, 2, vec![main(Idle, Some(Done), 4)]),
            Some(State::Background)
        );
        assert_eq!(
            at(&mut attention, "s2", 1, 1, vec![record("", Idle, None, 1)]),
            Some(State::Background)
        );
        assert_eq!(
            at(&mut attention, "s3", 1, 1, vec![record("", Error, None, 1)]),
            Some(State::Error)
        );
        assert_eq!(
            at(
                &mut attention,
                "s4",
                1,
                1,
                vec![record("", Blocked, None, 1)]
            ),
            Some(State::NeedsInput)
        );

        // Work that changed after the finish sees it, as without background:
        // a subagent starts, then ends with the background work.
        let agent = record("a8196066adeaa6815", Working, None, 5);
        assert_eq!(
            at(
                &mut attention,
                "s1",
                5,
                1,
                vec![main(Idle, Some(Done), 4), agent]
            ),
            Some(State::Working)
        );
        assert_eq!(
            at(&mut attention, "s1", 6, 0, vec![main(Idle, Some(Done), 4)]),
            Some(State::Idle)
        );
        // Focus sees a raise that background holds.
        assert_eq!(
            at(&mut attention, "s3", 2, 1, vec![record("", Done, None, 2)]),
            Some(State::Background)
        );
        attention.observe(Some("s3".into()), true);
        assert_eq!(
            at(&mut attention, "s3", 3, 0, vec![record("", Done, None, 2)]),
            Some(State::Idle)
        );

        // Background ranks below working and above idle.
        let rollup = |attention: &Attention| attention.rollup(&targets, 0, Some(0), base);
        at(
            &mut attention,
            "s3",
            4,
            0,
            vec![record("", Working, None, 4)],
        );
        at(&mut attention, "s4", 2, 0, vec![record("", Idle, None, 2)]);
        assert_eq!(rollup(&attention), Some(State::Working));
        at(&mut attention, "s3", 5, 0, vec![]);
        assert_eq!(rollup(&attention), Some(State::Background));
        at(&mut attention, "s2", 2, 0, vec![record("", Idle, None, 1)]);
        assert_eq!(rollup(&attention), Some(State::Idle));
    }

    #[test]
    fn identity_fixes_change_times_and_discards_older_replies_until_a_new_run() {
        let (targets, _) = targets(&[snapshot("t", vec![tab("t", &["s"], &[], None)])]);
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
            background: 0,
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
                background: 0,
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
        let (targets, _) = targets(slice::from_ref(&shown_snapshot));
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
            let window = [presented.clone()];
            attention.present(&mut presented, &window, false, late);
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
        let (targets, _) = targets(&[snapshot("t", vec![tab("t", &["s1", "s2"], &[], None)])]);
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

    /// Workspace `id`'s own snapshot while `shown` is the shown one.
    fn in_workspace(id: &str, shown: &str, active: &str, tabs: Vec<Tab>) -> Snapshot {
        Snapshot {
            workspace: id.into(),
            active_workspace: shown.into(),
            ..snapshot(active, tabs)
        }
    }

    #[test]
    fn the_snapshot_carries_the_queue_in_its_total_order_across_workspaces() {
        let mut window = [
            in_workspace(
                "w1",
                "w1",
                "t1",
                vec![
                    tab(
                        "t1",
                        &["finished", "working", "idle", "background", "seen"],
                        &[("agent", "unavailable"), ("project", "directory-picker-1")],
                        None,
                    ),
                    tab("t2", &["error", "equal-a", "equal-b"], &[], None),
                ],
            ),
            in_workspace(
                "w2",
                "w1",
                "t3",
                vec![tab("t3", &["needs", "other"], &[("agent", "hidden")], None)],
            ),
        ];
        // Target ids are local to a workspace, even when the sessions differ.
        window[1].tabs[0].columns[0].panes[1].id = "pane-finished".into();
        window[0].tabs[0].selected_pane = Some("pane-working".into());
        window[0].tabs[0].columns[0].expanded = "pane-working".into();
        let infos = window
            .iter()
            .enumerate()
            .map(|(index, workspace)| eonw::WorkspaceInfo {
                id: workspace.workspace.clone(),
                name: None,
                tab_count: workspace.tabs.len(),
                attention: None,
                recency: index as u8,
            })
            .collect::<Vec<_>>();
        for workspace in &mut window {
            workspace.workspaces = infos.clone();
        }
        let (targets, _) = targets(&window);
        let base = Instant::now();
        let now = base + Duration::from_secs(4);
        let mut attention = Attention::new(true);
        for (session, state, seconds) in [
            ("finished", Done, 0),
            ("error", Error, 1),
            ("needs", Blocked, 2),
            ("hidden", Blocked, 3),
            ("equal-b", Done, 0),
            ("equal-a", Done, 0),
            ("other", Done, 1),
            ("working", Working, 0),
            ("idle", Idle, 0),
            ("background", Done, 0),
            ("seen", Done, 0),
        ] {
            let mut reply = status(1, vec![record("a", state, None, 1)]);
            if session == "background" {
                reply.background = 1;
            }
            attention.apply(
                &targets,
                session,
                "run",
                reply,
                base + Duration::from_secs(seconds),
            );
        }
        attention.see("seen");
        let expected = [
            ("w2", "t3", "pane-needs", State::NeedsInput),
            ("w2", "t3", "popup-hidden", State::NeedsInput),
            ("w1", "t2", "pane-error", State::Error),
            ("w1", "t1", "pane-finished", State::Finished),
            ("w1", "t2", "pane-equal-a", State::Finished),
            ("w1", "t2", "pane-equal-b", State::Finished),
            ("w2", "t3", "pane-finished", State::Finished),
        ];
        let mut projected = window[0].clone();
        attention.present(&mut projected, &window, true, now);
        let entries = projected
            .attention_queue
            .iter()
            .map(|entry| {
                (
                    entry.workspace.as_str(),
                    entry.tab.as_str(),
                    entry.target.as_str(),
                    entry.attention.state,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(entries, expected);
        assert!(
            projected.attention_queue[3]
                .attention
                .record
                .as_ref()
                .unwrap()
                .stale
        );
        assert_eq!(
            projected.tabs[0].popups[0]
                .attention
                .as_ref()
                .unwrap()
                .state,
            State::Unavailable
        );
        for source in &window {
            let mut reply = source.clone();
            attention.present(&mut reply, &window, true, now);
            assert_eq!(reply.attention_queue, projected.attention_queue);
            for entry in reply
                .attention_queue
                .iter()
                .filter(|entry| entry.workspace == reply.workspace)
            {
                let tab = reply.tabs.iter().find(|tab| tab.id == entry.tab).unwrap();
                let carried = tab
                    .panes()
                    .map(|pane| (&pane.id, &pane.attention))
                    .chain(tab.popups.iter().map(|popup| (&popup.id, &popup.attention)))
                    .find(|(id, _)| **id == entry.target)
                    .unwrap()
                    .1;
                assert_eq!(carried.as_ref(), Some(&entry.attention));
            }
        }
        let mut absent = projected.clone();
        absent.workspace = "w9".into();
        attention.present(&mut absent, &window, true, now);
        assert_eq!(absent.attention_queue, projected.attention_queue);

        // Apply each jump's focus and returned cursor as the supervisor does.
        for &(workspace, tab, target, _) in expected.iter().chain(&expected[..1]) {
            let (destination, action, key) = attention.jump(&window, true).unwrap();
            assert_eq!(destination, workspace);
            assert_eq!(
                action,
                Action::FocusAttention {
                    tab: tab.into(),
                    target: target.into()
                }
            );
            for snapshot in &mut window {
                snapshot.active_workspace = workspace.into();
                if snapshot.workspace != workspace {
                    continue;
                }
                snapshot.active_tab = tab.into();
                let tab = snapshot
                    .tabs
                    .iter_mut()
                    .find(|item| item.id == tab)
                    .unwrap();
                if tab.popups.iter().any(|popup| popup.id == target) {
                    tab.selected_popup = Some(target.into());
                } else {
                    tab.selected_popup = None;
                    tab.selected_pane = Some(target.into());
                    tab.columns[0].expanded = target.into();
                }
            }
            attention.handled(&action, Some(key), &window, true, false);
        }
    }

    #[test]
    fn cross_workspace_focus_sees_the_finish_and_resets_the_queue_cursor() {
        use crate::{
            managed_environment::{PopupCatalog, PopupCommand, PopupDefinition},
            workspace::{Session, SessionEnd, WorkspaceSet},
        };
        use eon_workspace_protocol::v25::{PopupKey, WorkspaceAction};
        let mut window = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            (1..=3)
                .map(|n| {
                    (
                        n,
                        Session {
                            id: format!("session-{n}"),
                            endpoint: format!("/runtime/{n}.sock").into(),
                        },
                    )
                })
                .collect(),
            PopupCatalog {
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: vec![PopupDefinition {
                    id: "project".into(),
                    label: "Project".into(),
                    shortcut: PopupKey::Unbound,
                    command: PopupCommand::Project,
                    keep_alive: false,
                }],
            },
        )
        .unwrap();
        window
            .dispatch(
                "new",
                None,
                Action::Workspace(WorkspaceAction::NewWorkspace {
                    name: None,
                    focus: false,
                }),
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap();
        let picker = window.transient_sessions()[0].clone();
        window
            .session_ended(&picker, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        let before = window.snapshots();
        let hidden = before[1].tabs[0].panes().next().unwrap();
        let (targets, focus) = targets(&before);
        let mut attention = Attention::new(true);
        attention.observe(focus, true);
        let now = Instant::now();
        for (session, state) in [
            ("session-2", Blocked),
            ("session-3", Blocked),
            (&hidden.session, Done),
        ] {
            attention.apply(
                &targets,
                session,
                "run",
                status(1, vec![record("a", state, None, 1)]),
                now,
            );
        }
        assert_eq!(
            step(&mut attention, &targets, true).unwrap().session,
            "session-2"
        );
        // A hidden caller can focus the already-focused shown terminal too.
        let shown = before[0].tabs[0].panes().next().unwrap();
        let already_shown = Action::FocusAttention {
            tab: before[0].tabs[0].id.clone(),
            target: shown.id.clone(),
        };
        let cursor_before_local = attention.cursor;
        assert!(
            window
                .attention_destination(None, &before[0].tabs[0].id)
                .is_none()
        );
        window
            .dispatch(
                "local-same-focus",
                None,
                already_shown.clone(),
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap();
        attention.handled(&already_shown, None, &window.snapshots(), true, false);
        assert_eq!(
            attention.cursor, cursor_before_local,
            "same-workspace focus behavior stays unchanged"
        );
        let direct_cross_workspace = window
            .attention_destination(Some(&hidden.session), &before[0].tabs[0].id)
            .is_some();
        assert!(direct_cross_workspace);
        window
            .dispatch(
                "shown-from-hidden",
                Some(&hidden.session),
                already_shown.clone(),
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap();
        attention.handled(
            &already_shown,
            None,
            &window.snapshots(),
            true,
            direct_cross_workspace,
        );
        assert_eq!(
            attention.cursor, None,
            "a direct cross-workspace focus resets the cursor even when shown focus is unchanged"
        );
        step(&mut attention, &targets, true).unwrap();
        let cursor = attention.cursor;
        let tab = before[1].tabs[0].id.clone();
        let stale = window
            .dispatch(
                "stale",
                None,
                Action::FocusAttention {
                    tab: tab.clone(),
                    target: "missing-pane".into(),
                },
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap_err();
        assert_eq!(stale.code, "stale-target");
        assert_eq!(window.snapshots(), before);
        assert_eq!(attention.cursor, cursor);
        let action = Action::FocusAttention {
            tab,
            target: hidden.id.clone(),
        };
        window
            .dispatch(
                "focus",
                None,
                action.clone(),
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap();
        attention.handled(&action, None, &window.snapshots(), true, false);
        let mut reply = window.snapshot();
        attention.present(&mut reply, &window.snapshots(), true, now);
        assert_eq!(reply.workspace, before[1].workspace);
        assert_eq!(
            reply.tabs[0].selected_pane.as_deref(),
            Some(hidden.id.as_str())
        );
        assert!(
            !reply
                .attention_queue
                .iter()
                .any(|entry| entry.target == hidden.id)
        );
        assert_eq!(attention.cursor, None);
        let (_, next, _) = attention.jump(&window.snapshots(), true).unwrap();
        assert_eq!(
            next,
            Action::FocusAttention {
                tab: before[0].tabs[0].id.clone(),
                target: "p2".into()
            }
        );
    }

    #[test]
    fn hidden_raises_stay_unseen_and_the_queue_crosses_into_their_workspace() {
        let window = |shown: &str, focus: &str| {
            let mut hidden = tab("t2", &["s2", "s3"], &[], None);
            hidden.selected_pane = Some(format!("pane-{focus}"));
            hidden.columns[0].expanded = format!("pane-{focus}");
            [
                in_workspace("w1", shown, "t1", vec![tab("t1", &["s1"], &[], None)]),
                in_workspace("w2", shown, "t2", vec![hidden]),
            ]
        };
        let before = window("w1", "s2");
        let (targets, focus) = targets(&before);
        assert_eq!(
            focus.as_deref(),
            Some("s1"),
            "only the shown workspace focuses"
        );
        let mut attention = Attention::new(true);
        let now = Instant::now();
        attention.observe(focus, true);
        for session in ["s2", "s3"] {
            attention.apply(
                &targets,
                session,
                "run",
                status(1, vec![record("a", Done, None, 1)]),
                now,
            );
        }

        // A reply to a caller in the hidden workspace sees nothing there, and each workspace
        // carries its own rollup.
        let mut reply = before[1].clone();
        reply.workspaces = ["w1", "w2"]
            .map(|id| eon_workspace_protocol::v25::WorkspaceInfo {
                id: id.into(),
                ..reply.workspaces[0].clone()
            })
            .into();
        attention.present(&mut reply, &before, true, now);
        assert_eq!(
            reply.tabs[0].columns[0].panes[0]
                .attention
                .as_ref()
                .unwrap()
                .state,
            State::Finished
        );
        assert_eq!(reply.tabs[0].attention_rollup, Some(State::Finished));
        let rollups: Vec<_> = reply.workspaces.iter().map(|info| info.attention).collect();
        assert_eq!(rollups, [None, Some(State::Finished)]);

        // The queue reaches the hidden terminal; its jump shows w2 and sees
        // only the exact terminal it focuses.
        let (workspace, action, key) = attention.jump(&before, true).unwrap();
        assert_eq!(workspace, "w2");
        assert_eq!(
            action,
            Action::FocusAttention {
                tab: "t2".into(),
                target: "pane-s2".into(),
            }
        );
        attention.handled(&action, Some(key), &window("w2", "s2"), true, false);
        assert_eq!(state(&attention, &targets, "s2", now), Some(State::Idle));
        assert_eq!(
            state(&attention, &targets, "s3", now),
            Some(State::Finished)
        );

        // Showing w1 again and then w2 with s3 focused sees s3.
        attention.observe(Some("s1".into()), true);
        assert_eq!(
            state(&attention, &targets, "s3", now),
            Some(State::Finished)
        );
        let after = window("w2", "s3");
        let mut shown = after[1].clone();
        attention.present(&mut shown, &after, true, now);
        assert_eq!(state(&attention, &targets, "s3", now), Some(State::Idle));
    }

    #[test]
    fn a_focus_reply_already_shows_the_focused_finish_as_seen() {
        let before = snapshot("t", vec![tab("t", &["s1", "s2"], &[], None)]);
        let (targets, focus) = targets(slice::from_ref(&before));
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
        let window = [after.clone()];
        attention.present(&mut after, &window, true, now);
        let pane = &after.tabs[0].columns[0].panes[1];
        assert_eq!(pane.attention.as_ref().unwrap().state, State::Idle);
        assert_eq!(after.tabs[0].attention_rollup, Some(State::Idle));
        assert_eq!(after.sidebar, Sidebar::Open);
    }

    #[test]
    fn the_queue_walks_its_total_order_and_its_cursor_resets() {
        let (targets, _) = targets(&[snapshot(
            "t1",
            vec![
                tab("t1", &["s1", "s2"], &[], None),
                tab("t2", &["s3"], &[("agent", "s4")], None),
            ],
        )]);
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
        let (workspace, action, key) = attention.jump(slice::from_ref(&jumped), true).unwrap();
        assert_eq!(workspace, "w1");
        assert_eq!(
            action,
            Action::FocusAttention {
                tab: "t2".into(),
                target: "popup-s4".into(),
            }
        );
        attention.handled(&action, Some(key), slice::from_ref(&jumped), true, false);
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

        assert_eq!(
            attention.rollup(&targets, 0, Some(0), base),
            Some(State::NeedsInput)
        );
        assert_eq!(
            attention.rollup(&targets, 0, Some(1), base),
            Some(State::NeedsInput)
        );
        attention.apply(&targets, "s4", "run", status(3, Vec::new()), base);
        assert_eq!(
            attention.rollup(&targets, 0, Some(1), base),
            Some(State::Finished)
        );

        assert_eq!(attention.sidebar, Sidebar::Open);
        attention.handled(
            &Action::ToggleSidebar,
            None,
            slice::from_ref(&jumped),
            true,
            false,
        );
        assert_eq!(attention.sidebar, Sidebar::Closed);
        attention.toggle_sidebar();
        assert_eq!(attention.sidebar, Sidebar::Open);
        attention.sidebar = Sidebar::Off;
        attention.toggle_sidebar();
        assert_eq!(attention.sidebar, Sidebar::Off);
    }

    #[test]
    fn equal_times_order_by_tab_then_terminal_position() {
        let (targets, _) = targets(&[snapshot(
            "t1",
            vec![
                tab("t1", &["s1", "s2"], &[], None),
                tab("t2", &["s3"], &[], None),
            ],
        )]);
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
        let (targets, _) = targets(&[snapshot(
            "t",
            vec![tab("t", &["session-1", "session-2"], &[], None)],
        )]);
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
