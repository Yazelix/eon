//! Eon-owned values and bounded EONW v37 codec for local workspace clients.
//!
//! Every message is a 12-byte header (`EONW`, little-endian version, kind,
//! zero flags, little-endian payload length) followed by its payload.
//! Strings and byte fields carry a little-endian `u16` length prefix, except
//! pasted, visible and history terminal text, which carry a `u32` one. Every
//! message is at most 2 MiB, except a `History` response.

use std::{collections::HashSet, fmt, str};

mod subscription;

pub use subscription::{
    Change, ColumnOrder, End, Event, EventKind, Filter, Gap, GapReason, MAX_FILTER_TABS,
    MAX_FILTER_TARGETS, PaneChange, PopupChange, QueueItem, Resume, Subscribed,
    SubscriptionMessage, TabChange, Tabs, WindowSnapshot, WorkspaceChange, WorkspaceEntry,
    decode_subscription_message, encode_subscription_message,
};

pub const VERSION: u16 = 37;
pub const HEADER_BYTES: usize = 12;
const MAGIC: &[u8; 4] = b"EONW";
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_ENDPOINT_BYTES: usize = 4096;
const MAX_VERSION_BYTES: usize = 128;
const MAX_COMPONENT_REPORT_BYTES: usize = 32 * 1024;
pub const MAX_DETAIL_BYTES: usize = 1024;
pub const MAX_TABS: usize = 64;
pub const MAX_WORKSPACES: usize = 32;
pub const MAX_RUNNING_WINDOWS: usize = 32;
pub const MAX_WINDOW_NAME_CHARS: usize = 64;
const MAX_WINDOW_NAME_BYTES: usize = MAX_WINDOW_NAME_CHARS * 4;
pub const MAX_BUSY_TERMINALS: usize = 32;
pub const MAX_WORKSPACE_NAME_CHARS: usize = 64;
const MAX_WORKSPACE_NAME_BYTES: usize = MAX_WORKSPACE_NAME_CHARS * 4;
pub const MAX_SESSIONS: usize = 256;
pub const MAX_DIRECTORY_BYTES: usize = 4096;
pub const MAX_COLUMNS: usize = 4;
pub const MAX_ENTRIES: usize = 32;
pub const MAX_ENTRY_ID_BYTES: usize = 64;
pub const MAX_LABEL_BYTES: usize = 128;
pub const MAX_PANE_LABEL_BYTES: usize = 450;
pub const MAX_PANE_NAME_CHARS: usize = 80;
pub const MAX_PANE_NAME_BYTES: usize = 320;
pub const MAX_KEY_BYTES: usize = 16;
pub const MAX_CODEX_QUOTA_WINDOWS: usize = 2;
/// Claude's model-scoped weekly windows, after its unscoped five-hour and seven-day ones.
pub const MAX_SCOPED_QUOTA_WINDOWS: usize = 4;
pub const MAX_QUOTA_WINDOWS: usize = 2 + MAX_SCOPED_QUOTA_WINDOWS;
pub const MAX_QUOTA_SCOPE_BYTES: usize = 32;
pub const MAX_COMMAND_ARGUMENTS: usize = 256;
pub const MAX_COMMAND_ARGUMENT_BYTES: usize = 32 * 1024;
/// The bounds of a status record and the terminal title in Orbit's
/// ProgramStatus reply (ORB-C15).
const MAX_APP_BYTES: usize = 32;
const MAX_NAME_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 96;
const MAX_STATUS_MESSAGE_BYTES: usize = 256;
/// Orbit keeps at most 256 records per terminal.
const MAX_OTHER_RECORDS: u16 = 255;
/// The bound on a terminal's visible text (EON-C35, ORB-C17).
pub const MAX_TERMINAL_TEXT_BYTES: usize = 1024 * 1024;
/// The bound on a terminal's history text (EON-C35, ORB-C17).
pub const MAX_HISTORY_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// The bound on one paste (EON-C36, ORB-C5).
pub const MAX_PASTE_BYTES: usize = 64 * 1024;
/// The bound on a terminal's title and working directory together, as ORBS
/// bounds its metadata.
pub const MAX_TERMINAL_METADATA_BYTES: usize = 8 * 1024;
/// The longest `[agents] send_enter_delay_ms`.
pub const MAX_ENTER_DELAY_MS: u16 = 5000;
/// The largest encoded terminal target: four strings and the pane or popup tag.
const MAX_TARGET_BYTES: usize = 4 * (2 + MAX_ID_BYTES) + 1;
/// The largest `History` response, the one message over `MAX_MESSAGE_BYTES`:
/// the target, the caller, revision, screen, rows, columns, title,
/// directory, omitted lines, cut bytes and the text with their prefixes.
const MAX_HISTORY_MESSAGE_BYTES: usize = HEADER_BYTES
    + MAX_TARGET_BYTES
    + 1
    + MAX_TARGET_BYTES
    + 8
    + 1
    + 2
    + 2
    + 2
    + 2
    + MAX_TERMINAL_METADATA_BYTES
    + 4
    + 4
    + 4
    + MAX_HISTORY_TEXT_BYTES;
/// Project's entry: it has no popup of its own and invokes its tab's Files pair.
pub const PROJECT_ENTRY: &str = "project";
/// The Files pair's entry, the one popup a pending tab holds (EON-C18).
pub const FILES_ENTRY: &str = "files";
pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;

const REQUEST: u8 = 1;
const SNAPSHOT: u8 = 129;
const FAILURE: u8 = 130;
const RUNTIME: u8 = 131;
const ENDED: u8 = 133;
const TEXT: u8 = 134;
const SENT: u8 = 135;
const HISTORY: u8 = 141;
const EDITING: u8 = 142;

const INVOKE_POPUP: u8 = 19;
const DISMISS_POPUP: u8 = 20;
const CREATE_COLUMN: u8 = 22;
const CLOSE_PANE: u8 = 23;
const RENAME_PANE: u8 = 36;
const LAUNCH_TAB: u8 = 37;
const MOVE_PANE_LEFT: u8 = 24;
const MOVE_PANE_RIGHT: u8 = 25;
const EDIT: u8 = 26;
const CREATE_COMMAND_PANE: u8 = 27;
const TOGGLE_SIDEBAR: u8 = 28;
const FOCUS_ATTENTION: u8 = 29;
const NEXT_ATTENTION: u8 = 30;
const PREVIOUS_ATTENTION: u8 = 31;
const READ_TERMINAL: u8 = 38;
const SEND_TERMINAL: u8 = 39;
const READ_TERMINAL_HISTORY: u8 = 41;
const EDIT_TERMINAL_HISTORY: u8 = 42;
const NO_PROGRESS: u8 = 255;

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
    NewWorkspace {
        name: Option<String>,
        focus: bool,
    },
    SwitchWorkspace {
        workspace: String,
    },
    RenameWorkspace {
        workspace: Option<String>,
        name: String,
    },
    CloseWorkspace {
        workspace: String,
    },
    /// `focus: false` adds the pane without selecting it or dismissing the
    /// tab's popup.
    CreatePane {
        focus: bool,
    },
    FocusId(String),
    Focus(Direction),
    Move(Direction),
    CloseTab {
        tab: String,
    },
    /// A pick on a pending tab (EON-C18); explicit retargeting on an established one.
    SetTabDirectory {
        tab: String,
        directory: Vec<u8>,
    },
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
    /// Add a column right of the selected column with one new pane.
    CreateColumn,
    /// Stop the terminal of one named pane in the addressed workspace.
    ClosePane {
        pane: String,
    },
    RenamePane {
        pane: String,
        name: Option<String>,
    },
    /// Move the selected pane into the neighbouring column on `Side`.
    MovePaneSideways(Side),
    /// Open an absolute file path in the editor in a new pane, as
    /// `CreatePane` adds one; the editor is runtime policy.
    Edit {
        file: Vec<u8>,
    },
    /// Run an exact argv, with no shell, in a new pane, as `CreatePane` adds
    /// one with the same `focus`. The runtime accepts it only from a calling
    /// terminal.
    CreateCommandPane {
        argv: Vec<Vec<u8>>,
        focus: bool,
    },
    /// Start a tab at an absolute directory without a picker. Empty `argv`
    /// starts the default pane; a command requires a calling terminal.
    LaunchTab {
        directory: Vec<u8>,
        argv: Vec<Vec<u8>>,
        focus: bool,
    },
    /// Flip the window's agent sidebar between open and closed (EON-C31).
    ToggleSidebar,
    /// Focus the exact pane id or popup instance `target` of `tab` under
    /// EON-C31's covered-target rule.
    FocusAttention {
        tab: String,
        target: String,
    },
    /// Jump along EON-C31's attention queue.
    NextAttention,
    PreviousAttention,
    /// Read the visible text of the pane or popup `target`, in any workspace
    /// of the window (EON-C35).
    ReadTerminal {
        target: String,
    },
    /// Read the retained history and live area of the pane or popup
    /// `target`, in any workspace of the window (EON-C35).
    ReadTerminalHistory {
        target: String,
    },
    /// Open the retained history and live area of the pane or popup `target`
    /// in the configured editor, in a Scrollback popup over its tab, which
    /// must be the active tab of the shown workspace (EON-C35, EON-C10).
    EditTerminalHistory {
        target: String,
    },
    /// Send `input` to the pane or popup `target`, in any workspace of the
    /// window (EON-C36).
    SendTerminal {
        target: String,
        input: TerminalInput,
        force: bool,
    },
    /// Subscribe to the window's events (EON-C39). The connection then
    /// carries [`SubscriptionMessage`]s instead of one [`Response`].
    Subscribe {
        filter: Filter,
        after: Option<Resume>,
    },
}

/// One input for a terminal (EON-C36).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalInput {
    /// Text that [`validate_paste`] accepts.
    Paste(String),
    Enter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    /// The opaque `EON_TERMINAL` of the terminal that issued the request.
    /// It selects the caller's workspace; pane creation and moves and tab moves
    /// retain their calling-tab, column and pane rules. Close targets are explicit.
    pub caller: Option<String>,
    pub action: Action,
}

/// A terminal's normalized agent attention (EON-C31), most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttentionState {
    NeedsInput,
    Error,
    Finished,
    Working,
    /// A `done` or `idle` record while its terminal runs background
    /// processes (ORB-C15).
    Background,
    Idle,
    /// An Agent popup with no records.
    Unavailable,
}

/// What a program that needs input asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionKind {
    Permission,
    Question,
    Auth,
}

/// The terminal's most urgent program status record, as the program
/// reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionRecord {
    /// Only needs input has a kind.
    pub kind: Option<AttentionKind>,
    /// From 0 through 100.
    pub progress: Option<u8>,
    /// Empty when the program reports none.
    pub app: String,
    /// The terminal's window title, such as Claude Code's session name;
    /// empty when it has none.
    pub name: String,
    pub title: String,
    pub message: String,
    /// The record's fixed change time, in wall-clock Unix milliseconds.
    pub changed_at: u64,
    /// No good status reply for more than 3 s.
    pub stale: bool,
    /// The terminal's other records.
    pub others: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attention {
    pub state: AttentionState,
    /// Present for every state but unavailable.
    pub record: Option<AttentionRecord>,
}

/// One entry of the EON-C31 queue; `target` is the pane id or popup
/// instance that `FocusAttention` takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub workspace: String,
    pub tab: String,
    pub target: String,
    pub attention: Attention,
}

/// The window's agent sidebar visibility (EON-C31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sidebar {
    Open,
    Closed,
    /// `[sidebar] enabled = false`, and eonbase.
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub label: String,
    pub pinned_name: Option<String>,
    pub session: String,
    pub endpoint: Vec<u8>,
    pub live: bool,
    pub attention: Option<Attention>,
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

/// An entry's key: its shortcut, none because the configuration unbound it,
/// or none because the entry `by` holds its `default` shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupKey {
    Bound(Shortcut),
    Unbound,
    Overridden { default: Shortcut, by: String },
}

impl PopupKey {
    /// The shortcut that opens the entry, if any.
    pub fn bound(&self) -> Option<&Shortcut> {
        match self {
            Self::Bound(shortcut) => Some(shortcut),
            Self::Unbound | Self::Overridden { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupEntry {
    pub id: String,
    pub label: String,
    pub shortcut: PopupKey,
}

/// One live instance. A replacement receives a fresh id and endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub id: String,
    pub entry: String,
    pub session: String,
    pub endpoint: Vec<u8>,
    pub attention: Option<Attention>,
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
    /// The most urgent attention among the tab's terminals.
    pub attention_rollup: Option<AttentionState>,
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
    /// The provider's name for what a scoped window limits, such as a model (`Fable`); none for
    /// a window that limits all use.
    pub scope: Option<String>,
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

/// Why a provider's last quota read failed (EON-C27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaFailureReason {
    /// CodexBar could not be started or its output could not be read.
    Start,
    Deadline,
    OutputLimit,
    ExitStatus,
    /// The output is not one result of the expected JSON shape.
    Malformed,
    /// The result names another provider or source, or carries an error.
    ProviderError,
    Estimated,
    /// The observation time is missing, invalid, in the future or over an hour old.
    ObservationTime,
    /// A window is invalid, unsupported, repeated with other values, or one too many.
    InvalidWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaFailure {
    /// Wall-clock seconds at the end of the failed read.
    pub at: u64,
    pub reason: QuotaFailureReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: Option<String>,
    pub tab_count: usize,
    /// The most urgent attention among the workspace's terminals (EON-C33).
    pub attention: Option<AttentionState>,
    /// 0 for the shown workspace, then 1, 2, ... from the most recently shown.
    pub recency: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Live,
    Incompatible,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowEntry {
    /// Null for the default namespace.
    pub window: Option<String>,
    pub generation: String,
    pub name: Option<String>,
    /// Unix milliseconds at supervisor start; recovery restarts the clock.
    pub started_at: Option<u64>,
    pub live_terminals: Option<u16>,
    pub state: WindowState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningWindows {
    pub this: WindowEntry,
    /// Unknown until the first inspection completes, then at most 32 peers.
    pub others: Option<Vec<WindowEntry>>,
    pub omitted: u16,
}

/// A terminal that runs something other than its idle default shell (EON-C11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusyTerminal {
    pub target: TerminalTarget,
    /// What the person sees: a pane's label or a popup's entry label.
    pub name: String,
}

/// A window's busy terminals in window order, at most 32 named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusyTerminals {
    pub terminals: Vec<BusyTerminal>,
    /// Busy terminals past the named ones.
    pub omitted: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The workspace described by tabs/active_tab; caller replies may project a hidden one.
    pub workspace: String,
    pub active_workspace: String,
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_tab: String,
    pub geometry: PopupGeometry,
    pub entries: Vec<PopupEntry>,
    pub tabs: Vec<Tab>,
    pub codex_quota: Option<CodexQuota>,
    pub claude_quota: Option<ClaudeQuota>,
    /// Each provider's last failed read, until a read succeeds (EON-C27).
    pub codex_quota_failure: Option<QuotaFailure>,
    pub claude_quota_failure: Option<QuotaFailure>,
    pub sidebar: Sidebar,
    /// The window-wide queue in EON-C31's total order; consumers never sort it.
    pub attention_queue: Vec<QueueEntry>,
    pub running: Option<RunningWindows>,
    /// Window-wide; None in eonbase, which consumers read as every live
    /// terminal busy.
    pub busy: Option<BusyTerminals>,
}

// Snapshot payloads are already heap-owned; keep the direct value API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Snapshot(Snapshot),
    Failure(Failure),
    /// A tab or pane close stopped the window's last terminal and ended its
    /// generation, so no workspace remains to snapshot.
    Ended(Ended),
    Text(TerminalText),
    Sent(Sent),
    History(TerminalHistory),
    Editing(Editing),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub generation: String,
}

/// Which of a tab's terminals a target is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetId {
    Pane(String),
    Popup(String),
}

/// A pane or popup terminal of the window, named as `eon session` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalTarget {
    pub workspace: String,
    pub tab: String,
    pub id: TargetId,
    pub terminal: String,
}

/// Which screen of a terminal is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Primary,
    Alternate,
}

/// One read of a terminal's live area as lossy plain text (EON-C35, ORB-C17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalText {
    pub target: TerminalTarget,
    /// The calling terminal, when the request named one.
    pub caller: Option<TerminalTarget>,
    pub revision: u64,
    pub screen: Screen,
    pub rows: u16,
    pub columns: u16,
    pub title: String,
    /// The raw working directory.
    pub directory: Vec<u8>,
    /// How many leading rows the text bound left out.
    pub omitted_rows: u32,
    /// One line per display row joined by line feeds, with no other control
    /// character.
    pub text: String,
}

/// One read of a terminal's retained history and live area as lossy plain
/// text (EON-C35, ORB-C17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalHistory {
    pub target: TerminalTarget,
    /// The calling terminal, when the request named one.
    pub caller: Option<TerminalTarget>,
    pub revision: u64,
    pub screen: Screen,
    pub rows: u16,
    pub columns: u16,
    pub title: String,
    /// The raw working directory.
    pub directory: Vec<u8>,
    /// How many leading lines the text bound left out.
    pub omitted_lines: u32,
    /// How many leading bytes the bound cut from the newest line when that
    /// line alone exceeds it, else 0.
    pub cut_bytes: u32,
    /// One line per logical line, oldest first, joined by line feeds, with no
    /// other control character.
    pub text: String,
}

/// The Scrollback popup that opened a terminal's history in the editor
/// (EON-C35, EON-C10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    pub target: TerminalTarget,
    /// The calling terminal, when the request named one.
    pub caller: Option<TerminalTarget>,
    /// The Scrollback popup whose editor shows the snapshot.
    pub popup: TerminalTarget,
}

/// An input the target's terminal took (EON-C36).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    pub target: TerminalTarget,
    /// The calling terminal, when the request named one.
    pub caller: Option<TerminalTarget>,
    /// Whether the paste was bracketed; false for Enter.
    pub bracketed: bool,
    /// The window's `[agents] send_enter_delay_ms`: how long a sender waits
    /// after a paste before its Enter.
    pub enter_delay_ms: u16,
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
    pub name: Option<String>,
    pub started_at: u64,
    pub eon_version: String,
    pub workspace_protocol: u16,
    pub component_report: String,
    pub sessions: Vec<String>,
    pub attach: Availability,
    pub stop: Availability,
    /// The snapshot's list, without the caller of an inspection that names
    /// one; None in eonbase.
    pub busy: Option<BusyTerminals>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleResponse {
    Runtime(Runtime),
    Failure(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    InvalidMagic,
    UnsupportedVersion { version: u16 },
    InvalidFlags { value: u8 },
    MessageTooLarge { size: usize, maximum: usize },
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
            Self::MessageTooLarge { size, maximum } => write!(
                formatter,
                "EONW message is {size} bytes; maximum is {maximum}"
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

/// The popup of `tab` that invoking `entry` acts on: Project invokes the tab's
/// Files pair, and every other entry its own popup.
pub fn entry_popup<'a>(tab: &'a Tab, entry: &str) -> Option<&'a Popup> {
    let entry = if entry == PROJECT_ENTRY {
        FILES_ENTRY
    } else {
        entry
    };
    tab.popups.iter().find(|popup| popup.entry == entry)
}

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
                        entry_popup(tab, entry).map(|popup| &popup.id) == expected_instance.as_ref()
                    })
        }
        Action::DismissPopup(target) => snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == target.tab)
            .is_some_and(|tab| tab.popups.iter().any(|popup| popup.id == target.instance)),
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
    if let Some(caller) = &request.caller {
        identity("caller", caller)?;
    }
    let mut payload = Encoder::default();
    payload.string(&request.id);
    encode_optional(&mut payload, &request.caller);
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
        Action::CreateColumn => payload.byte(CREATE_COLUMN),
        Action::ClosePane { pane } => {
            identity("pane id", pane)?;
            payload.byte(CLOSE_PANE);
            payload.string(pane);
        }
        Action::RenamePane { pane, name } => {
            identity("pane id", pane)?;
            if let Some(name) = name {
                validate_pane_name(name)?;
            }
            payload.byte(RENAME_PANE);
            payload.string(pane);
            encode_optional(&mut payload, name);
        }
        Action::MovePaneSideways(side) => payload.byte(match side {
            Side::Left => MOVE_PANE_LEFT,
            Side::Right => MOVE_PANE_RIGHT,
        }),
        Action::Edit { file } => {
            validate_file(file)?;
            payload.byte(EDIT);
            payload.raw(file);
        }
        Action::CreateCommandPane { argv, focus } => {
            validate_command(argv)?;
            payload.byte(CREATE_COMMAND_PANE);
            payload.byte(u8::from(*focus));
            payload.count(argv.len());
            for argument in argv {
                payload.raw(argument);
            }
        }
        Action::LaunchTab {
            directory,
            argv,
            focus,
        } => {
            validate_directory(directory)?;
            if !argv.is_empty() {
                validate_command(argv)?;
            }
            payload.byte(LAUNCH_TAB);
            payload.raw(directory);
            payload.count(argv.len());
            for argument in argv {
                payload.raw(argument);
            }
            payload.byte(u8::from(*focus));
        }
        Action::ToggleSidebar => payload.byte(TOGGLE_SIDEBAR),
        Action::FocusAttention { tab, target } => {
            identity("tab id", tab)?;
            identity("attention target", target)?;
            payload.byte(FOCUS_ATTENTION);
            payload.string(tab);
            payload.string(target);
        }
        Action::NextAttention => payload.byte(NEXT_ATTENTION),
        Action::PreviousAttention => payload.byte(PREVIOUS_ATTENTION),
        Action::ReadTerminal { target } => {
            identity("terminal target", target)?;
            payload.byte(READ_TERMINAL);
            payload.string(target);
        }
        Action::ReadTerminalHistory { target } => {
            identity("terminal target", target)?;
            payload.byte(READ_TERMINAL_HISTORY);
            payload.string(target);
        }
        Action::EditTerminalHistory { target } => {
            identity("terminal target", target)?;
            payload.byte(EDIT_TERMINAL_HISTORY);
            payload.string(target);
        }
        Action::SendTerminal {
            target,
            input,
            force,
        } => {
            identity("terminal target", target)?;
            payload.byte(SEND_TERMINAL);
            payload.string(target);
            payload.byte(u8::from(*force));
            match input {
                TerminalInput::Paste(text) => {
                    validate_paste(text)?;
                    payload.byte(0);
                    payload.long_raw(text.as_bytes());
                }
                TerminalInput::Enter => payload.byte(1),
            }
        }
        Action::Subscribe { filter, after } => {
            subscription::validate_subscribe(filter, after.as_ref())?;
            subscription::encode_subscribe(&mut payload, filter, after.as_ref());
        }
    }
    frame(REQUEST, payload.bytes)
}

