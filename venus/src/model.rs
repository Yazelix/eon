use crate::scene::{DrawRow, Scene, ScenePreview, bar_workspace};
use eon_workspace_protocol::v21::{
    AttentionState, Pane, Popup, Response as WorkspaceResponse, Snapshot, Tab,
};
use orbit_protocol::FrameReducer;
use orbit_protocol::session::{
    ClipboardLocation, FailureCode, PixelFormat, PreviewOutcome, RESOURCE_CHUNK_BYTES,
    ResourceAnnounce, ResourceChunk, ScrollOutcome, ServerMessage, WheelOutcome,
};
use std::{
    collections::HashSet,
    error::Error,
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

/// ORB-C16's cap on resources announced and not retired in one attachment.
const MAX_UNRETIRED_RESOURCES: usize = 1024;

/// Numbers every attachment in this process, so resources and scenes name theirs.
static NEXT_ATTACHMENT: AtomicU64 = AtomicU64::new(1);

/// Bounded lifecycle state for one local Orbit attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Attached,
    Busy,
    Incompatible { version: u16 },
    Lost { detail: String },
    Exited { code: i32 },
}

/// A message violated the accepted local-session order or frame sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    UnexpectedMessage,
    Frame(orbit_protocol::Error),
    /// A resource message broke VEN-C31's order or size rules.
    Resource(&'static str),
}

/// One effect of an accepted Orbit message that the caller carries out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEffect {
    Clipboard(ClipboardEffect),
    /// A completely received resource, which the caller uploads or refuses before it handles the
    /// next message (VEN-C31).
    Resource(ImageResource),
}

/// One completely received image resource of one attachment.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageResource {
    pub attachment: u64,
    /// Orbit's identity for the resource in this attachment.
    pub resource: u64,
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl fmt::Debug for ImageResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageResource")
            .field("attachment", &self.attachment)
            .field("resource", &self.resource)
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &self.pixels.len())
            .finish()
    }
}

/// One transient native clipboard effect from Orbit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardEffect {
    SelectionCopy {
        location: ClipboardLocation,
        text: String,
    },
    TerminalWrite {
        location: ClipboardLocation,
        text: String,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedMessage => {
                formatter.write_str("Orbit sent a local-session message out of order")
            }
            Self::Frame(error) => write!(formatter, "Orbit frame rejected: {error}"),
            Self::Resource(detail) => write!(formatter, "Orbit image resource rejected: {detail}"),
        }
    }
}

impl Error for ModelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Frame(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum Notice {
    Orbit(FailureCode, String),
    Venus(LocalNoticeSource, String),
}

/// Venus path allowed to resolve its own client-visible notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalNoticeSource {
    Input,
    Queue,
    Resize,
    Clipboard,
}

/// The sole persistent owner of accepted presentation state in one client process.
#[derive(Debug)]
pub struct SessionModel {
    reducer: FrameReducer,
    scene: Option<Scene>,
    scroll_preview: Option<ScenePreview>,
    connection: ConnectionState,
    awaiting_current_frame: bool,
    notices: Vec<Notice>,
    attachment: u64,
    resources: Resources,
}

/// One attachment's resource order state (VEN-C31).
#[derive(Debug, Default)]
struct Resources {
    last_announced: Option<u64>,
    assembly: Option<Assembly>,
    /// Completely received resources not yet retired, including ones Venus freed.
    complete: HashSet<u64>,
}

struct Assembly {
    announce: ResourceAnnounce,
    next_chunk: u32,
    pixels: Vec<u8>,
}

impl fmt::Debug for Assembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Assembly")
            .field("announce", &self.announce)
            .field("next_chunk", &self.next_chunk)
            .finish_non_exhaustive()
    }
}

impl Resources {
    fn announce(&mut self, announce: ResourceAnnounce) -> Result<(), ModelError> {
        if self
            .last_announced
            .is_some_and(|last| announce.resource <= last)
        {
            return Err(ModelError::Resource(
                "identity is not greater than every earlier one",
            ));
        }
        if self.assembly.is_some() {
            return Err(ModelError::Resource("another resource is still incomplete"));
        }
        if self.complete.len() >= MAX_UNRETIRED_RESOURCES {
            return Err(ModelError::Resource("too many resources are not retired"));
        }
        let derived = ResourceAnnounce::new(
            announce.resource,
            announce.generation,
            announce.format,
            announce.width,
            announce.height,
        );
        if derived != Some(announce) {
            return Err(ModelError::Resource("metadata is outside the limits"));
        }
        self.last_announced = Some(announce.resource);
        self.assembly = Some(Assembly {
            announce,
            next_chunk: 0,
            pixels: Vec::with_capacity(announce.byte_length as usize),
        });
        Ok(())
    }

