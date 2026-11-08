//! EON-C39 event subscriptions: the `Subscribe` request's filter and resume
//! point, and the `SUBSCRIBED`, `TABS`, `EVENT`, `GAP` and `END` messages
//! that its connection carries, each one ordinary EONW frame.
//!
//! A snapshot crosses as a `SUBSCRIBED` or `GAP` message without tabs, then
//! one `TABS` message per workspace with admitted tabs, so every message
//! stays within the frame bound at the window's 256 terminals. The values
//! mirror EON-C39's JSON objects; Eon Runtime fills them from its event
//! projection, and the CLI joins a snapshot's messages into one JSON line.

use std::collections::HashSet;

use super::{
    Attention, AttentionState, BusyTerminal, Decoder, Encoder, Error, FAILURE, Failure,
    MAX_COLUMNS, MAX_ID_BYTES, MAX_PANE_LABEL_BYTES, MAX_SESSIONS, MAX_TABS,
    MAX_WORKSPACE_NAME_BYTES, MAX_WORKSPACES, Pane, Popup, Result, Tab, TerminalTarget, WireTab,
    WorkspaceInfo, assemble_tab, decode_attention, decode_bool, decode_caller, decode_failure,
    decode_layout, decode_optional, decode_pane, decode_popup, decode_state, decode_target,
    decode_u64, decode_wire_tab, encode_attention, encode_caller, encode_failure, encode_layout,
    encode_optional, encode_pane, encode_popup, encode_state, encode_target, encode_wire_tab,
    entry_id, frame, identity, queued, unframe, validate_attention, validate_busy_terminal,
    validate_columns, validate_endpoint, validate_pane_label, validate_tabs, validate_target,
    validate_window_id, validate_workspace_id, validate_workspace_info, validate_workspace_name,
};

pub(super) const SUBSCRIBE: u8 = 40;
const SUBSCRIBED: u8 = 136;
const TABS: u8 = 137;
const EVENT: u8 = 138;
const GAP: u8 = 139;
const END: u8 = 140;

/// How many attention states a filter can list.
const ATTENTION_STATES: usize = 7;

/// A window holds at most one tab per terminal.
pub const MAX_FILTER_TABS: usize = MAX_SESSIONS;
pub const MAX_FILTER_TARGETS: usize = MAX_SESSIONS;

/// An event kind, as `--kind` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Workspace,
    Tab,
    Pane,
    Popup,
    Attention,
    Queue,
    Busy,
}

const EVENT_KINDS: [EventKind; 7] = [
    EventKind::Workspace,
    EventKind::Tab,
    EventKind::Pane,
    EventKind::Popup,
    EventKind::Attention,
    EventKind::Queue,
    EventKind::Busy,
];

/// What a subscription admits. Different fields combine with AND, entries
/// of one field with OR, and an empty field admits everything. Entries are
/// distinct.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// `sN` ids or names, which the Runtime resolves once (EON-C33).
    pub workspaces: Vec<String>,
    pub tabs: Vec<String>,
    /// Pane ids or popup instances, followed by their terminal.
    pub targets: Vec<String>,
    pub kinds: Vec<EventKind>,
    /// An `attention` event passes when its new or previous state is listed.
    pub states: Vec<AttentionState>,
}

/// `--after STREAM:SEQ`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resume {
    pub stream: String,
    pub seq: u64,
}

/// A workspace as `eon session ls --json` lists it, plus its active tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub workspace: WorkspaceInfo,
    pub active_tab: String,
}

/// An attention queue entry with its state in place of its attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub workspace: String,
    pub tab: String,
    pub target: String,
    pub state: AttentionState,
}

/// A snapshot's state apart from its tabs, which follow in `TABS` messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub workspaces: Vec<WorkspaceEntry>,
    /// The window's whole queue in EON-C31 order.
    pub attention_queue: Vec<QueueItem>,
    /// The admitted busy terminals in window order, without the calling
    /// terminal or `eon session`'s 32-terminal bound; None in eonbase.
    pub busy: Option<Vec<BusyTerminal>>,
    /// How many `TABS` messages follow: one per workspace with admitted tabs.
    pub tab_messages: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscribed {
    /// One supervisor process.
    pub stream: String,
    pub seq: u64,
    pub generation: String,
    /// Null for the default namespace.
    pub window: Option<String>,
    pub caller: Option<TerminalTarget>,
    /// None when the subscription resumed after `seq`.
    pub snapshot: Option<WindowSnapshot>,
}

/// One workspace's admitted tabs, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tabs {
    pub workspace: String,
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceChange {
    Created,
    Renamed,
    Closed,
    Shown,
    Registered,
    RegistrationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabChange {
    Created,
    Moved,
    Directory,
    Closed,
    Activated,
}

impl TabChange {
    /// Whether the event carries the workspace's tab order.
    pub fn carries_order(self) -> bool {
        matches!(self, Self::Created | Self::Moved | Self::Closed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneChange {
    Created,
    Moved,
    Renamed,
    Ended,
    Closed,
    Selected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupChange {
    Created,
    Shown,
    Hidden,
    Closed,
}

/// One column of a tab by pane id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnOrder {
    pub panes: Vec<String>,
    pub expanded: String,
}

// Tab payloads are already heap-owned; keep the direct value API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Workspace {
        change: WorkspaceChange,
        workspace: WorkspaceEntry,
    },
    Tab {
        change: TabChange,
        workspace: String,
        tab: Tab,
        /// The workspace's tab ids in order, exactly when
        /// [`TabChange::carries_order`].
        tabs: Option<Vec<String>>,
    },
    Pane {
        change: PaneChange,
        workspace: String,
        tab: String,
        pane: Pane,
        /// The pane's column, from 1, and whether it is that column's
        /// expanded pane; a closed pane's as they were.
        column: usize,
        expanded: bool,
        /// The tab's columns in order; a closed pane is in none, and none
        /// remain once its tab is gone.
        columns: Vec<ColumnOrder>,
    },
    Popup {
        change: PopupChange,
        workspace: String,
        tab: String,
        popup: Popup,
        /// Whether it is its tab's chosen popup.
        chosen: bool,
    },
    Attention {
        target: TerminalTarget,
        previous: Option<AttentionState>,
        attention: Option<Attention>,
    },
    Queue(Vec<QueueItem>),
    Busy {
        target: TerminalTarget,
        busy: bool,
        /// Its busy name, exactly while busy.
        name: Option<String>,
    },
}

impl Change {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Workspace { .. } => EventKind::Workspace,
            Self::Tab { .. } => EventKind::Tab,
            Self::Pane { .. } => EventKind::Pane,
            Self::Popup { .. } => EventKind::Popup,
            Self::Attention { .. } => EventKind::Attention,
            Self::Queue(_) => EventKind::Queue,
            Self::Busy { .. } => EventKind::Busy,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub change: Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    Overflow,
}

/// Pending events were dropped; the snapshot's tabs follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    /// The current event counter.
    pub seq: u64,
    pub reason: GapReason,
    pub snapshot: WindowSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// Stop, quit or the window's final close.
    Ended {
        generation: String,
    },
    TargetsGone,
    TooSlow,
}

/// One message of a subscription connection; `Failure` is the refusal.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionMessage {
    Subscribed(Subscribed),
    Tabs(Tabs),
    Event(Event),
    Gap(Gap),
    End(End),
    Failure(Failure),
}

fn distinct<T: PartialEq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .all(|(index, item)| !items[..index].contains(item))
}

pub(super) fn validate_subscribe(filter: &Filter, after: Option<&Resume>) -> Result<()> {
    let invalid = |field| Err(Error::InvalidValue { field });
    if filter.workspaces.len() > MAX_WORKSPACES || !distinct(&filter.workspaces) {
        return invalid("workspace filter");
    }
    for workspace in &filter.workspaces {
        validate_workspace_name(workspace)?;
    }
    if filter.tabs.len() > MAX_FILTER_TABS || !distinct(&filter.tabs) {
        return invalid("tab filter");
    }
    for tab in &filter.tabs {
        identity("tab filter", tab)?;
    }
    if filter.targets.len() > MAX_FILTER_TARGETS || !distinct(&filter.targets) {
        return invalid("target filter");
    }
    for target in &filter.targets {
        identity("target filter", target)?;
    }
    if !distinct(&filter.kinds) {
        return invalid("kind filter");
    }
    if !distinct(&filter.states) {
        return invalid("state filter");
    }
    if let Some(after) = after {
        identity("stream", &after.stream)?;
    }
    Ok(())
}

/// The filter's workspaces, tabs, targets, kinds and states, each as a
/// count and its entries, then an optional stream and `u64` seq.
pub(super) fn encode_subscribe(encoder: &mut Encoder, filter: &Filter, after: Option<&Resume>) {
    encoder.byte(SUBSCRIBE);
    for values in [&filter.workspaces, &filter.tabs, &filter.targets] {
        encoder.count(values.len());
        for value in values {
            encoder.string(value);
        }
    }
    encoder.count(filter.kinds.len());
    for kind in &filter.kinds {
        encoder.byte(kind_tag(*kind));
    }
    encoder.count(filter.states.len());
    for state in &filter.states {
        encode_state(encoder, Some(*state));
    }
    encoder.byte(u8::from(after.is_some()));
    if let Some(after) = after {
        encoder.string(&after.stream);
        encoder.bytes.extend_from_slice(&after.seq.to_le_bytes());
    }
}