fn encode_workspace_action(payload: &mut Encoder, action: &WorkspaceAction) -> Result<()> {
    validate_workspace_request(action)?;
    match action {
        WorkspaceAction::NewWorkspace { name, focus } => {
            payload.byte(32);
            encode_optional(payload, name);
            payload.byte(u8::from(*focus));
        }
        WorkspaceAction::SwitchWorkspace { workspace } => {
            payload.byte(33);
            payload.string(workspace);
        }
        WorkspaceAction::RenameWorkspace { workspace, name } => {
            payload.byte(34);
            encode_optional(payload, workspace);
            payload.string(name);
        }
        WorkspaceAction::CloseWorkspace { workspace } => {
            payload.byte(35);
            payload.string(workspace);
        }
        WorkspaceAction::Inspect => payload.byte(0),
        WorkspaceAction::CreateTab => payload.byte(1),
        WorkspaceAction::CreatePane { focus } => {
            payload.byte(2);
            payload.byte(u8::from(*focus));
        }
        WorkspaceAction::FocusId(id) => {
            nonempty("focus id", id, MAX_ID_BYTES)?;
            payload.byte(3);
            payload.string(id);
        }
        WorkspaceAction::Focus(direction) => payload.byte(4 + direction_offset(*direction)),
        WorkspaceAction::InspectRuntime => payload.byte(8),
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
    let (kind, payload) = unframe(bytes)?;
    if kind != REQUEST {
        return Err(Error::InvalidKind { value: kind });
    }
    let mut decoder = Decoder::new(payload);
    let id = decoder.string("request id", MAX_ID_BYTES)?;
    let caller = decode_optional(&mut decoder, "caller")?;
    if let Some(caller) = &caller {
        identity("caller", caller)?;
    }
    let tag = decoder.byte()?;
    let action = match tag {
        CREATE_COLUMN => Some(Action::CreateColumn),
        CLOSE_PANE => {
            let pane = decoder.string("pane id", MAX_ID_BYTES)?;
            Some(Action::ClosePane { pane })
        }
        RENAME_PANE => Some(Action::RenamePane {
            pane: decoder.string("pane id", MAX_ID_BYTES)?,
            name: decode_pane_name(&mut decoder)?,
        }),
        MOVE_PANE_LEFT => Some(Action::MovePaneSideways(Side::Left)),
        MOVE_PANE_RIGHT => Some(Action::MovePaneSideways(Side::Right)),
        EDIT => Some(Action::Edit {
            file: decoder.raw("file", MAX_DIRECTORY_BYTES)?.to_vec(),
        }),
        CREATE_COMMAND_PANE => {
            let focus = decode_bool(&mut decoder, "pane focus")?;
            let count = decoder.count("command", MAX_COMMAND_ARGUMENTS)?;
            let argv = (0..count)
                .map(|_| {
                    Ok(decoder
                        .raw("command argument", MAX_COMMAND_ARGUMENT_BYTES)?
                        .to_vec())
                })
                .collect::<Result<_>>()?;
            Some(Action::CreateCommandPane { argv, focus })
        }
        LAUNCH_TAB => {
            let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
            let count = decoder.count("command", MAX_COMMAND_ARGUMENTS)?;
            let argv = (0..count)
                .map(|_| {
                    Ok(decoder
                        .raw("command argument", MAX_COMMAND_ARGUMENT_BYTES)?
                        .to_vec())
                })
                .collect::<Result<_>>()?;
            let focus = decode_bool(&mut decoder, "tab focus")?;
            Some(Action::LaunchTab {
                directory,
                argv,
                focus,
            })
        }
        TOGGLE_SIDEBAR => Some(Action::ToggleSidebar),
        FOCUS_ATTENTION => Some(Action::FocusAttention {
            tab: decoder.string("tab id", MAX_ID_BYTES)?,
            target: decoder.string("attention target", MAX_ID_BYTES)?,
        }),
        NEXT_ATTENTION => Some(Action::NextAttention),
        PREVIOUS_ATTENTION => Some(Action::PreviousAttention),
        READ_TERMINAL => Some(Action::ReadTerminal {
            target: decoder.string("terminal target", MAX_ID_BYTES)?,
        }),
        READ_TERMINAL_HISTORY => Some(Action::ReadTerminalHistory {
            target: decoder.string("terminal target", MAX_ID_BYTES)?,
        }),
        EDIT_TERMINAL_HISTORY => Some(Action::EditTerminalHistory {
            target: decoder.string("terminal target", MAX_ID_BYTES)?,
        }),
        SEND_TERMINAL => Some(Action::SendTerminal {
            target: decoder.string("terminal target", MAX_ID_BYTES)?,
            force: decode_bool(&mut decoder, "force")?,
            input: match decoder.byte()? {
                0 => TerminalInput::Paste(decoder.long_string("paste", MAX_PASTE_BYTES)?),
                1 => TerminalInput::Enter,
                value => {
                    return Err(Error::InvalidTag {
                        field: "terminal input",
                        value,
                    });
                }
            },
        }),
        subscription::SUBSCRIBE => {
            let (filter, after) = subscription::decode_subscribe(&mut decoder)?;
            Some(Action::Subscribe { filter, after })
        }
        _ => None,
    };
    if let Some(action) = action {
        decoder.finish()?;
        identity("request id", &id)?;
        match &action {
            Action::ClosePane { pane } => identity("pane id", pane)?,
            Action::RenamePane { pane, name } => {
                identity("pane id", pane)?;
                if let Some(name) = name {
                    validate_pane_name(name)?;
                }
            }
            Action::Edit { file } => validate_file(file)?,
            Action::CreateCommandPane { argv, .. } => validate_command(argv)?,
            Action::LaunchTab {
                directory, argv, ..
            } => {
                validate_directory(directory)?;
                if !argv.is_empty() {
                    validate_command(argv)?;
                }
            }
            Action::FocusAttention { tab, target } => {
                identity("tab id", tab)?;
                identity("attention target", target)?;
            }
            Action::ReadTerminal { target }
            | Action::ReadTerminalHistory { target }
            | Action::EditTerminalHistory { target } => identity("terminal target", target)?,
            Action::SendTerminal { target, input, .. } => {
                identity("terminal target", target)?;
                if let TerminalInput::Paste(text) = input {
                    validate_paste(text)?;
                }
            }
            Action::Subscribe { filter, after } => {
                subscription::validate_subscribe(filter, after.as_ref())?
            }
            _ => (),
        }
        return Ok(Request { id, caller, action });
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
        DISMISS_POPUP => Action::DismissPopup(PopupTarget {
            tab: decoder.string("tab id", MAX_ID_BYTES)?,
            instance: decoder.string("popup id", MAX_ID_BYTES)?,
        }),
        tag => Action::Workspace(decode_workspace_action(&mut decoder, tag)?),
    };
    decoder.finish()?;
    validate_popup_action(&action)?;
    Ok(Request { id, caller, action })
}

fn decode_workspace_action(decoder: &mut Decoder<'_>, tag: u8) -> Result<WorkspaceAction> {
    let direction = |offset| match offset {
        0 => Direction::Left,
        1 => Direction::Right,
        2 => Direction::Up,
        _ => Direction::Down,
    };
    let action = match tag {
        32 => WorkspaceAction::NewWorkspace {
            name: decode_workspace_optional(decoder)?,
            focus: decode_bool(decoder, "workspace focus")?,
        },
        33 => WorkspaceAction::SwitchWorkspace {
            workspace: decoder.string("workspace", MAX_WORKSPACE_NAME_BYTES)?,
        },
        34 => WorkspaceAction::RenameWorkspace {
            workspace: decode_workspace_optional(decoder)?,
            name: decoder.string("workspace name", MAX_WORKSPACE_NAME_BYTES)?,
        },
        35 => WorkspaceAction::CloseWorkspace {
            workspace: decoder.string("workspace", MAX_WORKSPACE_NAME_BYTES)?,
        },
        0 => WorkspaceAction::Inspect,
        1 => WorkspaceAction::CreateTab,
        2 => WorkspaceAction::CreatePane {
            focus: decode_bool(decoder, "pane focus")?,
        },
        3 => {
            let id = decoder.string("focus id", MAX_ID_BYTES)?;
            nonempty("focus id", &id, MAX_ID_BYTES)?;
            WorkspaceAction::FocusId(id)
        }
        4..=7 => WorkspaceAction::Focus(direction(tag - 4)),
        8 => WorkspaceAction::InspectRuntime,
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
    };
    validate_workspace_request(&action)?;
    Ok(action)
}

/// Display characters accepted for pane labels and user pins, matching ORB-C15's cleaner.
pub fn pane_label_character(character: char) -> bool {
    !character.is_control()
        && !matches!(character,
        '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}'
        | '\u{0890}'..='\u{0891}' | '\u{08e2}' | '\u{17b4}'..='\u{17b5}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
        | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}'
        | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// Validate a bounded plain-text pin; Runtime trims it and clears an empty result.
pub fn validate_pane_name(name: &str) -> Result<()> {
    if name.len() > MAX_PANE_NAME_BYTES {
        return Err(Error::FieldTooLong {
            field: "pane name",
            length: name.len(),
        });
    }
    if name.chars().count() > MAX_PANE_NAME_CHARS || !name.chars().all(pane_label_character) {
        return Err(Error::InvalidValue { field: "pane name" });
    }
    Ok(())
}

fn decode_pane_name(decoder: &mut Decoder<'_>) -> Result<Option<String>> {
    Ok(if decode_bool(decoder, "pane name")? {
        Some(decoder.string("pane name", MAX_PANE_NAME_BYTES)?)
    } else {
        None
    })
}

fn validate_pane_label(pane: &Pane) -> Result<()> {
    if pane.label.len() > MAX_PANE_LABEL_BYTES {
        return Err(Error::FieldTooLong {
            field: "pane label",
            length: pane.label.len(),
        });
    }
    if let Some(pin) = &pane.pinned_name {
        validate_pane_name(pin)?;
        if pin.is_empty() || pin != pin.trim() {
            return Err(Error::InvalidValue { field: "pane name" });
        }
    }
    if pane.label == pane.id
        || ["offline", "connecting", "unavailable"]
            .iter()
            .any(|state| pane.label == format!("{} {state}", pane.id))
    {
        return Ok(());
    }
    let Some(body) = pane
        .label
        .strip_prefix(&pane.id)
        .and_then(|rest| rest.strip_prefix("  "))
    else {
        return Err(Error::InvalidValue {
            field: "pane label",
        });
    };
    validate_pane_name(body).map_err(|_| Error::InvalidValue {
        field: "pane label",
    })?;
    if body.is_empty() {
        return Err(Error::InvalidValue {
            field: "pane label",
        });
    }
    Ok(())
}

/// Validate a workspace display name or selector, in Unicode characters rather than bytes.
pub fn validate_workspace_name(name: &str) -> Result<()> {
    nonempty("workspace name", name, MAX_WORKSPACE_NAME_BYTES)?;
    if name.chars().count() > MAX_WORKSPACE_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(Error::InvalidValue {
            field: "workspace name",
        });
    }
    Ok(())
}

fn validate_workspace_request(action: &WorkspaceAction) -> Result<()> {
    match action {
        WorkspaceAction::NewWorkspace {
            name: Some(name), ..
        } => validate_workspace_name(name)?,
        WorkspaceAction::SwitchWorkspace { workspace }
        | WorkspaceAction::CloseWorkspace { workspace } => validate_workspace_name(workspace)?,
        WorkspaceAction::RenameWorkspace { workspace, name } => {
            if let Some(workspace) = workspace {
                validate_workspace_name(workspace)?;
            }
            if !name.is_empty() {
                validate_workspace_name(name)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn decode_workspace_optional(decoder: &mut Decoder<'_>) -> Result<Option<String>> {
    Ok(if decode_bool(decoder, "workspace name")? {
        Some(decoder.string("workspace name", MAX_WORKSPACE_NAME_BYTES)?)
    } else {
        None
    })
}

fn validate_popup_action(action: &Action) -> Result<()> {
    match action {
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
        Action::DismissPopup(target) => {
            identity("tab id", &target.tab)?;
            identity("popup id", &target.instance)?;
        }
        Action::Workspace(_)
        | Action::CreateColumn
        | Action::ClosePane { .. }
        | Action::RenamePane { .. }
        | Action::MovePaneSideways(_)
        | Action::Edit { .. }
        | Action::CreateCommandPane { .. }
        | Action::LaunchTab { .. }
        | Action::ToggleSidebar
        | Action::FocusAttention { .. }
        | Action::NextAttention
        | Action::PreviousAttention
        | Action::ReadTerminal { .. }
        | Action::ReadTerminalHistory { .. }
        | Action::EditTerminalHistory { .. }
        | Action::SendTerminal { .. }
        | Action::Subscribe { .. } => (),
    }
    Ok(())
}

/// Accepts a paste for [`TerminalInput::Paste`] (EON-C36): nonempty text of
/// at most [`MAX_PASTE_BYTES`] with no control character other than tab, line
/// feed and carriage return. Too long text is `FieldTooLong`; anything else
/// it refuses is `InvalidValue`.
pub fn validate_paste(text: &str) -> Result<()> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(Error::FieldTooLong {
            field: "paste",
            length: text.len(),
        });
    }
    if text.is_empty()
        || text
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
    {
        return Err(Error::InvalidValue { field: "paste" });
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
    attention_rollup: Option<AttentionState>,
}

impl From<&Tab> for WireTab {
    fn from(tab: &Tab) -> Self {
        Self {
            id: tab.id.clone(),
            directory: tab.directory.clone(),
            pending: tab.pending,
            selected_pane: tab.selected_pane.clone(),
            selected_popup: tab.selected_popup.clone(),
            panes: tab.panes().cloned().collect(),
            popups: tab.popups.clone(),
            attention_rollup: tab.attention_rollup,
        }
    }
}

/// A snapshot payload is the column layout, the workspace with flattened
/// panes, then the Codex and Claude quotas and their last failures, sidebar,
/// workspace summaries, the window-wide attention queue and running windows.
pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    let mut payload = Encoder::default();
    let kind = match response {
        Response::Snapshot(snapshot) => {
            let tabs: Vec<_> = snapshot.tabs.iter().map(WireTab::from).collect();
            let claude = snapshot.claude_quota.as_ref().map(claude_as_codex);
            validate_workspace(snapshot, &tabs)?;
            validate_running(snapshot.running.as_ref())?;
            validate_busy(snapshot.busy.as_ref())?;
            validate_codex(snapshot.codex_quota.as_ref())?;
            validate_claude(claude.as_ref())?;
            validate_failure(snapshot.codex_quota_failure)?;
            validate_failure(snapshot.claude_quota_failure)?;
            payload.count(snapshot.tabs.len());
            for tab in &snapshot.tabs {
                encode_layout(&mut payload, tab)?;
            }
            encode_workspace(&mut payload, snapshot, &tabs);
            encode_quota(&mut payload, snapshot.codex_quota.as_ref());
            encode_quota(&mut payload, claude.as_ref());
            encode_failure_record(&mut payload, snapshot.codex_quota_failure);
            encode_failure_record(&mut payload, snapshot.claude_quota_failure);
            encode_sidebar(&mut payload, snapshot.sidebar);
            encode_workspace_set(&mut payload, snapshot);
            encode_attention_queue(&mut payload, &snapshot.attention_queue);
            encode_running(&mut payload, snapshot.running.as_ref());
            encode_busy(&mut payload, snapshot.busy.as_ref());
            SNAPSHOT
        }
        Response::Failure(failure) => {
            encode_failure(&mut payload, failure)?;
            FAILURE
        }
        Response::Ended(ended) => {
            identity("generation", &ended.generation)?;
            payload.string(&ended.generation);
            ENDED
        }
        Response::Text(text) => {
            validate_terminal_text(text)?;
            encode_target(&mut payload, &text.target);
            encode_caller(&mut payload, text.caller.as_ref());
            payload
                .bytes
                .extend_from_slice(&text.revision.to_le_bytes());
            payload.byte(match text.screen {
                Screen::Primary => 0,
                Screen::Alternate => 1,
            });
            payload.number(text.rows);
            payload.number(text.columns);
            payload.string(&text.title);
            payload.raw(&text.directory);
            payload
                .bytes
                .extend_from_slice(&text.omitted_rows.to_le_bytes());
            payload.long_raw(text.text.as_bytes());
            TEXT
        }
        Response::History(history) => {
            validate_terminal_history(history)?;
            encode_target(&mut payload, &history.target);
            encode_caller(&mut payload, history.caller.as_ref());
            payload
                .bytes
                .extend_from_slice(&history.revision.to_le_bytes());
            payload.byte(match history.screen {
                Screen::Primary => 0,
                Screen::Alternate => 1,
            });
            payload.number(history.rows);
            payload.number(history.columns);
            payload.string(&history.title);
            payload.raw(&history.directory);
            payload
                .bytes
                .extend_from_slice(&history.omitted_lines.to_le_bytes());
            payload
                .bytes
                .extend_from_slice(&history.cut_bytes.to_le_bytes());
            payload.long_raw(history.text.as_bytes());
            HISTORY
        }
        Response::Sent(sent) => {
            validate_target(&sent.target)?;
            if let Some(caller) = &sent.caller {
                validate_target(caller)?;
            }
            validate_enter_delay(sent.enter_delay_ms)?;
            encode_target(&mut payload, &sent.target);
            encode_caller(&mut payload, sent.caller.as_ref());
            payload.byte(u8::from(sent.bracketed));
            payload.number(sent.enter_delay_ms);
            SENT
        }
        Response::Editing(editing) => {
            validate_editing(editing)?;
            encode_target(&mut payload, &editing.target);
            encode_caller(&mut payload, editing.caller.as_ref());
            encode_target(&mut payload, &editing.popup);
            EDITING
        }
    };
    frame(kind, payload.bytes)
}

fn validate_target(target: &TerminalTarget) -> Result<()> {
    identity("target workspace", &target.workspace)?;
    identity("target tab", &target.tab)?;
    match &target.id {
        TargetId::Pane(id) => identity("target pane", id)?,
        TargetId::Popup(id) => identity("target popup", id)?,
    }
    identity("target terminal", &target.terminal)
}

fn encode_target(encoder: &mut Encoder, target: &TerminalTarget) {
    encoder.string(&target.workspace);
    encoder.string(&target.tab);
    let (tag, id) = match &target.id {
        TargetId::Pane(id) => (0, id),
        TargetId::Popup(id) => (1, id),
    };
    encoder.byte(tag);
    encoder.string(id);
    encoder.string(&target.terminal);
}

/// Accepts an [`Editing`] reply whose popup is a popup.
fn validate_editing(editing: &Editing) -> Result<()> {
    validate_target(&editing.target)?;
    if let Some(caller) = &editing.caller {
        validate_target(caller)?;
    }
    validate_target(&editing.popup)?;
    if !matches!(editing.popup.id, TargetId::Popup(_)) {
        return Err(Error::InvalidValue {
            field: "editing popup",
        });
    }
    Ok(())
}

fn decode_target(decoder: &mut Decoder<'_>) -> Result<TerminalTarget> {
    let workspace = decoder.string("target workspace", MAX_ID_BYTES)?;
    let tab = decoder.string("target tab", MAX_ID_BYTES)?;
    let id = match decoder.byte()? {
        0 => TargetId::Pane(decoder.string("target pane", MAX_ID_BYTES)?),
        1 => TargetId::Popup(decoder.string("target popup", MAX_ID_BYTES)?),
        value => {
            return Err(Error::InvalidTag {
                field: "target",
                value,
            });
        }
    };
    let target = TerminalTarget {
        workspace,
        tab,
        id,
        terminal: decoder.string("target terminal", MAX_ID_BYTES)?,
    };
    validate_target(&target)?;
    Ok(target)
}

fn encode_caller(encoder: &mut Encoder, caller: Option<&TerminalTarget>) {
    encoder.byte(u8::from(caller.is_some()));
    if let Some(caller) = caller {
        encode_target(encoder, caller);
    }
}

fn decode_caller(decoder: &mut Decoder<'_>) -> Result<Option<TerminalTarget>> {
    decode_bool(decoder, "caller")?
        .then(|| decode_target(decoder))
        .transpose()
}

fn validate_terminal_text(text: &TerminalText) -> Result<()> {
    validate_terminal_read(
        &text.target,
        text.caller.as_ref(),
        text.title.len() + text.directory.len(),
        ("terminal text", &text.text, MAX_TERMINAL_TEXT_BYTES),
    )
}

fn validate_terminal_history(history: &TerminalHistory) -> Result<()> {
    validate_terminal_read(
        &history.target,
        history.caller.as_ref(),
        history.title.len() + history.directory.len(),
        ("history text", &history.text, MAX_HISTORY_TEXT_BYTES),
    )
}

fn validate_terminal_read(
    target: &TerminalTarget,
    caller: Option<&TerminalTarget>,
    metadata: usize,
    (field, text, bound): (&'static str, &str, usize),
) -> Result<()> {
    validate_target(target)?;
    if let Some(caller) = caller {
        validate_target(caller)?;
    }
    if metadata > MAX_TERMINAL_METADATA_BYTES {
        return Err(Error::FieldTooLong {
            field: "terminal title and directory",
            length: metadata,
        });
    }
    if text.len() > bound {
        return Err(Error::FieldTooLong {
            field,
            length: text.len(),
        });
    }
    if text
        .chars()
        .any(|character| character.is_control() && character != '\n')
    {
        return Err(Error::InvalidValue { field });
    }
    Ok(())
}

fn decode_screen(decoder: &mut Decoder<'_>) -> Result<Screen> {
    match decoder.byte()? {
        0 => Ok(Screen::Primary),
        1 => Ok(Screen::Alternate),
        value => Err(Error::InvalidTag {
            field: "screen",
            value,
        }),
    }
}

fn validate_enter_delay(delay: u16) -> Result<()> {
    if delay > MAX_ENTER_DELAY_MS {
        return Err(Error::InvalidValue {
            field: "enter delay",
        });
    }
    Ok(())
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    let (kind, payload) = unframe(bytes)?;
    let mut decoder = Decoder::new(payload);
    if kind == FAILURE {
        let failure = decode_failure(&mut decoder)?;
        decoder.finish()?;
        return Ok(Response::Failure(failure));
    }
    if kind == ENDED {
        let ended = Ended {
            generation: decoder.string("generation", MAX_ID_BYTES)?,
        };
        identity("generation", &ended.generation)?;
        decoder.finish()?;
        return Ok(Response::Ended(ended));
    }
    if kind == TEXT {
        let text = TerminalText {
            target: decode_target(&mut decoder)?,
            caller: decode_caller(&mut decoder)?,
            revision: decode_u64(&mut decoder)?,
            screen: decode_screen(&mut decoder)?,
            rows: decoder.number()?,
            columns: decoder.number()?,
            title: decoder.string("terminal title", MAX_TERMINAL_METADATA_BYTES)?,
            directory: decoder
                .raw("terminal directory", MAX_TERMINAL_METADATA_BYTES)?
                .to_vec(),
            omitted_rows: decode_u32(&mut decoder)?,
            text: decoder.long_string("terminal text", MAX_TERMINAL_TEXT_BYTES)?,
        };
        decoder.finish()?;
        validate_terminal_text(&text)?;
        return Ok(Response::Text(text));
    }
    if kind == HISTORY {
        let history = TerminalHistory {
            target: decode_target(&mut decoder)?,
            caller: decode_caller(&mut decoder)?,
            revision: decode_u64(&mut decoder)?,
            screen: decode_screen(&mut decoder)?,
            rows: decoder.number()?,
            columns: decoder.number()?,
            title: decoder.string("terminal title", MAX_TERMINAL_METADATA_BYTES)?,
            directory: decoder
                .raw("terminal directory", MAX_TERMINAL_METADATA_BYTES)?
                .to_vec(),
            omitted_lines: decode_u32(&mut decoder)?,
            cut_bytes: decode_u32(&mut decoder)?,
            text: decoder.long_string("history text", MAX_HISTORY_TEXT_BYTES)?,
        };
        decoder.finish()?;
        validate_terminal_history(&history)?;
        return Ok(Response::History(history));
    }
    if kind == SENT {
        let sent = Sent {
            target: decode_target(&mut decoder)?,
            caller: decode_caller(&mut decoder)?,
            bracketed: decode_bool(&mut decoder, "bracketed")?,
            enter_delay_ms: decoder.number()?,
        };
        decoder.finish()?;
        validate_enter_delay(sent.enter_delay_ms)?;
        return Ok(Response::Sent(sent));
    }
    if kind == EDITING {
        let editing = Editing {
            target: decode_target(&mut decoder)?,
            caller: decode_caller(&mut decoder)?,
            popup: decode_target(&mut decoder)?,
        };
        decoder.finish()?;
        validate_editing(&editing)?;
        return Ok(Response::Editing(editing));
    }
    if kind != SNAPSHOT {
        return Err(Error::InvalidKind { value: kind });
    }
    let tab_count = decoder.count("tabs", MAX_TABS)?;
    let layouts = (0..tab_count)
        .map(|_| decode_layout(&mut decoder))
        .collect::<Result<Vec<_>>>()?;
    let (mut snapshot, wire_tabs) = decode_workspace(&mut decoder)?;
    snapshot.codex_quota = decode_quota(&mut decoder)?;
    let claude = decode_quota(&mut decoder)?;
    snapshot.codex_quota_failure = decode_failure_record(&mut decoder)?;
    snapshot.claude_quota_failure = decode_failure_record(&mut decoder)?;
    snapshot.sidebar = match decoder.byte()? {
        0 => Sidebar::Open,
        1 => Sidebar::Closed,
        2 => Sidebar::Off,
        value => {
            return Err(Error::InvalidTag {
                field: "sidebar",
                value,
            });
        }
    };
    snapshot.workspace = decoder.string("workspace", MAX_ID_BYTES)?;
    snapshot.active_workspace = decoder.string("active workspace", MAX_ID_BYTES)?;
    let count = decoder.count("workspaces", MAX_WORKSPACES)?;
    for _ in 0..count {
        snapshot.workspaces.push(WorkspaceInfo {
            id: decoder.string("workspace id", MAX_ID_BYTES)?,
            name: decode_workspace_optional(&mut decoder)?,
            tab_count: decoder.count("workspace tabs", MAX_TABS)?,
            attention: decode_state(&mut decoder, "workspace attention")?,
            recency: decoder.byte()?,
        });
    }
    let count = decoder.count("attention queue", MAX_SESSIONS)?;
    for _ in 0..count {
        snapshot.attention_queue.push(QueueEntry {
            workspace: decoder.string("queue workspace", MAX_ID_BYTES)?,
            tab: decoder.string("queue tab", MAX_ID_BYTES)?,
            target: decoder.string("queue target", MAX_ID_BYTES)?,
            attention: decode_attention(&mut decoder)?.ok_or(Error::InvalidSnapshot {
                field: "attention queue",
            })?,
        });
    }
    snapshot.running = decode_running(&mut decoder)?;
    validate_running(snapshot.running.as_ref())?;
    snapshot.busy = decode_busy(&mut decoder)?;
    validate_busy(snapshot.busy.as_ref())?;
    validate_workspace(&snapshot, &wire_tabs)?;
    validate_codex(snapshot.codex_quota.as_ref())?;
    validate_claude(claude.as_ref())?;
    validate_failure(snapshot.codex_quota_failure)?;
    validate_failure(snapshot.claude_quota_failure)?;
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
        snapshot.tabs.push(assemble_tab(tab, layout)?);
    }
    Ok(Response::Snapshot(snapshot))
}

fn encode_attention_queue(encoder: &mut Encoder, queue: &[QueueEntry]) {
    encoder.count(queue.len());
    for entry in queue {
        encoder.string(&entry.workspace);
        encoder.string(&entry.tab);
        encoder.string(&entry.target);
        encode_attention(encoder, Some(&entry.attention));
    }
}

fn encode_workspace_set(encoder: &mut Encoder, snapshot: &Snapshot) {
    encoder.string(&snapshot.workspace);
    encoder.string(&snapshot.active_workspace);
    encoder.count(snapshot.workspaces.len());
    for workspace in &snapshot.workspaces {
        encoder.string(&workspace.id);
        encode_optional(encoder, &workspace.name);
        encoder.count(workspace.tab_count);
        encode_state(encoder, workspace.attention);
        encoder.byte(workspace.recency);
    }
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
        encode_popup_key(encoder, &entry.shortcut);
    }
    encoder.count(tabs.len());
    for tab in tabs {
        encode_wire_tab(encoder, tab);
    }
}

fn encode_wire_tab(encoder: &mut Encoder, tab: &WireTab) {
    encoder.string(&tab.id);
    encoder.raw(&tab.directory);
    encoder.byte(u8::from(tab.pending));
    encode_optional(encoder, &tab.selected_pane);
    encode_optional(encoder, &tab.selected_popup);
    encoder.count(tab.panes.len());
    for pane in &tab.panes {
        encode_pane(encoder, pane);
    }
    encoder.count(tab.popups.len());
    for popup in &tab.popups {
        encode_popup(encoder, popup);
    }
    encode_state(encoder, tab.attention_rollup);
}

fn encode_pane(encoder: &mut Encoder, pane: &Pane) {
    encoder.string(&pane.id);
    encoder.string(&pane.session);
    encoder.raw(&pane.endpoint);
    encoder.byte(u8::from(pane.live));
    encoder.string(&pane.label);
    encode_optional(encoder, &pane.pinned_name);
    encode_attention(encoder, pane.attention.as_ref());
}

fn encode_popup(encoder: &mut Encoder, popup: &Popup) {
    encoder.string(&popup.id);
    encoder.string(&popup.entry);
    encoder.string(&popup.session);
    encoder.raw(&popup.endpoint);
    encode_attention(encoder, popup.attention.as_ref());
}

/// A tab's column layout: the column count, then each column's pane count
/// and expanded pane index.
fn encode_layout(encoder: &mut Encoder, tab: &Tab) -> Result<()> {
    validate_columns(tab)?;
    encoder.count(tab.columns.len());
    for column in &tab.columns {
        encoder.count(column.panes.len());
        let expanded = column
            .panes
            .iter()
            .position(|pane| pane.id == column.expanded)
            .expect("validated expanded pane");
        encoder.count(expanded);
    }
    Ok(())
}

type Layout = Vec<(usize, usize)>;

fn decode_layout(decoder: &mut Decoder<'_>) -> Result<Layout> {
    let columns = decoder.count("columns", MAX_COLUMNS)?;
    let mut layout = Vec::with_capacity(columns);
    for _ in 0..columns {
        let panes = decoder.count("column panes", MAX_SESSIONS)?;
        layout.push((panes, decoder.count("expanded pane", MAX_SESSIONS)?));
    }
    Ok(layout)
}

/// Rebuilds a carried tab's columns from its layout.
fn assemble_tab(tab: WireTab, layout: Layout) -> Result<Tab> {
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
        attention_rollup: tab.attention_rollup,
    };
    validate_columns(&tab)?;
    Ok(tab)
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
            shortcut: decode_popup_key(decoder)?,
        });
    }
    let count = decoder.count("tabs", MAX_TABS)?;
    let mut tabs = Vec::with_capacity(count);
    let mut sessions = 0;
    for _ in 0..count {
        tabs.push(decode_wire_tab(decoder, &mut sessions)?);
    }
    let snapshot = Snapshot {
        workspace: String::new(),
        active_workspace: String::new(),
        workspaces: Vec::new(),
        active_tab,
        geometry,
        entries,
        tabs: Vec::with_capacity(tabs.len()),
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        sidebar: Sidebar::Off,
        attention_queue: Vec::new(),
        running: None,
        busy: None,
    };
    Ok((snapshot, tabs))
}

