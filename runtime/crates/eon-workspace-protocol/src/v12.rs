//! Eon-owned values and bounded EONW v12 codec for local workspace clients.
//!
//! Every message is a 12-byte header (`EONW`, little-endian version, kind,
//! zero flags, little-endian payload length) followed by its payload.
//! Strings and byte fields carry a little-endian `u16` length prefix.

use std::{collections::HashSet, fmt, str};

pub const VERSION: u16 = 12;
pub const HEADER_BYTES: usize = 12;
const MAGIC: &[u8; 4] = b"EONW";
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_ID_BYTES: usize = 128;
pub(crate) const MAX_ENDPOINT_BYTES: usize = 4096;
pub(crate) const MAX_VERSION_BYTES: usize = 128;
pub(crate) const MAX_COMPONENT_REPORT_BYTES: usize = 32 * 1024;
pub const MAX_DETAIL_BYTES: usize = 1024;
pub const MAX_TABS: usize = 64;
pub const MAX_SESSIONS: usize = 256;
pub const MAX_DIRECTORY_BYTES: usize = 4096;
pub const MAX_COLUMNS: usize = 4;
pub const MAX_ENTRIES: usize = 32;
pub const MAX_ENTRY_ID_BYTES: usize = 64;
pub const MAX_LABEL_BYTES: usize = 128;
pub const MAX_KEY_BYTES: usize = 16;
pub const MAX_CODEX_QUOTA_WINDOWS: usize = 2;
pub const PROJECT_ENTRY: &str = "project";
pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;

pub(crate) const REQUEST: u8 = 1;
pub(crate) const SNAPSHOT: u8 = 129;
pub(crate) const FAILURE: u8 = 130;
pub(crate) const RUNTIME: u8 = 131;
pub(crate) const STOPPED: u8 = 132;

const INVOKE_POPUP: u8 = 19;
const DISMISS_POPUP: u8 = 20;
const COMMIT_DIRECTORY: u8 = 21;
const CREATE_COLUMN: u8 = 22;
const CLOSE_PANE: u8 = 23;
const MOVE_PANE_LEFT: u8 = 24;
const MOVE_PANE_RIGHT: u8 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceAction {
    Inspect,
    InspectRuntime,
    InspectPresentation,
    Present {
        workspace: bool,
    },
    CreateTab,
    CreatePane,
    FocusId(String),
    Focus(Direction),
    Move(Direction),
    CloseTab {
        tab: String,
    },
    Stop {
        generation: String,
    },
    /// Explicit retargeting, not chooser completion.
    SetTabDirectory {
        tab: String,
        directory: Vec<u8>,
    },
    /// Retired in v5: every v5 or later request carrying it is rejected.
    PickTabDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupTarget {
    pub tab: String,
    pub instance: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvokeIntent {
    Toggle,
    /// A visible but unfocused popup receives focus instead of being hidden.
    Focus,
}

/// The column a sideways pane move goes towards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Workspace(WorkspaceAction),
    InvokePopup {
        tab: String,
        entry: String,
        /// The tab-scoped instance the requester last observed.
        expected_instance: Option<String>,
        intent: InvokeIntent,
    },
    DismissPopup(PopupTarget),
    CommitDirectory {
        target: PopupTarget,
        directory: Vec<u8>,
    },
    /// Add a column right of the selected column with one new pane.
    CreateColumn,
    /// Stop the terminal of one pane in the active tab.
    ClosePane {
        pane: String,
    },
    /// Move the selected pane into the neighbouring column on `Side`.
    MovePaneSideways(Side),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub session: String,
    pub endpoint: Vec<u8>,
    pub live: bool,
}

/// One accordion of panes; `expanded` names the pane it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub panes: Vec<Pane>,
    pub expanded: String,
}

/// Canonical physical key names, independent of keyboard layout or display text.
/// KeyA..KeyZ, Digit0..Digit9, F1..F24 and the named keys in `validate` are supported.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: u8,
    pub key: String,
}