    fn chunk(&mut self, chunk: ResourceChunk) -> Result<Option<Assembly>, ModelError> {
        let assembly = self
            .assembly
            .as_mut()
            .ok_or(ModelError::Resource("chunk of no incomplete resource"))?;
        if chunk.resource != assembly.announce.resource || chunk.index != assembly.next_chunk {
            return Err(ModelError::Resource("chunk is out of order"));
        }
        let last = chunk.index + 1 == assembly.announce.chunk_count;
        let expected = if last {
            assembly.announce.byte_length - chunk.index * RESOURCE_CHUNK_BYTES
        } else {
            RESOURCE_CHUNK_BYTES
        };
        if chunk.bytes.len() != expected as usize {
            return Err(ModelError::Resource("chunk has the wrong size"));
        }
        assembly.pixels.extend_from_slice(&chunk.bytes);
        assembly.next_chunk += 1;
        if !last {
            return Ok(None);
        }
        let assembly = self.assembly.take();
        self.complete.insert(chunk.resource);
        Ok(assembly)
    }

    fn retire(&mut self, resource: u64) -> Result<(), ModelError> {
        if self.complete.remove(&resource) {
            Ok(())
        } else {
            Err(ModelError::Resource(
                "retirement names no complete, unretired resource",
            ))
        }
    }
}

/// The last complete Eon-authored workspace accepted by Venus.
#[derive(Debug, Default)]
pub struct WorkspaceModel {
    snapshot: Option<Snapshot>,
    notice: Option<WorkspaceNotice>,
    /// Counts new notices, so a newer notice can outrank older client notices.
    notice_serial: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkspaceNotice {
    Action(String),
    /// Eon's answer to a queue jump with no agent needing attention (EON-C31).
    NoAttention(String),
    Unavailable(String),
}

/// The selected column's panes, which size the initial window.
#[must_use]
pub(crate) fn presented_panes(tab: &Tab) -> &[Pane] {
    tab.columns
        .iter()
        .find(|column| tab.selected_pane.as_ref() == Some(&column.expanded))
        .or(tab.columns.first())
        .map_or(&[], |column| &column.panes)
}

/// Each terminal's attention state, without the record text a working agent keeps changing.
fn attention_states(tabs: &[Tab]) -> Vec<Option<AttentionState>> {
    tabs.iter()
        .flat_map(|tab| {
            tab.columns
                .iter()
                .flat_map(|column| &column.panes)
                .map(|pane| pane.attention.as_ref())
                .chain(tab.popups.iter().map(|popup| popup.attention.as_ref()))
        })
        .map(|attention| attention.map(|attention| attention.state))
        .collect()
}

/// Topology comparison excludes pane labels, pins and EON-C31 attention. Text-only
/// refreshes keep notices and disturb no navigation, scroll or attachment.
fn without_attention(tabs: &[Tab]) -> Vec<Tab> {
    tabs.iter()
        .cloned()
        .map(|mut tab| {
            tab.attention_rollup = None;
            for pane in tab.columns.iter_mut().flat_map(|column| &mut column.panes) {
                pane.attention = None;
                pane.label.clear();
                pane.pinned_name = None;
            }
            for popup in &mut tab.popups {
                popup.attention = None;
            }
            tab
        })
        .collect()
}

/// Whether the view differs only in labels, pins or attention, ignoring workspace metadata the
/// Eon Bar's workspace label does not show. The caller separately checks pure label changes and
/// moving sidebar rows.
#[must_use]
pub fn attention_only_change(current: &Snapshot, next: &Snapshot) -> bool {
    let plain = |snapshot: &Snapshot| Snapshot {
        tabs: without_attention(&snapshot.tabs),
        workspaces: Vec::new(),
        active_workspace: String::new(),
        ..snapshot.clone()
    };
    current.tabs != next.tabs
        && plain(current) == plain(next)
        && bar_workspace(current) == bar_workspace(next)
}

/// Label/pin updates change painted text without changing any input geometry; the workspace
/// label's text sizes it, so it is geometry.
#[must_use]
pub fn labels_only_change(current: &Snapshot, next: &Snapshot) -> bool {
    let plain = |snapshot: &Snapshot| {
        let mut snapshot = snapshot.clone();
        snapshot.workspaces.clear();
        snapshot.active_workspace.clear();
        for pane in snapshot
            .tabs
            .iter_mut()
            .flat_map(|tab| &mut tab.columns)
            .flat_map(|column| &mut column.panes)
        {
            pane.label.clear();
            pane.pinned_name = None;
        }
        snapshot
    };
    current.tabs != next.tabs
        && plain(current) == plain(next)
        && bar_workspace(current) == bar_workspace(next)
}

#[must_use]
pub fn active_popup(snapshot: &Snapshot) -> Option<&Popup> {
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)?;
    let selected = tab.selected_popup.as_ref()?;
    tab.popups.iter().find(|popup| popup.id == *selected)
}

/// The terminal a snapshot focuses: the active tab's shown popup, or else its selected pane.
#[must_use]
pub fn focused_terminal(snapshot: &Snapshot) -> Option<&str> {
    active_popup(snapshot)
        .map(|popup| popup.id.as_str())
        .or_else(|| {
            snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == snapshot.active_tab)?
                .selected_pane
                .as_deref()
        })
}