/// Decodes one carried tab; `sessions` counts the terminals already decoded
/// against the shared session bound.
fn decode_wire_tab(decoder: &mut Decoder<'_>, sessions: &mut usize) -> Result<WireTab> {
    let id = decoder.string("tab id", MAX_ID_BYTES)?;
    let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
    let pending = decode_bool(decoder, "pending tab")?;
    let selected_pane = decode_optional(decoder, "selected pane")?;
    let selected_popup = decode_optional(decoder, "selected popup")?;
    let count = decoder.count("sessions", MAX_SESSIONS - *sessions)?;
    *sessions += count;
    let mut panes = Vec::with_capacity(count);
    for _ in 0..count {
        panes.push(decode_pane(decoder)?);
    }
    let count = decoder.count("sessions", (MAX_SESSIONS - *sessions).min(MAX_ENTRIES))?;
    *sessions += count;
    let mut popups = Vec::with_capacity(count);
    for _ in 0..count {
        popups.push(decode_popup(decoder)?);
    }
    Ok(WireTab {
        id,
        directory,
        pending,
        selected_pane,
        selected_popup,
        panes,
        popups,
        attention_rollup: decode_state(decoder, "attention rollup")?,
    })
}

fn decode_pane(decoder: &mut Decoder<'_>) -> Result<Pane> {
    Ok(Pane {
        id: decoder.string("pane id", MAX_ID_BYTES)?,
        session: decoder.string("session id", MAX_ID_BYTES)?,
        endpoint: decoder.raw("endpoint", MAX_ENDPOINT_BYTES)?.to_vec(),
        live: decode_bool(decoder, "liveness")?,
        label: decoder.string("pane label", MAX_PANE_LABEL_BYTES)?,
        pinned_name: decode_pane_name(decoder)?,
        attention: decode_attention(decoder)?,
    })
}

fn decode_popup(decoder: &mut Decoder<'_>) -> Result<Popup> {
    Ok(Popup {
        id: decoder.string("popup id", MAX_ID_BYTES)?,
        entry: decoder.string("entry id", MAX_ENTRY_ID_BYTES)?,
        session: decoder.string("session id", MAX_ID_BYTES)?,
        endpoint: decoder.raw("endpoint", MAX_ENDPOINT_BYTES)?.to_vec(),
        attention: decode_attention(decoder)?,
    })
}

/// A tag (0 bound, 1 unbound, 2 overridden), then a bound or default
/// shortcut as modifiers and key, then an overriding entry's id.
fn encode_popup_key(encoder: &mut Encoder, key: &PopupKey) {
    let shortcut = |encoder: &mut Encoder, shortcut: &Shortcut| {
        encoder.byte(shortcut.modifiers);
        encoder.string(&shortcut.key);
    };
    match key {
        PopupKey::Bound(bound) => {
            encoder.byte(0);
            shortcut(encoder, bound);
        }
        PopupKey::Unbound => encoder.byte(1),
        PopupKey::Overridden { default, by } => {
            encoder.byte(2);
            shortcut(encoder, default);
            encoder.string(by);
        }
    }
}

fn decode_popup_key(decoder: &mut Decoder<'_>) -> Result<PopupKey> {
    let shortcut = |decoder: &mut Decoder<'_>| -> Result<Shortcut> {
        Ok(Shortcut {
            modifiers: decoder.byte()?,
            key: decoder.string("shortcut key", MAX_KEY_BYTES)?,
        })
    };
    Ok(match decoder.byte()? {
        0 => PopupKey::Bound(shortcut(decoder)?),
        1 => PopupKey::Unbound,
        2 => PopupKey::Overridden {
            default: shortcut(decoder)?,
            by: decoder.string("entry id", MAX_ENTRY_ID_BYTES)?,
        },
        value => {
            return Err(Error::InvalidTag {
                field: "popup key",
                value,
            });
        }
    })
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
        let unique = match &entry.shortcut {
            PopupKey::Bound(shortcut) => {
                shortcut.validate()?;
                shortcuts.insert(shortcut)
            }
            PopupKey::Unbound => true,
            PopupKey::Overridden { default, by } => {
                default.validate()?;
                entry_id(by)?;
                // The overriding entry is another listed one bound to the default.
                snapshot
                    .entries
                    .iter()
                    .any(|other| &other.id == by && other.shortcut.bound() == Some(default))
            }
        };
        if !entries.insert(entry.id.as_str()) || !unique {
            return Err(Error::InvalidSnapshot { field: "entries" });
        }
    }
    validate_tabs(tabs, Some(&entries))?;
    if !tabs.iter().any(|tab| tab.id == snapshot.active_tab) {
        return Err(Error::InvalidSnapshot {
            field: "active tab",
        });
    }
    if snapshot.workspaces.is_empty() || snapshot.workspaces.len() > MAX_WORKSPACES {
        return Err(Error::InvalidSnapshot {
            field: "workspaces",
        });
    }
    let mut workspace_ids = HashSet::new();
    for workspace in &snapshot.workspaces {
        validate_workspace_info(workspace)?;
        if !workspace_ids.insert(workspace.id.as_str()) {
            return Err(Error::InvalidSnapshot {
                field: "workspace id",
            });
        }
    }
    if !workspace_ids.contains(snapshot.active_workspace.as_str()) {
        return Err(Error::InvalidSnapshot {
            field: "active workspace",
        });
    }
    // The ranks are 0 to M-1 once each, with 0 on the shown workspace.
    let mut ranks: Vec<_> = snapshot.workspaces.iter().map(|w| w.recency).collect();
    ranks.sort_unstable();
    if ranks
        .iter()
        .enumerate()
        .any(|(index, rank)| usize::from(*rank) != index)
        || snapshot.workspaces.iter().any(|workspace| {
            (workspace.recency == 0) != (workspace.id == snapshot.active_workspace)
        })
    {
        return Err(Error::InvalidSnapshot {
            field: "workspace recency",
        });
    }
    if snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.id == snapshot.workspace)
        .is_none_or(|workspace| {
            workspace.tab_count != tabs.len()
                || workspace.attention != tabs.iter().filter_map(|tab| tab.attention_rollup).min()
        })
    {
        return Err(Error::InvalidSnapshot {
            field: "workspace projection",
        });
    }
    if snapshot.attention_queue.len() > MAX_SESSIONS {
        return Err(Error::InvalidSnapshot {
            field: "attention queue",
        });
    }
    for entry in &snapshot.attention_queue {
        identity("queue workspace", &entry.workspace)?;
        identity("queue tab", &entry.tab)?;
        identity("queue target", &entry.target)?;
        if !workspace_ids.contains(entry.workspace.as_str())
            || !queued(entry.attention.state)
            || entry.attention.record.is_none()
        {
            return Err(Error::InvalidSnapshot {
                field: "attention queue",
            });
        }
        validate_attention(&entry.attention)?;
    }
    Ok(())
}

/// The states EON-C31's attention queue holds.
fn queued(state: AttentionState) -> bool {
    matches!(
        state,
        AttentionState::NeedsInput | AttentionState::Error | AttentionState::Finished
    )
}

/// A session id is `s` and a positive number without leading zeros.
fn validate_workspace_id(id: &str) -> Result<()> {
    let number = id.strip_prefix('s').unwrap_or("");
    if number.starts_with('0')
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || number
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
            .is_none()
    {
        return Err(Error::InvalidSnapshot {
            field: "workspace id",
        });
    }
    Ok(())
}

/// Checks one workspace summary on its own; set rules stay with the caller.
fn validate_workspace_info(workspace: &WorkspaceInfo) -> Result<()> {
    validate_workspace_id(&workspace.id)?;
    if let Some(name) = &workspace.name {
        validate_workspace_name(name)?;
    }
    if workspace.tab_count == 0 || workspace.tab_count > MAX_TABS {
        return Err(Error::InvalidSnapshot {
            field: "workspace tabs",
        });
    }
    Ok(())
}