impl Shortcut {
    /// Validate normalized wire syntax. The config owner also checks collisions
    /// against fixed workspace/native bindings before publishing the catalog.
    pub fn validate(&self) -> Result<()> {
        let key = self.key.as_str();
        let valid_key = match key.as_bytes() {
            [b'K', b'e', b'y', b'A'..=b'Z'] | [b'D', b'i', b'g', b'i', b't', b'0'..=b'9'] => true,
            _ => {
                key.strip_prefix('F')
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=24).contains(&n) && key == format!("F{n}"))
                    || matches!(
                        key,
                        "Escape"
                            | "Tab"
                            | "Enter"
                            | "Backspace"
                            | "Space"
                            | "Insert"
                            | "Delete"
                            | "Home"
                            | "End"
                            | "PageUp"
                            | "PageDown"
                            | "ArrowLeft"
                            | "ArrowRight"
                            | "ArrowUp"
                            | "ArrowDown"
                            | "Backquote"
                            | "Backslash"
                            | "BracketLeft"
                            | "BracketRight"
                            | "Comma"
                            | "Equal"
                            | "Minus"
                            | "Period"
                            | "Quote"
                            | "Semicolon"
                            | "Slash"
                    )
            }
        };
        if self.modifiers == 0
            || self.modifiers & !(SHIFT | CTRL | ALT | SUPER) != 0
            || !valid_key
            || (self.modifiers == CTRL && matches!(key, "KeyC" | "Backslash"))
        {
            return Err(Error::InvalidValue { field: "shortcut" });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupEntry {
    pub id: String,
    pub label: String,
    pub shortcut: Shortcut,
}

/// One live instance. A replacement receives a fresh id and endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub id: String,
    pub entry: String,
    pub session: String,
    pub endpoint: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupGeometry {
    pub side_margin: f32,
    pub vertical_margin: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub directory: Vec<u8>,
    pub pending: bool,
    /// The focused pane: one column's expanded pane.
    pub selected_pane: Option<String>,
    /// Retained when another tab is active; not native keyboard-focus state.
    pub selected_popup: Option<String>,
    pub columns: Vec<Column>,
    pub popups: Vec<Popup>,
}

impl Tab {
    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.columns.iter().flat_map(|column| &column.panes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexQuotaState {
    Fresh,
    Stale,
    Blocked,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexQuotaWindow {
    pub duration_minutes: u32,
    pub remaining_percent: u8,
    pub resets_at: Option<u64>,
}

pub use self::CodexQuotaWindow as QuotaWindow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexQuota {
    pub state: CodexQuotaState,
    pub observed_at: u64,
    pub windows: Vec<CodexQuotaWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeQuotaState {
    Fresh,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeQuota {
    pub state: ClaudeQuotaState,
    pub observed_at: u64,
    pub windows: Vec<QuotaWindow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub active_tab: String,
    pub geometry: PopupGeometry,
    pub entries: Vec<PopupEntry>,
    pub tabs: Vec<Tab>,
    pub codex_quota: Option<CodexQuota>,
    pub claude_quota: Option<ClaudeQuota>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Snapshot(Snapshot),
    Failure(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Availability {
    pub available: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub generation: String,
    pub eon_version: String,
    pub workspace_protocol: u16,
    pub component_report: String,
    pub sessions: Vec<String>,
    pub attach: Availability,
    pub stop: Availability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    pub generation: String,
    pub sessions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleResponse {
    Runtime(Runtime),
    Stopped(Stopped),
    Failure(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    InvalidMagic,
    UnsupportedVersion { version: u16 },
    InvalidFlags { value: u8 },
    MessageTooLarge { size: usize },
    Truncated,
    TrailingBytes { count: usize },
    InvalidKind { value: u8 },
    InvalidTag { field: &'static str, value: u8 },
    InvalidUtf8 { field: &'static str },
    InvalidValue { field: &'static str },
    FieldTooLong { field: &'static str, length: usize },
    InvalidSnapshot { field: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("invalid EONW message magic"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported EONW version {version}")
            }
            Self::InvalidFlags { value } => write!(formatter, "invalid EONW flags {value:#x}"),
            Self::MessageTooLarge { size } => write!(
                formatter,
                "EONW message is {size} bytes; maximum is {MAX_MESSAGE_BYTES}"
            ),
            Self::Truncated => formatter.write_str("truncated EONW message"),
            Self::TrailingBytes { count } => {
                write!(formatter, "EONW message has {count} trailing bytes")
            }
            Self::InvalidKind { value } => write!(formatter, "invalid EONW message kind {value}"),
            Self::InvalidTag { field, value } => {
                write!(formatter, "invalid EONW {field} tag {value}")
            }
            Self::InvalidUtf8 { field } => write!(formatter, "EONW {field} is not UTF-8"),
            Self::InvalidValue { field } => write!(formatter, "invalid EONW {field}"),
            Self::FieldTooLong { field, length } => {
                write!(formatter, "EONW {field} is too long: {length} bytes")
            }
            Self::InvalidSnapshot { field } => {
                write!(formatter, "invalid EONW snapshot {field}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Check popup preconditions against the owner's current, valid snapshot
/// immediately before mutation. This does not execute lifecycle policy or
/// validate ordinary workspace actions. Decoding alone cannot detect staleness.
pub fn check_popup_action(snapshot: &Snapshot, action: &Action) -> Result<()> {
    validate_popup_action(action)?;
    let matches = match action {
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            ..
        } => {
            snapshot.entries.iter().any(|item| item.id == *entry)
                && snapshot
                    .tabs
                    .iter()
                    .find(|item| item.id == *tab)
                    .is_some_and(|tab| {
                        tab.popups
                            .iter()
                            .find(|popup| popup.entry == *entry)
                            .map(|popup| &popup.id)
                            == expected_instance.as_ref()
                    })
        }
        Action::DismissPopup(target) | Action::CommitDirectory { target, .. } => snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == target.tab)
            .is_some_and(|tab| {
                tab.popups.iter().any(|popup| {
                    popup.id == target.instance
                        && (!matches!(action, Action::CommitDirectory { .. })
                            || (popup.entry == PROJECT_ENTRY
                                && tab.selected_popup.as_ref() == Some(&popup.id)))
                })
            }),
        _ => return Ok(()),
    };
    if matches {
        Ok(())
    } else {
        Err(Error::InvalidValue {
            field: "popup target",
        })
    }
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    validate_popup_action(&request.action)?;
    identity("request id", &request.id)?;
    let mut payload = Encoder::default();
    payload.string(&request.id);
    match &request.action {
        Action::Workspace(action) => encode_workspace_action(&mut payload, action)?,
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        } => {
            payload.byte(INVOKE_POPUP);
            payload.string(tab);
            payload.string(entry);
            encode_optional(&mut payload, expected_instance);
            payload.byte(match intent {
                InvokeIntent::Toggle => 0,
                InvokeIntent::Focus => 1,
            });
        }
        Action::DismissPopup(target) => {
            payload.byte(DISMISS_POPUP);
            payload.string(&target.tab);
            payload.string(&target.instance);
        }
        Action::CommitDirectory { target, directory } => {
            payload.byte(COMMIT_DIRECTORY);
            payload.string(&target.tab);
            payload.string(&target.instance);
            payload.raw(directory);
        }
        Action::CreateColumn => payload.byte(CREATE_COLUMN),
        Action::ClosePane { pane } => {
            identity("pane id", pane)?;
            payload.byte(CLOSE_PANE);
            payload.string(pane);
        }
        Action::MovePaneSideways(side) => payload.byte(match side {
            Side::Left => MOVE_PANE_LEFT,
            Side::Right => MOVE_PANE_RIGHT,
        }),
    }
    frame_version(VERSION, REQUEST, payload.bytes)
}

fn encode_workspace_action(payload: &mut Encoder, action: &WorkspaceAction) -> Result<()> {
    match action {
        WorkspaceAction::Inspect => payload.byte(0),
        WorkspaceAction::CreateTab => payload.byte(1),
        WorkspaceAction::CreatePane => payload.byte(2),
        WorkspaceAction::FocusId(id) => {
            nonempty("focus id", id, MAX_ID_BYTES)?;
            payload.byte(3);
            payload.string(id);
        }
        WorkspaceAction::Focus(direction) => payload.byte(4 + direction_offset(*direction)),
        WorkspaceAction::InspectRuntime => payload.byte(8),
        WorkspaceAction::Stop { generation } => {
            identity("generation", generation)?;
            payload.byte(9);
            payload.string(generation);
        }
        WorkspaceAction::Present { workspace } => {
            payload.byte(10);
            payload.byte(u8::from(*workspace));
        }
        WorkspaceAction::InspectPresentation => payload.byte(11),
        WorkspaceAction::SetTabDirectory { tab, directory } => {
            identity("tab id", tab)?;
            validate_directory(directory)?;
            payload.byte(12);
            payload.string(tab);
            payload.raw(directory);
        }
        WorkspaceAction::PickTabDirectory => unreachable!("rejected by validate_popup_action"),
        WorkspaceAction::Move(direction) => payload.byte(14 + direction_offset(*direction)),
        WorkspaceAction::CloseTab { tab } => {
            identity("tab id", tab)?;
            payload.byte(18);
            payload.string(tab);
        }
    }
    Ok(())
}

fn direction_offset(direction: Direction) -> u8 {
    match direction {
        Direction::Left => 0,
        Direction::Right => 1,
        Direction::Up => 2,
        Direction::Down => 3,
    }
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let (kind, payload) = unframe_version(bytes, VERSION)?;
    if kind != REQUEST {
        return Err(Error::InvalidKind { value: kind });
    }
    let mut decoder = Decoder::new(payload);
    let id = decoder.string("request id", MAX_ID_BYTES)?;
    let tag = decoder.byte()?;
    let action = match tag {
        CREATE_COLUMN => Some(Action::CreateColumn),
        CLOSE_PANE => {
            let pane = decoder.string("pane id", MAX_ID_BYTES)?;
            Some(Action::ClosePane { pane })
        }
        MOVE_PANE_LEFT => Some(Action::MovePaneSideways(Side::Left)),
        MOVE_PANE_RIGHT => Some(Action::MovePaneSideways(Side::Right)),
        _ => None,
    };
    if let Some(action) = action {
        decoder.finish()?;
        identity("request id", &id)?;
        if let Action::ClosePane { pane } = &action {
            identity("pane id", pane)?;
        }
        return Ok(Request { id, action });
    }
    identity("request id", &id)?;
    let action = match tag {
        INVOKE_POPUP => Action::InvokePopup {
            tab: decoder.string("tab id", MAX_ID_BYTES)?,
            entry: decoder.string("entry id", MAX_ENTRY_ID_BYTES)?,
            expected_instance: decode_optional(&mut decoder, "expected instance")?,
            intent: match decoder.byte()? {
                0 => InvokeIntent::Toggle,
                1 => InvokeIntent::Focus,
                value => {
                    return Err(Error::InvalidTag {
                        field: "popup intent",
                        value,
                    });
                }
            },
        },
        DISMISS_POPUP | COMMIT_DIRECTORY => {
            let target = PopupTarget {
                tab: decoder.string("tab id", MAX_ID_BYTES)?,
                instance: decoder.string("popup id", MAX_ID_BYTES)?,
            };
            if tag == DISMISS_POPUP {
                Action::DismissPopup(target)
            } else {
                Action::CommitDirectory {
                    target,
                    directory: decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec(),
                }
            }
        }
        tag => Action::Workspace(decode_workspace_action(&mut decoder, tag)?),
    };
    decoder.finish()?;
    validate_popup_action(&action)?;
    Ok(Request { id, action })
}

fn decode_workspace_action(decoder: &mut Decoder<'_>, tag: u8) -> Result<WorkspaceAction> {
    let direction = |offset| match offset {
        0 => Direction::Left,
        1 => Direction::Right,
        2 => Direction::Up,
        _ => Direction::Down,
    };
    Ok(match tag {
        0 => WorkspaceAction::Inspect,
        1 => WorkspaceAction::CreateTab,
        2 => WorkspaceAction::CreatePane,
        3 => {
            let id = decoder.string("focus id", MAX_ID_BYTES)?;
            nonempty("focus id", &id, MAX_ID_BYTES)?;
            WorkspaceAction::FocusId(id)
        }
        4..=7 => WorkspaceAction::Focus(direction(tag - 4)),
        8 => WorkspaceAction::InspectRuntime,
        9 => {
            let generation = decoder.string("generation", MAX_ID_BYTES)?;
            identity("generation", &generation)?;
            WorkspaceAction::Stop { generation }
        }
        10 => WorkspaceAction::Present {
            workspace: decode_bool(decoder, "presentation mode")?,
        },
        11 => WorkspaceAction::InspectPresentation,
        12 => {
            let tab = decoder.string("tab id", MAX_ID_BYTES)?;
            identity("tab id", &tab)?;
            let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
            validate_directory(&directory)?;
            WorkspaceAction::SetTabDirectory { tab, directory }
        }
        13 => WorkspaceAction::PickTabDirectory,
        14..=17 => WorkspaceAction::Move(direction(tag - 14)),
        18 => {
            let tab = decoder.string("tab id", MAX_ID_BYTES)?;
            identity("tab id", &tab)?;
            WorkspaceAction::CloseTab { tab }
        }
        value => {
            return Err(Error::InvalidTag {
                field: "action",
                value,
            });
        }
    })
}

fn validate_popup_action(action: &Action) -> Result<()> {
    match action {
        Action::Workspace(WorkspaceAction::PickTabDirectory) => {
            return Err(Error::InvalidValue {
                field: "retired picker action",
            });
        }
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            ..
        } => {
            identity("tab id", tab)?;
            entry_id(entry)?;
            if let Some(id) = expected_instance {
                identity("popup id", id)?;
            }
        }
        Action::DismissPopup(target) | Action::CommitDirectory { target, .. } => {
            identity("tab id", &target.tab)?;
            identity("popup id", &target.instance)?;
            if let Action::CommitDirectory { directory, .. } = action {
                validate_directory(directory)?;
            }
        }
        Action::Workspace(_)
        | Action::CreateColumn
        | Action::ClosePane { .. }
        | Action::MovePaneSideways(_) => (),
    }
    Ok(())
}

/// A tab as the snapshot payload carries it: panes flattened in column
/// order, with the column layout sent ahead of the workspace.
struct WireTab {
    id: String,
    directory: Vec<u8>,
    pending: bool,
    selected_pane: Option<String>,
    selected_popup: Option<String>,
    panes: Vec<Pane>,
    popups: Vec<Popup>,
}

/// A snapshot payload is the column layout, the workspace with flattened
/// panes, then the Codex and Claude quotas.
pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    let mut payload = Encoder::default();
    let kind = match response {
        Response::Snapshot(snapshot) => {
            let tabs: Vec<_> = snapshot
                .tabs
                .iter()
                .map(|tab| WireTab {
                    id: tab.id.clone(),
                    directory: tab.directory.clone(),
                    pending: tab.pending,
                    selected_pane: tab.selected_pane.clone(),
                    selected_popup: tab.selected_popup.clone(),
                    panes: tab.panes().cloned().collect(),
                    popups: tab.popups.clone(),
                })
                .collect();
            let claude = snapshot.claude_quota.as_ref().map(claude_as_codex);
            validate_workspace(snapshot, &tabs)?;
            validate_quota(snapshot.codex_quota.as_ref())?;
            validate_claude(claude.as_ref())?;
            payload.count(snapshot.tabs.len());
            for tab in &snapshot.tabs {
                validate_columns(tab)?;
                payload.count(tab.columns.len());
                for column in &tab.columns {
                    payload.count(column.panes.len());
                    let expanded = column
                        .panes
                        .iter()
                        .position(|pane| pane.id == column.expanded)
                        .expect("validated expanded pane");
                    payload.count(expanded);
                }
            }
            encode_workspace(&mut payload, snapshot, &tabs);
            encode_quota(&mut payload, snapshot.codex_quota.as_ref());
            encode_quota(&mut payload, claude.as_ref());
            SNAPSHOT
        }
        Response::Failure(failure) => {
            encode_failure(&mut payload, failure)?;
            FAILURE
        }
    };
    frame_version(VERSION, kind, payload.bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    let (kind, payload) = unframe_version(bytes, VERSION)?;
    let mut decoder = Decoder::new(payload);
    if kind == FAILURE {
        let failure = decode_failure(&mut decoder)?;
        decoder.finish()?;
        return Ok(Response::Failure(failure));
    }
    if kind != SNAPSHOT {
        return Err(Error::InvalidKind { value: kind });
    }
    let tab_count = decoder.count("tabs", MAX_TABS)?;
    let mut layouts = Vec::with_capacity(tab_count);
    for _ in 0..tab_count {
        let columns = decoder.count("columns", MAX_COLUMNS)?;
        let mut layout = Vec::with_capacity(columns);
        for _ in 0..columns {
            let panes = decoder.count("column panes", MAX_SESSIONS)?;
            layout.push((panes, decoder.count("expanded pane", MAX_SESSIONS)?));
        }
        layouts.push(layout);
    }
    let (mut snapshot, wire_tabs) = decode_workspace(&mut decoder)?;
    snapshot.codex_quota = decode_quota(&mut decoder)?;
    let claude = decode_quota(&mut decoder)?;
    validate_workspace(&snapshot, &wire_tabs)?;
    validate_quota(snapshot.codex_quota.as_ref())?;
    validate_claude(claude.as_ref())?;
    snapshot.claude_quota = claude.map(|quota| ClaudeQuota {
        state: if quota.state == CodexQuotaState::Stale {
            ClaudeQuotaState::Stale
        } else {
            ClaudeQuotaState::Fresh
        },
        observed_at: quota.observed_at,
        windows: quota.windows,
    });
    decoder.finish()?;

    if wire_tabs.len() != layouts.len() {
        return Err(Error::InvalidSnapshot { field: "columns" });
    }
    for (tab, layout) in wire_tabs.into_iter().zip(layouts) {
        if layout.iter().map(|(panes, _)| panes).sum::<usize>() != tab.panes.len() {
            return Err(Error::InvalidSnapshot { field: "columns" });
        }
        let mut panes = tab.panes.into_iter();
        let mut columns = Vec::with_capacity(layout.len());
        for (count, expanded) in layout {
            let panes: Vec<_> = panes.by_ref().take(count).collect();
            let Some(pane) = panes.get(expanded) else {
                return Err(Error::InvalidSnapshot {
                    field: "expanded pane",
                });
            };
            columns.push(Column {
                expanded: pane.id.clone(),
                panes,
            });
        }
        let tab = Tab {
            id: tab.id,
            directory: tab.directory,
            pending: tab.pending,
            selected_pane: tab.selected_pane,
            selected_popup: tab.selected_popup,
            columns,
            popups: tab.popups,
        };
        validate_columns(&tab)?;
        snapshot.tabs.push(tab);
    }
    Ok(Response::Snapshot(snapshot))
}

fn encode_workspace(encoder: &mut Encoder, snapshot: &Snapshot, tabs: &[WireTab]) {
    encoder.string(&snapshot.active_tab);
    for margin in [
        snapshot.geometry.side_margin,
        snapshot.geometry.vertical_margin,
    ] {
        encoder.bytes.extend_from_slice(&margin.to_le_bytes());
    }
    encoder.count(snapshot.entries.len());
    for entry in &snapshot.entries {
        encoder.string(&entry.id);
        encoder.string(&entry.label);
        encoder.byte(entry.shortcut.modifiers);
        encoder.string(&entry.shortcut.key);
    }
    encoder.count(tabs.len());
    for tab in tabs {
        encoder.string(&tab.id);
        encoder.raw(&tab.directory);
        encoder.byte(u8::from(tab.pending));
        encode_optional(encoder, &tab.selected_pane);
        encode_optional(encoder, &tab.selected_popup);
        encoder.count(tab.panes.len());
        for pane in &tab.panes {
            encoder.string(&pane.id);
            encoder.string(&pane.session);
            encoder.raw(&pane.endpoint);
            encoder.byte(u8::from(pane.live));
        }
        encoder.count(tab.popups.len());
        for popup in &tab.popups {
            encoder.string(&popup.id);
            encoder.string(&popup.entry);
            encoder.string(&popup.session);
            encoder.raw(&popup.endpoint);
        }
    }
}

/// Returns the snapshot without tabs or quotas, and its tabs as carried.
fn decode_workspace(decoder: &mut Decoder<'_>) -> Result<(Snapshot, Vec<WireTab>)> {
    let active_tab = decoder.string("active tab", MAX_ID_BYTES)?;
    let geometry = PopupGeometry {
        side_margin: f32::from_bits(decode_u32(decoder)?),
        vertical_margin: f32::from_bits(decode_u32(decoder)?),
    };
    let count = decoder.count("entries", MAX_ENTRIES)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(PopupEntry {
            id: decoder.string("entry id", MAX_ENTRY_ID_BYTES)?,
            label: decoder.string("entry label", MAX_LABEL_BYTES)?,
            shortcut: Shortcut {
                modifiers: decoder.byte()?,
                key: decoder.string("shortcut key", MAX_KEY_BYTES)?,
            },
        });
    }
    let count = decoder.count("tabs", MAX_TABS)?;
    let mut tabs = Vec::with_capacity(count);
    let mut sessions = 0;
    for _ in 0..count {
        let id = decoder.string("tab id", MAX_ID_BYTES)?;
        let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
        let pending = decode_bool(decoder, "pending tab")?;
        let selected_pane = decode_optional(decoder, "selected pane")?;
        let selected_popup = decode_optional(decoder, "selected popup")?;
        let count = decoder.count("sessions", MAX_SESSIONS - sessions)?;
        sessions += count;
        let mut panes = Vec::with_capacity(count);
        for _ in 0..count {
            panes.push(Pane {
                id: decoder.string("pane id", MAX_ID_BYTES)?,
                session: decoder.string("session id", MAX_ID_BYTES)?,
                endpoint: decoder.raw("endpoint", MAX_ENDPOINT_BYTES)?.to_vec(),
                live: decode_bool(decoder, "liveness")?,
            });
        }
        let count = decoder.count("sessions", (MAX_SESSIONS - sessions).min(MAX_ENTRIES))?;
        sessions += count;
        let mut popups = Vec::with_capacity(count);
        for _ in 0..count {
            popups.push(Popup {
                id: decoder.string("popup id", MAX_ID_BYTES)?,
                entry: decoder.string("entry id", MAX_ENTRY_ID_BYTES)?,
                session: decoder.string("session id", MAX_ID_BYTES)?,
                endpoint: decoder.raw("endpoint", MAX_ENDPOINT_BYTES)?.to_vec(),
            });
        }
        tabs.push(WireTab {
            id,
            directory,
            pending,
            selected_pane,
            selected_popup,
            panes,
            popups,
        });
    }
    let snapshot = Snapshot {
        active_tab,
        geometry,
        entries,
        tabs: Vec::with_capacity(tabs.len()),
        codex_quota: None,
        claude_quota: None,
    };
    Ok((snapshot, tabs))
}

fn entry_id(id: &str) -> Result<()> {
    nonempty("entry id", id, MAX_ENTRY_ID_BYTES)?;
    identity("entry id", id)
}

/// Checks everything but the column layout, over the tabs as carried; the
/// snapshot's own tabs are not read.
fn validate_workspace(snapshot: &Snapshot, tabs: &[WireTab]) -> Result<()> {
    if tabs.is_empty() || tabs.len() > MAX_TABS {
        return Err(Error::InvalidSnapshot { field: "tabs" });
    }
    if snapshot.entries.len() > MAX_ENTRIES {
        return Err(Error::InvalidSnapshot { field: "entries" });
    }
    for margin in [
        snapshot.geometry.side_margin,
        snapshot.geometry.vertical_margin,
    ] {
        if !margin.is_finite() || !(0.0..=128.0).contains(&margin) {
            return Err(Error::InvalidValue {
                field: "popup margin",
            });
        }
    }
    let mut entries = HashSet::new();
    let mut shortcuts = HashSet::new();
    for entry in &snapshot.entries {
        entry_id(&entry.id)?;
        nonempty("entry label", &entry.label, MAX_LABEL_BYTES)?;
        if entry.label.chars().any(char::is_control) {
            return Err(Error::InvalidValue {
                field: "entry label",
            });
        }
        entry.shortcut.validate()?;
        if !entries.insert(entry.id.as_str()) || !shortcuts.insert(&entry.shortcut) {
            return Err(Error::InvalidSnapshot { field: "entries" });
        }
    }
    let mut identities = HashSet::new();
    let mut sessions = HashSet::new();
    let mut endpoints = HashSet::new();
    for tab in tabs {
        identity("tab id", &tab.id)?;
        validate_directory(&tab.directory)?;
        if !identities.insert(tab.id.as_str()) {
            return Err(Error::InvalidSnapshot { field: "tab id" });
        }
        if tab.panes.is_empty() && tab.popups.is_empty() {
            return Err(Error::InvalidSnapshot { field: "empty tab" });
        }
        if tab
            .selected_pane
            .as_ref()
            .is_some_and(|id| !tab.panes.iter().any(|pane| pane.id == *id))
            || (tab.selected_pane.is_none() && !tab.panes.is_empty())
        {
            return Err(Error::InvalidSnapshot {
                field: "selected pane",
            });
        }
        if tab
            .selected_popup
            .as_ref()
            .is_some_and(|id| !tab.popups.iter().any(|popup| popup.id == *id))
        {
            return Err(Error::InvalidSnapshot {
                field: "selected popup",
            });
        }
        if tab.pending
            && (!tab.panes.is_empty()
                || tab.popups.len() != 1
                || tab.popups[0].entry != PROJECT_ENTRY
                || tab.selected_popup.as_ref() != Some(&tab.popups[0].id))
        {
            return Err(Error::InvalidSnapshot {
                field: "pending tab",
            });
        }
        let mut tab_entries = HashSet::new();
        for popup in &tab.popups {
            if !entries.contains(popup.entry.as_str()) || !tab_entries.insert(popup.entry.as_str())
            {
                return Err(Error::InvalidSnapshot {
                    field: "popup entry",
                });
            }
        }
        for (id, session, endpoint) in tab
            .panes
            .iter()
            .map(|p| (&p.id, &p.session, &p.endpoint))
            .chain(tab.popups.iter().map(|p| (&p.id, &p.session, &p.endpoint)))
        {
            identity("surface id", id)?;
            identity("session id", session)?;
            validate_endpoint(endpoint, "endpoint")?;
            if !identities.insert(id.as_str())
                || !sessions.insert(session.as_str())
                || !endpoints.insert(endpoint.as_slice())
            {
                return Err(Error::InvalidSnapshot {
                    field: "surface identity",
                });
            }
            if sessions.len() > MAX_SESSIONS {
                return Err(Error::InvalidSnapshot { field: "sessions" });
            }
        }
    }
    if !tabs.iter().any(|tab| tab.id == snapshot.active_tab) {
        return Err(Error::InvalidSnapshot {
            field: "active tab",
        });
    }
    Ok(())
}

/// Checks the column rules; workspace validation of the flattened panes covers the rest.
fn validate_columns(tab: &Tab) -> Result<()> {
    let invalid = |field| Err(Error::InvalidSnapshot { field });
    if tab.columns.len() > MAX_COLUMNS {
        return invalid("columns");
    }
    for column in &tab.columns {
        if column.panes.is_empty() {
            return invalid("empty column");
        }
        if !column.panes.iter().any(|pane| pane.id == column.expanded) {
            return invalid("expanded pane");
        }
    }
    if tab.selected_pane.as_ref().is_some_and(|selected| {
        !tab.columns
            .iter()
            .any(|column| column.expanded == *selected)
    }) {
        return invalid("selected pane");
    }
    Ok(())
}

/// Claude quotas share the Codex quota wire form.
fn claude_as_codex(quota: &ClaudeQuota) -> CodexQuota {
    CodexQuota {
        state: match quota.state {
            ClaudeQuotaState::Fresh => CodexQuotaState::Fresh,
            ClaudeQuotaState::Stale => CodexQuotaState::Stale,
        },
        observed_at: quota.observed_at,
        windows: quota.windows.clone(),
    }
}

fn validate_quota(quota: Option<&CodexQuota>) -> Result<()> {
    let Some(quota) = quota else {
        return Ok(());
    };
    if quota.observed_at == 0 || quota.windows.len() > MAX_CODEX_QUOTA_WINDOWS {
        return Err(Error::InvalidSnapshot {
            field: "Codex quota",
        });
    }
    let carries_windows = matches!(quota.state, CodexQuotaState::Fresh | CodexQuotaState::Stale);
    if carries_windows == quota.windows.is_empty() {
        return Err(Error::InvalidSnapshot {
            field: "Codex quota state",
        });
    }
    // A window need not carry a reset time: not every provider reports one.
    for window in &quota.windows {
        if window.duration_minutes == 0
            || window.remaining_percent > 100
            || window
                .resets_at
                .is_some_and(|reset| reset <= quota.observed_at)
        {
            return Err(Error::InvalidSnapshot {
                field: "Codex quota window",
            });
        }
    }
    Ok(())
}

fn validate_claude(quota: Option<&CodexQuota>) -> Result<()> {
    validate_quota(quota)?;
    let Some(quota) = quota else { return Ok(()) };
    if !matches!(quota.state, CodexQuotaState::Fresh | CodexQuotaState::Stale)
        || quota.windows.iter().enumerate().any(|(index, window)| {
            !matches!(window.duration_minutes, 300 | 10080)
                || quota.windows[..index]
                    .iter()
                    .any(|previous| previous.duration_minutes == window.duration_minutes)
        })
    {
        return Err(Error::InvalidSnapshot {
            field: "Claude quota",
        });
    }
    Ok(())
}

fn encode_quota(encoder: &mut Encoder, quota: Option<&CodexQuota>) {
    encoder.byte(u8::from(quota.is_some()));
    let Some(quota) = quota else { return };
    encoder.byte(match quota.state {
        CodexQuotaState::Fresh => 0,
        CodexQuotaState::Stale => 1,
        CodexQuotaState::Blocked => 2,
        CodexQuotaState::Unknown => 3,
    });
    encoder
        .bytes
        .extend_from_slice(&quota.observed_at.to_le_bytes());
    encoder.count(quota.windows.len());
    for window in &quota.windows {
        encoder
            .bytes
            .extend_from_slice(&window.duration_minutes.to_le_bytes());
        encoder.byte(window.remaining_percent);
        encoder.byte(u8::from(window.resets_at.is_some()));
        if let Some(reset) = window.resets_at {
            encoder.bytes.extend_from_slice(&reset.to_le_bytes());
        }
    }
}

fn decode_quota(decoder: &mut Decoder<'_>) -> Result<Option<CodexQuota>> {
    if !decode_bool(decoder, "Codex quota presence")? {
        return Ok(None);
    }
    let state = match decoder.byte()? {
        0 => CodexQuotaState::Fresh,
        1 => CodexQuotaState::Stale,
        2 => CodexQuotaState::Blocked,
        3 => CodexQuotaState::Unknown,
        value => {
            return Err(Error::InvalidTag {
                field: "Codex quota state",
                value,
            });
        }
    };
    let observed_at = decode_u64(decoder)?;
    let count = decoder.count("Codex quota windows", MAX_CODEX_QUOTA_WINDOWS)?;
    let mut windows = Vec::with_capacity(count);
    for _ in 0..count {
        windows.push(CodexQuotaWindow {
            duration_minutes: decode_u32(decoder)?,
            remaining_percent: decoder.byte()?,
            resets_at: decode_bool(decoder, "Codex quota reset")?
                .then(|| decode_u64(decoder))
                .transpose()?,
        });
    }
    Ok(Some(CodexQuota {
        state,
        observed_at,
        windows,
    }))
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    let mut payload = Encoder::default();
    let kind = match response {
        LifecycleResponse::Runtime(runtime) => {
            validate_runtime(runtime)?;
            payload.string(&runtime.generation);
            payload.string(&runtime.eon_version);
            payload.number(runtime.workspace_protocol);
            payload.string(&runtime.component_report);
            encode_sessions(&mut payload, &runtime.sessions);
            encode_availability(&mut payload, &runtime.attach);
            encode_availability(&mut payload, &runtime.stop);
            RUNTIME
        }
        LifecycleResponse::Stopped(stopped) => {
            identity("generation", &stopped.generation)?;
            validate_sessions(&stopped.sessions)?;
            payload.string(&stopped.generation);
            encode_sessions(&mut payload, &stopped.sessions);
            STOPPED
        }
        LifecycleResponse::Failure(failure) => {
            encode_failure(&mut payload, failure)?;
            FAILURE
        }
    };
    frame_version(VERSION, kind, payload.bytes)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    let (kind, payload) = unframe_version(bytes, VERSION)?;
    let mut decoder = Decoder::new(payload);
    let response = match kind {
        RUNTIME => {
            let runtime = Runtime {
                generation: decoder.string("generation", MAX_ID_BYTES)?,
                eon_version: decoder.string("Eon version", MAX_VERSION_BYTES)?,
                workspace_protocol: decoder.number()?,
                component_report: decoder.string("component report", MAX_COMPONENT_REPORT_BYTES)?,
                sessions: decode_sessions(&mut decoder)?,
                attach: decode_availability(&mut decoder, "attach reason")?,
                stop: decode_availability(&mut decoder, "stop reason")?,
            };
            validate_runtime(&runtime)?;
            LifecycleResponse::Runtime(runtime)
        }
        STOPPED => {
            let stopped = Stopped {
                generation: decoder.string("generation", MAX_ID_BYTES)?,
                sessions: decode_sessions(&mut decoder)?,
            };
            identity("generation", &stopped.generation)?;
            validate_sessions(&stopped.sessions)?;
            LifecycleResponse::Stopped(stopped)
        }
        FAILURE => LifecycleResponse::Failure(decode_failure(&mut decoder)?),
        value => return Err(Error::InvalidKind { value }),
    };
    decoder.finish()?;
    Ok(response)
}

fn validate_runtime(runtime: &Runtime) -> Result<()> {
    identity("generation", &runtime.generation)?;
    nonempty("Eon version", &runtime.eon_version, MAX_VERSION_BYTES)?;
    if runtime.workspace_protocol == 0 {
        return Err(Error::InvalidValue {
            field: "workspace protocol",
        });
    }
    nonempty(
        "component report",
        &runtime.component_report,
        MAX_COMPONENT_REPORT_BYTES,
    )?;
    validate_sessions(&runtime.sessions)?;
    nonempty("attach reason", &runtime.attach.reason, MAX_DETAIL_BYTES)?;
    nonempty("stop reason", &runtime.stop.reason, MAX_DETAIL_BYTES)
}

/// A generation may report no sessions.
fn validate_sessions(sessions: &[String]) -> Result<()> {
    if sessions.len() > MAX_SESSIONS {
        return Err(Error::InvalidValue { field: "sessions" });
    }
    let mut identities = HashSet::new();
    for session in sessions {
        identity("session id", session)?;
        if !identities.insert(session) {
            return Err(Error::InvalidValue { field: "sessions" });
        }
    }
    Ok(())
}

fn encode_sessions(encoder: &mut Encoder, sessions: &[String]) {
    encoder.count(sessions.len());
    for session in sessions {
        encoder.string(session);
    }
}

fn decode_sessions(decoder: &mut Decoder<'_>) -> Result<Vec<String>> {
    let count = decoder.count("sessions", MAX_SESSIONS)?;
    (0..count)
        .map(|_| decoder.string("session id", MAX_ID_BYTES))
        .collect()
}

fn encode_availability(encoder: &mut Encoder, availability: &Availability) {
    encoder.byte(u8::from(availability.available));
    encoder.string(&availability.reason);
}

fn decode_availability(decoder: &mut Decoder<'_>, field: &'static str) -> Result<Availability> {
    Ok(Availability {
        available: decode_bool(decoder, "availability")?,
        reason: decoder.string(field, MAX_DETAIL_BYTES)?,
    })
}

pub(crate) fn encode_failure(encoder: &mut Encoder, failure: &Failure) -> Result<()> {
    identity("failure code", &failure.code)?;
    nonempty("failure detail", &failure.detail, MAX_DETAIL_BYTES)?;
    encoder.string(&failure.code);
    encoder.string(&failure.detail);
    Ok(())
}

pub(crate) fn decode_failure(decoder: &mut Decoder<'_>) -> Result<Failure> {
    let code = decoder.string("failure code", MAX_ID_BYTES)?;
    identity("failure code", &code)?;
    let detail = decoder.string("failure detail", MAX_DETAIL_BYTES)?;
    nonempty("failure detail", &detail, MAX_DETAIL_BYTES)?;
    Ok(Failure { code, detail })
}

fn encode_optional(encoder: &mut Encoder, value: &Option<String>) {
    encoder.byte(u8::from(value.is_some()));
    if let Some(value) = value {
        encoder.string(value);
    }
}

fn decode_optional(decoder: &mut Decoder<'_>, field: &'static str) -> Result<Option<String>> {
    if decode_bool(decoder, field)? {
        Ok(Some(decoder.string(field, MAX_ID_BYTES)?))
    } else {
        Ok(None)
    }
}

fn decode_bool(decoder: &mut Decoder<'_>, field: &'static str) -> Result<bool> {
    match decoder.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::InvalidValue { field }),
    }
}

fn decode_u32(decoder: &mut Decoder<'_>) -> Result<u32> {
    Ok(u32::from(decoder.number()?) | (u32::from(decoder.number()?) << 16))
}

fn decode_u64(decoder: &mut Decoder<'_>) -> Result<u64> {
    let mut value = 0;
    for shift in [0, 16, 32, 48] {
        value |= u64::from(decoder.number()?) << shift;
    }
    Ok(value)
}

pub(crate) fn validate_directory(directory: &[u8]) -> Result<()> {
    if directory.is_empty() || directory.contains(&0) {
        return Err(Error::InvalidValue { field: "directory" });
    }
    if directory.len() > MAX_DIRECTORY_BYTES {
        return Err(Error::FieldTooLong {
            field: "directory",
            length: directory.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_endpoint(endpoint: &[u8], field: &'static str) -> Result<()> {
    if endpoint.is_empty() {
        return Err(Error::InvalidSnapshot { field });
    }
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(Error::FieldTooLong {
            field,
            length: endpoint.len(),
        });
    }
    Ok(())
}

pub(crate) fn identity(field: &'static str, value: &str) -> Result<()> {
    nonempty(field, value, MAX_ID_BYTES)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidValue { field });
    }
    Ok(())
}

pub(crate) fn nonempty(field: &'static str, value: &str, maximum: usize) -> Result<()> {
    if value.len() > maximum {
        return Err(Error::FieldTooLong {
            field,
            length: value.len(),
        });
    }
    if value.is_empty() {
        return Err(Error::InvalidValue { field });
    }
    Ok(())
}

/// Returns the bounded length declared by a complete EONW header.
pub fn declared_message_len(header: &[u8]) -> Result<usize> {
    if header.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    let payload_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let size = HEADER_BYTES
        .checked_add(payload_len)
        .ok_or(Error::MessageTooLarge { size: usize::MAX })?;
    if size > MAX_MESSAGE_BYTES {
        return Err(Error::MessageTooLarge { size });
    }
    Ok(size)
}

pub(crate) fn frame_version(version: u16, kind: u8, payload: Vec<u8>) -> Result<Vec<u8>> {
    let size = HEADER_BYTES + payload.len();
    if size > MAX_MESSAGE_BYTES {
        return Err(Error::MessageTooLarge { size });
    }
    let mut message = Vec::with_capacity(size);
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&version.to_le_bytes());
    message.push(kind);
    message.push(0);
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&payload);
    Ok(message)
}

pub(crate) fn unframe_version(bytes: &[u8], expected_version: u16) -> Result<(u8, &[u8])> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(Error::MessageTooLarge { size: bytes.len() });
    }
    if bytes.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    if &bytes[..4] != MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != expected_version {
        return Err(Error::UnsupportedVersion { version });
    }
    if bytes[7] != 0 {
        return Err(Error::InvalidFlags { value: bytes[7] });
    }
    let declared = declared_message_len(bytes)?;
    if bytes.len() < declared {
        return Err(Error::Truncated);
    }
    if bytes.len() > declared {
        return Err(Error::TrailingBytes {
            count: bytes.len() - declared,
        });
    }
    Ok((bytes[6], &bytes[HEADER_BYTES..]))
}

#[derive(Default)]
pub(crate) struct Encoder {
    pub(crate) bytes: Vec<u8>,
}

impl Encoder {
    pub(crate) fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn count(&mut self, value: usize) {
        let value = u16::try_from(value).expect("validated EONW count");
        self.number(value);
    }

    pub(crate) fn number(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn string(&mut self, value: &str) {
        self.raw(value.as_bytes());
    }

    pub(crate) fn raw(&mut self, value: &[u8]) {
        let length = u16::try_from(value.len()).expect("validated EONW field length");
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
    }
}

pub(crate) struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(crate) fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn count(&mut self, field: &'static str, maximum: usize) -> Result<usize> {
        let value = self.number()? as usize;
        if value > maximum {
            return Err(Error::InvalidSnapshot { field });
        }
        Ok(value)
    }

    pub(crate) fn number(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn string(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        str::from_utf8(self.raw(field, maximum)?)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidUtf8 { field })
    }

    pub(crate) fn raw(&mut self, field: &'static str, maximum: usize) -> Result<&'a [u8]> {
        let bytes = self.take(2)?;
        let length = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        if length > maximum {
            return Err(Error::FieldTooLong { field, length });
        }
        self.take(length)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    pub(crate) fn rest(self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::TrailingBytes {
                count: self.bytes.len() - self.offset,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn pane(id: &str) -> Pane {
        Pane {
            id: id.into(),
            session: format!("s{id}"),
            endpoint: format!("/{id}").into_bytes(),
            live: true,
        }
    }

    #[test]
    fn sideways_pane_moves_reject_malformed_requests() {
        let overlong = "r".repeat(MAX_ID_BYTES + 1);
        for (side, tag) in [(Side::Left, MOVE_PANE_LEFT), (Side::Right, MOVE_PANE_RIGHT)] {
            for id in ["", "r 1", overlong.as_str()] {
                assert!(
                    encode_request(&Request {
                        id: id.into(),
                        action: Action::MovePaneSideways(side),
                    })
                    .is_err()
                );
                let mut payload = Encoder::default();
                payload.string(id);
                payload.byte(tag);
                let bytes = frame_version(VERSION, REQUEST, payload.bytes).unwrap();
                assert!(decode_request(&bytes).is_err());
            }

            let mut payload = Encoder::default();
            payload.string("r1");
            let truncated = frame_version(VERSION, REQUEST, payload.bytes.clone()).unwrap();
            assert_eq!(decode_request(&truncated), Err(Error::Truncated));
            payload.byte(tag);
            payload.byte(0);
            let trailing = frame_version(VERSION, REQUEST, payload.bytes).unwrap();
            assert_eq!(
                decode_request(&trailing),
                Err(Error::TrailingBytes { count: 1 })
            );
        }
    }

    #[test]
    fn messages_keep_their_wire_bytes() {
        let target = PopupTarget {
            tab: "t1".into(),
            instance: "u1".into(),
        };
        for (action, expected) in [
            (
                Action::Workspace(WorkspaceAction::CloseTab { tab: "t1".into() }),
                "454f4e570c00010009000000020072311202007431",
            ),
            (
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: b"/tmp".to_vec(),
                }),
                "454f4e570c0001000f000000020072310c0200743104002f746d70",
            ),
            (
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "git".into(),
                    expected_instance: Some("u1".into()),
                    intent: InvokeIntent::Focus,
                },
                "454f4e570c000100140000000200723113020074310300676974010200753101",
            ),
            (
                Action::CommitDirectory {
                    target,
                    directory: b"/tmp".to_vec(),
                },
                "454f4e570c000100130000000200723115020074310200753104002f746d70",
            ),
            (Action::CreateColumn, "454f4e570c000100050000000200723116"),
            (
                Action::ClosePane { pane: "p2".into() },
                "454f4e570c00010009000000020072311702007032",
            ),
            (
                Action::MovePaneSideways(Side::Right),
                "454f4e570c000100050000000200723119",
            ),
        ] {
            let request = Request {
                id: "r1".into(),
                action,
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(hex(&bytes), expected, "{request:?}");
            assert_eq!(decode_request(&bytes).unwrap(), request);
        }

        let snapshot = Response::Snapshot(Snapshot {
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "git".into(),
                label: "Git".into(),
                shortcut: Shortcut {
                    modifiers: ALT,
                    key: "KeyG".into(),
                },
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: false,
                selected_pane: Some("p3".into()),
                selected_popup: Some("u1".into()),
                columns: vec![
                    Column {
                        panes: vec![pane("p1"), pane("p2")],
                        expanded: "p2".into(),
                    },
                    Column {
                        panes: vec![pane("p3")],
                        expanded: "p3".into(),
                    },
                ],
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "git".into(),
                    session: "su1".into(),
                    endpoint: b"/u1".to_vec(),
                }],
            }],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Fresh,
                observed_at: 100,
                windows: vec![QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 42,
                    resets_at: Some(500),
                }],
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Stale,
                observed_at: 90,
                windows: vec![QuotaWindow {
                    duration_minutes: 10080,
                    remaining_percent: 7,
                    resets_at: None,
                }],
            }),
        });
        let failure = Response::Failure(Failure {
            code: "busy".into(),
            detail: "no".into(),
        });
        for (response, expected) in [
            (
                snapshot,
                concat!(
                    "454f4e570c008100b20000000100020002000100010000000200743100000041",
                    "000080400100030067697403004769740404004b65794701000200743104002f",
                    "746d700001020070330102007531030002007031030073703103002f70310102",
                    "007032030073703203002f70320102007033030073703303002f703301010002",
                    "0075310300676974030073753103002f75310100640000000000000001002c01",
                    "00002a01f40100000000000001015a000000000000000100602700000700",
                ),
            ),
            (failure, "454f4e570c0082000a00000004006275737902006e6f"),
        ] {
            let bytes = encode_response(&response).unwrap();
            assert_eq!(hex(&bytes), expected, "{response:?}");
            assert_eq!(decode_response(&bytes).unwrap(), response);
        }

        for (response, expected) in [
            (
                LifecycleResponse::Runtime(Runtime {
                    generation: "g1".into(),
                    eon_version: "0.9.0".into(),
                    workspace_protocol: VERSION,
                    component_report: "ok".into(),
                    sessions: vec!["s1".into()],
                    attach: Availability {
                        available: true,
                        reason: "ready".into(),
                    },
                    stop: Availability {
                        available: false,
                        reason: "busy".into(),
                    },
                }),
                concat!(
                    "454f4e570c00830026000000020067310500302e392e300c0002006f6b010002",
                    "007331010500726561647900040062757379",
                ),
            ),
            (
                LifecycleResponse::Stopped(Stopped {
                    generation: "g1".into(),
                    sessions: vec![],
                }),
                "454f4e570c00840006000000020067310000",
            ),
        ] {
            let bytes = encode_lifecycle_response(&response).unwrap();
            assert_eq!(hex(&bytes), expected, "{response:?}");
            assert_eq!(decode_lifecycle_response(&bytes).unwrap(), response);
        }
    }
}