impl WorkspaceModel {
    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| match notice {
            WorkspaceNotice::Action(text)
            | WorkspaceNotice::NoAttention(text)
            | WorkspaceNotice::Unavailable(text) => text.as_str(),
        })
    }

    /// Apply a response and report `(view changed, workspace changed)`.
    pub fn apply(&mut self, response: WorkspaceResponse) -> (bool, bool) {
        match response {
            WorkspaceResponse::Snapshot(snapshot) => {
                let workspace_changed = self.snapshot.as_ref().is_none_or(|current| {
                    current.workspace != snapshot.workspace
                        || current.active_tab != snapshot.active_tab
                        || current.geometry != snapshot.geometry
                        || current.entries != snapshot.entries
                        || (current.tabs != snapshot.tabs
                            && without_attention(&current.tabs)
                                != without_attention(&snapshot.tabs))
                });
                // The workspace label's inputs repaint the bar; other workspaces' names and tab
                // counts show nowhere. Neither disturbs the shown workspace (EON-C33).
                let snapshot_changed = workspace_changed
                    || self.snapshot.as_ref().is_some_and(|current| {
                        current.tabs != snapshot.tabs
                            || current.codex_quota != snapshot.codex_quota
                            || current.claude_quota != snapshot.claude_quota
                            || current.sidebar != snapshot.sidebar
                            || bar_workspace(current) != bar_workspace(&snapshot)
                    });
                let notice_cleared = (workspace_changed
                    || matches!(self.notice, Some(WorkspaceNotice::Unavailable(_)))
                    || matches!(self.notice, Some(WorkspaceNotice::NoAttention(_)))
                        && self.snapshot.as_ref().is_some_and(|current| {
                            attention_states(&current.tabs) != attention_states(&snapshot.tabs)
                        }))
                    && self.notice.take().is_some();
                self.snapshot = Some(snapshot);
                (snapshot_changed || notice_cleared, workspace_changed)
            }
            WorkspaceResponse::Failure(failure) => (
                self.set_notice(
                    if failure.code == "no-attention" {
                        WorkspaceNotice::NoAttention(bounded(failure.detail))
                    } else {
                        WorkspaceNotice::Action(bounded(format!(
                            "Eon workspace {}: {}",
                            failure.code, failure.detail
                        )))
                    },
                    false,
                ),
                false,
            ),
            // The client exits on an ended generation before applying it.
            WorkspaceResponse::Ended(_) => (false, false),
        }
    }

    #[must_use]
    pub fn notice_serial(&self) -> u64 {
        self.notice_serial
    }

    /// Mark the current notice as a refused action's answer, so it outranks older client notices.
    pub fn renew_notice(&mut self) -> bool {
        self.notice.is_some() && {
            self.notice_serial += 1;
            true
        }
    }

    /// Clear the empty-queue notice, which the next terminal input retires (VEN-C30).
    pub fn dismiss_no_attention(&mut self) -> bool {
        matches!(self.notice, Some(WorkspaceNotice::NoAttention(_))) && self.notice.take().is_some()
    }

    /// Refuse an action locally with the same lifetime as an Eon rejection.
    pub fn reject_action(&mut self, detail: impl Into<String>) -> bool {
        self.set_notice(WorkspaceNotice::Action(bounded(detail.into())), true)
    }

    /// Mark Eon unavailable; `answers_action` when the failure itself answers a user action.
    pub fn mark_unavailable(&mut self, detail: impl Into<String>, answers_action: bool) -> bool {
        let expired_quota = self.snapshot.as_mut().is_some_and(|snapshot| {
            let codex = snapshot.codex_quota.take().is_some();
            let claude = snapshot.claude_quota.take().is_some();
            codex || claude
        });
        self.set_notice(
            WorkspaceNotice::Unavailable(bounded(detail.into())),
            answers_action,
        ) || expired_quota
    }

    #[must_use]
    pub fn active_attachment(&self) -> Option<(&[u8], bool)> {
        let snapshot = self.snapshot.as_ref()?;
        if let Some(popup) = active_popup(snapshot) {
            return Some((&popup.endpoint, true));
        }
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab)?;
        let selected = tab.selected_pane.as_ref()?;
        tab.panes()
            .find(|pane| pane.id == *selected)
            .map(|pane| (pane.endpoint.as_slice(), pane.live))
    }

    /// The expanded pane of each visible column other than the selected one, as
    /// `(column index, endpoint, live)`; none while a popup covers the tab.
    #[must_use]
    pub fn column_attachments(&self) -> Vec<(usize, &[u8], bool)> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Vec::new();
        };
        if active_popup(snapshot).is_some() {
            return Vec::new();
        }
        let Some(tab) = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab)
        else {
            return Vec::new();
        };
        tab.columns
            .iter()
            .enumerate()
            .filter(|(_, column)| tab.selected_pane.as_ref() != Some(&column.expanded))
            .filter_map(|(index, column)| {
                column
                    .panes
                    .iter()
                    .find(|pane| pane.id == column.expanded)
                    .map(|pane| (index, pane.endpoint.as_slice(), pane.live))
            })
            .collect()
    }

    /// A notice that answers an action is new even when it repeats; a repeated poll result is not.
    fn set_notice(&mut self, notice: WorkspaceNotice, answers_action: bool) -> bool {
        let fresh = answers_action || self.notice.as_ref() != Some(&notice);
        self.notice = Some(notice);
        self.notice_serial += u64::from(fresh);
        fresh
    }
}