pub(super) fn decode_subscribe(decoder: &mut Decoder<'_>) -> Result<(Filter, Option<Resume>)> {
    let strings = |decoder: &mut Decoder<'_>, field, count, length| {
        let count = decoder.count(field, count)?;
        (0..count)
            .map(|_| decoder.string(field, length))
            .collect::<Result<Vec<_>>>()
    };
    let workspaces = strings(
        decoder,
        "workspace filter",
        MAX_WORKSPACES,
        MAX_WORKSPACE_NAME_BYTES,
    )?;
    let tabs = strings(decoder, "tab filter", MAX_FILTER_TABS, MAX_ID_BYTES)?;
    let targets = strings(decoder, "target filter", MAX_FILTER_TARGETS, MAX_ID_BYTES)?;
    let count = decoder.count("kind filter", EVENT_KINDS.len())?;
    let kinds = (0..count)
        .map(|_| {
            let value = decoder.byte()?;
            EVENT_KINDS
                .get(usize::from(value))
                .copied()
                .ok_or(Error::InvalidTag {
                    field: "event kind",
                    value,
                })
        })
        .collect::<Result<_>>()?;
    let count = decoder.count("state filter", ATTENTION_STATES)?;
    let states = (0..count)
        .map(|_| {
            decode_state(decoder, "state filter")?.ok_or(Error::InvalidValue {
                field: "state filter",
            })
        })
        .collect::<Result<_>>()?;
    let after = decode_bool(decoder, "resume")?
        .then(|| {
            Ok(Resume {
                stream: decoder.string("stream", MAX_ID_BYTES)?,
                seq: decode_u64(decoder)?,
            })
        })
        .transpose()?;
    Ok((
        Filter {
            workspaces,
            tabs,
            targets,
            kinds,
            states,
        },
        after,
    ))
}

fn kind_tag(kind: EventKind) -> u8 {
    EVENT_KINDS
        .iter()
        .position(|item| *item == kind)
        .expect("listed event kind") as u8
}

fn change_tag(tag: u8, field: &'static str, count: u8) -> Result<u8> {
    if tag < count {
        Ok(tag)
    } else {
        Err(Error::InvalidTag { field, value: tag })
    }
}

fn validate_workspace_entry(entry: &WorkspaceEntry) -> Result<()> {
    validate_workspace_info(&entry.workspace)?;
    if usize::from(entry.workspace.recency) >= MAX_WORKSPACES {
        return Err(Error::InvalidSnapshot {
            field: "workspace recency",
        });
    }
    identity("active tab", &entry.active_tab)
}

fn validate_queue(queue: &[QueueItem]) -> Result<()> {
    if queue.len() > MAX_SESSIONS {
        return Err(Error::InvalidSnapshot {
            field: "attention queue",
        });
    }
    for item in queue {
        validate_workspace_id(&item.workspace)?;
        identity("queue tab", &item.tab)?;
        identity("queue target", &item.target)?;
        if !queued(item.state) {
            return Err(Error::InvalidSnapshot {
                field: "attention queue",
            });
        }
    }
    Ok(())
}

fn validate_snapshot(snapshot: &WindowSnapshot) -> Result<()> {
    let workspaces = &snapshot.workspaces;
    if workspaces.len() > MAX_WORKSPACES {
        return Err(Error::InvalidSnapshot {
            field: "workspaces",
        });
    }
    for entry in workspaces {
        validate_workspace_entry(entry)?;
    }
    if !distinct(
        &workspaces
            .iter()
            .map(|e| &e.workspace.id)
            .collect::<Vec<_>>(),
    ) || !distinct(
        &workspaces
            .iter()
            .filter_map(|e| e.workspace.persistent_id.as_ref())
            .collect::<Vec<_>>(),
    ) || !distinct(
        &workspaces
            .iter()
            .map(|e| e.workspace.recency)
            .collect::<Vec<_>>(),
    ) {
        return Err(Error::InvalidSnapshot {
            field: "workspaces",
        });
    }
    validate_queue(&snapshot.attention_queue)?;
    if let Some(busy) = &snapshot.busy {
        validate_busy_list(busy)?;
    }
    if snapshot.tab_messages > MAX_WORKSPACES {
        return Err(Error::InvalidSnapshot {
            field: "tab messages",
        });
    }
    Ok(())
}

fn encode_workspace_entry(encoder: &mut Encoder, entry: &WorkspaceEntry) {
    super::encode_workspace_info(encoder, &entry.workspace);
    encoder.string(&entry.active_tab);
}

fn decode_workspace_entry(decoder: &mut Decoder<'_>) -> Result<WorkspaceEntry> {
    Ok(WorkspaceEntry {
        workspace: super::decode_workspace_info(decoder)?,
        active_tab: decoder.string("active tab", MAX_ID_BYTES)?,
    })
}

fn encode_queue(encoder: &mut Encoder, queue: &[QueueItem]) {
    encoder.count(queue.len());
    for item in queue {
        encoder.string(&item.workspace);
        encoder.string(&item.tab);
        encoder.string(&item.target);
        encode_state(encoder, Some(item.state));
    }
}

fn decode_queue(decoder: &mut Decoder<'_>) -> Result<Vec<QueueItem>> {
    let count = decoder.count("attention queue", MAX_SESSIONS)?;
    (0..count)
        .map(|_| {
            Ok(QueueItem {
                workspace: decoder.string("queue workspace", MAX_ID_BYTES)?,
                tab: decoder.string("queue tab", MAX_ID_BYTES)?,
                target: decoder.string("queue target", MAX_ID_BYTES)?,
                state: decode_state(decoder, "queue state")?.ok_or(Error::InvalidSnapshot {
                    field: "attention queue",
                })?,
            })
        })
        .collect()
}

/// At most one entry per terminal of the window.
fn validate_busy_list(busy: &[BusyTerminal]) -> Result<()> {
    if busy.len() > MAX_SESSIONS
        || !distinct(
            &busy
                .iter()
                .map(|busy| &busy.target.terminal)
                .collect::<Vec<_>>(),
        )
    {
        return Err(Error::InvalidValue {
            field: "busy terminals",
        });
    }
    for busy in busy {
        validate_busy_terminal(&busy.target, &busy.name)?;
    }
    Ok(())
}

/// A presence flag, then the count and each terminal's target and name.
fn encode_busy_list(encoder: &mut Encoder, busy: Option<&Vec<BusyTerminal>>) {
    encoder.byte(u8::from(busy.is_some()));
    if let Some(busy) = busy {
        encoder.count(busy.len());
        for busy in busy {
            encode_target(encoder, &busy.target);
            encoder.string(&busy.name);
        }
    }
}

fn decode_busy_list(decoder: &mut Decoder<'_>) -> Result<Option<Vec<BusyTerminal>>> {
    if !decode_bool(decoder, "busy terminals")? {
        return Ok(None);
    }
    let count = decoder.count("busy terminals", MAX_SESSIONS)?;
    (0..count)
        .map(|_| {
            Ok(BusyTerminal {
                target: decode_target(decoder)?,
                name: decoder.string("busy name", MAX_PANE_LABEL_BYTES)?,
            })
        })
        .collect::<Result<_>>()
        .map(Some)
}

/// The workspaces, the queue, the busy terminals, then the `TABS` count.
fn encode_snapshot(encoder: &mut Encoder, snapshot: &WindowSnapshot) {
    encoder.count(snapshot.workspaces.len());
    for entry in &snapshot.workspaces {
        encode_workspace_entry(encoder, entry);
    }
    encode_queue(encoder, &snapshot.attention_queue);
    encode_busy_list(encoder, snapshot.busy.as_ref());
    encoder.count(snapshot.tab_messages);
}

fn decode_snapshot(decoder: &mut Decoder<'_>) -> Result<WindowSnapshot> {
    let count = decoder.count("workspaces", MAX_WORKSPACES)?;
    Ok(WindowSnapshot {
        workspaces: (0..count)
            .map(|_| decode_workspace_entry(decoder))
            .collect::<Result<_>>()?,
        attention_queue: decode_queue(decoder)?,
        busy: decode_busy_list(decoder)?,
        tab_messages: decoder.count("tab messages", MAX_WORKSPACES)?,
    })
}

/// Checks tabs carried together: one workspace's run, or one event's tab.
fn validate_carried_tabs(tabs: &[Tab]) -> Result<()> {
    let wire: Vec<_> = tabs.iter().map(WireTab::from).collect();
    validate_tabs(&wire, None)?;
    for tab in tabs {
        validate_columns(tab)?;
    }
    Ok(())
}

/// Each tab as its column layout followed by the snapshot's tab form.
fn encode_tab(encoder: &mut Encoder, tab: &Tab) -> Result<()> {
    encode_layout(encoder, tab)?;
    encode_wire_tab(encoder, &WireTab::from(tab));
    Ok(())
}

fn decode_tab(decoder: &mut Decoder<'_>, sessions: &mut usize) -> Result<Tab> {
    let layout = decode_layout(decoder)?;
    let tab = decode_wire_tab(decoder, sessions)?;
    assemble_tab(tab, layout)
}

fn validate_order(change: TabChange, tab: &str, tabs: Option<&Vec<String>>) -> Result<()> {
    let invalid = Err(Error::InvalidValue { field: "tab order" });
    let Some(tabs) = tabs else {
        return if change.carries_order() {
            invalid
        } else {
            Ok(())
        };
    };
    if !change.carries_order() || tabs.len() > MAX_TABS || !distinct(tabs) {
        return invalid;
    }
    for id in tabs {
        identity("tab order", id)?;
    }
    if tabs.iter().any(|id| id == tab) == (change == TabChange::Closed) {
        return invalid;
    }
    Ok(())
}

fn validate_pane(pane: &Pane) -> Result<()> {
    identity("surface id", &pane.id)?;
    identity("session id", &pane.session)?;
    validate_endpoint(&pane.endpoint)?;
    if let Some(attention) = &pane.attention {
        validate_attention(attention)?;
    }
    validate_pane_label(pane)
}