/// Checks a run of one workspace's tabs as carried: identities, selections,
/// surfaces, rollups and pane labels. `entries` is the popup catalog when the
/// message carries one.
fn validate_tabs(tabs: &[WireTab], entries: Option<&HashSet<&str>>) -> Result<()> {
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
        if tab.popups.len() > MAX_ENTRIES {
            return Err(Error::InvalidSnapshot { field: "sessions" });
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
                || tab.popups[0].entry != FILES_ENTRY
                || tab.selected_popup.as_ref() != Some(&tab.popups[0].id))
        {
            return Err(Error::InvalidSnapshot {
                field: "pending tab",
            });
        }
        let mut tab_entries = HashSet::new();
        for popup in &tab.popups {
            if entries.is_none() {
                entry_id(&popup.entry)?;
            }
            // Project shows the tab's Files pair and never has a popup of its own.
            if entries.is_some_and(|entries| !entries.contains(popup.entry.as_str()))
                || popup.entry == PROJECT_ENTRY
                || !tab_entries.insert(popup.entry.as_str())
            {
                return Err(Error::InvalidSnapshot {
                    field: "popup entry",
                });
            }
        }
        let surfaces = tab
            .panes
            .iter()
            .map(|p| (&p.id, &p.session, &p.endpoint, &p.attention))
            .chain(
                tab.popups
                    .iter()
                    .map(|p| (&p.id, &p.session, &p.endpoint, &p.attention)),
            );
        let mut rollup = None;
        for (id, session, endpoint, attention) in surfaces {
            identity("surface id", id)?;
            identity("session id", session)?;
            validate_endpoint(endpoint)?;
            if let Some(attention) = attention {
                validate_attention(attention)?;
                rollup = Some(rollup.map_or(attention.state, |state: AttentionState| {
                    state.min(attention.state)
                }));
            }
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
        if tab.attention_rollup != rollup {
            return Err(Error::InvalidSnapshot {
                field: "attention rollup",
            });
        }
    }
    for pane in tabs.iter().flat_map(|tab| &tab.panes) {
        validate_pane_label(pane)?;
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
    if quota.observed_at == 0 || quota.windows.len() > MAX_QUOTA_WINDOWS {
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
            || window.scope.as_deref().is_some_and(|scope| {
                scope.is_empty()
                    || scope.len() > MAX_QUOTA_SCOPE_BYTES
                    || scope.chars().any(char::is_control)
            })
        {
            return Err(Error::InvalidSnapshot {
                field: "Codex quota window",
            });
        }
    }
    Ok(())
}

fn validate_codex(quota: Option<&CodexQuota>) -> Result<()> {
    validate_quota(quota)?;
    if quota.is_some_and(|quota| {
        quota.windows.len() > MAX_CODEX_QUOTA_WINDOWS
            || quota.windows.iter().any(|w| w.scope.is_some())
    }) {
        return Err(Error::InvalidSnapshot {
            field: "Codex quota",
        });
    }
    Ok(())
}

fn validate_claude(quota: Option<&CodexQuota>) -> Result<()> {
    validate_quota(quota)?;
    let Some(quota) = quota else { return Ok(()) };
    let scoped = quota.windows.iter().filter(|w| w.scope.is_some()).count();
    if !matches!(quota.state, CodexQuotaState::Fresh | CodexQuotaState::Stale)
        || quota.windows.len() - scoped > 2
        || scoped > MAX_SCOPED_QUOTA_WINDOWS
        || quota.windows.iter().enumerate().any(|(index, window)| {
            !matches!(window.duration_minutes, 300 | 10080)
                || quota.windows[..index].iter().any(|previous| {
                    previous.duration_minutes == window.duration_minutes
                        && previous.scope == window.scope
                })
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
        encoder.byte(u8::from(window.scope.is_some()));
        if let Some(scope) = &window.scope {
            encoder.string(scope);
        }
    }
}

fn validate_failure(failure: Option<QuotaFailure>) -> Result<()> {
    if failure.is_some_and(|failure| failure.at == 0) {
        return Err(Error::InvalidSnapshot {
            field: "quota failure",
        });
    }
    Ok(())
}

fn encode_failure_record(encoder: &mut Encoder, failure: Option<QuotaFailure>) {
    encoder.byte(u8::from(failure.is_some()));
    let Some(failure) = failure else { return };
    encoder.bytes.extend_from_slice(&failure.at.to_le_bytes());
    encoder.byte(match failure.reason {
        QuotaFailureReason::Start => 0,
        QuotaFailureReason::Deadline => 1,
        QuotaFailureReason::OutputLimit => 2,
        QuotaFailureReason::ExitStatus => 3,
        QuotaFailureReason::Malformed => 4,
        QuotaFailureReason::ProviderError => 5,
        QuotaFailureReason::Estimated => 6,
        QuotaFailureReason::ObservationTime => 7,
        QuotaFailureReason::InvalidWindow => 8,
    });
}

fn decode_failure_record(decoder: &mut Decoder<'_>) -> Result<Option<QuotaFailure>> {
    if !decode_bool(decoder, "quota failure presence")? {
        return Ok(None);
    }
    let at = decode_u64(decoder)?;
    let reason = match decoder.byte()? {
        0 => QuotaFailureReason::Start,
        1 => QuotaFailureReason::Deadline,
        2 => QuotaFailureReason::OutputLimit,
        3 => QuotaFailureReason::ExitStatus,
        4 => QuotaFailureReason::Malformed,
        5 => QuotaFailureReason::ProviderError,
        6 => QuotaFailureReason::Estimated,
        7 => QuotaFailureReason::ObservationTime,
        8 => QuotaFailureReason::InvalidWindow,
        value => {
            return Err(Error::InvalidTag {
                field: "quota failure reason",
                value,
            });
        }
    };
    Ok(Some(QuotaFailure { at, reason }))
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
    let count = decoder.count("quota windows", MAX_QUOTA_WINDOWS)?;
    let mut windows = Vec::with_capacity(count);
    for _ in 0..count {
        windows.push(CodexQuotaWindow {
            duration_minutes: decode_u32(decoder)?,
            remaining_percent: decoder.byte()?,
            resets_at: decode_bool(decoder, "Codex quota reset")?
                .then(|| decode_u64(decoder))
                .transpose()?,
            scope: decode_bool(decoder, "quota scope")?
                .then(|| decoder.string("quota scope", MAX_QUOTA_SCOPE_BYTES))
                .transpose()?,
        });
    }
    Ok(Some(CodexQuota {
        state,
        observed_at,
        windows,
    }))
}

fn validate_attention(attention: &Attention) -> Result<()> {
    let invalid = || Err(Error::InvalidSnapshot { field: "attention" });
    let Some(record) = &attention.record else {
        return if attention.state == AttentionState::Unavailable {
            Ok(())
        } else {
            invalid()
        };
    };
    if attention.state == AttentionState::Unavailable
        || (record.kind.is_some() && attention.state != AttentionState::NeedsInput)
        || record.progress.is_some_and(|progress| progress > 100)
        || record.others > MAX_OTHER_RECORDS
    {
        return invalid();
    }
    for (field, value, maximum) in [
        ("attention app", &record.app, MAX_APP_BYTES),
        ("attention name", &record.name, MAX_NAME_BYTES),
        ("attention title", &record.title, MAX_TITLE_BYTES),
        (
            "attention message",
            &record.message,
            MAX_STATUS_MESSAGE_BYTES,
        ),
    ] {
        if value.len() > maximum {
            return Err(Error::FieldTooLong {
                field,
                length: value.len(),
            });
        }
        if value.chars().any(char::is_control) {
            return Err(Error::InvalidValue { field });
        }
    }
    Ok(())
}

fn encode_state(encoder: &mut Encoder, state: Option<AttentionState>) {
    encoder.byte(match state {
        None => 0,
        Some(AttentionState::NeedsInput) => 1,
        Some(AttentionState::Error) => 2,
        Some(AttentionState::Finished) => 3,
        Some(AttentionState::Working) => 4,
        Some(AttentionState::Idle) => 5,
        Some(AttentionState::Unavailable) => 6,
        Some(AttentionState::Background) => 7,
    });
}

fn decode_state(decoder: &mut Decoder<'_>, field: &'static str) -> Result<Option<AttentionState>> {
    Ok(Some(match decoder.byte()? {
        0 => return Ok(None),
        1 => AttentionState::NeedsInput,
        2 => AttentionState::Error,
        3 => AttentionState::Finished,
        4 => AttentionState::Working,
        5 => AttentionState::Idle,
        6 => AttentionState::Unavailable,
        7 => AttentionState::Background,
        value => return Err(Error::InvalidTag { field, value }),
    }))
}

/// The state, then, for every state but unavailable, the record.
fn encode_attention(encoder: &mut Encoder, attention: Option<&Attention>) {
    encode_state(encoder, attention.map(|attention| attention.state));
    let Some(record) = attention.and_then(|attention| attention.record.as_ref()) else {
        return;
    };
    encoder.byte(match record.kind {
        None => 0,
        Some(AttentionKind::Permission) => 1,
        Some(AttentionKind::Question) => 2,
        Some(AttentionKind::Auth) => 3,
    });
    encoder.byte(record.progress.unwrap_or(NO_PROGRESS));
    encoder.string(&record.app);
    encoder.string(&record.name);
    encoder.string(&record.title);
    encoder.string(&record.message);
    encoder
        .bytes
        .extend_from_slice(&record.changed_at.to_le_bytes());
    encoder.byte(u8::from(record.stale));
    encoder.number(record.others);
}

fn decode_attention(decoder: &mut Decoder<'_>) -> Result<Option<Attention>> {
    let Some(state) = decode_state(decoder, "attention state")? else {
        return Ok(None);
    };
    if state == AttentionState::Unavailable {
        return Ok(Some(Attention {
            state,
            record: None,
        }));
    }
    let kind = match decoder.byte()? {
        0 => None,
        1 => Some(AttentionKind::Permission),
        2 => Some(AttentionKind::Question),
        3 => Some(AttentionKind::Auth),
        value => {
            return Err(Error::InvalidTag {
                field: "attention kind",
                value,
            });
        }
    };
    let progress = Some(decoder.byte()?).filter(|progress| *progress != NO_PROGRESS);
    let record = AttentionRecord {
        kind,
        progress,
        app: decoder.string("attention app", MAX_APP_BYTES)?,
        name: decoder.string("attention name", MAX_NAME_BYTES)?,
        title: decoder.string("attention title", MAX_TITLE_BYTES)?,
        message: decoder.string("attention message", MAX_STATUS_MESSAGE_BYTES)?,
        changed_at: decode_u64(decoder)?,
        stale: decode_bool(decoder, "attention stale")?,
        others: decoder.number()?,
    };
    Ok(Some(Attention {
        state,
        record: Some(record),
    }))
}

fn encode_sidebar(encoder: &mut Encoder, sidebar: Sidebar) {
    encoder.byte(match sidebar {
        Sidebar::Open => 0,
        Sidebar::Closed => 1,
        Sidebar::Off => 2,
    });
}

fn validate_window_name(name: Option<&str>) -> Result<()> {
    if name.is_some_and(|name| {
        name.chars().count() > MAX_WINDOW_NAME_CHARS || !name.chars().all(pane_label_character)
    }) {
        return Err(Error::InvalidValue {
            field: "window name",
        });
    }
    Ok(())
}

/// A window id is eight lowercase hex digits; none names the default
/// namespace.
fn validate_window_id(window: Option<&str>) -> Result<()> {
    if window.is_some_and(|id| {
        id.len() != 8
            || !id
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        return Err(Error::InvalidValue { field: "window id" });
    }
    Ok(())
}

fn validate_window(entry: &WindowEntry) -> Result<()> {
    validate_window_id(entry.window.as_deref())?;
    identity("generation", &entry.generation)?;
    validate_window_name(entry.name.as_deref())?;
    if entry
        .live_terminals
        .is_some_and(|count| usize::from(count) > MAX_SESSIONS)
    {
        return Err(Error::InvalidValue {
            field: "live terminals",
        });
    }
    let known =
        entry.name.is_some() && entry.started_at.is_some() && entry.live_terminals.is_some();
    let unknown =
        entry.name.is_none() && entry.started_at.is_none() && entry.live_terminals.is_none();
    if !(if entry.state == WindowState::Live {
        known
    } else {
        unknown
    }) {
        return Err(Error::InvalidValue {
            field: "window state",
        });
    }
    Ok(())
}

fn validate_running(running: Option<&RunningWindows>) -> Result<()> {
    let Some(running) = running else {
        return Ok(());
    };
    validate_window(&running.this)?;
    if running.this.state != WindowState::Live {
        return Err(Error::InvalidValue {
            field: "this window",
        });
    }
    if let Some(others) = &running.others {
        if others.len() > MAX_RUNNING_WINDOWS {
            return Err(Error::InvalidValue {
                field: "running windows",
            });
        }
        for entry in others {
            validate_window(entry)?;
        }
    }
    Ok(())
}

fn encode_window(encoder: &mut Encoder, entry: &WindowEntry) {
    encode_optional(encoder, &entry.window);
    encoder.string(&entry.generation);
    encode_optional(encoder, &entry.name);
    encoder.byte(u8::from(entry.started_at.is_some()));
    if let Some(started_at) = entry.started_at {
        encoder.bytes.extend_from_slice(&started_at.to_le_bytes());
    }
    encoder.byte(u8::from(entry.live_terminals.is_some()));
    if let Some(count) = entry.live_terminals {
        encoder.number(count);
    }
    encoder.byte(match entry.state {
        WindowState::Live => 0,
        WindowState::Incompatible => 1,
        WindowState::Unreachable => 2,
    });
}

fn decode_window_name(decoder: &mut Decoder<'_>) -> Result<Option<String>> {
    decode_bool(decoder, "window name")?
        .then(|| decoder.string("window name", MAX_WINDOW_NAME_BYTES))
        .transpose()
}

fn decode_window(decoder: &mut Decoder<'_>) -> Result<WindowEntry> {
    let window = decode_bool(decoder, "window id")?
        .then(|| decoder.string("window id", 8))
        .transpose()?;
    let generation = decoder.string("generation", MAX_ID_BYTES)?;
    let name = decode_window_name(decoder)?;
    let started_at = decode_bool(decoder, "started at")?
        .then(|| decode_u64(decoder))
        .transpose()?;
    let live_terminals = decode_bool(decoder, "live terminals")?
        .then(|| decoder.number())
        .transpose()?;
    let state = match decoder.byte()? {
        0 => WindowState::Live,
        1 => WindowState::Incompatible,
        2 => WindowState::Unreachable,
        value => {
            return Err(Error::InvalidTag {
                field: "window state",
                value,
            });
        }
    };
    Ok(WindowEntry {
        window,
        generation,
        name,
        started_at,
        live_terminals,
        state,
    })
}

fn encode_running(encoder: &mut Encoder, running: Option<&RunningWindows>) {
    encoder.byte(u8::from(running.is_some()));
    if let Some(running) = running {
        encode_window(encoder, &running.this);
        encoder.byte(u8::from(running.others.is_some()));
        if let Some(others) = &running.others {
            encoder.count(others.len());
            for entry in others {
                encode_window(encoder, entry);
            }
        }
        encoder.number(running.omitted);
    }
}

fn decode_running(decoder: &mut Decoder<'_>) -> Result<Option<RunningWindows>> {
    if !decode_bool(decoder, "running windows")? {
        return Ok(None);
    }
    let this = decode_window(decoder)?;
    let others = if decode_bool(decoder, "other windows")? {
        let count = decoder.count("running windows", MAX_RUNNING_WINDOWS)?;
        Some(
            (0..count)
                .map(|_| decode_window(decoder))
                .collect::<Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    Ok(Some(RunningWindows {
        this,
        others,
        omitted: decoder.number()?,
    }))
}

fn validate_busy(busy: Option<&BusyTerminals>) -> Result<()> {
    let Some(busy) = busy else {
        return Ok(());
    };
    let invalid = Error::InvalidValue {
        field: "busy terminals",
    };
    if busy.terminals.len() > MAX_BUSY_TERMINALS
        || (busy.omitted > 0 && busy.terminals.len() < MAX_BUSY_TERMINALS)
        || busy.terminals.len() + usize::from(busy.omitted) > MAX_SESSIONS
    {
        return Err(invalid);
    }
    for (index, busy_terminal) in busy.terminals.iter().enumerate() {
        validate_busy_terminal(&busy_terminal.target, &busy_terminal.name)?;
        if busy.terminals[..index]
            .iter()
            .any(|earlier| earlier.target.terminal == busy_terminal.target.terminal)
        {
            return Err(invalid);
        }
    }
    Ok(())
}

fn validate_busy_terminal(target: &TerminalTarget, name: &str) -> Result<()> {
    validate_target(target)?;
    nonempty("busy name", name, MAX_PANE_LABEL_BYTES)?;
    // Pane labels and popup entry labels both hold no control character.
    if name.chars().any(char::is_control) {
        return Err(Error::InvalidValue { field: "busy name" });
    }
    Ok(())
}

fn encode_busy(encoder: &mut Encoder, busy: Option<&BusyTerminals>) {
    encoder.byte(u8::from(busy.is_some()));
    if let Some(busy) = busy {
        encoder.count(busy.terminals.len());
        for busy_terminal in &busy.terminals {
            encode_target(encoder, &busy_terminal.target);
            encoder.string(&busy_terminal.name);
        }
        encoder.number(busy.omitted);
    }
}

fn decode_busy(decoder: &mut Decoder<'_>) -> Result<Option<BusyTerminals>> {
    if !decode_bool(decoder, "busy terminals")? {
        return Ok(None);
    }
    let count = decoder.count("busy terminals", MAX_BUSY_TERMINALS)?;
    let terminals = (0..count)
        .map(|_| {
            Ok(BusyTerminal {
                target: decode_target(decoder)?,
                name: decoder.string("busy name", MAX_PANE_LABEL_BYTES)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(BusyTerminals {
        terminals,
        omitted: decoder.number()?,
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
            encode_optional(&mut payload, &runtime.name);
            payload
                .bytes
                .extend_from_slice(&runtime.started_at.to_le_bytes());
            encode_busy(&mut payload, runtime.busy.as_ref());
            RUNTIME
        }
        LifecycleResponse::Failure(failure) => {
            encode_failure(&mut payload, failure)?;
            FAILURE
        }
    };
    frame(kind, payload.bytes)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    let (kind, payload) = unframe(bytes)?;
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
                name: decode_window_name(&mut decoder)?,
                started_at: decode_u64(&mut decoder)?,
                busy: decode_busy(&mut decoder)?,
            };
            validate_runtime(&runtime)?;
            LifecycleResponse::Runtime(runtime)
        }
        FAILURE => LifecycleResponse::Failure(decode_failure(&mut decoder)?),
        value => return Err(Error::InvalidKind { value }),
    };
    decoder.finish()?;
    Ok(response)
}

fn validate_runtime(runtime: &Runtime) -> Result<()> {
    validate_window_name(runtime.name.as_deref())?;
    validate_busy(runtime.busy.as_ref())?;
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

fn encode_failure(encoder: &mut Encoder, failure: &Failure) -> Result<()> {
    identity("failure code", &failure.code)?;
    nonempty("failure detail", &failure.detail, MAX_DETAIL_BYTES)?;
    encoder.string(&failure.code);
    encoder.string(&failure.detail);
    Ok(())
}

fn decode_failure(decoder: &mut Decoder<'_>) -> Result<Failure> {
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

/// A file to edit is an absolute path within the directory bounds.
fn validate_file(file: &[u8]) -> Result<()> {
    if file.first() != Some(&b'/') || file.contains(&0) {
        return Err(Error::InvalidValue { field: "file" });
    }
    if file.len() > MAX_DIRECTORY_BYTES {
        return Err(Error::FieldTooLong {
            field: "file",
            length: file.len(),
        });
    }
    Ok(())
}

fn validate_command(argv: &[Vec<u8>]) -> Result<()> {
    if argv.len() > MAX_COMMAND_ARGUMENTS {
        return Err(Error::InvalidValue { field: "command" });
    }
    if argv.first().is_none_or(|program| program.is_empty()) {
        return Err(Error::InvalidValue { field: "command" });
    }
    for argument in argv {
        if argument.contains(&0) {
            return Err(Error::InvalidValue {
                field: "command argument",
            });
        }
        if argument.len() > MAX_COMMAND_ARGUMENT_BYTES {
            return Err(Error::FieldTooLong {
                field: "command argument",
                length: argument.len(),
            });
        }
    }
    Ok(())
}

fn validate_directory(directory: &[u8]) -> Result<()> {
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

fn validate_endpoint(endpoint: &[u8]) -> Result<()> {
    if endpoint.is_empty() {
        return Err(Error::InvalidSnapshot { field: "endpoint" });
    }
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(Error::FieldTooLong {
            field: "endpoint",
            length: endpoint.len(),
        });
    }
    Ok(())
}

/// Whether a request may carry `caller` as its calling terminal.
pub fn valid_caller(caller: &str) -> bool {
    identity("caller", caller).is_ok()
}

fn identity(field: &'static str, value: &str) -> Result<()> {
    nonempty(field, value, MAX_ID_BYTES)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidValue { field });
    }
    Ok(())
}

fn nonempty(field: &'static str, value: &str, maximum: usize) -> Result<()> {
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

/// Returns the bounded length declared by a complete EONW header, at most
/// 2 MiB whatever its kind.
pub fn declared_message_len(header: &[u8]) -> Result<usize> {
    declared_len(header, MAX_MESSAGE_BYTES)
}

/// Returns the bounded length declared by a complete EONW response header.
/// Unlike [`declared_message_len`], it admits a `History` response up to its
/// own bound; only the CLI's reply reader uses it.
pub fn declared_response_len(header: &[u8]) -> Result<usize> {
    declared_len(
        header,
        message_bound(*header.get(6).ok_or(Error::Truncated)?),
    )
}

fn declared_len(header: &[u8], maximum: usize) -> Result<usize> {
    if header.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    let payload_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let size = HEADER_BYTES
        .checked_add(payload_len)
        .ok_or(Error::MessageTooLarge {
            size: usize::MAX,
            maximum,
        })?;
    if size > maximum {
        return Err(Error::MessageTooLarge { size, maximum });
    }
    Ok(size)
}

/// The longest message of `kind`: 2 MiB for all but a `History` response.
fn message_bound(kind: u8) -> usize {
    if kind == HISTORY {
        MAX_HISTORY_MESSAGE_BYTES
    } else {
        MAX_MESSAGE_BYTES
    }
}

fn frame(kind: u8, payload: Vec<u8>) -> Result<Vec<u8>> {
    let size = HEADER_BYTES + payload.len();
    let maximum = message_bound(kind);
    if size > maximum {
        return Err(Error::MessageTooLarge { size, maximum });
    }
    let mut message = Vec::with_capacity(size);
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&VERSION.to_le_bytes());
    message.push(kind);
    message.push(0);
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&payload);
    Ok(message)
}

fn unframe(bytes: &[u8]) -> Result<(u8, &[u8])> {
    if bytes.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    let maximum = message_bound(bytes[6]);
    if bytes.len() > maximum {
        return Err(Error::MessageTooLarge {
            size: bytes.len(),
            maximum,
        });
    }
    if &bytes[..4] != MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(Error::UnsupportedVersion { version });
    }
    if bytes[7] != 0 {
        return Err(Error::InvalidFlags { value: bytes[7] });
    }
    let declared = declared_len(bytes, maximum)?;
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
struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn count(&mut self, value: usize) {
        let value = u16::try_from(value).expect("validated EONW count");
        self.number(value);
    }

    fn number(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.raw(value.as_bytes());
    }

    fn raw(&mut self, value: &[u8]) {
        let length = u16::try_from(value.len()).expect("validated EONW field length");
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
    }

    /// A field whose bound exceeds a `u16` length: a `u32` length prefix.
    fn long_raw(&mut self, value: &[u8]) {
        let length = u32::try_from(value.len()).expect("validated EONW field length");
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn count(&mut self, field: &'static str, maximum: usize) -> Result<usize> {
        let value = self.number()? as usize;
        if value > maximum {
            return Err(Error::InvalidSnapshot { field });
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn string(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        str::from_utf8(self.raw(field, maximum)?)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidUtf8 { field })
    }

    fn raw(&mut self, field: &'static str, maximum: usize) -> Result<&'a [u8]> {
        let bytes = self.take(2)?;
        let length = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        if length > maximum {
            return Err(Error::FieldTooLong { field, length });
        }
        self.take(length)
    }

    fn long_string(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        let bytes = self.take(4)?;
        let length = u32::from_le_bytes(bytes.try_into().unwrap()) as usize;
        if length > maximum {
            return Err(Error::FieldTooLong { field, length });
        }
        str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidUtf8 { field })
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self) -> Result<()> {
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

    fn window(duration_minutes: u32, resets_at: Option<u64>) -> QuotaWindow {
        QuotaWindow {
            duration_minutes,
            remaining_percent: 50,
            resets_at,
            scope: None,
        }
    }

    fn attention(state: AttentionState) -> Option<Attention> {
        Some(Attention {
            state,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: "codex".into(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1,
                stale: false,
                others: 0,
            }),
        })
    }

    /// Hidden and inactive popups, a popup-only tab, two pending tabs, columns,
    /// attention of every shape and both quotas.
    #[test]
    fn workspace_set_requests_are_supported() {
        let mut payload = Encoder::default();
        payload.string("new-workspace");
        encode_optional(&mut payload, &None);
        payload.byte(32);
        encode_optional(&mut payload, &None);
        payload.byte(0);
        let bytes = frame(REQUEST, payload.bytes).unwrap();
        assert!(
            decode_request(&bytes).is_ok(),
            "the new-workspace action must decode"
        );
    }

    #[test]
    fn workspace_set_fields_and_requests_round_trip_with_bounds() {
        let name = "🦀".repeat(MAX_WORKSPACE_NAME_CHARS);
        for action in [
            WorkspaceAction::NewWorkspace {
                name: None,
                focus: false,
            },
            WorkspaceAction::NewWorkspace {
                name: Some(name.clone()),
                focus: true,
            },
            WorkspaceAction::SwitchWorkspace {
                workspace: name.clone(),
            },
            WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: name.clone(),
            },
            WorkspaceAction::RenameWorkspace {
                workspace: Some("s1".into()),
                name: String::new(),
            },
            WorkspaceAction::CloseWorkspace {
                workspace: "s1".into(),
            },
        ] {
            let request = Request {
                id: "workspaces".into(),
                caller: Some("terminal-1".into()),
                action: Action::Workspace(action),
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
            let mut old = bytes;
            old[4..6].copy_from_slice(&(VERSION - 1).to_le_bytes());
            assert!(decode_request(&old).is_err());
        }
        for name in [String::new(), "a".repeat(65), "🦀".repeat(65), "a\n".into()] {
            let request = Request {
                id: "bad".into(),
                caller: None,
                action: Action::Workspace(WorkspaceAction::NewWorkspace {
                    name: Some(name.clone()),
                    focus: false,
                }),
            };
            assert!(encode_request(&request).is_err());
            let mut payload = Encoder::default();
            payload.string("bad");
            encode_optional(&mut payload, &None);
            payload.byte(32);
            encode_optional(&mut payload, &Some(name));
            payload.byte(0);
            assert!(decode_request(&frame(REQUEST, payload.bytes).unwrap()).is_err());
        }
        let mut state = snapshot();
        state.workspaces = (1..=MAX_WORKSPACES)
            .map(|index| WorkspaceInfo {
                id: format!("s{index}"),
                name: Some(name.clone()),
                tab_count: 4,
                attention: (index % 3 == 0).then_some(AttentionState::Working),
                recency: (index - 1) as u8,
            })
            .collect();
        state.workspace = "s2".into();
        state.active_workspace = "s1".into();
        state.workspaces[1].attention = Some(AttentionState::NeedsInput);
        let response = Response::Snapshot(state.clone());
        let bytes = encode_response(&response).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), response);
        for mutate in [
            (|s: &mut Snapshot| s.workspaces.clear()) as fn(&mut Snapshot),
            |s| {
                s.workspaces.push(s.workspaces[0].clone());
            },
            |s| s.workspaces[0].id = "s0".into(),
            |s| s.workspaces[0].id = "s01".into(),
            |s| s.workspaces[0].name = Some("a".repeat(65)),
            |s| s.workspaces[0].name = Some("a\n".into()),
            |s| s.workspaces[0].name = Some(String::new()),
            |s| s.workspaces[0].tab_count = 0,
            |s| s.workspaces[0].tab_count = MAX_TABS + 1,
            |s| s.workspaces[1].tab_count = 3,
            |s| s.active_workspace = "s100".into(),
            |s| s.workspace = "s100".into(),
            // A rank twice, a rank past M-1, and a shown workspace not ranked 0.
            |s| s.workspaces[2].recency = 1,
            |s| s.workspaces[31].recency = 32,
            |s| s.active_workspace = "s3".into(),
            // The projected workspace's rollup is its tabs' most urgent one.
            |s| s.workspaces[1].attention = Some(AttentionState::Error),
            |s| s.workspaces[1].attention = None,
        ] {
            let mut bad = state.clone();
            mutate(&mut bad);
            assert!(encode_response(&Response::Snapshot(bad.clone())).is_err());
            assert!(decode_response(&forge(&bad)).is_err());
        }
        let mut old = bytes;
        old[4..6].copy_from_slice(&(VERSION - 1).to_le_bytes());
        assert!(decode_response(&old).is_err());
    }

    fn snapshot() -> Snapshot {
        let entry = |id: &str, key: &str| PopupEntry {
            id: id.into(),
            label: id.into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: key.into(),
            }),
        };
        let tab = |id: &str, columns, popups: Vec<Popup>| Tab {
            id: id.into(),
            directory: b"/tmp/\xff".to_vec(),
            pending: false,
            selected_pane: None,
            selected_popup: None,
            columns,
            popups,
            attention_rollup: None,
        };
        let pending = |id: &str, popup_id: &str| Tab {
            pending: true,
            selected_popup: Some(popup_id.into()),
            ..tab(id, vec![], vec![popup(popup_id, FILES_ENTRY)])
        };
        let mut asking = attention(AttentionState::NeedsInput);
        let record = asking.as_mut().unwrap().record.as_mut().unwrap();
        record.kind = Some(AttentionKind::Question);
        record.name = "eon-9a".into();
        record.title = "Allow?".into();
        record.message = "Run make".into();
        record.others = 2;
        let mut working = attention(AttentionState::Working);
        working.as_mut().unwrap().record.as_mut().unwrap().progress = Some(40);
        let mut finished = attention(AttentionState::Finished);
        finished.as_mut().unwrap().record.as_mut().unwrap().stale = true;
        let columns = vec![
            Column {
                panes: vec![
                    pane("p1"),
                    Pane {
                        attention: asking,
                        ..pane("p2")
                    },
                ],
                expanded: "p2".into(),
            },
            Column {
                panes: vec![Pane {
                    attention: working,
                    ..pane("p3")
                }],
                expanded: "p3".into(),
            },
        ];
        let agent = |id: &str, attention| Popup {
            attention,
            ..popup(id, "agent")
        };
        let unavailable = Some(Attention {
            state: AttentionState::Unavailable,
            record: None,
        });
        Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                name: None,
                tab_count: 4,
                attention: Some(AttentionState::NeedsInput),
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.5,
                vertical_margin: 4.0,
            },
            entries: vec![
                entry(PROJECT_ENTRY, "KeyZ"),
                entry("agent", "KeyL"),
                entry(FILES_ENTRY, "KeyY"),
            ],
            tabs: vec![
                Tab {
                    selected_pane: Some("p3".into()),
                    attention_rollup: Some(AttentionState::NeedsInput),
                    ..tab("t1", columns, vec![agent("u1", unavailable)])
                },
                pending("t2", "u2"),
                Tab {
                    attention_rollup: Some(AttentionState::Finished),
                    ..tab("t3", vec![], vec![agent("u3", finished)])
                },
                pending("t4", "u4"),
            ],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Fresh,
                observed_at: 100,
                windows: vec![window(300, Some(200)), window(10080, None)],
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Fresh,
                observed_at: 100,
                windows: vec![
                    window(300, Some(200)),
                    window(10080, None),
                    QuotaWindow {
                        scope: Some("Fable".into()),
                        ..window(10080, Some(200))
                    },
                ],
            }),
            codex_quota_failure: None,
            claude_quota_failure: Some(QuotaFailure {
                at: 120,
                reason: QuotaFailureReason::InvalidWindow,
            }),
            sidebar: Sidebar::Closed,
            attention_queue: Vec::new(),
            running: None,
            busy: None,
        }
    }

    /// Encodes a snapshot without validating it, as a faulty peer would.
    fn forge(snapshot: &Snapshot) -> Vec<u8> {
        let mut payload = Encoder::default();
        payload.count(snapshot.tabs.len());
        for tab in &snapshot.tabs {
            payload.count(tab.columns.len());
            for column in &tab.columns {
                let expanded = column.panes.iter().position(|p| p.id == column.expanded);
                payload.count(column.panes.len());
                payload.count(expanded.unwrap_or(column.panes.len()));
            }
        }
        let tabs: Vec<_> = snapshot.tabs.iter().map(WireTab::from).collect();
        encode_workspace(&mut payload, snapshot, &tabs);
        encode_quota(&mut payload, snapshot.codex_quota.as_ref());
        let claude = snapshot.claude_quota.as_ref().map(claude_as_codex);
        encode_quota(&mut payload, claude.as_ref());
        encode_failure_record(&mut payload, snapshot.codex_quota_failure);
        encode_failure_record(&mut payload, snapshot.claude_quota_failure);
        encode_sidebar(&mut payload, snapshot.sidebar);
        encode_workspace_set(&mut payload, snapshot);
        encode_attention_queue(&mut payload, &snapshot.attention_queue);
        encode_running(&mut payload, snapshot.running.as_ref());
        encode_busy(&mut payload, snapshot.busy.as_ref());
        frame(SNAPSHOT, payload.bytes).unwrap()
    }

    fn window_entry() -> WindowEntry {
        WindowEntry {
            window: Some("abcdef01".into()),
            generation: "g1".into(),
            name: Some("🦀".repeat(64)),
            started_at: Some(u64::MAX),
            live_terminals: Some(MAX_SESSIONS as u16),
            state: WindowState::Live,
        }
    }

    fn busy_terminal(n: usize) -> BusyTerminal {
        BusyTerminal {
            target: terminal_target(TargetId::Pane(format!("p{n}")), &format!("s{n}")),
            name: format!("p{n}  🦀"),
        }
    }

    #[test]
    fn busy_terminals_round_trip_and_reject_forged_bounds() {
        let mut state = snapshot();
        let runtime = Runtime {
            generation: "g1".into(),
            name: None,
            started_at: 1,
            eon_version: "0.1.13".into(),
            workspace_protocol: VERSION,
            component_report: "ok".into(),
            sessions: vec![],
            attach: Availability {
                available: true,
                reason: "ready".into(),
            },
            stop: Availability {
                available: true,
                reason: "ready".into(),
            },
            busy: None,
        };
        let full: Vec<_> = (0..MAX_BUSY_TERMINALS).map(busy_terminal).collect();
        for busy in [
            None,
            Some(BusyTerminals {
                terminals: vec![],
                omitted: 0,
            }),
            Some(BusyTerminals {
                terminals: vec![BusyTerminal {
                    target: terminal_target(TargetId::Popup("u1".into()), "s1"),
                    name: "x".repeat(MAX_PANE_LABEL_BYTES),
                }],
                omitted: 0,
            }),
            Some(BusyTerminals {
                terminals: full.clone(),
                omitted: (MAX_SESSIONS - MAX_BUSY_TERMINALS) as u16,
            }),
        ] {
            state.busy = busy.clone();
            let response = Response::Snapshot(state.clone());
            assert_eq!(
                decode_response(&encode_response(&response).unwrap()).unwrap(),
                response
            );
            let response = LifecycleResponse::Runtime(Runtime {
                busy,
                ..runtime.clone()
            });
            assert_eq!(
                decode_lifecycle_response(&encode_lifecycle_response(&response).unwrap()).unwrap(),
                response
            );
        }
        for index in 0..8 {
            let mut busy = BusyTerminals {
                terminals: vec![busy_terminal(1), busy_terminal(2)],
                omitted: 0,
            };
            match index {
                0 => busy.terminals = (0..=MAX_BUSY_TERMINALS).map(busy_terminal).collect(),
                1 => busy.terminals[1].target.terminal = "s1".into(),
                2 => busy.terminals[0].name.clear(),
                3 => busy.terminals[0].name = "p1  \u{1b}[31m".into(),
                4 => busy.terminals[0].name = "x".repeat(MAX_PANE_LABEL_BYTES + 1),
                5 => busy.terminals[0].target.tab = "t 1".into(),
                // Only a full list leaves terminals out, and never more than
                // the window holds.
                6 => busy.omitted = 1,
                7 => {
                    busy.terminals = full.clone();
                    busy.omitted = (MAX_SESSIONS - MAX_BUSY_TERMINALS + 1) as u16;
                }
                _ => unreachable!(),
            }
            state.busy = Some(busy.clone());
            assert!(
                encode_response(&Response::Snapshot(state.clone())).is_err(),
                "case {index}"
            );
            assert!(decode_response(&forge(&state)).is_err(), "case {index}");
            let response = LifecycleResponse::Runtime(Runtime {
                busy: Some(busy),
                ..runtime.clone()
            });
            assert!(
                encode_lifecycle_response(&response).is_err(),
                "case {index}"
            );
            // Forge the lifecycle reply by swapping the valid list's bytes.
            let valid = Runtime {
                busy: None,
                ..runtime.clone()
            };
            let mut bytes = encode_lifecycle_response(&LifecycleResponse::Runtime(valid)).unwrap();
            bytes.pop();
            let mut list = Encoder::default();
            encode_busy(&mut list, state.busy.as_ref());
            bytes.extend_from_slice(&list.bytes);
            let length = (bytes.len() - HEADER_BYTES) as u32;
            bytes[8..12].copy_from_slice(&length.to_le_bytes());
            assert!(decode_lifecycle_response(&bytes).is_err(), "case {index}");
        }
        // An unknown option tag fails before consumers see it.
        state.busy = None;
        let mut bytes = encode_response(&Response::Snapshot(state)).unwrap();
        *bytes.last_mut().unwrap() = 2;
        assert!(decode_response(&bytes).is_err());
    }

    #[test]
    fn running_windows_round_trip_and_reject_forged_bounds() {
        let mut state = snapshot();
        let this = window_entry();
        let unknown = WindowEntry {
            window: None,
            generation: "g2".into(),
            name: None,
            started_at: None,
            live_terminals: None,
            state: WindowState::Incompatible,
        };
        for others in [
            None,
            Some(vec![]),
            Some(vec![this.clone(); 32]),
            Some(vec![
                unknown.clone(),
                WindowEntry {
                    state: WindowState::Unreachable,
                    generation: "g3".into(),
                    ..unknown.clone()
                },
            ]),
        ] {
            state.running = Some(RunningWindows {
                this: this.clone(),
                others,
                omitted: 0,
            });
            let response = Response::Snapshot(state.clone());
            assert_eq!(
                decode_response(&encode_response(&response).unwrap()).unwrap(),
                response
            );
        }
        state.running.as_mut().unwrap().this.window = None;
        let response = Response::Snapshot(state.clone());
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
        for index in 0..17 {
            let mut running = RunningWindows {
                this: this.clone(),
                others: Some(vec![unknown.clone()]),
                omitted: 0,
            };
            match index {
                0 => running.this.window = Some("abcdef0g".into()),
                1 => running.this.window = Some("abcdef0".into()),
                2 => running.this.generation.clear(),
                3 => running.this.name = Some("🦀".repeat(65)),
                4 => running.this.live_terminals = Some(MAX_SESSIONS as u16 + 1),
                5 => running.this.name = None,
                6 => running.this.started_at = None,
                7 => running.this.live_terminals = None,
                8 => running.this = unknown.clone(),
                9 => running.others = Some(vec![unknown.clone(); 33]),
                10 => running.others.as_mut().unwrap()[0].name = Some("forged".into()),
                11 => running.others.as_mut().unwrap()[0].state = WindowState::Live,
                12 => running.others.as_mut().unwrap()[0].started_at = Some(1),
                13 => running.others.as_mut().unwrap()[0].live_terminals = Some(1),
                14 => running.this.name = Some("unsafe\u{1b}name".into()),
                15 => running.this.name = Some("unsafe\nname".into()),
                16 => running.this.name = Some("unsafe\u{202e}name".into()),
                _ => unreachable!(),
            }
            state.running = Some(running);
            assert!(
                encode_response(&Response::Snapshot(state.clone())).is_err(),
                "case {index}"
            );
            assert!(decode_response(&forge(&state)).is_err(), "case {index}");
        }
        // Unknown option and state tags must fail before consumers see them.
        state.running = Some(RunningWindows {
            this,
            others: None,
            omitted: 0,
        });
        let bytes = encode_response(&Response::Snapshot(state.clone())).unwrap();
        // The running option byte precedes the trailing absent busy list.
        let offset = encode_response(&Response::Snapshot(Snapshot {
            running: None,
            ..state.clone()
        }))
        .unwrap()
        .len()
            - 2;
        for (offset, tag) in [(offset, 2), (bytes.len() - 5, 3)] {
            let mut forged = bytes.clone();
            forged[offset] = tag;
            assert!(decode_response(&forged).is_err());
        }
        let runtime = Runtime {
            generation: "g1".into(),
            name: Some("🦀".repeat(64)),
            started_at: u64::MAX,
            eon_version: "0.1.11".into(),
            workspace_protocol: VERSION,
            component_report: "ok".into(),
            sessions: vec![],
            attach: Availability {
                available: true,
                reason: "ready".into(),
            },
            stop: Availability {
                available: true,
                reason: "ready".into(),
            },
            busy: None,
        };
        let response = LifecycleResponse::Runtime(runtime.clone());
        assert_eq!(
            decode_lifecycle_response(&encode_lifecycle_response(&response).unwrap()).unwrap(),
            response
        );
        // An ASCII name can exceed the character bound without exceeding the byte bound.
        let mut forged = encode_lifecycle_response(&response).unwrap();
        let name_offset = forged.len() - 1 - 8 - 256 - 2;
        forged[name_offset..name_offset + 2].copy_from_slice(&65u16.to_le_bytes());
        forged.splice(
            name_offset + 2..forged.len() - 1 - 8,
            "a".repeat(65).bytes(),
        );
        let length = (forged.len() - HEADER_BYTES) as u32;
        forged[8..12].copy_from_slice(&length.to_le_bytes());
        assert!(decode_lifecycle_response(&forged).is_err());
        let bad = LifecycleResponse::Runtime(Runtime {
            name: Some("🦀".repeat(65)),
            ..runtime
        });
        assert!(encode_lifecycle_response(&bad).is_err());
    }

    #[test]
    fn pane_rename_tag_is_supported() {
        let mut payload = Encoder::default();
        payload.string("rename");
        payload.byte(0);
        payload.byte(36);
        payload.string("p1");
        payload.byte(1);
        payload.string("Review pane labels");
        let decoded = decode_request(&frame(REQUEST, payload.bytes).unwrap());
        assert!(decoded.is_ok(), "{decoded:?}");
    }

    #[test]
    fn pane_labels_and_pins_round_trip_and_reject_forged_bounds() {
        for name in [None, Some(String::new()), Some("🦀".repeat(80))] {
            let request = Request {
                id: "rename".into(),
                caller: None,
                action: Action::RenamePane {
                    pane: "p1".into(),
                    name,
                },
            };
            assert_eq!(
                decode_request(&encode_request(&request).unwrap()).unwrap(),
                request
            );
        }
        for name in [
            "x".repeat(81),
            "🦀".repeat(81),
            "bad\nname".into(),
            "bad\u{202e}name".into(),
            "bad\u{17b4}name".into(),
        ] {
            let request = Request {
                id: "rename".into(),
                caller: None,
                action: Action::RenamePane {
                    pane: "p1".into(),
                    name: Some(name.clone()),
                },
            };
            assert!(encode_request(&request).is_err());
            let mut payload = Encoder::default();
            payload.string("rename");
            payload.byte(0);
            payload.byte(RENAME_PANE);
            payload.string("p1");
            payload.byte(1);
            payload.string(&name);
            assert!(decode_request(&frame(REQUEST, payload.bytes).unwrap()).is_err());
        }
        let mut state = snapshot();
        let pane = &mut state.tabs[0].columns[0].panes[0];
        pane.id = "p".repeat(128);
        pane.label = format!("{}  {}", pane.id, "🦀".repeat(80));
        pane.pinned_name = Some("🦀".repeat(80));
        assert_eq!(pane.label.len(), 450);
        let response = Response::Snapshot(state.clone());
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
        for label in [
            "wrong identity".into(),
            format!("{}  {}", "p".repeat(128), "🦀".repeat(81)),
            format!("{}  bad\u{2066}name", "p".repeat(128)),
        ] {
            let mut invalid = state.clone();
            invalid.tabs[0].columns[0].panes[0].label = label;
            assert!(encode_response(&Response::Snapshot(invalid.clone())).is_err());
            assert!(decode_response(&forge(&invalid)).is_err());
        }
        state.tabs[0].columns[0].panes[0].pinned_name = Some("bad\u{200b}name".into());
        assert!(encode_response(&Response::Snapshot(state.clone())).is_err());
        assert!(decode_response(&forge(&state)).is_err());
    }

    fn terminal_target(id: TargetId, terminal: &str) -> TerminalTarget {
        TerminalTarget {
            workspace: "s1".into(),
            tab: "t1".into(),
            id,
            terminal: terminal.into(),
        }
    }

    #[test]
    fn history_alone_exceeds_two_mib_and_only_where_asked() {
        let longest = |id: char| id.to_string().repeat(MAX_ID_BYTES);
        let history = |text: String| TerminalHistory {
            target: TerminalTarget {
                workspace: longest('s'),
                tab: longest('t'),
                id: TargetId::Popup(longest('u')),
                terminal: longest('s'),
            },
            caller: Some(TerminalTarget {
                workspace: longest('s'),
                tab: longest('t'),
                id: TargetId::Pane(longest('p')),
                terminal: longest('s'),
            }),
            revision: u64::MAX,
            screen: Screen::Alternate,
            rows: u16::MAX,
            columns: u16::MAX,
            title: "t".repeat(MAX_TERMINAL_METADATA_BYTES / 2),
            directory: vec![0xff; MAX_TERMINAL_METADATA_BYTES / 2],
            omitted_lines: u32::MAX,
            cut_bytes: u32::MAX,
            text,
        };
        // The largest history is exactly the bound, and only the response
        // reader admits it.
        let largest = Response::History(history("\u{4e00}\n".repeat(MAX_HISTORY_TEXT_BYTES / 4)));
        let bytes = encode_response(&largest).unwrap();
        assert_eq!(bytes.len(), MAX_HISTORY_MESSAGE_BYTES);
        assert_eq!(declared_response_len(&bytes), Ok(bytes.len()));
        assert_eq!(
            declared_message_len(&bytes),
            Err(Error::MessageTooLarge {
                size: bytes.len(),
                maximum: MAX_MESSAGE_BYTES,
            })
        );
        assert_eq!(decode_response(&bytes), Ok(largest));
        let mut over = bytes.clone();
        over.push(0);
        let payload = (over.len() - HEADER_BYTES) as u32;
        over[8..12].copy_from_slice(&payload.to_le_bytes());
        let too_large = Error::MessageTooLarge {
            size: MAX_HISTORY_MESSAGE_BYTES + 1,
            maximum: MAX_HISTORY_MESSAGE_BYTES,
        };
        assert_eq!(declared_response_len(&over), Err(too_large.clone()));
        assert_eq!(decode_response(&over), Err(too_large));
        // Every other response keeps 2 MiB.
        let mut text = over;
        text[6] = TEXT;
        assert_eq!(
            declared_response_len(&text),
            Err(Error::MessageTooLarge {
                size: MAX_HISTORY_MESSAGE_BYTES + 1,
                maximum: MAX_MESSAGE_BYTES,
            })
        );

        let mut short = history("a".into());
        short.target = terminal_target(TargetId::Pane("p1".into()), "terminal-1");
        short.caller = None;
        let forge = |history: &TerminalHistory| {
            let mut payload = Encoder::default();
            encode_target(&mut payload, &history.target);
            encode_caller(&mut payload, history.caller.as_ref());
            payload
                .bytes
                .extend_from_slice(&history.revision.to_le_bytes());
            payload.byte(1);
            payload.number(history.rows);
            payload.number(history.columns);
            payload.string(&history.title);
            payload.raw(&history.directory);
            payload
                .bytes
                .extend_from_slice(&history.omitted_lines.to_le_bytes());
            payload
                .bytes
                .extend_from_slice(&history.cut_bytes.to_le_bytes());
            payload.long_raw(history.text.as_bytes());
            frame(HISTORY, payload.bytes).unwrap()
        };
        assert_eq!(
            decode_response(&forge(&short)),
            Ok(Response::History(short.clone()))
        );
        for text in ["a".repeat(MAX_HISTORY_TEXT_BYTES + 1), "a\u{1b}[0m".into()] {
            let invalid = TerminalHistory {
                text,
                ..short.clone()
            };
            assert_eq!(
                encode_response(&Response::History(invalid.clone())).map_err(field),
                Err("history text")
            );
            assert_eq!(
                decode_response(&forge(&invalid)).map_err(field),
                Err("history text")
            );
        }
    }

    #[test]
    fn terminal_requests_bound_their_paste_on_both_sides() {
        let request = |input| Request {
            id: "r1".into(),
            caller: None,
            action: Action::SendTerminal {
                target: "p1".into(),
                force: false,
                input,
            },
        };
        let forge = |text: &[u8]| {
            let mut payload = Encoder::default();
            payload.string("r1");
            payload.byte(0);
            payload.byte(SEND_TERMINAL);
            payload.string("p1");
            payload.byte(0); // force
            payload.byte(0);
            payload.long_raw(text);
            frame(REQUEST, payload.bytes).unwrap()
        };
        let longest = "a".repeat(MAX_PASTE_BYTES);
        let paste = request(TerminalInput::Paste(longest.clone()));
        assert_eq!(decode_request(&encode_request(&paste).unwrap()), Ok(paste));
        let multiline = request(TerminalInput::Paste("a\tb\r\nc".into()));
        assert_eq!(
            decode_request(&encode_request(&multiline).unwrap()),
            Ok(multiline)
        );
        let overlong = longest + "a";
        for (text, error) in [
            (
                overlong.as_str(),
                Error::FieldTooLong {
                    field: "paste",
                    length: MAX_PASTE_BYTES + 1,
                },
            ),
            ("", Error::InvalidValue { field: "paste" }),
            ("a\u{1b}[31m", Error::InvalidValue { field: "paste" }),
            ("a\u{7f}", Error::InvalidValue { field: "paste" }),
            ("a\u{9b}", Error::InvalidValue { field: "paste" }),
        ] {
            assert_eq!(validate_paste(text), Err(error.clone()), "{text:?}");
            assert_eq!(
                encode_request(&request(TerminalInput::Paste(text.into()))),
                Err(error.clone())
            );
            assert_eq!(decode_request(&forge(text.as_bytes())), Err(error));
        }
        assert_eq!(
            decode_request(&forge(b"\xff")),
            Err(Error::InvalidUtf8 { field: "paste" })
        );
        let mut enter = encode_request(&request(TerminalInput::Enter)).unwrap();
        *enter.last_mut().unwrap() = 2;
        assert_eq!(
            decode_request(&enter),
            Err(Error::InvalidTag {
                field: "terminal input",
                value: 2,
            })
        );
        let mut invalid_force = encode_request(&request(TerminalInput::Enter)).unwrap();
        let force_index = invalid_force.len() - 2;
        invalid_force[force_index] = 2;
        assert_eq!(
            decode_request(&invalid_force),
            Err(Error::InvalidValue { field: "force" })
        );
        for target in ["", "p 1"] {
            for action in [
                Action::ReadTerminal {
                    target: target.into(),
                },
                Action::ReadTerminalHistory {
                    target: target.into(),
                },
                Action::EditTerminalHistory {
                    target: target.into(),
                },
                Action::SendTerminal {
                    target: target.into(),
                    force: false,
                    input: TerminalInput::Enter,
                },
            ] {
                let request = Request {
                    id: "r1".into(),
                    caller: None,
                    action,
                };
                assert_eq!(
                    encode_request(&request).map_err(field),
                    Err("terminal target")
                );
            }
        }
    }

    #[test]
    fn terminal_text_is_bounded_plain_text_on_both_sides() {
        let text = |text: String| TerminalText {
            target: terminal_target(TargetId::Popup("u1".into()), "terminal-2"),
            caller: Some(terminal_target(TargetId::Pane("p1".into()), "terminal-1")),
            revision: u64::MAX,
            screen: Screen::Primary,
            rows: u16::MAX,
            columns: u16::MAX,
            title: "t".repeat(MAX_TERMINAL_METADATA_BYTES / 2),
            directory: vec![0xff; MAX_TERMINAL_METADATA_BYTES / 2],
            omitted_rows: u32::MAX,
            text,
        };
        // The largest read fits one frame.
        let largest = Response::Text(text("\u{4e00}\n".repeat(MAX_TERMINAL_TEXT_BYTES / 4)));
        let bytes = encode_response(&largest).unwrap();
        assert_eq!(decode_response(&bytes), Ok(largest));
        let forge = |text: &TerminalText| {
            let mut payload = Encoder::default();
            encode_target(&mut payload, &text.target);
            encode_caller(&mut payload, text.caller.as_ref());
            payload
                .bytes
                .extend_from_slice(&text.revision.to_le_bytes());
            payload.byte(0);
            payload.number(text.rows);
            payload.number(text.columns);
            payload.string(&text.title);
            payload.raw(&text.directory);
            payload
                .bytes
                .extend_from_slice(&text.omitted_rows.to_le_bytes());
            payload.long_raw(text.text.as_bytes());
            frame(TEXT, payload.bytes).unwrap()
        };
        let mut invalid = Vec::new();
        for value in ["\u{1b}[0m", "a\rb", "\t"] {
            invalid.push((text(value.into()), "terminal text"));
        }
        invalid.push((
            text("a".repeat(MAX_TERMINAL_TEXT_BYTES + 1)),
            "terminal text",
        ));
        let mut metadata = text("a".into());
        metadata.title.push('t');
        invalid.push((metadata, "terminal title and directory"));
        let mut tab = text("a".into());
        tab.target.tab = "t 1".into();
        invalid.push((tab, "target tab"));
        let mut caller = text("a".into());
        caller.caller.as_mut().unwrap().terminal = String::new();
        invalid.push((caller, "target terminal"));
        for (text, expected) in invalid {
            assert_eq!(
                encode_response(&Response::Text(text.clone())).map_err(field),
                Err(expected)
            );
            assert_eq!(decode_response(&forge(&text)).map_err(field), Err(expected));
        }

        let sent = |enter_delay_ms| {
            Response::Sent(Sent {
                target: terminal_target(TargetId::Pane("p1".into()), "terminal-1"),
                caller: None,
                bracketed: false,
                enter_delay_ms,
            })
        };
        let bytes = encode_response(&sent(MAX_ENTER_DELAY_MS)).unwrap();
        assert_eq!(decode_response(&bytes), Ok(sent(MAX_ENTER_DELAY_MS)));
        assert_eq!(
            encode_response(&sent(MAX_ENTER_DELAY_MS + 1)).map_err(field),
            Err("enter delay")
        );
        let mut forged = bytes;
        let end = forged.len();
        forged[end - 2..].copy_from_slice(&(MAX_ENTER_DELAY_MS + 1).to_le_bytes());
        assert_eq!(decode_response(&forged).map_err(field), Err("enter delay"));

        // An editing reply's popup is a popup, on either side.
        let editing = |popup| {
            Response::Editing(Editing {
                target: terminal_target(TargetId::Pane("p1".into()), "terminal-1"),
                caller: None,
                popup: terminal_target(popup, "terminal-2"),
            })
        };
        assert_eq!(
            encode_response(&editing(TargetId::Pane("p2".into()))).map_err(field),
            Err("editing popup")
        );
        let mut forged = encode_response(&editing(TargetId::Popup("u1".into()))).unwrap();
        // The popup's tag follows its workspace and tab, 17 bytes from the end.
        let tag = forged.len() - 17;
        assert_eq!(forged[tag], 1);
        forged[tag] = 0;
        assert_eq!(
            decode_response(&forged).map_err(field),
            Err("editing popup")
        );
    }

    fn field(error: Error) -> &'static str {
        match error {
            Error::InvalidTag { field, .. }
            | Error::InvalidUtf8 { field }
            | Error::InvalidValue { field }
            | Error::FieldTooLong { field, .. }
            | Error::InvalidSnapshot { field } => field,
            error => panic!("{error:?} names no field"),
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
                        caller: None,
                        action: Action::MovePaneSideways(side),
                    })
                    .is_err()
                );
                let mut payload = Encoder::default();
                payload.string(id);
                payload.byte(0);
                payload.byte(tag);
                let bytes = frame(REQUEST, payload.bytes).unwrap();
                assert!(decode_request(&bytes).is_err());
            }

            let mut payload = Encoder::default();
            payload.string("r1");
            let truncated = frame(REQUEST, payload.bytes.clone()).unwrap();
            assert_eq!(decode_request(&truncated), Err(Error::Truncated));
            payload.byte(0);
            payload.byte(tag);
            payload.byte(0);
            let trailing = frame(REQUEST, payload.bytes).unwrap();
            assert_eq!(
                decode_request(&trailing),
                Err(Error::TrailingBytes { count: 1 })
            );
        }
    }

    #[test]
    fn command_requests_bound_their_argv() {
        for tab in [false, true] {
            let request = |argv| Request {
                id: "r1".into(),
                caller: None,
                action: if tab {
                    Action::LaunchTab {
                        directory: b"/".to_vec(),
                        argv,
                        focus: true,
                    }
                } else {
                    Action::CreateCommandPane { argv, focus: true }
                },
            };
            let largest = request(vec![vec![b'a'; MAX_COMMAND_ARGUMENT_BYTES]]);
            let bytes = encode_request(&largest).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), largest);
            assert_eq!(
                encode_request(&request(vec![vec![b'a'; MAX_COMMAND_ARGUMENT_BYTES + 1]])),
                Err(Error::FieldTooLong {
                    field: "command argument",
                    length: MAX_COMMAND_ARGUMENT_BYTES + 1,
                })
            );
            assert_eq!(
                encode_request(&request(vec![b"a".to_vec(); MAX_COMMAND_ARGUMENTS + 1])),
                Err(Error::InvalidValue { field: "command" })
            );
        }
    }

    #[test]
    fn messages_keep_their_wire_bytes() {
        for (action, expected) in [
            (
                Action::Workspace(WorkspaceAction::CloseTab { tab: "t1".into() }),
                "454f4e57250001000a00000002007231001202007431",
            ),
            (
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: b"/tmp".to_vec(),
                }),
                "454f4e57250001001000000002007231000c0200743104002f746d70",
            ),
            (
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "git".into(),
                    expected_instance: Some("u1".into()),
                    intent: InvokeIntent::Focus,
                },
                "454f4e572500010015000000020072310013020074310300676974010200753101",
            ),
            (Action::CreateColumn, "454f4e572500010006000000020072310016"),
            (
                Action::ClosePane { pane: "p2".into() },
                "454f4e57250001000a00000002007231001702007032",
            ),
            (
                Action::MovePaneSideways(Side::Right),
                "454f4e572500010006000000020072310019",
            ),
            (
                Action::Edit {
                    file: b"/tmp/a".to_vec(),
                },
                "454f4e57250001000e00000002007231001a06002f746d702f61",
            ),
            (
                Action::CreateCommandPane {
                    argv: vec![b"hx".to_vec(), b"a".to_vec()],
                    focus: false,
                },
                "454f4e57250001001000000002007231001b00020002006878010061",
            ),
            (
                Action::LaunchTab {
                    directory: b"/tmp".to_vec(),
                    argv: vec![],
                    focus: true,
                },
                "454f4e57250001000f00000002007231002504002f746d70000001",
            ),
            (
                Action::LaunchTab {
                    directory: b"/tmp".to_vec(),
                    argv: vec![b"hx".to_vec(), b"a".to_vec()],
                    focus: false,
                },
                "454f4e57250001001600000002007231002504002f746d7002000200687801006100",
            ),
            (
                Action::ToggleSidebar,
                "454f4e57250001000600000002007231001c",
            ),
            (
                Action::FocusAttention {
                    tab: "t1".into(),
                    target: "p2".into(),
                },
                "454f4e57250001000e00000002007231001d0200743102007032",
            ),
            (
                Action::NextAttention,
                "454f4e57250001000600000002007231001e",
            ),
            (
                Action::PreviousAttention,
                "454f4e57250001000600000002007231001f",
            ),
            (
                Action::ReadTerminal {
                    target: "p2".into(),
                },
                "454f4e57250001000a00000002007231002602007032",
            ),
            (
                Action::ReadTerminalHistory {
                    target: "p2".into(),
                },
                "454f4e57250001000a00000002007231002902007032",
            ),
            (
                Action::EditTerminalHistory {
                    target: "p2".into(),
                },
                "454f4e57250001000a00000002007231002a02007032",
            ),
            (
                Action::SendTerminal {
                    target: "p2".into(),
                    force: false,
                    input: TerminalInput::Paste("hi".into()),
                },
                // v27's u32-length paste.
                "454f4e572500010012000000020072310027020070320000020000006869",
            ),
            (
                Action::SendTerminal {
                    target: "u1".into(),
                    force: true,
                    input: TerminalInput::Enter,
                },
                "454f4e57250001000c000000020072310027020075310101",
            ),
        ] {
            let request = Request {
                id: "r1".into(),
                caller: None,
                action,
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(hex(&bytes), expected, "{request:?}");
            assert_eq!(decode_request(&bytes).unwrap(), request);
        }
        let request = Request {
            id: "r1".into(),
            caller: Some("terminal-2.9-1".into()),
            action: Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        };
        let bytes = encode_request(&request).unwrap();
        assert_eq!(
            hex(&bytes),
            "454f4e57250001001700000002007231010e007465726d696e616c2d322e392d310201",
        );
        assert_eq!(decode_request(&bytes).unwrap(), request);
        for caller in ["", "session 2"] {
            let request = Request {
                caller: Some(caller.into()),
                ..request.clone()
            };
            assert!(encode_request(&request).is_err(), "{caller:?}");
        }

        let snapshot = Response::Snapshot(Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                name: None,
                tab_count: 1,
                attention: Some(AttentionState::NeedsInput),
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "git".into(),
                label: "Git".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: ALT,
                    key: "KeyG".into(),
                }),
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
                        panes: vec![Pane {
                            attention: Some(Attention {
                                state: AttentionState::NeedsInput,
                                record: Some(AttentionRecord {
                                    kind: Some(AttentionKind::Question),
                                    progress: None,
                                    app: "codex".into(),
                                    name: "eon-9a".into(),
                                    title: "t".into(),
                                    message: "m".into(),
                                    changed_at: 1,
                                    stale: false,
                                    others: 2,
                                }),
                            }),
                            ..pane("p3")
                        }],
                        expanded: "p3".into(),
                    },
                ],
                popups: vec![popup("u1", "git")],
                attention_rollup: Some(AttentionState::NeedsInput),
            }],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Fresh,
                observed_at: 100,
                windows: vec![QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 42,
                    resets_at: Some(500),
                    scope: None,
                }],
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Stale,
                observed_at: 90,
                windows: vec![
                    QuotaWindow {
                        duration_minutes: 10080,
                        remaining_percent: 7,
                        resets_at: None,
                        scope: None,
                    },
                    QuotaWindow {
                        duration_minutes: 10080,
                        remaining_percent: 60,
                        resets_at: None,
                        scope: Some("Fable".into()),
                    },
                ],
            }),
            codex_quota_failure: None,
            claude_quota_failure: Some(QuotaFailure {
                at: 95,
                reason: QuotaFailureReason::Deadline,
            }),
            sidebar: Sidebar::Closed,
            attention_queue: Vec::new(),
            running: None,
            busy: None,
        });
        let failure = Response::Failure(Failure {
            code: "busy".into(),
            detail: "no".into(),
        });
        for (response, expected) in [
            (
                snapshot,
                concat!(
                    "454f4e57250081001c0100000100020002000100010000000200743100000041",
                    "00008040010003006769740300476974000404004b6579470100020074310400",
                    "2f746d700001020070330102007531030002007031030073703103002f703101",
                    "02007031000002007032030073703203002f7032010200703200000200703303",
                    "0073703303002f70330102007033000102ff0500636f6465780600656f6e2d39",
                    "6101007401006d01000000000000000002000100020075310300676974030073",
                    "753103002f753100010100640000000000000001002c0100002a01f401000000",
                    "0000000001015a00000000000000020060270000070000602700003c00010500",
                    "4661626c6500015f00000000000000010102007331020073310100",
                    // s1's tab count, then v22's needs-input rollup and shown rank 0.
                    "02007331000100",
                    "0100",
                    // v27's empty attention queue, v28's absent running summary,
                    // then v29's absent busy list.
                    "00000000",
                ),
            ),
            (failure, "454f4e57250082000a00000004006275737902006e6f"),
            (
                Response::Text(TerminalText {
                    target: terminal_target(TargetId::Pane("p2".into()), "terminal-2"),
                    caller: None,
                    revision: 7,
                    screen: Screen::Alternate,
                    rows: 24,
                    columns: 80,
                    title: "t".into(),
                    directory: b"/".to_vec(),
                    omitted_rows: 0,
                    text: "a\nb".into(),
                }),
                concat!(
                    "454f4e572500860038000000",
                    // Workspace, tab, pane p2 and its terminal; no caller.
                    "020073310200743100020070320a007465726d696e616c2d3200",
                    // Revision, alternate screen, 24x80, title, directory,
                    // omitted rows, then the u32-length text.
                    "0700000000000000011800500001007401002f0000000003000000610a62",
                ),
            ),
            (
                Response::History(TerminalHistory {
                    target: terminal_target(TargetId::Pane("p2".into()), "terminal-2"),
                    caller: None,
                    revision: 7,
                    screen: Screen::Primary,
                    rows: 24,
                    columns: 80,
                    title: "t".into(),
                    directory: b"/".to_vec(),
                    omitted_lines: 2,
                    cut_bytes: 1,
                    text: "a\nb".into(),
                }),
                concat!(
                    "454f4e5725008d003c000000",
                    "020073310200743100020070320a007465726d696e616c2d3200",
                    // Revision, primary screen, 24x80, title, directory,
                    // omitted lines, cut bytes, then the u32-length text.
                    "0700000000000000001800500001007401002f",
                    "020000000100000003000000610a62",
                ),
            ),
            (
                Response::Sent(Sent {
                    target: terminal_target(TargetId::Pane("p2".into()), "terminal-2"),
                    caller: Some(terminal_target(TargetId::Popup("u1".into()), "terminal-3")),
                    bracketed: true,
                    enter_delay_ms: 300,
                }),
                concat!(
                    "454f4e572500870036000000",
                    "020073310200743100020070320a007465726d696e616c2d32",
                    "01020073310200743101020075310a007465726d696e616c2d33",
                    "012c01",
                ),
            ),
            (
                Response::Editing(Editing {
                    target: terminal_target(TargetId::Pane("p2".into()), "terminal-2"),
                    caller: None,
                    popup: terminal_target(TargetId::Popup("u1".into()), "terminal-3"),
                }),
                concat!(
                    "454f4e5725008e0033000000",
                    // Pane p2 and its terminal, no caller, then popup u1 and
                    // its terminal.
                    "020073310200743100020070320a007465726d696e616c2d3200",
                    "020073310200743101020075310a007465726d696e616c2d33",
                ),
            ),
            (
                Response::Ended(Ended {
                    generation: "g1".into(),
                }),
                "454f4e57250085000400000002006731",
            ),
        ] {
            let bytes = encode_response(&response).unwrap();
            assert_eq!(hex(&bytes), expected, "{response:?}");
            assert_eq!(decode_response(&bytes).unwrap(), response);
        }

        let (response, expected) = (
            LifecycleResponse::Runtime(Runtime {
                generation: "g1".into(),
                name: None,
                started_at: 0,
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
                busy: None,
            }),
            concat!(
                "454f4e572500830030000000020067310500302e392e30250002006f6b010002",
                "007331010500726561647900040062757379",
                // v28: absent name, supervisor start in Unix milliseconds.
                "000000000000000000",
                // v29: absent busy list.
                "00",
            ),
        );
        let bytes = encode_lifecycle_response(&response).unwrap();
        assert_eq!(hex(&bytes), expected, "{response:?}");
        assert_eq!(decode_lifecycle_response(&bytes).unwrap(), response);
    }

    fn attention_queue_snapshot() -> Snapshot {
        let attention = Attention {
            state: AttentionState::NeedsInput,
            record: Some(AttentionRecord {
                kind: Some(AttentionKind::Permission),
                progress: Some(40),
                app: String::new(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1,
                stale: false,
                others: 0,
            }),
        };
        Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                name: None,
                tab_count: 1,
                attention: Some(AttentionState::NeedsInput),
                recency: 0,
            }],
            active_tab: "t".into(),
            geometry: PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: Vec::new(),
            tabs: vec![Tab {
                id: "t".into(),
                directory: b"/".to_vec(),
                pending: false,
                selected_pane: Some("p".into()),
                selected_popup: None,
                columns: vec![Column {
                    panes: vec![Pane {
                        attention: Some(attention.clone()),
                        ..pane("p")
                    }],
                    expanded: "p".into(),
                }],
                popups: Vec::new(),
                attention_rollup: Some(AttentionState::NeedsInput),
            }],
            codex_quota: None,
            claude_quota: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            sidebar: Sidebar::Off,
            attention_queue: vec![QueueEntry {
                workspace: "s1".into(),
                tab: "t".into(),
                target: "p".into(),
                attention,
            }],
            running: None,
            busy: None,
        }
    }

    #[test]
    fn attention_queue_keeps_its_wire_bytes() {
        let snapshot = Response::Snapshot(attention_queue_snapshot());
        let bytes = encode_response(&snapshot).unwrap();
        assert_eq!(
            hex(&bytes),
            concat!(
                "454f4e57250081008a00000001000100010000000100740000000000000000",
                "0000010001007401002f00010100700001000100700200737002002f700101007000",
                "010128000000000000000001000000000000000000000000010000000002",
                "02007331020073310100020073310001000100",
                // u16 count, workspace, tab, target, then needs-input permission at 40%.
                "0100020073310100740100700101280000000000000000010000000000000000000000",
                // v29's absent busy list.
                "00",
            ),
        );
        assert_eq!(decode_response(&bytes).unwrap(), snapshot);
    }

    #[test]
    fn running_windows_keep_their_wire_bytes() {
        let mut snapshot = attention_queue_snapshot();
        snapshot.running = Some(RunningWindows {
            this: WindowEntry {
                window: Some("1234abcd".into()),
                generation: "g1".into(),
                name: Some("here".into()),
                started_at: Some(42),
                live_terminals: Some(3),
                state: WindowState::Live,
            },
            others: Some(vec![WindowEntry {
                window: None,
                generation: "g2".into(),
                name: None,
                started_at: None,
                live_terminals: None,
                state: WindowState::Incompatible,
            }]),
            omitted: 7,
        });
        let response = Response::Snapshot(snapshot);
        let bytes = encode_response(&response).unwrap();
        assert_eq!(
            hex(&bytes),
            concat!(
                "454f4e5725008100bb0000000100010001000000010074000000000000000000",
                "00010001007401002f00010100700001000100700200737002002f7001010070",
                "0001012800000000000000000100000000000000000000000001000000000202",
                "0073310200733101000200733100010001000100020073310100740100700101",
                "2800000000000000000100000000000000000000010108003132333461626364",
                "0200673101040068657265012a00000000000000010300000101000002006732",
                "000000010700",
                // v29's absent busy list.
                "00",
            )
        );
        assert_eq!(decode_response(&bytes).unwrap(), response);
    }

    #[test]
    fn busy_terminals_keep_their_wire_bytes() {
        let busy = BusyTerminals {
            terminals: vec![
                BusyTerminal {
                    target: terminal_target(TargetId::Pane("p2".into()), "s2"),
                    name: "p2  vim".into(),
                },
                BusyTerminal {
                    target: terminal_target(TargetId::Popup("u1".into()), "s3"),
                    name: "Agent".into(),
                },
            ],
            omitted: 0,
        };
        let busy_bytes = concat!(
            // Present, two terminals: workspace, tab, pane or popup, terminal,
            // name; then none omitted.
            "010200",
            "020073310200743100020070320200733207007032202076696d",
            "020073310200743101020075310200733305004167656e74",
            "0000",
        );
        let mut snapshot = attention_queue_snapshot();
        snapshot.busy = Some(busy.clone());
        let response = Response::Snapshot(snapshot.clone());
        let bytes = encode_response(&response).unwrap();
        snapshot.busy = None;
        let without = hex(&encode_response(&Response::Snapshot(snapshot)).unwrap());
        // The list replaces the absent byte at the snapshot's end.
        assert_eq!(
            &hex(&bytes)[24..],
            format!("{}{busy_bytes}", &without[24..without.len() - 2])
        );
        assert_eq!(decode_response(&bytes).unwrap(), response);

        let runtime = LifecycleResponse::Runtime(Runtime {
            generation: "g1".into(),
            name: None,
            started_at: 0,
            eon_version: "0.9.0".into(),
            workspace_protocol: VERSION,
            component_report: "ok".into(),
            sessions: vec![],
            attach: Availability {
                available: true,
                reason: "ready".into(),
            },
            stop: Availability {
                available: true,
                reason: "ready".into(),
            },
            busy: Some(busy),
        });
        let bytes = encode_lifecycle_response(&runtime).unwrap();
        assert_eq!(
            hex(&bytes),
            concat!(
                "454f4e572500830063000000020067310500302e392e30250002006f6b0000",
                "01050072656164790105007265616479",
                "000000000000000000",
            )
            .to_owned()
                + busy_bytes
        );
        assert_eq!(decode_lifecycle_response(&bytes).unwrap(), runtime);
    }

    #[test]
    fn frames_reject_other_versions_and_malformed_headers() {
        let request = encode_request(&Request {
            id: "r1".into(),
            caller: None,
            action: Action::Workspace(WorkspaceAction::Inspect),
        })
        .unwrap();
        let edit = |offset: usize, bytes: &[u8]| {
            let mut message = request.clone();
            message[offset..offset + bytes.len()].copy_from_slice(bytes);
            message
        };
        for (message, error) in [
            (edit(0, b"EONX"), Error::InvalidMagic),
            (
                edit(4, &12_u16.to_le_bytes()),
                Error::UnsupportedVersion { version: 12 },
            ),
            (edit(6, &[SNAPSHOT]), Error::InvalidKind { value: SNAPSHOT }),
            (edit(7, &[1]), Error::InvalidFlags { value: 1 }),
            (
                edit(8, &u32::MAX.to_le_bytes()),
                Error::MessageTooLarge {
                    size: HEADER_BYTES + u32::MAX as usize,
                    maximum: MAX_MESSAGE_BYTES,
                },
            ),
            (
                [request.as_slice(), &[0][..]].concat(),
                Error::TrailingBytes { count: 1 },
            ),
            (
                vec![0; MAX_MESSAGE_BYTES + 1],
                Error::MessageTooLarge {
                    size: MAX_MESSAGE_BYTES + 1,
                    maximum: MAX_MESSAGE_BYTES,
                },
            ),
        ] {
            assert_eq!(decode_request(&message), Err(error));
        }
        for end in 0..request.len() {
            assert_eq!(decode_request(&request[..end]), Err(Error::Truncated));
        }
        assert_eq!(declared_message_len(&request), Ok(request.len()));
        assert_eq!(
            declared_message_len(&request[..HEADER_BYTES - 1]),
            Err(Error::Truncated)
        );

        // Workspace and lifecycle replies share only the failure kind.
        let state = encode_response(&Response::Snapshot(snapshot())).unwrap();
        assert_eq!(
            decode_lifecycle_response(&state),
            Err(Error::InvalidKind { value: SNAPSHOT })
        );
        let ended = encode_response(&Response::Ended(Ended {
            generation: "g1".into(),
        }))
        .unwrap();
        assert_eq!(
            decode_lifecycle_response(&ended),
            Err(Error::InvalidKind { value: ENDED })
        );
        let failure = Failure {
            code: "busy".into(),
            detail: "no".into(),
        };
        let bytes = encode_response(&Response::Failure(failure.clone())).unwrap();
        assert_eq!(
            decode_lifecycle_response(&bytes),
            Ok(LifecycleResponse::Failure(failure))
        );
    }

    #[test]
    fn requests_keep_their_tags_and_reject_malformed_actions() {
        let frame_action = |id: &str, action: &[u8]| {
            let mut payload = Encoder::default();
            payload.string(id);
            payload.byte(0);
            payload.bytes.extend_from_slice(action);
            frame(REQUEST, payload.bytes).unwrap()
        };
        let w = Action::Workspace;
        let target = |instance: &str| PopupTarget {
            tab: "t1".into(),
            instance: instance.into(),
        };
        let invoke = |entry: &str, intent| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: None,
            intent,
        };

        let valid: [(Action, &[u8]); 20] = [
            (w(WorkspaceAction::Inspect), &[0]),
            (w(WorkspaceAction::CreateTab), &[1]),
            (w(WorkspaceAction::CreatePane { focus: false }), &[2, 0]),
            (w(WorkspaceAction::CreatePane { focus: true }), &[2, 1]),
            (
                w(WorkspaceAction::FocusId("p1".into())),
                &[3, 2, 0, b'p', b'1'],
            ),
            (w(WorkspaceAction::Focus(Direction::Left)), &[4]),
            (w(WorkspaceAction::Focus(Direction::Right)), &[5]),
            (w(WorkspaceAction::Focus(Direction::Up)), &[6]),
            (w(WorkspaceAction::Focus(Direction::Down)), &[7]),
            (w(WorkspaceAction::InspectRuntime), &[8]),
            (w(WorkspaceAction::Present { workspace: false }), &[10, 0]),
            (w(WorkspaceAction::Present { workspace: true }), &[10, 1]),
            (w(WorkspaceAction::InspectPresentation), &[11]),
            (w(WorkspaceAction::Move(Direction::Left)), &[14]),
            (w(WorkspaceAction::Move(Direction::Right)), &[15]),
            (w(WorkspaceAction::Move(Direction::Up)), &[16]),
            (w(WorkspaceAction::Move(Direction::Down)), &[17]),
            (
                invoke("git", InvokeIntent::Toggle),
                &[19, 2, 0, b't', b'1', 3, 0, b'g', b'i', b't', 0, 0],
            ),
            (
                Action::DismissPopup(target("u1")),
                &[20, 2, 0, b't', b'1', 2, 0, b'u', b'1'],
            ),
            (Action::MovePaneSideways(Side::Left), &[24]),
        ];
        for (action, bytes) in valid {
            let request = Request {
                id: "r1".into(),
                caller: None,
                action,
            };
            let bytes = frame_action("r1", bytes);
            assert_eq!(encode_request(&request).unwrap(), bytes, "{request:?}");
            assert_eq!(decode_request(&bytes).unwrap(), request);
        }

        // Neither side accepts what the other would reject.
        let invalid: [(&str, Action, &[u8], &str); 17] = [
            ("r 1", w(WorkspaceAction::Inspect), &[0], "request id"),
            (
                "r1",
                w(WorkspaceAction::FocusId(String::new())),
                &[3, 0, 0],
                "focus id",
            ),
            (
                "r1",
                w(WorkspaceAction::FocusId("x".repeat(MAX_ID_BYTES + 1))),
                &[3, MAX_ID_BYTES as u8 + 1, 0],
                "focus id",
            ),
            (
                "r1",
                w(WorkspaceAction::SetTabDirectory {
                    tab: String::new(),
                    directory: b"/".to_vec(),
                }),
                &[12, 0, 0, 1, 0, b'/'],
                "tab id",
            ),
            (
                "r1",
                w(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: vec![],
                }),
                &[12, 2, 0, b't', b'1', 0, 0],
                "directory",
            ),
            (
                "r1",
                w(WorkspaceAction::CloseTab { tab: String::new() }),
                &[18, 0, 0],
                "tab id",
            ),
            (
                "r1",
                invoke("", InvokeIntent::Toggle),
                &[19, 2, 0, b't', b'1', 0, 0, 0, 0],
                "entry id",
            ),
            (
                "r1",
                Action::DismissPopup(target("u 1")),
                &[20, 2, 0, b't', b'1', 3, 0, b'u', b' ', b'1'],
                "popup id",
            ),
            (
                "r1",
                Action::ClosePane {
                    pane: String::new(),
                },
                &[23, 0, 0],
                "pane id",
            ),
            (
                "r1",
                Action::ClosePane { pane: "p 2".into() },
                &[23, 3, 0, b'p', b' ', b'2'],
                "pane id",
            ),
            (
                "r1",
                Action::Edit {
                    file: b"a".to_vec(),
                },
                &[26, 1, 0, b'a'],
                "file",
            ),
            (
                "r1",
                Action::Edit {
                    file: b"/a\0".to_vec(),
                },
                &[26, 3, 0, b'/', b'a', 0],
                "file",
            ),
            (
                "r1",
                Action::CreateCommandPane {
                    argv: Vec::new(),
                    focus: true,
                },
                &[27, 1, 0, 0],
                "command",
            ),
            (
                "r1",
                Action::CreateCommandPane {
                    argv: vec![Vec::new()],
                    focus: true,
                },
                &[27, 1, 1, 0, 0, 0],
                "command",
            ),
            (
                "r1",
                Action::CreateCommandPane {
                    argv: vec![b"hx".to_vec(), b"a\0".to_vec()],
                    focus: true,
                },
                &[27, 1, 2, 0, 2, 0, b'h', b'x', 2, 0, b'a', 0],
                "command argument",
            ),
            (
                "r1",
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: vec![vec![]],
                    focus: true,
                },
                &[37, 1, 0, b'/', 1, 0, 0, 0, 1],
                "command",
            ),
            (
                "r1",
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: vec![b"a\0".to_vec()],
                    focus: false,
                },
                &[37, 1, 0, b'/', 1, 0, 2, 0, b'a', 0, 0],
                "command argument",
            ),
        ];
        for (id, action, bytes, expected) in invalid {
            let request = Request {
                id: id.into(),
                caller: None,
                action,
            };
            let encoded = encode_request(&request).unwrap_err();
            assert_eq!(field(encoded), expected, "{request:?}");
            let decoded = decode_request(&frame_action(id, bytes)).unwrap_err();
            assert_eq!(field(decoded), expected, "{request:?}");
        }

        let decode_only: [(&[u8], Error); 14] = [
            // The retired picker opcode.
            (
                &[13],
                Error::InvalidTag {
                    field: "action",
                    value: 13,
                },
            ),
            // v30's CommitDirectory: a pick is SetTabDirectory (EON-C18).
            (
                &[
                    21, 2, 0, b't', b'1', 2, 0, b'u', b'1', 4, 0, b'/', b't', b'm', b'p',
                ],
                Error::InvalidTag {
                    field: "action",
                    value: 21,
                },
            ),
            (
                &[43],
                Error::InvalidTag {
                    field: "action",
                    value: 43,
                },
            ),
            (
                &[37, 1, 0, b'/', 1, 1],
                Error::InvalidSnapshot { field: "command" },
            ),
            (
                &[37, 1, 0, b'/', 1, 0, 1, 128],
                Error::FieldTooLong {
                    field: "command argument",
                    length: MAX_COMMAND_ARGUMENT_BYTES + 1,
                },
            ),
            (
                &[37, 1, 0, b'/', 0, 0, 2],
                Error::InvalidValue { field: "tab focus" },
            ),
            (
                &[29, 2, 0, b't', b'1', 0, 0],
                Error::InvalidValue {
                    field: "attention target",
                },
            ),
            (
                &[10, 2],
                Error::InvalidValue {
                    field: "presentation mode",
                },
            ),
            (&[2], Error::Truncated),
            (
                &[2, 2],
                Error::InvalidValue {
                    field: "pane focus",
                },
            ),
            (
                &[27, 2, 1, 0, 1, 0, b'a'],
                Error::InvalidValue {
                    field: "pane focus",
                },
            ),
            (
                &[19, 2, 0, b't', b'1', 3, 0, b'g', b'i', b't', 2],
                Error::InvalidValue {
                    field: "expected instance",
                },
            ),
            (
                &[19, 2, 0, b't', b'1', 3, 0, b'g', b'i', b't', 0, 2],
                Error::InvalidTag {
                    field: "popup intent",
                    value: 2,
                },
            ),
            (&[0, 0], Error::TrailingBytes { count: 1 }),
        ];
        for (bytes, error) in decode_only {
            assert_eq!(decode_request(&frame_action("r1", bytes)), Err(error));
        }
    }

    #[test]
    fn popup_actions_target_the_observed_instance() {
        let target = |tab: &str, instance: &str| PopupTarget {
            tab: tab.into(),
            instance: instance.into(),
        };
        let invoke = |entry: &str, expected: Option<&str>, intent| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: expected.map(Into::into),
            intent,
        };
        let mut state = snapshot();
        let observed = [
            invoke("agent", Some("u1"), InvokeIntent::Toggle),
            invoke("agent", Some("u1"), InvokeIntent::Focus),
            Action::DismissPopup(target("t1", "u1")),
            Action::DismissPopup(target("t2", "u2")),
        ];
        for action in &observed {
            assert_eq!(check_popup_action(&state, action), Ok(()), "{action:?}");
        }

        // A replacement has a fresh id, so requests aimed at the old instance fail.
        state.tabs[0].popups[0] = popup("u5", "agent");
        state.tabs[1].popups[0] = popup("u6", FILES_ENTRY);
        state.tabs[1].selected_popup = Some("u6".into());
        for action in &observed {
            assert!(check_popup_action(&state, action).is_err(), "{action:?}");
        }

        let checks = |state: &Snapshot, cases: &[(Action, bool)]| {
            for (action, valid) in cases {
                let result = check_popup_action(state, action);
                assert_eq!(result.is_ok(), *valid, "{action:?}: {result:?}");
            }
        };
        checks(
            &state,
            &[
                (invoke(PROJECT_ENTRY, None, InvokeIntent::Toggle), true),
                (invoke(FILES_ENTRY, None, InvokeIntent::Toggle), true),
                (invoke("agent", None, InvokeIntent::Toggle), false),
                (invoke("missing", None, InvokeIntent::Toggle), false),
                (Action::DismissPopup(target("t2", "u6")), true),
                (Action::DismissPopup(target("t3", "u3")), true),
                (Action::DismissPopup(target("t1", "u3")), false),
            ],
        );
        // Project and Files both observe the tab's one Files pair.
        state.tabs[0].popups.push(popup("u7", FILES_ENTRY));
        checks(
            &state,
            &[
                (invoke(PROJECT_ENTRY, None, InvokeIntent::Toggle), false),
                (invoke(FILES_ENTRY, None, InvokeIntent::Toggle), false),
                (
                    invoke(PROJECT_ENTRY, Some("u7"), InvokeIntent::Toggle),
                    true,
                ),
                (invoke(FILES_ENTRY, Some("u7"), InvokeIntent::Focus), true),
            ],
        );
    }

    #[test]
    fn attention_queue_round_trips_and_rejects_invalid_entries_on_both_sides() {
        let mut state = snapshot();
        state.workspaces.push(WorkspaceInfo {
            id: "s2".into(),
            name: None,
            tab_count: 1,
            attention: None,
            recency: 1,
        });
        state.attention_queue = [
            AttentionState::NeedsInput,
            AttentionState::Error,
            AttentionState::Finished,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, class)| QueueEntry {
            workspace: if index == 0 { "s2" } else { "s1" }.into(),
            tab: "t1".into(),
            target: "p2".into(),
            attention: attention(class).unwrap(),
        })
        .collect();
        let response = Response::Snapshot(state.clone());
        let bytes = encode_response(&response).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), response);
        assert_eq!(forge(&state), bytes);
        let invalid: &[fn(&mut QueueEntry)] = &[
            |entry| entry.attention.state = AttentionState::Working,
            |entry| entry.attention.state = AttentionState::Idle,
            |entry| entry.attention.state = AttentionState::Background,
            |entry| {
                entry.attention = Attention {
                    state: AttentionState::Unavailable,
                    record: None,
                }
            },
            |entry| entry.attention.record = None,
            |entry| entry.workspace = "s3".into(),
            |entry| entry.workspace.clear(),
            |entry| entry.tab = "bad id".into(),
            |entry| entry.target.clear(),
            |entry| entry.workspace = "w".repeat(MAX_ID_BYTES + 1),
            |entry| entry.tab = "t".repeat(MAX_ID_BYTES + 1),
            |entry| entry.target = "p".repeat(MAX_ID_BYTES + 1),
            |entry| entry.attention.record.as_mut().unwrap().progress = Some(101),
        ];
        for change in invalid {
            let mut bad = state.clone();
            change(&mut bad.attention_queue[0]);
            assert!(
                encode_response(&Response::Snapshot(bad.clone())).is_err(),
                "{bad:?}"
            );
            assert!(decode_response(&forge(&bad)).is_err(), "{bad:?}");
        }
        state.attention_queue = vec![state.attention_queue[0].clone(); MAX_SESSIONS + 1];
        assert!(encode_response(&Response::Snapshot(state.clone())).is_err());
        assert!(decode_response(&forge(&state)).is_err());

        // A queue entry is mandatory attention, so its absent wire tag is invalid.
        let mut empty = snapshot();
        empty.attention_queue.push(QueueEntry {
            workspace: "s1".into(),
            tab: "t1".into(),
            target: "p2".into(),
            attention: Attention {
                state: AttentionState::Unavailable,
                record: None,
            },
        });
        let mut bytes = forge(&empty);
        *bytes.last_mut().unwrap() = 0;
        assert!(decode_response(&bytes).is_err());
    }

    #[test]
    fn snapshots_reject_broken_ownership_on_both_sides() {
        let state = snapshot();
        let bytes = encode_response(&Response::Snapshot(state.clone())).unwrap();
        assert_eq!(forge(&state), bytes);

        let valid: [fn(&mut Snapshot); 12] = [
            |_| {},
            |s| s.sidebar = Sidebar::Off,
            // An entry may be keyless, or overridden by another listed entry.
            |s| s.entries[0].shortcut = PopupKey::Unbound,
            |s| s.entries[0].shortcut = overridden("KeyL", "agent"),
            // The states, kinds and visibility the fixture leaves out.
            |s| {
                s.sidebar = Sidebar::Open;
                working_attention(s).state = AttentionState::Error;
                asking(s).kind = Some(AttentionKind::Auth);
            },
            |s| {
                working_attention(s).state = AttentionState::Idle;
                asking(s).kind = Some(AttentionKind::Permission);
            },
            |s| working_attention(s).state = AttentionState::Background,
            |s| {
                s.codex_quota = None;
                s.claude_quota = None;
            },
            |s| {
                let quota = s.codex_quota.as_mut().unwrap();
                quota.state = CodexQuotaState::Blocked;
                quota.windows.clear();
            },
            |s| {
                let quota = s.codex_quota.as_mut().unwrap();
                quota.state = CodexQuotaState::Unknown;
                quota.windows.clear();
            },
            |s| {
                s.codex_quota.as_mut().unwrap().state = CodexQuotaState::Stale;
                s.claude_quota.as_mut().unwrap().state = ClaudeQuotaState::Stale;
            },
            // A reading of scoped windows alone, the longest scope, and every failure reason.
            |s| {
                s.claude_quota.as_mut().unwrap().windows.drain(..2);
                s.claude_quota.as_mut().unwrap().windows[0].scope =
                    Some("x".repeat(MAX_QUOTA_SCOPE_BYTES));
            },
        ];
        for change in valid {
            let mut candidate = snapshot();
            change(&mut candidate);
            let response = Response::Snapshot(candidate);
            let bytes = encode_response(&response).unwrap();
            assert_eq!(decode_response(&bytes).unwrap(), response);
        }
        for reason in [
            QuotaFailureReason::Start,
            QuotaFailureReason::Deadline,
            QuotaFailureReason::OutputLimit,
            QuotaFailureReason::ExitStatus,
            QuotaFailureReason::Malformed,
            QuotaFailureReason::ProviderError,
            QuotaFailureReason::Estimated,
            QuotaFailureReason::ObservationTime,
            QuotaFailureReason::InvalidWindow,
        ] {
            let mut candidate = snapshot();
            candidate.codex_quota_failure = Some(QuotaFailure { at: 1, reason });
            let response = Response::Snapshot(candidate);
            let bytes = encode_response(&response).unwrap();
            assert_eq!(decode_response(&bytes).unwrap(), response);
        }

        fn codex(s: &mut Snapshot) -> &mut CodexQuota {
            s.codex_quota.as_mut().unwrap()
        }
        fn claude(s: &mut Snapshot) -> &mut ClaudeQuota {
            s.claude_quota.as_mut().unwrap()
        }
        /// p3's working attention.
        fn working_attention(s: &mut Snapshot) -> &mut Attention {
            s.tabs[0].columns[1].panes[0].attention.as_mut().unwrap()
        }
        /// p3's working record.
        fn working(s: &mut Snapshot) -> &mut AttentionRecord {
            working_attention(s).record.as_mut().unwrap()
        }
        /// p2's needs-input record.
        fn asking(s: &mut Snapshot) -> &mut AttentionRecord {
            let pane = &mut s.tabs[0].columns[0].panes[1];
            pane.attention.as_mut().unwrap().record.as_mut().unwrap()
        }
        fn bound(s: &mut Snapshot, entry: usize) -> &mut Shortcut {
            match &mut s.entries[entry].shortcut {
                PopupKey::Bound(shortcut) => shortcut,
                key => panic!("{key:?}"),
            }
        }
        fn overridden(key: &str, by: &str) -> PopupKey {
            PopupKey::Overridden {
                default: Shortcut {
                    modifiers: ALT | SHIFT,
                    key: key.into(),
                },
                by: by.into(),
            }
        }
        type Change = fn(&mut Snapshot);
        let invalid: [(&str, &[Change]); 29] = [
            // Every tab, pane and popup owns its identity, session and endpoint.
            (
                "surface identity",
                &[
                    |s| s.tabs[0].popups[0].id = "p1".into(),
                    |s| s.tabs[0].popups[0].session = "sp1".into(),
                    |s| s.tabs[0].popups[0].endpoint = b"/p1".to_vec(),
                    |s| s.tabs[3].popups[0].endpoint = b"/u2".to_vec(),
                ],
            ),
            ("tab id", &[|s| s.tabs[2].id = "t1".into()]),
            (
                "popup entry",
                &[
                    |s| s.tabs[0].popups.push(popup("u5", "agent")),
                    |s| s.tabs[0].popups[0].entry = "missing".into(),
                ],
            ),
            (
                "selected popup",
                &[|s| s.tabs[0].selected_popup = Some("u3".into())],
            ),
            (
                "selected pane",
                &[
                    |s| s.tabs[0].selected_pane = Some("u1".into()),
                    |s| s.tabs[0].selected_pane = None,
                    |s| s.tabs[2].selected_pane = Some("p1".into()),
                    // The focus is an expanded pane.
                    |s| s.tabs[0].selected_pane = Some("p1".into()),
                ],
            ),
            ("empty tab", &[|s| s.tabs[2].popups.clear()]),
            ("active tab", &[|s| s.active_tab = "t5".into()]),
            // A pending tab holds exactly one selected Files pair.
            (
                "pending tab",
                &[
                    |s| s.tabs[0].pending = true,
                    |s| s.tabs[1].selected_popup = None,
                    |s| s.tabs[1].popups[0].entry = "agent".into(),
                ],
            ),
            // Project never has a popup of its own.
            (
                "popup entry",
                &[|s| s.tabs[0].popups[0].entry = PROJECT_ENTRY.into()],
            ),
            // Bound shortcuts are unique, and an overriding entry is another
            // listed one bound to the overridden default.
            (
                "entries",
                &[
                    |s| s.entries[1].id = PROJECT_ENTRY.into(),
                    |s| bound(s, 1).key = "KeyZ".into(),
                    |s| s.entries[0].shortcut = overridden("KeyL", "missing"),
                    |s| s.entries[1].shortcut = overridden("KeyL", "agent"),
                    |s| s.entries[0].shortcut = overridden("KeyZ", "agent"),
                ],
            ),
            (
                "shortcut",
                &[
                    |s| bound(s, 0).modifiers = 128,
                    |s| bound(s, 0).key = "keyz".into(),
                    |s| s.entries[0].shortcut = overridden("keyl", "agent"),
                ],
            ),
            (
                "entry id",
                &[
                    |s| s.entries[0].id = "x".repeat(MAX_ENTRY_ID_BYTES + 1),
                    |s| {
                        let by = "x".repeat(MAX_ENTRY_ID_BYTES + 1);
                        s.entries[0].shortcut = overridden("KeyL", &by);
                    },
                ],
            ),
            (
                "entry label",
                &[
                    |s| s.entries[0].label = "x".repeat(MAX_LABEL_BYTES + 1),
                    |s| s.entries[0].label = "tool\nnotice".into(),
                ],
            ),
            (
                "popup margin",
                &[
                    |s| s.geometry.side_margin = f32::NAN,
                    |s| s.geometry.vertical_margin = 129.0,
                ],
            ),
            (
                "directory",
                &[
                    |s| s.tabs[0].directory.push(0),
                    |s| s.tabs[0].directory = vec![b'/'; MAX_DIRECTORY_BYTES + 1],
                ],
            ),
            (
                "columns",
                &[|s| {
                    s.tabs[0]
                        .columns
                        .extend(["p4", "p5", "p6"].map(|id| Column {
                            panes: vec![pane(id)],
                            expanded: id.into(),
                        }))
                }],
            ),
            (
                "empty column",
                &[|s| {
                    s.tabs[0].columns.push(Column {
                        panes: vec![],
                        expanded: "p1".into(),
                    })
                }],
            ),
            (
                "expanded pane",
                &[|s| s.tabs[0].columns[0].expanded = "p3".into()],
            ),
            (
                "Codex quota",
                &[
                    |s| codex(s).observed_at = 0,
                    |s| {
                        codex(s).windows = (0..=MAX_QUOTA_WINDOWS as u32)
                            .map(|n| window(60 + n, None))
                            .collect()
                    },
                    // Codex has two windows at most, and none scoped.
                    |s| codex(s).windows.push(window(43200, None)),
                    |s| codex(s).windows[0].scope = Some("Spark".into()),
                ],
            ),
            (
                "Codex quota state",
                &[
                    |s| codex(s).state = CodexQuotaState::Blocked,
                    |s| codex(s).windows.clear(),
                    |s| claude(s).windows.clear(),
                ],
            ),
            (
                "Codex quota window",
                &[
                    |s| codex(s).windows[0].duration_minutes = 0,
                    |s| codex(s).windows[0].remaining_percent = 101,
                    |s| codex(s).windows[0].resets_at = Some(100),
                    |s| claude(s).windows[0].remaining_percent = 101,
                    |s| claude(s).windows[0].resets_at = Some(100),
                    |s| claude(s).windows[2].scope = Some(String::new()),
                    |s| claude(s).windows[2].scope = Some("x".repeat(MAX_QUOTA_SCOPE_BYTES + 1)),
                    |s| claude(s).windows[2].scope = Some("Fable\n".into()),
                ],
            ),
            (
                "Claude quota",
                &[
                    |s| claude(s).windows[0].duration_minutes = 60,
                    |s| claude(s).windows[1].duration_minutes = 300,
                    // One scope has one window per duration.
                    |s| claude(s).windows[1].scope = Some("Fable".into()),
                    // At most four scoped windows.
                    |s| {
                        let scoped = claude(s).windows[2].clone();
                        claude(s).windows = ["a", "b", "c", "d", "e"]
                            .map(|scope| QuotaWindow {
                                scope: Some(scope.into()),
                                ..scoped.clone()
                            })
                            .to_vec()
                    },
                ],
            ),
            (
                "quota failure",
                &[|s| {
                    s.codex_quota_failure = Some(QuotaFailure {
                        at: 0,
                        reason: QuotaFailureReason::Start,
                    })
                }],
            ),
            // Unavailable alone has no record, and only needs input a kind.
            (
                "attention",
                &[
                    |s| {
                        let record = Some(working(s).clone());
                        s.tabs[0].popups[0].attention.as_mut().unwrap().record = record;
                    },
                    |s| working_attention(s).record = None,
                    |s| working(s).kind = Some(AttentionKind::Auth),
                    |s| working(s).progress = Some(101),
                    |s| working(s).others = MAX_OTHER_RECORDS + 1,
                ],
            ),
            // A tab's rollup is the most urgent state among its terminals.
            (
                "attention rollup",
                &[
                    |s| s.tabs[0].attention_rollup = Some(AttentionState::Working),
                    |s| s.tabs[1].attention_rollup = Some(AttentionState::Idle),
                    |s| s.tabs[2].attention_rollup = None,
                ],
            ),
            (
                "attention app",
                &[|s| working(s).app = "a".repeat(MAX_APP_BYTES + 1)],
            ),
            (
                "attention name",
                &[
                    |s| working(s).name = "n".repeat(MAX_NAME_BYTES + 1),
                    |s| working(s).name = "eon\u{7}".into(),
                ],
            ),
            (
                "attention title",
                &[
                    |s| working(s).title = "t".repeat(MAX_TITLE_BYTES + 1),
                    |s| working(s).title = "a\u{1b}[2J".into(),
                ],
            ),
            (
                "attention message",
                &[
                    |s| working(s).message = "m".repeat(MAX_STATUS_MESSAGE_BYTES + 1),
                    |s| working(s).message = "line\nbreak".into(),
                ],
            ),
        ];
        for (expected, changes) in invalid {
            for (index, change) in changes.iter().enumerate() {
                let mut candidate = snapshot();
                change(&mut candidate);
                let error = encode_response(&Response::Snapshot(candidate.clone())).unwrap_err();
                assert_eq!(field(error), expected, "{expected} case {index}");
                let forged = forge(&candidate);
                assert!(decode_response(&forged).is_err(), "{expected} case {index}");
            }
        }

        // A popup key carries a known tag; agent's follows its id and label.
        let mut forged = bytes.clone();
        let named = [&[5, 0][..], b"agent", &[5, 0], b"agent"].concat();
        let tag = forged
            .windows(named.len())
            .position(|window| window == named)
            .unwrap()
            + named.len();
        forged[tag] = 3;
        assert_eq!(
            decode_response(&forged),
            Err(Error::InvalidTag {
                field: "popup key",
                value: 3
            })
        );

        // A layout covering fewer panes than the tab carries cannot drop the rest.
        // The payload opens with the tab count, then t1's column count and its
        // first column's pane count.
        let mut forged = bytes;
        forged[HEADER_BYTES + 4] = 1;
        assert_eq!(
            decode_response(&forged),
            Err(Error::InvalidSnapshot { field: "columns" })
        );
    }

    #[test]
    fn every_legal_snapshot_fits_the_frame_budget() {
        // Upper bound: every session takes the larger popup form, every tab
        // reserves both selections and the most columns, and both quotas carry
        // every window with a reset and the longest scope, and a failure.
        let id = 2 + MAX_ID_BYTES;
        let entry_id = 2 + MAX_ENTRY_ID_BYTES;
        let quota = 1
            + 1
            + 8
            + 2
            + MAX_QUOTA_WINDOWS * (4 + 1 + 1 + 8 + 1 + 2 + MAX_QUOTA_SCOPE_BYTES)
            + 1
            + 8
            + 1;
        let attention = 3
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
        let upper_bound = HEADER_BYTES
            + 2
            + MAX_TABS * (2 + MAX_COLUMNS * 4)
            + id
            + 8
            + 2
            + MAX_ENTRIES * (entry_id + 2 + MAX_LABEL_BYTES + 1 + 1 + 2 + MAX_KEY_BYTES + entry_id)
            + 2
            + MAX_TABS * (id + 2 + MAX_DIRECTORY_BYTES + 1 + 2 * (1 + id) + 2 + 2 + 1)
            + MAX_SESSIONS
                * (id
                    + entry_id
                    + id
                    + 2
                    + MAX_ENDPOINT_BYTES
                    + attention
                    + 1
                    + 2
                    + MAX_PANE_LABEL_BYTES
                    + 1
                    + 2
                    + MAX_PANE_NAME_BYTES)
            + 2 * quota
            + 1
            + 2 * id
            + 2
            + MAX_WORKSPACES * (id + 1 + 2 + MAX_WORKSPACE_NAME_BYTES + 2 + 1 + 1)
            + 2
            + MAX_SESSIONS * (3 * id + attention)
            + 1
            + (1 + MAX_RUNNING_WINDOWS) * (1 + 2 + 8 + id + 1 + 2 + 256 + 1 + 8 + 1 + 2 + 1)
            + 1
            + 2
            + 2
            + 1
            + 2
            + MAX_BUSY_TERMINALS * (4 * id + 1 + 2 + MAX_PANE_LABEL_BYTES)
            + 2;
        assert!(upper_bound <= MAX_MESSAGE_BYTES, "{upper_bound}");

        let name = |prefix: &str, n: usize, length: usize| {
            let name = format!("{prefix}{n}");
            format!("{name}{}", "x".repeat(length - name.len()))
        };
        let mut state = snapshot();
        // The longest entry form is one overridden by the longest id.
        let default = Shortcut {
            modifiers: SHIFT | CTRL | ALT | SUPER,
            key: "BracketRight".into(),
        };
        let first = name("entry", 0, MAX_ENTRY_ID_BYTES);
        state.entries = (0..MAX_ENTRIES)
            .map(|n| PopupEntry {
                id: name("entry", n, MAX_ENTRY_ID_BYTES),
                label: "x".repeat(MAX_LABEL_BYTES),
                shortcut: if n == 0 {
                    PopupKey::Bound(default.clone())
                } else {
                    PopupKey::Overridden {
                        default: default.clone(),
                        by: first.clone(),
                    }
                },
            })
            .collect();
        // Each tab holds two single-pane columns and two popups, each of them
        // needing input with the longest record text.
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
        let per_tab = MAX_SESSIONS / MAX_TABS;
        state.tabs = (0..MAX_TABS)
            .map(|t| {
                let surface = |i: usize| {
                    let n = t * per_tab + i;
                    let session = name("s", n, MAX_ID_BYTES);
                    let endpoint = name("/", n, MAX_ENDPOINT_BYTES).into_bytes();
                    (n, session, endpoint)
                };
                let columns: Vec<_> = (0..per_tab / 2)
                    .map(|i| {
                        let (n, session, endpoint) = surface(i);
                        let id = name("p", n, MAX_ID_BYTES);
                        Column {
                            expanded: id.clone(),
                            panes: vec![Pane {
                                label: format!("{id}  {}", "🦀".repeat(MAX_PANE_NAME_CHARS)),
                                pinned_name: Some("🦀".repeat(MAX_PANE_NAME_CHARS)),
                                id,
                                session,
                                endpoint,
                                live: true,
                                attention: attention.clone(),
                            }],
                        }
                    })
                    .collect();
                let popups: Vec<_> = (per_tab / 2..per_tab)
                    .map(|i| {
                        let (n, session, endpoint) = surface(i);
                        Popup {
                            id: name("u", n, MAX_ID_BYTES),
                            entry: state.entries[i].id.clone(),
                            session,
                            endpoint,
                            attention: attention.clone(),
                        }
                    })
                    .collect();
                Tab {
                    id: name("t", t, MAX_ID_BYTES),
                    directory: vec![b'/'; MAX_DIRECTORY_BYTES],
                    pending: false,
                    selected_pane: Some(columns[0].expanded.clone()),
                    selected_popup: Some(popups[0].id.clone()),
                    columns,
                    popups,
                    attention_rollup: Some(AttentionState::NeedsInput),
                }
            })
            .collect();
        state.active_tab = state.tabs[0].id.clone();
        state.workspaces = (1..=MAX_WORKSPACES)
            .map(|index| WorkspaceInfo {
                id: format!("s{index}"),
                name: Some("🦀".repeat(MAX_WORKSPACE_NAME_CHARS)),
                tab_count: MAX_TABS,
                attention: Some(AttentionState::NeedsInput),
                recency: (index - 1) as u8,
            })
            .collect();
        // The largest valid workspace id is shorter than MAX_ID_BYTES.
        state.workspaces[MAX_WORKSPACES - 1].id = format!("s{}", u64::MAX);
        state.attention_queue = (0..MAX_SESSIONS)
            .map(|index| QueueEntry {
                workspace: state.workspaces[MAX_WORKSPACES - 1].id.clone(),
                tab: name("t", index / per_tab, MAX_ID_BYTES),
                target: name("p", index, MAX_ID_BYTES),
                attention: attention.clone().unwrap(),
            })
            .collect();
        state.running = Some(RunningWindows {
            this: WindowEntry {
                generation: "g".repeat(MAX_ID_BYTES),
                ..window_entry()
            },
            others: Some(
                (0..MAX_RUNNING_WINDOWS)
                    .map(|n| WindowEntry {
                        window: Some(format!("{n:08x}")),
                        generation: "g".repeat(MAX_ID_BYTES),
                        ..window_entry()
                    })
                    .collect(),
            ),
            omitted: u16::MAX,
        });
        state.busy = Some(BusyTerminals {
            terminals: (0..MAX_BUSY_TERMINALS)
                .map(|n| BusyTerminal {
                    target: TerminalTarget {
                        workspace: "w".repeat(MAX_ID_BYTES),
                        tab: "t".repeat(MAX_ID_BYTES),
                        id: TargetId::Popup("u".repeat(MAX_ID_BYTES)),
                        terminal: name("s", n, MAX_ID_BYTES),
                    },
                    name: "🦀".repeat(MAX_PANE_LABEL_BYTES / 4),
                })
                .collect(),
            omitted: (MAX_SESSIONS - MAX_BUSY_TERMINALS) as u16,
        });
        let response = Response::Snapshot(state.clone());
        let bytes = encode_response(&response).unwrap();
        assert!(bytes.len() <= upper_bound);
        assert_eq!(decode_response(&bytes).unwrap(), response);

        // The bound rests on the session limit, which neither side lets a
        // snapshot exceed.
        let extra = popup("extra", &state.entries[per_tab].id);
        state.tabs[0].popups.push(extra);
        let error = encode_response(&Response::Snapshot(state.clone())).unwrap_err();
        assert_eq!(field(error), "sessions");
        assert!(decode_response(&forge(&state)).is_err());
    }

    /// What every older supervisor does with the Reserved Stop: it reads the
    /// whole request and rejects its version, so its reply names its own.
    #[test]
    fn reserved_stop_is_not_a_versioned_message() {
        let request = crate::stop::encode_request("g1-0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(declared_message_len(&request), Ok(request.len()));
        assert_eq!(
            decode_request(&request),
            Err(Error::UnsupportedVersion { version: 0 })
        );
    }
}