impl Default for SessionModel {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionModel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            reducer: FrameReducer::default(),
            scene: None,
            scroll_preview: None,
            connection: ConnectionState::Connecting,
            awaiting_current_frame: true,
            notices: Vec::with_capacity(4),
            attachment: next_attachment(),
            resources: Resources::default(),
        }
    }

    #[must_use]
    pub fn scene(&self) -> Option<&Scene> {
        self.scene.as_ref()
    }

    /// Position of the accepted viewport, never extrapolated from a preview.
    #[must_use]
    pub fn scrollback_label(&self) -> Option<String> {
        if !self.is_attached()
            || self.awaiting_current_frame
            || matches!(
                self.scroll_preview,
                Some(ScenePreview::TerminalOwned { .. })
            )
        {
            return None;
        }
        let rows = self.reducer.current()?.scroll_position.rows_from_live;
        (rows > 0).then(|| format!("↓ {rows} row{}", if rows == 1 { "" } else { "s" }))
    }

    #[must_use]
    pub fn scroll_preview(&self) -> Option<&ScenePreview> {
        self.scroll_preview.as_ref()
    }

    /// Whether the accepted viewport is at live output, where later output moves the
    /// history above it; away from live, Orbit keeps the viewport pinned.
    #[must_use]
    pub fn at_live(&self) -> bool {
        self.reducer
            .current()
            .is_some_and(|frame| frame.scroll_position.rows_from_live == 0)
    }

    /// Release preview rows while keeping routing evidence until the next frame.
    pub fn clear_viewport_preview(&mut self) {
        if matches!(self.scroll_preview, Some(ScenePreview::Viewport { .. })) {
            self.scroll_preview = None;
        }
    }

    #[must_use]
    pub fn connection(&self) -> &ConnectionState {
        &self.connection
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notices.last().map(|notice| match notice {
            Notice::Orbit(_, detail) | Notice::Venus(_, detail) => detail.as_str(),
        })
    }

    #[must_use]
    pub fn is_attached(&self) -> bool {
        self.connection == ConnectionState::Attached
    }

    #[must_use]
    pub fn awaiting_current_frame(&self) -> bool {
        self.awaiting_current_frame
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.connection,
            ConnectionState::Busy
                | ConnectionState::Incompatible { .. }
                | ConnectionState::Lost { .. }
                | ConnectionState::Exited { .. }
        )
    }

    pub fn apply(&mut self, message: ServerMessage) -> Result<Option<ModelEffect>, ModelError> {
        let connecting = matches!(self.connection, ConnectionState::Connecting);
        let attached = self.is_attached();
        match message {
            ServerMessage::Attached if connecting => {
                self.connection = ConnectionState::Attached;
                self.notices.clear();
            }
            ServerMessage::Frame(frame) if attached => {
                let preview_compatible = matches!(
                    self.scroll_preview.as_ref(),
                    Some(ScenePreview::Viewport { .. })
                ) && self.reducer.current().is_some_and(|current| {
                    current.dimensions == frame.dimensions
                        && current.screen == frame.screen
                        && current.colors.background == frame.colors.background
                        && current.colors.foreground == frame.colors.foreground
                        && current.colors.palette == frame.colors.palette
                });
                let frame = self.reducer.push(*frame).map_err(ModelError::Frame)?;
                self.scene = Some(Scene::from_frame(frame, self.attachment));
                if !preview_compatible {
                    self.scroll_preview = None;
                }
                self.awaiting_current_frame = false;
            }
            ServerMessage::WheelOutcome(WheelOutcome::Viewport { frame, .. }) if attached => {
                let frame = self.reducer.push(*frame).map_err(ModelError::Frame)?;
                self.scene = Some(Scene::from_frame(frame, self.attachment));
                self.scroll_preview = None;
                self.awaiting_current_frame = false;
            }
            ServerMessage::VerticalPreview(preview) if attached => {
                let frame = self
                    .reducer
                    .current()
                    .ok_or(ModelError::UnexpectedMessage)?;
                if preview.frame_revision != frame.revision {
                    return Ok(None);
                }
                self.scroll_preview =
                    Some(scene_preview(frame, preview.direction, preview.outcome)?);
            }
            ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { requested_rows })
                if attached =>
            {
                let frame_revision = self
                    .reducer
                    .current()
                    .ok_or(ModelError::UnexpectedMessage)?
                    .revision;
                self.scroll_preview = Some(ScenePreview::TerminalOwned {
                    frame_revision,
                    direction: scroll_direction(requested_rows),
                });
            }
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                requested_rows,
                frame,
                next,
                ..
            }) if attached => {
                let preview =
                    scene_preview(frame.as_ref(), scroll_direction(requested_rows), next)?;
                let frame = self.reducer.push(*frame).map_err(ModelError::Frame)?;
                self.scene = Some(Scene::from_frame(frame, self.attachment));
                self.scroll_preview = Some(preview);
                self.awaiting_current_frame = false;
            }
            ServerMessage::Accepted
            | ServerMessage::SelectionFinished { .. }
            | ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted)
                if attached =>
            {
                self.clear_orbit_notice();
            }
            ServerMessage::Failure(failure) if connecting || attached => {
                self.clear_viewport_preview();
                let label = match failure.code {
                    FailureCode::InvalidInput => "rejected input",
                    FailureCode::Protocol => "protocol failure",
                    FailureCode::Terminal => "terminal failure",
                };
                self.clear_orbit_notice();
                self.notices.push(Notice::Orbit(
                    failure.code,
                    bounded(format!("Orbit {label}: {}", failure.detail)),
                ));
            }
            ServerMessage::Busy if connecting => {
                self.connection = ConnectionState::Busy;
                self.scroll_preview = None;
                self.notices.clear();
            }
            ServerMessage::Exited { code } if attached => {
                self.connection = ConnectionState::Exited { code };
                self.scroll_preview = None;
                self.notices.clear();
                self.resources = Resources::default();
            }
            ServerMessage::CopiedText { location, text } if attached => {
                self.clear_orbit_notice();
                return Ok(Some(ModelEffect::Clipboard(
                    ClipboardEffect::SelectionCopy { location, text },
                )));
            }
            ServerMessage::ClipboardWrite { location, text } if attached => {
                return Ok(Some(ModelEffect::Clipboard(
                    ClipboardEffect::TerminalWrite { location, text },
                )));
            }
            ServerMessage::ResourceAnnounce(_)
            | ServerMessage::ResourceChunk(_)
            | ServerMessage::ResourceRetire { .. }
                if attached =>
            {
                return self.apply_resource(message).inspect_err(|_| {
                    self.resources.assembly = None;
                });
            }
            _ => return Err(ModelError::UnexpectedMessage),
        }
        Ok(None)
    }

    fn apply_resource(
        &mut self,
        message: ServerMessage,
    ) -> Result<Option<ModelEffect>, ModelError> {
        if self.reducer.current().is_none() {
            return Err(ModelError::Resource(
                "resource message before the first frame",
            ));
        }
        match message {
            ServerMessage::ResourceAnnounce(announce) => self.resources.announce(announce)?,
            ServerMessage::ResourceChunk(chunk) => {
                if let Some(assembly) = self.resources.chunk(chunk)? {
                    let announce = assembly.announce;
                    return Ok(Some(ModelEffect::Resource(ImageResource {
                        attachment: self.attachment,
                        resource: announce.resource,
                        format: announce.format,
                        width: announce.width,
                        height: announce.height,
                        pixels: assembly.pixels,
                    })));
                }
            }
            ServerMessage::ResourceRetire { resource } => self.resources.retire(resource)?,
            _ => return Err(ModelError::UnexpectedMessage),
        }
        Ok(None)
    }

    pub fn mark_incompatible(&mut self, version: u16) {
        if self.is_terminal() {
            return;
        }
        self.connection = ConnectionState::Incompatible { version };
        self.scroll_preview = None;
        self.notices.clear();
        self.resources = Resources::default();
    }

    pub fn mark_lost(&mut self, detail: impl Into<String>) {
        if self.is_terminal() {
            return;
        }
        self.connection = ConnectionState::Lost {
            detail: bounded(detail.into()),
        };
        self.scroll_preview = None;
        self.notices.clear();
        self.resources = Resources::default();
    }

    pub fn mark_lost_preserving_constraining_notice(&mut self, detail: impl Into<String>) {
        if self.is_terminal() {
            return;
        }
        self.connection = ConnectionState::Lost {
            detail: bounded(detail.into()),
        };
        self.scroll_preview = None;
        self.notices.retain(|notice| {
            matches!(
                notice,
                Notice::Orbit(FailureCode::Protocol | FailureCode::Terminal, _)
            )
        });
        self.resources = Resources::default();
    }

    /// Start a new attachment, which discards the old one's resources and identities.
    pub fn prepare_reconnect(&mut self) {
        self.reducer = FrameReducer::default();
        self.connection = ConnectionState::Connecting;
        self.awaiting_current_frame = true;
        self.scroll_preview = None;
        self.notices.clear();
        self.attachment = next_attachment();
        self.resources = Resources::default();
    }

    pub fn set_venus_notice(&mut self, source: LocalNoticeSource, detail: impl Into<String>) {
        if self.is_terminal() {
            return;
        }
        self.clear_venus_notice(source);
        self.notices
            .push(Notice::Venus(source, bounded(detail.into())));
    }

    pub fn clear_venus_notice(&mut self, source: LocalNoticeSource) -> bool {
        let previous_len = self.notices.len();
        self.notices
            .retain(|notice| !matches!(notice, Notice::Venus(owner, _) if *owner == source));
        self.notices.len() != previous_len
    }

    fn clear_orbit_notice(&mut self) {
        self.notices
            .retain(|notice| !matches!(notice, Notice::Orbit(_, _)));
    }
}