fn validate_popup(popup: &Popup) -> Result<()> {
    identity("surface id", &popup.id)?;
    entry_id(&popup.entry)?;
    identity("session id", &popup.session)?;
    validate_endpoint(&popup.endpoint)?;
    if let Some(attention) = &popup.attention {
        validate_attention(attention)?;
    }
    Ok(())
}

/// Checks a pane event's `columns`, and its pane's `column` and `expanded`
/// against them unless the pane closed.
fn validate_column_order(
    change: PaneChange,
    pane: &str,
    column: usize,
    expanded: bool,
    columns: &[ColumnOrder],
) -> Result<()> {
    let invalid = |field| Err(Error::InvalidSnapshot { field });
    if columns.len() > MAX_COLUMNS || !(1..=MAX_COLUMNS).contains(&column) {
        return invalid("columns");
    }
    let mut panes = HashSet::new();
    for column in columns {
        if column.panes.is_empty() {
            return invalid("empty column");
        }
        if !column.panes.contains(&column.expanded) {
            return invalid("expanded pane");
        }
        for id in &column.panes {
            identity("column pane", id)?;
            if !panes.insert(id.as_str()) {
                return invalid("column pane");
            }
        }
    }
    if panes.len() > MAX_SESSIONS {
        return invalid("sessions");
    }
    if change == PaneChange::Closed {
        return if panes.contains(pane) {
            invalid("columns")
        } else {
            Ok(())
        };
    }
    match columns.get(column - 1) {
        Some(holder) if holder.panes.iter().any(|id| id == pane) => {
            if (holder.expanded == pane) == expanded {
                Ok(())
            } else {
                invalid("expanded pane")
            }
        }
        _ => invalid("columns"),
    }
}

fn validate_change(change: &Change) -> Result<()> {
    match change {
        Change::Workspace { change, workspace } => {
            validate_workspace_entry(workspace)?;
            match change {
                WorkspaceChange::Registered if workspace.workspace.persistent_id.is_none() => {
                    Err(Error::InvalidSnapshot {
                        field: "registered persistent id",
                    })
                }
                WorkspaceChange::RegistrationFailed
                    if workspace.workspace.registration_error.is_none() =>
                {
                    Err(Error::InvalidSnapshot {
                        field: "registration error",
                    })
                }
                _ => Ok(()),
            }
        }
        Change::Tab {
            change,
            workspace,
            tab,
            tabs,
        } => {
            validate_workspace_id(workspace)?;
            validate_carried_tabs(std::slice::from_ref(tab))?;
            validate_order(*change, &tab.id, tabs.as_ref())
        }
        Change::Pane {
            change,
            workspace,
            tab,
            pane,
            column,
            expanded,
            columns,
        } => {
            validate_workspace_id(workspace)?;
            identity("tab id", tab)?;
            validate_pane(pane)?;
            validate_column_order(*change, &pane.id, *column, *expanded, columns)
        }
        Change::Popup {
            workspace,
            tab,
            popup,
            ..
        } => {
            validate_workspace_id(workspace)?;
            identity("tab id", tab)?;
            validate_popup(popup)
        }
        Change::Attention {
            target, attention, ..
        } => {
            validate_target(target)?;
            if let Some(attention) = attention {
                validate_attention(attention)?;
            }
            Ok(())
        }
        Change::Queue(queue) => validate_queue(queue),
        Change::Busy { target, busy, name } => match name {
            Some(name) if *busy => validate_busy_terminal(target, name),
            None if !*busy => validate_target(target),
            _ => Err(Error::InvalidValue { field: "busy name" }),
        },
    }
}

/// The kind tag, then the change's payload; a change's name is a tag in its
/// kind's listed order.
fn encode_change(encoder: &mut Encoder, change: &Change) -> Result<()> {
    encoder.byte(kind_tag(change.kind()));
    match change {
        Change::Workspace { change, workspace } => {
            encoder.byte(*change as u8);
            encode_workspace_entry(encoder, workspace);
        }
        Change::Tab {
            change,
            workspace,
            tab,
            tabs,
        } => {
            encoder.byte(*change as u8);
            encoder.string(workspace);
            encode_tab(encoder, tab)?;
            if let Some(tabs) = tabs {
                encoder.count(tabs.len());
                for id in tabs {
                    encoder.string(id);
                }
            }
        }
        Change::Pane {
            change,
            workspace,
            tab,
            pane,
            column,
            expanded,
            columns,
        } => {
            encoder.byte(*change as u8);
            encoder.string(workspace);
            encoder.string(tab);
            encode_pane(encoder, pane);
            encoder.byte(*column as u8);
            encoder.byte(u8::from(*expanded));
            encoder.count(columns.len());
            for column in columns {
                encoder.count(column.panes.len());
                for id in &column.panes {
                    encoder.string(id);
                }
                let expanded = column
                    .panes
                    .iter()
                    .position(|id| *id == column.expanded)
                    .expect("validated expanded pane");
                encoder.count(expanded);
            }
        }
        Change::Popup {
            change,
            workspace,
            tab,
            popup,
            chosen,
        } => {
            encoder.byte(*change as u8);
            encoder.string(workspace);
            encoder.string(tab);
            encode_popup(encoder, popup);
            encoder.byte(u8::from(*chosen));
        }
        Change::Attention {
            target,
            previous,
            attention,
        } => {
            encode_target(encoder, target);
            encode_state(encoder, *previous);
            encode_attention(encoder, attention.as_ref());
        }
        Change::Queue(queue) => encode_queue(encoder, queue),
        Change::Busy { target, busy, name } => {
            encode_target(encoder, target);
            encoder.byte(u8::from(*busy));
            encoder.byte(u8::from(name.is_some()));
            if let Some(name) = name {
                encoder.string(name);
            }
        }
    }
    Ok(())
}