fn scroll_direction(rows: i16) -> orbit_protocol::session::VerticalDirection {
    if rows < 0 {
        orbit_protocol::session::VerticalDirection::Up
    } else {
        orbit_protocol::session::VerticalDirection::Down
    }
}

fn scene_preview(
    frame: &orbit_protocol::Frame,
    direction: orbit_protocol::session::VerticalDirection,
    outcome: PreviewOutcome,
) -> Result<ScenePreview, ModelError> {
    Ok(match outcome {
        PreviewOutcome::TerminalRouted => ScenePreview::TerminalOwned {
            frame_revision: frame.revision,
            direction,
        },
        PreviewOutcome::Viewport {
            cols,
            edge_reached,
            rows,
        } => {
            if cols != frame.dimensions.cols || rows.len() > usize::from(frame.dimensions.rows) {
                return Err(ModelError::UnexpectedMessage);
            }
            ScenePreview::Viewport {
                frame_revision: frame.revision,
                direction,
                edge_reached,
                rows: rows
                    .iter()
                    .map(|row| DrawRow::from_protocol(row, frame))
                    .collect(),
            }
        }
    })
}

fn next_attachment() -> u64 {
    NEXT_ATTACHMENT.fetch_add(1, Ordering::Relaxed)
}

fn bounded(mut detail: String) -> String {
    const MAX_CHARS: usize = 1024;
    if let Some((index, _)) = detail.char_indices().nth(MAX_CHARS) {
        detail.truncate(index);
        detail.push('…');
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v21::{
        self as workspace, CodexQuota, CodexQuotaState, Failure, Pane, PopupEntry, PopupKey,
        Shortcut, Tab,
    };

    #[test]
    fn pane_label_updates_keep_notices_and_attachment_identity() {
        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![workspace::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
            }],
            active_tab: "t1".into(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: vec![workspace::Column {
                    expanded: "p1".into(),
                    panes: vec![Pane {
                        id: "p1".into(),
                        label: "p1  shell".into(),
                        pinned_name: None,
                        session: "session-1".into(),
                        endpoint: b"/tmp/p1.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                }],
                popups: vec![],
                attention_rollup: None,
            }],
            geometry: workspace::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![],
            codex_quota: None,
            claude_quota: None,
            sidebar: workspace::Sidebar::Open,
        };
        let mut model = WorkspaceModel::default();
        model.apply(WorkspaceResponse::Snapshot(snapshot.clone()));
        model.reject_action("keep this notice");
        let attachment = model
            .active_attachment()
            .map(|(endpoint, live)| (endpoint.to_vec(), live));
        let before = snapshot.clone();
        snapshot.tabs[0].columns[0].panes[0].label = "p1  Review · ../repo".into();
        snapshot.tabs[0].columns[0].panes[0].pinned_name = Some("Review".into());
        assert!(labels_only_change(&before, &snapshot));
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot)),
            (true, false)
        );
        assert_eq!(model.notice(), Some("keep this notice"));
        assert_eq!(
            model
                .active_attachment()
                .map(|(endpoint, live)| (endpoint.to_vec(), live)),
            attachment
        );
    }

    #[test]
    fn refused_actions_are_new_notices_but_repeated_polls_are_not() {
        let mut model = WorkspaceModel::default();
        model.reject_action("refused");
        let first = model.notice_serial();
        assert!(model.reject_action("refused"), "a repeated refusal is new");
        assert!(model.notice_serial() > first);
        model.mark_unavailable("full", true);
        let queued = model.notice_serial();
        assert!(
            model.mark_unavailable("full", true),
            "so is a queue refusal"
        );
        assert!(model.notice_serial() > queued);
        model.mark_unavailable("gone", false);
        let unavailable = model.notice_serial();
        model.mark_unavailable("gone", false);
        assert_eq!(
            model.notice_serial(),
            unavailable,
            "a retry poll is no new notice"
        );
        let stopping = || {
            WorkspaceResponse::Failure(Failure {
                code: "stop-incomplete".into(),
                detail: "stopping".into(),
            })
        };
        model.apply(stopping());
        let polled = model.notice_serial();
        assert!(!model.apply(stopping()).0);
        assert_eq!(
            model.notice_serial(),
            polled,
            "nor is a repeated poll failure"
        );
        assert!(model.renew_notice(), "an action's refusal renews it");
        assert!(model.notice_serial() > polled);
    }

    #[test]
    fn quota_refresh_and_workspace_failures_preserve_structure() {
        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 2,
            }],
            active_tab: "t1".into(),
            tabs: vec![
                Tab {
                    pending: false,
                    selected_popup: None,
                    popups: Vec::new(),
                    id: "t1".into(),
                    directory: b"/tmp/eon".to_vec(),
                    selected_pane: Some("pane-1".into()),
                    columns: vec![eon_workspace_protocol::v21::Column {
                        panes: vec![Pane {
                            label: "pane-1".into(),
                            pinned_name: None,
                            id: "pane-1".into(),
                            session: "session-1".into(),
                            endpoint: b"/run/eon/orbit.sock".to_vec(),
                            live: true,
                            attention: None,
                        }],
                        expanded: "pane-1".into(),
                    }],
                    attention_rollup: None,
                },
                Tab {
                    pending: false,
                    selected_popup: None,
                    popups: Vec::new(),
                    id: "t2".into(),
                    directory: b"/tmp/nova".to_vec(),
                    selected_pane: Some("pane-2".into()),
                    columns: vec![eon_workspace_protocol::v21::Column {
                        panes: vec![Pane {
                            label: "pane-2".into(),
                            pinned_name: None,
                            id: "pane-2".into(),
                            session: "session-2".into(),
                            endpoint: b"/run/eon/nova.sock".to_vec(),
                            live: true,
                            attention: None,
                        }],
                        expanded: "pane-2".into(),
                    }],
                    attention_rollup: None,
                },
            ],
            geometry: eon_workspace_protocol::v21::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Blocked,
                observed_at: 1_800_000_000,
                windows: Vec::new(),
            }),
            claude_quota: Some(workspace::ClaudeQuota {
                state: workspace::ClaudeQuotaState::Fresh,
                observed_at: 1_800_000_000,
                windows: vec![workspace::QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 98,
                    resets_at: Some(1_800_001_000),
                }],
            }),
            sidebar: workspace::Sidebar::Open,
        };
        let mut model = WorkspaceModel::default();

        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot.clone())),
            (true, true)
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot.clone())),
            (false, false)
        );
        let mut refreshed_quota = snapshot.clone();
        refreshed_quota.codex_quota.as_mut().unwrap().observed_at += 1;
        refreshed_quota.claude_quota.as_mut().unwrap().observed_at += 1;
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(refreshed_quota.clone())),
            (true, false),
            "quota-only refreshes are not structural workspace changes"
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Failure(Failure {
                code: "edge".into(),
                detail: "there is no pane above the selected pane".into(),
            })),
            (true, false)
        );

        assert_eq!(model.snapshot(), Some(&refreshed_quota));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/orbit.sock"[..], true))
        );
        assert_eq!(
            model.notice(),
            Some("Eon workspace edge: there is no pane above the selected pane")
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(refreshed_quota.clone())),
            (false, false),
            "an unchanged poll must not dismiss an action failure"
        );
        assert!(model.notice().is_some());
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot.clone())),
            (true, false)
        );
        assert!(model.notice().is_some(), "quota updates retain the failure");
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot.clone())),
            (false, false)
        );
        // The workspace label's name and count repaint the bar, and leave the shown
        // workspace's navigation, attachments and notice as they were (EON-C33).
        let mut summaries = snapshot.clone();
        summaries.workspaces[0].name = Some("renamed".into());
        summaries.workspaces.push(workspace::WorkspaceInfo {
            id: "w2".into(),
            name: None,
            tab_count: 3,
        });
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(summaries.clone())),
            (true, false),
            "the workspace label's inputs repaint the bar"
        );
        assert_eq!(model.snapshot(), Some(&summaries));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/orbit.sock"[..], true))
        );
        assert!(
            model.notice().is_some(),
            "workspace label edits retain the action failure"
        );
        let mut hidden = summaries.clone();
        hidden.workspaces[1].name = Some("notes".into());
        hidden.workspaces[1].tab_count = 1;
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(hidden.clone())),
            (false, false),
            "hidden workspace edits do not invalidate the shown view"
        );
        assert_eq!(model.snapshot(), Some(&hidden));
        assert!(
            model.notice().is_some(),
            "hidden workspace edits retain the action failure"
        );
        let mut attention = snapshot.clone();
        attention.tabs[1].columns[0].panes[0].attention = Some(workspace::Attention {
            state: workspace::AttentionState::Unavailable,
            record: None,
        });
        attention.tabs[1].attention_rollup = Some(workspace::AttentionState::Unavailable);
        assert!(attention_only_change(&snapshot, &attention));
        let mut combined = attention.clone();
        combined.workspaces[0].tab_count = 5;
        assert!(
            attention_only_change(&snapshot, &combined),
            "unpainted workspace metadata must not retire a held link during an attention refresh"
        );
        combined.workspaces = summaries.workspaces.clone();
        assert!(
            !attention_only_change(&snapshot, &combined),
            "the workspace label's width moves the tabs"
        );
        assert!(!attention_only_change(&snapshot, &snapshot));
        attention.sidebar = workspace::Sidebar::Closed;
        assert!(
            !attention_only_change(&snapshot, &attention),
            "the sidebar moves the content area"
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(attention)),
            (true, false),
            "attention and sidebar refreshes are not structural workspace changes"
        );
        assert!(
            model.notice().is_some(),
            "attention updates retain the failure"
        );

        let agent = |state, message: &str| {
            let mut next = snapshot.clone();
            next.tabs[1].columns[0].panes[0].attention = Some(workspace::Attention {
                state,
                record: Some(workspace::AttentionRecord {
                    kind: None,
                    progress: None,
                    app: "claude-code".into(),
                    name: String::new(),
                    title: String::new(),
                    message: message.into(),
                    changed_at: 0,
                    stale: false,
                    others: 0,
                }),
            });
            next
        };
        model.apply(WorkspaceResponse::Snapshot(agent(
            workspace::AttentionState::Working,
            "Reading",
        )));
        assert_eq!(
            model.apply(WorkspaceResponse::Failure(Failure {
                code: "no-attention".into(),
                detail: "No agent needs attention".into(),
            })),
            (true, false)
        );
        assert_eq!(model.notice(), Some("No agent needs attention"));
        assert!(model.dismiss_no_attention(), "terminal input retires it");
        assert!(!model.dismiss_no_attention());
        model.apply(WorkspaceResponse::Failure(Failure {
            code: "no-attention".into(),
            detail: "No agent needs attention".into(),
        }));
        model.apply(WorkspaceResponse::Snapshot(agent(
            workspace::AttentionState::Working,
            "Editing",
        )));
        assert_eq!(
            model.notice(),
            Some("No agent needs attention"),
            "a working agent's new message keeps the empty-queue notice"
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(agent(
                workspace::AttentionState::NeedsInput,
                "Editing",
            ))),
            (true, false)
        );
        assert_eq!(
            model.notice(),
            None,
            "an attention state change retires the empty-queue notice"
        );
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(snapshot.clone())),
            (true, false)
        );
        model.apply(WorkspaceResponse::Failure(Failure {
            code: "edge".into(),
            detail: "there is no pane above the selected pane".into(),
        }));
        assert!(
            !model.dismiss_no_attention(),
            "terminal input keeps other rejections"
        );

        let mut selected = snapshot.clone();
        selected.active_tab = "t2".into();
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(selected)),
            (true, true)
        );
        assert_eq!(
            model.notice(),
            None,
            "a changed workspace retires the failure"
        );

        assert!(model.mark_unavailable("Cannot connect to Eon", false));
        assert!(model.snapshot().unwrap().codex_quota.is_none());
        assert!(model.snapshot().unwrap().claude_quota.is_none());
        assert!(!model.mark_unavailable("Cannot connect to Eon", false));
        let recovered = model.snapshot().unwrap().clone();
        assert_eq!(
            model.apply(WorkspaceResponse::Snapshot(recovered)),
            (true, false)
        );
        assert_eq!(
            model.notice(),
            None,
            "an identical snapshot proves reconnection"
        );

        let mut offline = snapshot.clone();
        offline.tabs[0].columns[0].panes[0].live = false;
        model.apply(WorkspaceResponse::Snapshot(offline));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/orbit.sock"[..], false))
        );
    }

    #[test]
    fn independent_pending_picker_tabs_attach_only_the_active_endpoint() {
        let pending = |number: usize| Tab {
            id: format!("t{number}"),
            directory: b"/tmp".to_vec(),
            pending: true,
            selected_pane: None,
            selected_popup: Some(format!("u{number}")),
            columns: Vec::new(),
            popups: vec![Popup {
                id: format!("u{number}"),
                entry: "project".into(),
                session: format!("directory-picker-{number}"),
                endpoint: format!("/run/eon/k{number}.sock").into_bytes(),
                attention: None,
            }],
            attention_rollup: None,
        };
        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 2,
            }],
            active_tab: "t2".into(),
            geometry: workspace::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "project".into(),
                label: "Project".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: workspace::ALT,
                    key: "KeyZ".into(),
                }),
            }],
            tabs: vec![pending(1), pending(2)],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v21::Sidebar::Open,
        };
        let wire =
            workspace::encode_response(&WorkspaceResponse::Snapshot(snapshot.clone())).unwrap();
        let mut model = WorkspaceModel::default();
        model.apply(workspace::decode_response(&wire).unwrap());
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/k2.sock"[..], true))
        );

        snapshot.active_tab = "t1".into();
        model.apply(WorkspaceResponse::Snapshot(snapshot.clone()));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/k1.sock"[..], true))
        );

        snapshot.tabs.remove(0);
        snapshot.workspaces[0].tab_count = snapshot.tabs.len();
        snapshot.active_tab = "t2".into();
        model.apply(WorkspaceResponse::Snapshot(snapshot));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/k2.sock"[..], true))
        );
    }

    #[test]
    fn other_columns_attach_their_expanded_panes_until_a_popup_covers_the_tab() {
        let pane = |id: &str, live| Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: format!("/run/eon/{id}.sock").into_bytes(),
            live,
            attention: None,
        };
        let column = |panes: Vec<Pane>, expanded: &str| workspace::Column {
            panes,
            expanded: expanded.into(),
        };
        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
            }],
            active_tab: "t1".into(),
            geometry: workspace::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: false,
                selected_pane: Some("x".into()),
                selected_popup: None,
                columns: vec![
                    column(vec![pane("a", true)], "a"),
                    column(vec![pane("x", true), pane("y", true)], "x"),
                    column(vec![pane("c", true), pane("d", false)], "d"),
                ],
                popups: Vec::new(),
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v21::Sidebar::Open,
        };
        let mut model = WorkspaceModel::default();
        model.apply(WorkspaceResponse::Snapshot(snapshot.clone()));
        assert_eq!(
            model.column_attachments(),
            [
                (0, &b"/run/eon/a.sock"[..], true),
                (2, &b"/run/eon/d.sock"[..], false),
            ]
        );
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/x.sock"[..], true))
        );

        snapshot.tabs[0].popups = vec![Popup {
            id: "u1".into(),
            entry: "project".into(),
            session: "popup".into(),
            endpoint: b"/run/eon/popup.sock".to_vec(),
            attention: None,
        }];
        snapshot.tabs[0].selected_popup = Some("u1".into());
        model.apply(WorkspaceResponse::Snapshot(snapshot));
        assert!(model.column_attachments().is_empty());
    }
}