fn decode_change(decoder: &mut Decoder<'_>) -> Result<Change> {
    let value = decoder.byte()?;
    let kind = EVENT_KINDS
        .get(usize::from(value))
        .copied()
        .ok_or(Error::InvalidTag {
            field: "event kind",
            value,
        })?;
    Ok(match kind {
        EventKind::Workspace => Change::Workspace {
            change: match change_tag(decoder.byte()?, "workspace change", 6)? {
                0 => WorkspaceChange::Created,
                1 => WorkspaceChange::Renamed,
                2 => WorkspaceChange::Closed,
                3 => WorkspaceChange::Shown,
                4 => WorkspaceChange::Registered,
                _ => WorkspaceChange::RegistrationFailed,
            },
            workspace: decode_workspace_entry(decoder)?,
        },
        EventKind::Tab => {
            let change = match change_tag(decoder.byte()?, "tab change", 5)? {
                0 => TabChange::Created,
                1 => TabChange::Moved,
                2 => TabChange::Directory,
                3 => TabChange::Closed,
                _ => TabChange::Activated,
            };
            let workspace = decoder.string("workspace id", MAX_ID_BYTES)?;
            let tab = decode_tab(decoder, &mut 0)?;
            let tabs = change
                .carries_order()
                .then(|| {
                    let count = decoder.count("tab order", MAX_TABS)?;
                    (0..count)
                        .map(|_| decoder.string("tab order", MAX_ID_BYTES))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;
            Change::Tab {
                change,
                workspace,
                tab,
                tabs,
            }
        }
        EventKind::Pane => {
            let change = match change_tag(decoder.byte()?, "pane change", 6)? {
                0 => PaneChange::Created,
                1 => PaneChange::Moved,
                2 => PaneChange::Renamed,
                3 => PaneChange::Ended,
                4 => PaneChange::Closed,
                _ => PaneChange::Selected,
            };
            let workspace = decoder.string("workspace id", MAX_ID_BYTES)?;
            let tab = decoder.string("tab id", MAX_ID_BYTES)?;
            let pane = decode_pane(decoder)?;
            let column = usize::from(decoder.byte()?);
            let expanded = decode_bool(decoder, "expanded")?;
            let count = decoder.count("columns", MAX_COLUMNS)?;
            let mut panes = 0;
            let mut columns = Vec::with_capacity(count);
            for _ in 0..count {
                let count = decoder.count("sessions", MAX_SESSIONS - panes)?;
                panes += count;
                let ids = (0..count)
                    .map(|_| decoder.string("column pane", MAX_ID_BYTES))
                    .collect::<Result<Vec<_>>>()?;
                let expanded = decoder.count("expanded pane", MAX_SESSIONS)?;
                let expanded = ids.get(expanded).cloned().ok_or(Error::InvalidSnapshot {
                    field: "expanded pane",
                })?;
                columns.push(ColumnOrder {
                    panes: ids,
                    expanded,
                });
            }
            Change::Pane {
                change,
                workspace,
                tab,
                pane,
                column,
                expanded,
                columns,
            }
        }
        EventKind::Popup => Change::Popup {
            change: match change_tag(decoder.byte()?, "popup change", 4)? {
                0 => PopupChange::Created,
                1 => PopupChange::Shown,
                2 => PopupChange::Hidden,
                _ => PopupChange::Closed,
            },
            workspace: decoder.string("workspace id", MAX_ID_BYTES)?,
            tab: decoder.string("tab id", MAX_ID_BYTES)?,
            popup: decode_popup(decoder)?,
            chosen: decode_bool(decoder, "chosen")?,
        },
        EventKind::Attention => Change::Attention {
            target: decode_target(decoder)?,
            previous: decode_state(decoder, "previous state")?,
            attention: decode_attention(decoder)?,
        },
        EventKind::Queue => Change::Queue(decode_queue(decoder)?),
        EventKind::Busy => Change::Busy {
            target: decode_target(decoder)?,
            busy: decode_bool(decoder, "busy")?,
            name: decode_bool(decoder, "busy name")?
                .then(|| decoder.string("busy name", MAX_PANE_LABEL_BYTES))
                .transpose()?,
        },
    })
}

/// Encodes one message of a subscription connection after validating it.
pub fn encode_subscription_message(message: &SubscriptionMessage) -> Result<Vec<u8>> {
    let mut payload = Encoder::default();
    let kind = match message {
        // The stream, seq, generation, window and caller, then the snapshot
        // when it did not resume.
        SubscriptionMessage::Subscribed(subscribed) => {
            identity("stream", &subscribed.stream)?;
            identity("generation", &subscribed.generation)?;
            validate_window_id(subscribed.window.as_deref())?;
            if let Some(caller) = &subscribed.caller {
                validate_target(caller)?;
            }
            if let Some(snapshot) = &subscribed.snapshot {
                validate_snapshot(snapshot)?;
            }
            payload.string(&subscribed.stream);
            payload
                .bytes
                .extend_from_slice(&subscribed.seq.to_le_bytes());
            payload.string(&subscribed.generation);
            encode_optional(&mut payload, &subscribed.window);
            encode_caller(&mut payload, subscribed.caller.as_ref());
            payload.byte(u8::from(subscribed.snapshot.is_some()));
            if let Some(snapshot) = &subscribed.snapshot {
                encode_snapshot(&mut payload, snapshot);
            }
            SUBSCRIBED
        }
        SubscriptionMessage::Tabs(tabs) => {
            validate_workspace_id(&tabs.workspace)?;
            if tabs.tabs.is_empty() || tabs.tabs.len() > MAX_TABS {
                return Err(Error::InvalidSnapshot { field: "tabs" });
            }
            validate_carried_tabs(&tabs.tabs)?;
            payload.string(&tabs.workspace);
            payload.count(tabs.tabs.len());
            for tab in &tabs.tabs {
                encode_tab(&mut payload, tab)?;
            }
            TABS
        }
        SubscriptionMessage::Event(event) => {
            validate_change(&event.change)?;
            payload.bytes.extend_from_slice(&event.seq.to_le_bytes());
            encode_change(&mut payload, &event.change)?;
            EVENT
        }
        SubscriptionMessage::Gap(gap) => {
            validate_snapshot(&gap.snapshot)?;
            payload.bytes.extend_from_slice(&gap.seq.to_le_bytes());
            payload.byte(match gap.reason {
                GapReason::Overflow => 0,
            });
            encode_snapshot(&mut payload, &gap.snapshot);
            GAP
        }
        SubscriptionMessage::End(end) => {
            match end {
                End::Ended { generation } => {
                    identity("generation", generation)?;
                    payload.byte(0);
                    payload.string(generation);
                }
                End::TargetsGone => payload.byte(1),
                End::TooSlow => payload.byte(2),
            }
            END
        }
        SubscriptionMessage::Failure(failure) => {
            encode_failure(&mut payload, failure)?;
            FAILURE
        }
    };
    frame(kind, payload.bytes)
}

/// Decodes and validates one message of a subscription connection.
pub fn decode_subscription_message(bytes: &[u8]) -> Result<SubscriptionMessage> {
    let (kind, payload) = unframe(bytes)?;
    let mut decoder = Decoder::new(payload);
    let message = match kind {
        SUBSCRIBED => {
            let subscribed = Subscribed {
                stream: decoder.string("stream", MAX_ID_BYTES)?,
                seq: decode_u64(&mut decoder)?,
                generation: decoder.string("generation", MAX_ID_BYTES)?,
                window: decode_optional(&mut decoder, "window id")?,
                caller: decode_caller(&mut decoder)?,
                snapshot: decode_bool(&mut decoder, "snapshot")?
                    .then(|| decode_snapshot(&mut decoder))
                    .transpose()?,
            };
            identity("stream", &subscribed.stream)?;
            identity("generation", &subscribed.generation)?;
            validate_window_id(subscribed.window.as_deref())?;
            if let Some(snapshot) = &subscribed.snapshot {
                validate_snapshot(snapshot)?;
            }
            SubscriptionMessage::Subscribed(subscribed)
        }
        TABS => {
            let workspace = decoder.string("workspace id", MAX_ID_BYTES)?;
            validate_workspace_id(&workspace)?;
            let count = decoder.count("tabs", MAX_TABS)?;
            if count == 0 {
                return Err(Error::InvalidSnapshot { field: "tabs" });
            }
            let mut sessions = 0;
            let tabs = (0..count)
                .map(|_| decode_tab(&mut decoder, &mut sessions))
                .collect::<Result<Vec<_>>>()?;
            validate_carried_tabs(&tabs)?;
            SubscriptionMessage::Tabs(Tabs { workspace, tabs })
        }
        EVENT => {
            let event = Event {
                seq: decode_u64(&mut decoder)?,
                change: decode_change(&mut decoder)?,
            };
            validate_change(&event.change)?;
            SubscriptionMessage::Event(event)
        }
        GAP => {
            let seq = decode_u64(&mut decoder)?;
            let reason = match decoder.byte()? {
                0 => GapReason::Overflow,
                value => {
                    return Err(Error::InvalidTag {
                        field: "gap reason",
                        value,
                    });
                }
            };
            let snapshot = decode_snapshot(&mut decoder)?;
            validate_snapshot(&snapshot)?;
            SubscriptionMessage::Gap(Gap {
                seq,
                reason,
                snapshot,
            })
        }
        END => SubscriptionMessage::End(match decoder.byte()? {
            0 => {
                let generation = decoder.string("generation", MAX_ID_BYTES)?;
                identity("generation", &generation)?;
                End::Ended { generation }
            }
            1 => End::TargetsGone,
            2 => End::TooSlow,
            value => {
                return Err(Error::InvalidTag {
                    field: "end reason",
                    value,
                });
            }
        }),
        FAILURE => SubscriptionMessage::Failure(decode_failure(&mut decoder)?),
        value => return Err(Error::InvalidKind { value }),
    };
    decoder.finish()?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::super::{
        Action, AttentionKind, AttentionRecord, BusyTerminal, Column, HEADER_BYTES, MAX_APP_BYTES,
        MAX_DIRECTORY_BYTES, MAX_ENDPOINT_BYTES, MAX_ENTRIES, MAX_ENTRY_ID_BYTES,
        MAX_MESSAGE_BYTES, MAX_NAME_BYTES, MAX_OTHER_RECORDS, MAX_PANE_LABEL_BYTES,
        MAX_PANE_NAME_BYTES, MAX_PANE_NAME_CHARS, MAX_STATUS_MESSAGE_BYTES, MAX_TITLE_BYTES,
        MAX_WORKSPACE_NAME_CHARS, Request, TargetId, decode_request, encode_request,
    };
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn pane(id: &str) -> Pane {
        Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("s{id}"),
            endpoint: format!("/{id}").into_bytes(),
            live: true,
            attention: None,
        }
    }

    fn popup(id: &str, entry: &str) -> Popup {
        Popup {
            id: id.into(),
            entry: entry.into(),
            session: format!("s{id}"),
            endpoint: format!("/{id}").into_bytes(),
            attention: None,
        }
    }

    fn tab(id: &str, panes: &[&str]) -> Tab {
        Tab {
            id: id.into(),
            directory: b"/".to_vec(),
            pending: false,
            selected_pane: Some(panes[0].into()),
            selected_popup: None,
            columns: vec![Column {
                panes: panes.iter().map(|id| pane(id)).collect(),
                expanded: panes[0].into(),
            }],
            popups: vec![],
            attention_rollup: None,
        }
    }

    fn target(workspace: &str, tab: &str, id: TargetId) -> TerminalTarget {
        TerminalTarget {
            workspace: workspace.into(),
            tab: tab.into(),
            id,
            terminal: "s1".into(),
        }
    }

    fn attention(state: AttentionState) -> Attention {
        Attention {
            state,
            record: Some(AttentionRecord {
                kind: (state == AttentionState::NeedsInput).then_some(AttentionKind::Question),
                progress: None,
                app: "claude".into(),
                name: String::new(),
                title: "Continue?".into(),
                message: String::new(),
                changed_at: 7,
                stale: false,
                others: 0,
            }),
        }
    }

    fn entry(id: &str, persistent_id: &str, recency: u8) -> WorkspaceEntry {
        WorkspaceEntry {
            workspace: WorkspaceInfo {
                id: id.into(),
                persistent_id: Some(persistent_id.into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency,
            },
            active_tab: "t1".into(),
        }
    }

    fn window_snapshot() -> WindowSnapshot {
        WindowSnapshot {
            workspaces: vec![entry(
                "s1",
                "0123456789abcdef0123456789abcdef0000000000000001",
                0,
            )],
            attention_queue: vec![QueueItem {
                workspace: "s1".into(),
                tab: "t1".into(),
                target: "p1".into(),
                state: AttentionState::NeedsInput,
            }],
            busy: None,
            tab_messages: 1,
        }
    }

    fn subscribed(snapshot: Option<WindowSnapshot>) -> SubscriptionMessage {
        SubscriptionMessage::Subscribed(Subscribed {
            stream: "g1.1700000000000".into(),
            seq: 41,
            generation: "g1".into(),
            window: None,
            caller: None,
            snapshot,
        })
    }

    fn event(change: Change) -> SubscriptionMessage {
        SubscriptionMessage::Event(Event { seq: 42, change })
    }

    fn subscribe(filter: Filter, after: Option<Resume>) -> Request {
        Request {
            id: "r1".into(),
            caller: None,
            action: Action::Subscribe { filter, after },
        }
    }

    /// One sample per message form and change name.
    fn samples() -> Vec<SubscriptionMessage> {
        let mut samples = vec![
            subscribed(None),
            subscribed(Some(window_snapshot())),
            SubscriptionMessage::Subscribed(Subscribed {
                window: Some("0123abcd".into()),
                caller: Some(target("s1", "t1", TargetId::Pane("p1".into()))),
                ..match subscribed(None) {
                    SubscriptionMessage::Subscribed(subscribed) => subscribed,
                    _ => unreachable!(),
                }
            }),
            SubscriptionMessage::Tabs(Tabs {
                workspace: "s1".into(),
                tabs: vec![tab("t1", &["p1", "p2"]), tab("t2", &["p3"])],
            }),
            SubscriptionMessage::Gap(Gap {
                seq: 90,
                reason: GapReason::Overflow,
                snapshot: window_snapshot(),
            }),
            SubscriptionMessage::End(End::Ended {
                generation: "g1".into(),
            }),
            SubscriptionMessage::End(End::TargetsGone),
            SubscriptionMessage::End(End::TooSlow),
            SubscriptionMessage::Failure(Failure {
                code: "subscriber-limit".into(),
                detail: "a supervisor serves at most 32 subscribers".into(),
            }),
        ];
        for change in [
            WorkspaceChange::Created,
            WorkspaceChange::Renamed,
            WorkspaceChange::Closed,
            WorkspaceChange::Shown,
            WorkspaceChange::Registered,
        ] {
            samples.push(event(Change::Workspace {
                change,
                workspace: entry("s2", "0123456789abcdef0123456789abcdef0000000000000002", 1),
            }));
        }
        let mut failed = entry("s2", "0123456789abcdef0123456789abcdef0000000000000002", 1);
        failed.workspace.persistent_id = None;
        failed.workspace.registration_error = Some(Failure {
            code: "registry-busy".into(),
            detail: "held".into(),
        });
        samples.push(event(Change::Workspace {
            change: WorkspaceChange::RegistrationFailed,
            workspace: failed,
        }));
        for change in [
            TabChange::Created,
            TabChange::Moved,
            TabChange::Directory,
            TabChange::Closed,
            TabChange::Activated,
        ] {
            samples.push(event(Change::Tab {
                change,
                workspace: "s1".into(),
                tab: tab("t2", &["p3"]),
                tabs: change.carries_order().then(|| match change {
                    TabChange::Closed => vec!["t1".into()],
                    _ => vec!["t2".into(), "t1".into()],
                }),
            }));
        }
        for change in [
            PaneChange::Created,
            PaneChange::Moved,
            PaneChange::Renamed,
            PaneChange::Ended,
            PaneChange::Closed,
            PaneChange::Selected,
        ] {
            let closed = change == PaneChange::Closed;
            samples.push(event(Change::Pane {
                change,
                workspace: "s1".into(),
                tab: "t1".into(),
                pane: pane("p2"),
                column: 2,
                expanded: false,
                columns: vec![
                    ColumnOrder {
                        panes: vec!["p1".into()],
                        expanded: "p1".into(),
                    },
                    ColumnOrder {
                        panes: if closed {
                            vec!["p3".into()]
                        } else {
                            vec!["p3".into(), "p2".into()]
                        },
                        expanded: "p3".into(),
                    },
                ],
            }));
        }
        for change in [
            PopupChange::Created,
            PopupChange::Shown,
            PopupChange::Hidden,
            PopupChange::Closed,
        ] {
            samples.push(event(Change::Popup {
                change,
                workspace: "s1".into(),
                tab: "t1".into(),
                popup: popup("u1", "agent"),
                chosen: change != PopupChange::Hidden,
            }));
        }
        samples.extend([
            event(Change::Attention {
                target: target("s1", "t1", TargetId::Pane("p1".into())),
                previous: None,
                attention: Some(attention(AttentionState::NeedsInput)),
            }),
            event(Change::Attention {
                target: target("s1", "t1", TargetId::Popup("u1".into())),
                previous: Some(AttentionState::Working),
                attention: None,
            }),
            event(Change::Queue(window_snapshot().attention_queue)),
            event(Change::Queue(vec![])),
            event(Change::Busy {
                target: target("s1", "t1", TargetId::Pane("p1".into())),
                busy: true,
                name: Some("p1  cargo build".into()),
            }),
            event(Change::Busy {
                target: target("s1", "t1", TargetId::Popup("u1".into())),
                busy: false,
                name: None,
            }),
        ]);
        samples
    }

    #[test]
    fn subscribe_requests_round_trip_and_reject_forged_filters() {
        let filter = Filter {
            workspaces: vec!["s2".into(), "notes 🦀".into()],
            tabs: vec!["t1".into()],
            targets: vec!["p1".into(), "u1".into()],
            kinds: vec![EventKind::Attention, EventKind::Busy],
            states: vec![AttentionState::NeedsInput, AttentionState::Finished],
        };
        let after = Resume {
            stream: "g1.1700000000000".into(),
            seq: u64::MAX,
        };
        for request in [
            subscribe(Filter::default(), None),
            subscribe(filter.clone(), Some(after.clone())),
            Request {
                caller: Some("t-terminal".into()),
                ..subscribe(filter.clone(), None)
            },
        ] {
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
        }
        // Every bound and identity rule holds on both sides.
        let too_many = |n: usize, prefix: &str| (0..n).map(|i| format!("{prefix}{i}")).collect();
        let invalid = [
            Filter {
                workspaces: too_many(MAX_WORKSPACES + 1, "w"),
                ..Filter::default()
            },
            Filter {
                tabs: too_many(MAX_FILTER_TABS + 1, "t"),
                ..Filter::default()
            },
            Filter {
                targets: too_many(MAX_FILTER_TARGETS + 1, "p"),
                ..Filter::default()
            },
            Filter {
                workspaces: vec!["s1".into(), "s1".into()],
                ..Filter::default()
            },
            Filter {
                workspaces: vec!["bad\nname".into()],
                ..Filter::default()
            },
            Filter {
                tabs: vec!["t 1".into()],
                ..Filter::default()
            },
            Filter {
                targets: vec!["p1".into(), "p1".into()],
                ..Filter::default()
            },
            Filter {
                kinds: vec![EventKind::Pane, EventKind::Pane],
                ..Filter::default()
            },
            Filter {
                states: vec![AttentionState::Idle, AttentionState::Idle],
                ..Filter::default()
            },
        ];
        for filter in invalid {
            let request = subscribe(filter, None);
            assert!(encode_request(&request).is_err(), "{request:?}");
            let mut payload = Encoder::default();
            payload.string(&request.id);
            encode_optional(&mut payload, &request.caller);
            let Action::Subscribe { filter, after } = &request.action else {
                unreachable!()
            };
            encode_subscribe(&mut payload, filter, after.as_ref());
            assert!(
                decode_request(&frame(super::super::REQUEST, payload.bytes).unwrap()).is_err(),
                "{request:?}"
            );
        }
        let bad_stream = subscribe(
            Filter::default(),
            Some(Resume {
                stream: "g1:7".into(),
                seq: 1,
            }),
        );
        assert!(encode_request(&bad_stream).is_err());

        // Forged kind and state tags, an absent state, and a bad resume flag.
        let base = encode_request(&subscribe(
            Filter {
                kinds: vec![EventKind::Busy],
                states: vec![AttentionState::Background],
                ..Filter::default()
            },
            None,
        ))
        .unwrap();
        // The tail is: kind count, kind, state count, state, resume flag.
        let tail = base.len() - 7;
        assert_eq!(hex(&base[tail..]), "010006010007".to_string() + "00");
        for (offset, value) in [(2, 7), (5, 0), (5, 8), (6, 2)] {
            let mut forged = base.clone();
            forged[tail + offset] = value;
            assert!(decode_request(&forged).is_err(), "{offset} {value}");
        }
    }

    #[test]
    fn c38_subscription_mapping_round_trips_and_rejects_duplicate_identities() {
        let mut snapshot = window_snapshot();
        let second = entry("s2", "fedcba9876543210fedcba98765432100000000000000001", 1);
        snapshot.workspaces.push(second.clone());
        let messages = |snapshot: WindowSnapshot| {
            [
                subscribed(Some(snapshot.clone())),
                SubscriptionMessage::Gap(Gap {
                    seq: 90,
                    reason: GapReason::Overflow,
                    snapshot,
                }),
            ]
        };
        for message in messages(snapshot.clone()) {
            let bytes = encode_subscription_message(&message).unwrap();
            assert_eq!(decode_subscription_message(&bytes).unwrap(), message);
        }
        snapshot.workspaces[1].workspace.persistent_id =
            snapshot.workspaces[0].workspace.persistent_id.clone();
        for message in messages(snapshot) {
            assert!(encode_subscription_message(&message).is_err());
            assert!(decode_subscription_message(&forge(&message)).is_err());
        }
        let mut bad = second;
        bad.workspace.persistent_id = Some(String::new());
        let message = event(Change::Workspace {
            change: WorkspaceChange::Renamed,
            workspace: bad,
        });
        assert!(encode_subscription_message(&message).is_err());
        assert!(decode_subscription_message(&forge(&message)).is_err());
    }

    #[test]
    fn subscription_messages_round_trip() {
        for message in samples() {
            let bytes = encode_subscription_message(&message).unwrap();
            assert_eq!(
                decode_subscription_message(&bytes).unwrap(),
                message,
                "{message:?}"
            );
        }
        // A subscription message is not a one-shot response, nor the reverse.
        let bytes = encode_subscription_message(&subscribed(None)).unwrap();
        assert!(super::super::decode_response(&bytes).is_err());
        let ended =
            super::super::encode_response(&super::super::Response::Ended(super::super::Ended {
                generation: "g1".into(),
            }))
            .unwrap();
        assert!(decode_subscription_message(&ended).is_err());
    }

    #[test]
    fn registration_events_require_the_state_they_claim_on_both_sides() {
        let mut registered = entry("s1", "confirmed", 0);
        registered.workspace.persistent_id = None;
        for message in [
            event(Change::Workspace {
                change: WorkspaceChange::Registered,
                workspace: registered,
            }),
            event(Change::Workspace {
                change: WorkspaceChange::RegistrationFailed,
                workspace: entry("s1", "confirmed", 0),
            }),
        ] {
            assert!(encode_subscription_message(&message).is_err());
            assert!(decode_subscription_message(&forge(&message)).is_err());
        }
    }

    #[test]
    fn c38_pending_registration_and_errors_reach_initial_gap_and_events() {
        let mut snapshot = window_snapshot();
        snapshot.workspaces[0].workspace.persistent_id = None;
        snapshot.workspaces[0].workspace.registration_error = Some(Failure {
            code: "registry-busy".into(),
            detail: "held".into(),
        });
        for message in [
            subscribed(Some(snapshot.clone())),
            SubscriptionMessage::Gap(Gap {
                seq: 90,
                reason: GapReason::Overflow,
                snapshot: snapshot.clone(),
            }),
            event(Change::Workspace {
                change: WorkspaceChange::RegistrationFailed,
                workspace: snapshot.workspaces[0].clone(),
            }),
        ] {
            assert_eq!(
                decode_subscription_message(&encode_subscription_message(&message).unwrap())
                    .unwrap(),
                message
            );
        }
        for length in [MAX_ID_BYTES, MAX_ID_BYTES + 1] {
            snapshot.workspaces[0].workspace.persistent_id = Some("a".repeat(length));
            let message = event(Change::Workspace {
                change: WorkspaceChange::Registered,
                workspace: snapshot.workspaces[0].clone(),
            });
            assert_eq!(
                encode_subscription_message(&message).is_ok(),
                length == MAX_ID_BYTES
            );
            assert_eq!(
                decode_subscription_message(&forge(&message)).is_ok(),
                length == MAX_ID_BYTES
            );
        }
        snapshot.workspaces[0].workspace.persistent_id = None;
        snapshot.workspaces[0]
            .workspace
            .registration_error
            .as_mut()
            .unwrap()
            .detail = "a".repeat(super::super::MAX_DETAIL_BYTES + 1);
        let message = subscribed(Some(snapshot));
        assert!(encode_subscription_message(&message).is_err());
        assert!(decode_subscription_message(&forge(&message)).is_err());
    }

    /// Encodes `message` without its validation, as a forging peer would.
    fn forge(message: &SubscriptionMessage) -> Vec<u8> {
        let mut payload = Encoder::default();
        let kind = match message {
            SubscriptionMessage::Event(event) => {
                payload.bytes.extend_from_slice(&event.seq.to_le_bytes());
                encode_change(&mut payload, &event.change).unwrap();
                EVENT
            }
            SubscriptionMessage::Gap(gap) => {
                payload.bytes.extend_from_slice(&gap.seq.to_le_bytes());
                payload.byte(0);
                encode_snapshot(&mut payload, &gap.snapshot);
                GAP
            }
            SubscriptionMessage::Tabs(tabs) => {
                payload.string(&tabs.workspace);
                payload.count(tabs.tabs.len());
                for tab in &tabs.tabs {
                    encode_layout(&mut payload, tab).unwrap();
                    encode_wire_tab(&mut payload, &WireTab::from(tab));
                }
                TABS
            }
            SubscriptionMessage::Subscribed(subscribed) => {
                payload.string(&subscribed.stream);
                payload
                    .bytes
                    .extend_from_slice(&subscribed.seq.to_le_bytes());
                payload.string(&subscribed.generation);
                encode_optional(&mut payload, &subscribed.window);
                encode_caller(&mut payload, subscribed.caller.as_ref());
                payload.byte(1);
                encode_snapshot(&mut payload, subscribed.snapshot.as_ref().unwrap());
                SUBSCRIBED
            }
            _ => unreachable!(),
        };
        frame(kind, payload.bytes).unwrap()
    }

    #[test]
    fn subscription_messages_reject_invalid_values_on_both_sides() {
        let gap = |change: &dyn Fn(&mut WindowSnapshot)| {
            let mut snapshot = window_snapshot();
            change(&mut snapshot);
            SubscriptionMessage::Gap(Gap {
                seq: 1,
                reason: GapReason::Overflow,
                snapshot,
            })
        };
        let tab_event = |change: TabChange, tabs: Option<Vec<String>>| {
            event(Change::Tab {
                change,
                workspace: "s1".into(),
                tab: tab("t2", &["p3"]),
                tabs,
            })
        };
        let placed = |change: PaneChange, column: usize, expanded: bool, columns| {
            event(Change::Pane {
                change,
                workspace: "s1".into(),
                tab: "t1".into(),
                pane: pane("p2"),
                column,
                expanded,
                columns,
            })
        };
        let pane_event = |change, columns| placed(change, 1, false, columns);
        let busy_event = |busy, name: Option<&str>| {
            event(Change::Busy {
                target: target("s1", "t1", TargetId::Pane("p1".into())),
                busy,
                name: name.map(str::to_owned),
            })
        };
        let column = |panes: &[&str], expanded: &str| ColumnOrder {
            panes: panes.iter().map(|id| id.to_string()).collect(),
            expanded: expanded.into(),
        };
        let mut duplicate_surface = tab("t2", &["p1"]);
        duplicate_surface.columns[0].panes[0].session = "sp1".into();
        let mut rollup = tab("t1", &["p1"]);
        rollup.attention_rollup = Some(AttentionState::Idle);
        let invalid = [
            gap(&|s| s.attention_queue[0].state = AttentionState::Working),
            gap(&|s| s.attention_queue[0].workspace = "s01".into()),
            gap(&|s| {
                s.workspaces.push(entry(
                    "s1",
                    "0123456789abcdef0123456789abcdef0000000000000002",
                    1,
                ))
            }),
            gap(&|s| {
                s.workspaces.push(entry(
                    "s2",
                    "0123456789abcdef0123456789abcdef0000000000000002",
                    0,
                ))
            }),
            gap(&|s| s.workspaces[0].workspace.recency = MAX_WORKSPACES as u8),
            gap(&|s| s.workspaces[0].workspace.tab_count = 0),
            gap(&|s| s.workspaces[0].active_tab = "t 1".into()),
            gap(&|s| s.tab_messages = MAX_WORKSPACES + 1),
            gap(&|s| {
                s.workspaces = (1..=MAX_WORKSPACES + 1)
                    .map(|n| entry(&format!("s{n}"), &format!("{n:0128x}"), (n - 1) as u8))
                    .collect()
            }),
            SubscriptionMessage::Tabs(Tabs {
                workspace: "s1".into(),
                tabs: vec![tab("t1", &["p1"]), duplicate_surface],
            }),
            SubscriptionMessage::Tabs(Tabs {
                workspace: "s1".into(),
                tabs: vec![rollup],
            }),
            SubscriptionMessage::Tabs(Tabs {
                workspace: "t1".into(),
                tabs: vec![tab("t1", &["p1"])],
            }),
            tab_event(TabChange::Moved, None),
            tab_event(TabChange::Activated, Some(vec!["t2".into()])),
            tab_event(TabChange::Created, Some(vec!["t1".into()])),
            tab_event(TabChange::Closed, Some(vec!["t2".into()])),
            tab_event(TabChange::Moved, Some(vec!["t2".into(), "t2".into()])),
            pane_event(PaneChange::Moved, vec![column(&["p1"], "p1")]),
            pane_event(PaneChange::Closed, vec![column(&["p1", "p2"], "p1")]),
            pane_event(
                PaneChange::Selected,
                vec![column(&["p2"], "p2"), column(&["p2"], "p2")],
            ),
            pane_event(
                PaneChange::Selected,
                vec![column(&["p2"], "p2"); MAX_COLUMNS + 1],
            ),
            pane_event(
                PaneChange::Selected,
                vec![column(&["p2"], "p2"), column(&[], "p3")],
            ),
            event(Change::Attention {
                target: target("s1", "t1", TargetId::Pane("p 1".into())),
                previous: None,
                attention: None,
            }),
            event(Change::Attention {
                target: target("s1", "t1", TargetId::Pane("p1".into())),
                previous: None,
                attention: Some(Attention {
                    state: AttentionState::Working,
                    record: None,
                }),
            }),
            event(Change::Queue(vec![QueueItem {
                state: AttentionState::Idle,
                ..window_snapshot().attention_queue[0].clone()
            }])),
            event(Change::Popup {
                change: PopupChange::Shown,
                workspace: "s1".into(),
                tab: "t1".into(),
                popup: popup("u1", ""),
                chosen: false,
            }),
            event(Change::Pane {
                change: PaneChange::Renamed,
                workspace: "s1".into(),
                tab: "t1".into(),
                pane: Pane {
                    label: "p2 x".into(),
                    ..pane("p2")
                },
                column: 1,
                expanded: true,
                columns: vec![column(&["p2"], "p2")],
            }),
            // The pane's place must match `columns` unless it closed.
            placed(PaneChange::Moved, 0, false, vec![column(&["p2"], "p2")]),
            placed(PaneChange::Closed, 5, false, vec![]),
            placed(
                PaneChange::Moved,
                2,
                true,
                vec![column(&["p2"], "p2"), column(&["p1"], "p1")],
            ),
            placed(PaneChange::Selected, 1, false, vec![column(&["p2"], "p2")]),
            placed(
                PaneChange::Selected,
                1,
                true,
                vec![column(&["p1", "p2"], "p1")],
            ),
            // A busy name exactly while busy, as a busy list names it.
            busy_event(true, None),
            busy_event(false, Some("p1")),
            busy_event(true, Some("")),
            busy_event(true, Some("p1\tvim")),
            gap(&|s| {
                s.busy = Some(vec![
                    BusyTerminal {
                        target: target("s1", "t1", TargetId::Pane("p1".into())),
                        name: "p1".into(),
                    };
                    2
                ])
            }),
        ];
        for message in invalid {
            assert!(
                encode_subscription_message(&message).is_err(),
                "{message:?}"
            );
            // Pane events whose expanded pane is in no column cannot be
            // forged by index; every other case decodes to the same refusal.
            if let SubscriptionMessage::Event(Event {
                change: Change::Pane { columns, .. },
                ..
            }) = &message
                && columns
                    .iter()
                    .any(|column| !column.panes.contains(&column.expanded))
            {
                continue;
            }
            assert!(
                decode_subscription_message(&forge(&message)).is_err(),
                "{message:?}"
            );
        }
        // Subscribed carries the same snapshot rules, and its own.
        let mut bad = window_snapshot();
        bad.attention_queue[0].state = AttentionState::Background;
        let message = subscribed(Some(bad));
        assert!(encode_subscription_message(&message).is_err());
        assert!(decode_subscription_message(&forge(&message)).is_err());
        for message in [
            SubscriptionMessage::Subscribed(Subscribed {
                window: Some("default".into()),
                ..match subscribed(None) {
                    SubscriptionMessage::Subscribed(subscribed) => subscribed,
                    _ => unreachable!(),
                }
            }),
            SubscriptionMessage::Subscribed(Subscribed {
                stream: "g1:1".into(),
                ..match subscribed(None) {
                    SubscriptionMessage::Subscribed(subscribed) => subscribed,
                    _ => unreachable!(),
                }
            }),
            SubscriptionMessage::Tabs(Tabs {
                workspace: "s1".into(),
                tabs: vec![],
            }),
            SubscriptionMessage::End(End::Ended {
                generation: String::new(),
            }),
        ] {
            assert!(
                encode_subscription_message(&message).is_err(),
                "{message:?}"
            );
        }
        // Unknown tags, kinds and trailing bytes.
        let end = encode_subscription_message(&SubscriptionMessage::End(End::TooSlow)).unwrap();
        let mut forged = end.clone();
        forged[HEADER_BYTES] = 3;
        assert!(decode_subscription_message(&forged).is_err());
        let mut forged = end.clone();
        forged[6] = 141;
        assert!(decode_subscription_message(&forged).is_err());
        let mut forged = end.clone();
        forged.push(0);
        forged[8] += 1;
        assert!(decode_subscription_message(&forged).is_err());
        let busy = encode_subscription_message(&event(Change::Busy {
            target: target("s1", "t1", TargetId::Pane("p1".into())),
            busy: false,
            name: None,
        }))
        .unwrap();
        for (offset, value) in [
            (HEADER_BYTES + 8, 7),
            (busy.len() - 2, 2),
            (busy.len() - 1, 2),
        ] {
            let mut forged = busy.clone();
            forged[offset] = value;
            assert!(decode_subscription_message(&forged).is_err(), "{offset}");
        }
    }

    #[test]
    fn subscription_messages_keep_their_wire_bytes() {
        let confirmed = entry("s1", "r", 0);
        let mut failed = confirmed.clone();
        failed.workspace.persistent_id = None;
        failed.workspace.registration_error = Some(Failure {
            code: "registry-busy".into(),
            detail: "held".into(),
        });
        for (change, workspace, expected) in [
            (
                WorkspaceChange::Registered,
                confirmed,
                "454f4e5726008a001c0000002a000000000000000004020073310101007200000100000002007431",
            ),
            (
                WorkspaceChange::RegistrationFailed,
                failed,
                "454f4e5726008a002e0000002a0000000000000000050200733100010d0072656769737472792d62757379040068656c64000100000002007431",
            ),
        ] {
            let message = event(Change::Workspace { change, workspace });
            let bytes = encode_subscription_message(&message).unwrap();
            assert_eq!(hex(&bytes), expected);
            assert_eq!(decode_subscription_message(&bytes).unwrap(), message);
        }
        let request = subscribe(
            Filter {
                workspaces: vec!["s2".into()],
                tabs: vec!["t1".into()],
                targets: vec!["p1".into()],
                kinds: vec![EventKind::Attention],
                states: vec![AttentionState::NeedsInput],
            },
            Some(Resume {
                stream: "g1.7".into(),
                seq: 5,
            }),
        );
        assert_eq!(
            hex(&encode_request(&request).unwrap()),
            "454f4e57260001002d00000002007231002801000200733201000200743101000200703101000401000101040067312e370500000000000000"
        );
        let attention_event = event(Change::Attention {
            target: target("s1", "t1", TargetId::Pane("p1".into())),
            previous: Some(AttentionState::Working),
            attention: Some(attention(AttentionState::NeedsInput)),
        });
        let tab_event = event(Change::Tab {
            change: TabChange::Moved,
            workspace: "s1".into(),
            tab: tab("t2", &["p3"]),
            tabs: Some(vec!["t2".into(), "t1".into()]),
        });
        let pane_event = event(Change::Pane {
            change: PaneChange::Moved,
            workspace: "s1".into(),
            tab: "t1".into(),
            pane: pane("p2"),
            column: 1,
            expanded: true,
            columns: vec![ColumnOrder {
                panes: vec!["p1".into(), "p2".into()],
                expanded: "p2".into(),
            }],
        });
        let popup_event = event(Change::Popup {
            change: PopupChange::Shown,
            workspace: "s1".into(),
            tab: "t1".into(),
            popup: popup("u1", "agent"),
            chosen: true,
        });
        let busy_event = event(Change::Busy {
            target: target("s1", "t1", TargetId::Pane("p1".into())),
            busy: true,
            name: Some("p1  vim".into()),
        });
        for (message, expected) in [
            (
                subscribed(None),
                "454f4e572600880021000000100067312e31373030303030303030303030290000000000000002006731000000",
            ),
            (
                subscribed(Some(window_snapshot())),
                "454f4e572600880076000000100067312e3137303030303030303030303029000000000000000200673100000101000200733101300030313233343536373839616263646566303132333435363738396162636465663030303030303030303030303030303100000100000002007431010002007331020074310200703101000100",
            ),
            (
                SubscriptionMessage::Tabs(Tabs {
                    workspace: "s1".into(),
                    tabs: vec![tab("t1", &["p1"])],
                }),
                "454f4e5726008900340000000200733101000100010000000200743101002f00010200703100010002007031030073703103002f703101020070310000000000",
            ),
            (
                attention_event,
                "454f4e5726008a00400000002a00000000000000040200733102007431000200703102007331040102ff0600636c6175646500000900436f6e74696e75653f00000700000000000000000000",
            ),
            (
                tab_event,
                "454f4e5726008a00460000002a000000000000000101020073310100010000000200743201002f00010200703300010002007033030073703303002f70330102007033000000000002000200743202007431",
            ),
            (
                pane_event,
                "454f4e5726008a00370000002a000000000000000201020073310200743102007032030073703203002f70320102007032000001010100020002007031020070320100",
            ),
            (
                popup_event,
                "454f4e5726008a00290000002a00000000000000030102007331020074310200753105006167656e74030073753103002f75310001",
            ),
            (
                busy_event,
                "454f4e5726008a00250000002a00000000000000060200733102007431000200703102007331010107007031202076696d",
            ),
            (
                SubscriptionMessage::Gap(Gap {
                    seq: 90,
                    reason: GapReason::Overflow,
                    snapshot: window_snapshot(),
                }),
                "454f4e5726008b005e0000005a000000000000000001000200733101300030313233343536373839616263646566303132333435363738396162636465663030303030303030303030303030303100000100000002007431010002007331020074310200703101000100",
            ),
            (
                SubscriptionMessage::End(End::Ended {
                    generation: "g1".into(),
                }),
                "454f4e5726008c00050000000002006731",
            ),
            (
                SubscriptionMessage::End(End::TargetsGone),
                "454f4e5726008c000100000001",
            ),
            (
                SubscriptionMessage::End(End::TooSlow),
                "454f4e5726008c000100000002",
            ),
        ] {
            assert_eq!(
                hex(&encode_subscription_message(&message).unwrap()),
                expected,
                "{message:?}"
            );
        }
    }

    #[test]
    fn every_legal_subscription_message_fits_the_frame_budget() {
        // Upper bounds from each field's largest form.
        let id = 2 + MAX_ID_BYTES;
        let attention_bytes = 3
            + 2
            + MAX_APP_BYTES
            + 2
            + MAX_NAME_BYTES
            + 2
            + MAX_TITLE_BYTES
            + 2
            + MAX_STATUS_MESSAGE_BYTES
            + 8
            + 1
            + 2;
        let pane_bytes = id
            + id
            + 2
            + MAX_ENDPOINT_BYTES
            + 1
            + 2
            + MAX_PANE_LABEL_BYTES
            + 1
            + 2
            + MAX_PANE_NAME_BYTES
            + attention_bytes;
        let popup_bytes =
            id + 2 + MAX_ENTRY_ID_BYTES + id + 2 + MAX_ENDPOINT_BYTES + attention_bytes;
        let surface = pane_bytes.max(popup_bytes);
        // A tab's layout and own fields.
        let tab_bytes =
            2 + MAX_COLUMNS * 4 + id + 2 + MAX_DIRECTORY_BYTES + 1 + 2 * (1 + id) + 2 + 2 + 1;
        let target_bytes = 4 * id + 1;
        let queue_bytes = 2 + MAX_SESSIONS * (3 * id + 1);
        let workspace_bytes = 2 * id
            + 2
            + id
            + 2
            + super::super::MAX_DETAIL_BYTES
            + 1
            + 2
            + MAX_WORKSPACE_NAME_BYTES
            + 2
            + 1
            + 1
            + id;
        let snapshot_bytes = 2
            + MAX_WORKSPACES * workspace_bytes
            + queue_bytes
            + 1
            + 2
            + MAX_SESSIONS * (target_bytes + 2 + MAX_PANE_LABEL_BYTES)
            + 2;

        // A window holds one tab per terminal, so its tabs cannot share one
        // frame: hence one TABS message per workspace.
        assert!(MAX_SESSIONS * (tab_bytes + surface) > MAX_MESSAGE_BYTES);
        let tabs_bound = HEADER_BYTES + id + 2 + MAX_TABS * tab_bytes + MAX_SESSIONS * surface;
        let subscribed_bound =
            HEADER_BYTES + id + 8 + id + 1 + id + 1 + target_bytes + 1 + snapshot_bytes;
        let gap_bound = HEADER_BYTES + 8 + 1 + snapshot_bytes;
        let event_head = HEADER_BYTES + 8 + 1;
        let tab_event_bound =
            event_head + 1 + id + tab_bytes + MAX_SESSIONS * surface + 2 + MAX_TABS * id;
        let pane_event_bound =
            event_head + 1 + 2 * id + pane_bytes + 2 + 2 + MAX_COLUMNS * 4 + MAX_SESSIONS * id;
        let queue_event_bound = event_head + queue_bytes;
        let other_event_bound = event_head
            + (1 + workspace_bytes)
                .max(1 + 2 * id + popup_bytes + 1)
                .max(target_bytes + 1 + attention_bytes)
                .max(target_bytes + 1 + 1 + 2 + MAX_PANE_LABEL_BYTES);
        let largest_event = tab_event_bound
            .max(pane_event_bound)
            .max(queue_event_bound)
            .max(other_event_bound);
        for bound in [tabs_bound, subscribed_bound, gap_bound, largest_event] {
            assert!(bound <= MAX_MESSAGE_BYTES, "{bound}");
        }
        assert_eq!(largest_event, tab_event_bound);

        let name = |prefix: &str, n: usize, length: usize| {
            let name = format!("{prefix}{n}");
            format!("{name}{}", "x".repeat(length - name.len()))
        };
        let attention = Some(Attention {
            state: AttentionState::NeedsInput,
            record: Some(AttentionRecord {
                kind: Some(AttentionKind::Permission),
                progress: Some(100),
                app: "a".repeat(MAX_APP_BYTES),
                name: "n".repeat(MAX_NAME_BYTES),
                title: "t".repeat(MAX_TITLE_BYTES),
                message: "m".repeat(MAX_STATUS_MESSAGE_BYTES),
                changed_at: u64::MAX,
                stale: true,
                others: MAX_OTHER_RECORDS,
            }),
        });
        let long_pane = |n: usize| {
            let id = name("p", n, MAX_ID_BYTES);
            Pane {
                label: format!("{id}  {}", "🦀".repeat(MAX_PANE_NAME_CHARS)),
                pinned_name: Some("🦀".repeat(MAX_PANE_NAME_CHARS)),
                id,
                session: name("s", n, MAX_ID_BYTES),
                endpoint: name("/", n, MAX_ENDPOINT_BYTES).into_bytes(),
                live: true,
                attention: attention.clone(),
            }
        };
        // `columns` columns of `per_column` panes, numbered from `first`.
        let long_tab = |t: usize, first: usize, columns: usize, per_column: usize| {
            let columns: Vec<_> = (0..columns)
                .map(|c| {
                    let panes: Vec<_> = (0..per_column)
                        .map(|i| long_pane(first + c * per_column + i))
                        .collect();
                    Column {
                        expanded: panes[0].id.clone(),
                        panes,
                    }
                })
                .collect();
            Tab {
                id: name("t", t, MAX_ID_BYTES),
                directory: vec![b'/'; MAX_DIRECTORY_BYTES],
                pending: false,
                selected_pane: Some(columns[0].expanded.clone()),
                selected_popup: None,
                columns,
                popups: vec![],
                attention_rollup: Some(AttentionState::NeedsInput),
            }
        };
        let check = |message: &SubscriptionMessage, bound: usize| {
            let bytes = encode_subscription_message(message).unwrap();
            assert!(bytes.len() <= bound, "{} > {bound}", bytes.len());
            assert_eq!(&decode_subscription_message(&bytes).unwrap(), message);
        };
        let long_workspace = "s".to_string() + &u64::MAX.to_string();

        // One workspace's most tabs, each with the most columns, holding the
        // window's every terminal.
        let per_tab = MAX_SESSIONS / MAX_TABS;
        let mut tabs = Tabs {
            workspace: long_workspace.clone(),
            tabs: (0..MAX_TABS)
                .map(|t| long_tab(t, t * per_tab, MAX_COLUMNS, per_tab / MAX_COLUMNS))
                .collect(),
        };
        check(&SubscriptionMessage::Tabs(tabs.clone()), tabs_bound);
        // Neither side lets a TABS message past the session bound.
        tabs.tabs[0].columns[0].panes.push(long_pane(MAX_SESSIONS));
        let message = SubscriptionMessage::Tabs(tabs);
        assert!(encode_subscription_message(&message).is_err());
        assert!(decode_subscription_message(&forge(&message)).is_err());

        // The longest snapshot head, with the longest caller.
        let long_target = |n: usize| TerminalTarget {
            workspace: long_workspace.clone(),
            tab: "t".repeat(MAX_ID_BYTES),
            id: TargetId::Popup("u".repeat(MAX_ID_BYTES)),
            terminal: name("s", n, MAX_ID_BYTES),
        };
        let snapshot = WindowSnapshot {
            workspaces: (1..=MAX_WORKSPACES)
                .map(|n| WorkspaceEntry {
                    workspace: WorkspaceInfo {
                        id: if n == MAX_WORKSPACES {
                            long_workspace.clone()
                        } else {
                            format!("s{n}")
                        },
                        persistent_id: Some(format!("{n:0128x}")),
                        registration_error: Some(Failure {
                            code: "e".repeat(MAX_ID_BYTES),
                            detail: "e".repeat(super::super::MAX_DETAIL_BYTES),
                        }),
                        name: Some("🦀".repeat(MAX_WORKSPACE_NAME_CHARS)),
                        tab_count: MAX_TABS,
                        attention: Some(AttentionState::NeedsInput),
                        recency: (n - 1) as u8,
                    },
                    active_tab: name("t", n, MAX_ID_BYTES),
                })
                .collect(),
            attention_queue: (0..MAX_SESSIONS)
                .map(|n| QueueItem {
                    workspace: long_workspace.clone(),
                    tab: name("t", n / per_tab, MAX_ID_BYTES),
                    target: name("p", n, MAX_ID_BYTES),
                    state: AttentionState::NeedsInput,
                })
                .collect(),
            busy: Some(
                (0..MAX_SESSIONS)
                    .map(|n| BusyTerminal {
                        target: long_target(n),
                        name: "🦀".repeat(MAX_PANE_LABEL_BYTES / 4),
                    })
                    .collect(),
            ),
            tab_messages: MAX_WORKSPACES,
        };
        check(
            &SubscriptionMessage::Subscribed(Subscribed {
                stream: "g".repeat(MAX_ID_BYTES),
                seq: u64::MAX,
                generation: "g".repeat(MAX_ID_BYTES),
                window: Some("0123abcd".into()),
                caller: Some(long_target(MAX_SESSIONS)),
                snapshot: Some(snapshot.clone()),
            }),
            subscribed_bound,
        );
        check(
            &SubscriptionMessage::Gap(Gap {
                seq: u64::MAX,
                reason: GapReason::Overflow,
                snapshot: snapshot.clone(),
            }),
            gap_bound,
        );

        // The largest event: one tab holding every terminal, with the
        // longest tab order.
        let tab = long_tab(0, 0, MAX_COLUMNS, MAX_SESSIONS / MAX_COLUMNS);
        let mut order: Vec<_> = (1..MAX_TABS).map(|t| name("t", t, MAX_ID_BYTES)).collect();
        order.push(tab.id.clone());
        check(
            &event(Change::Tab {
                change: TabChange::Moved,
                workspace: long_workspace.clone(),
                tab: tab.clone(),
                tabs: Some(order),
            }),
            tab_event_bound,
        );
        check(
            &event(Change::Pane {
                change: PaneChange::Created,
                workspace: long_workspace.clone(),
                tab: tab.id.clone(),
                pane: tab.columns[0].panes[0].clone(),
                column: 1,
                expanded: true,
                columns: tab
                    .columns
                    .iter()
                    .map(|column| ColumnOrder {
                        panes: column.panes.iter().map(|pane| pane.id.clone()).collect(),
                        expanded: column.expanded.clone(),
                    })
                    .collect(),
            }),
            pane_event_bound,
        );
        check(
            &event(Change::Queue(snapshot.attention_queue.clone())),
            queue_event_bound,
        );
    }

    #[test]
    fn carried_tabs_bound_their_popups_on_both_sides() {
        let with_popups = |count: usize| {
            let mut tab = tab("t1", &["p1"]);
            tab.popups = (0..count)
                .map(|n| popup(&format!("u{n}"), &format!("e{n}")))
                .collect();
            tab
        };
        let fits = SubscriptionMessage::Tabs(Tabs {
            workspace: "s1".into(),
            tabs: vec![with_popups(MAX_ENTRIES)],
        });
        let bytes = encode_subscription_message(&fits).unwrap();
        assert_eq!(decode_subscription_message(&bytes).unwrap(), fits);
        for over in [
            SubscriptionMessage::Tabs(Tabs {
                workspace: "s1".into(),
                tabs: vec![with_popups(MAX_ENTRIES + 1)],
            }),
            event(Change::Tab {
                change: TabChange::Directory,
                workspace: "s1".into(),
                tab: with_popups(MAX_ENTRIES + 1),
                tabs: None,
            }),
        ] {
            assert!(encode_subscription_message(&over).is_err());
            assert!(decode_subscription_message(&forge(&over)).is_err());
        }
    }
}
