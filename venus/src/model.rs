use crate::scene::{DrawRow, Scene, ScenePreview};
use eon_workspace_protocol::v9::{Popup, Response as WorkspaceResponse, Snapshot};
use orbit_protocol::FrameReducer;
use orbit_protocol::session::{
    ClipboardLocation, FailureCode, PreviewOutcome, ScrollOutcome, ServerMessage, WheelOutcome,
};
use std::{error::Error, fmt};

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
}

/// The last complete Eon-authored workspace accepted by Venus.
#[derive(Debug, Default)]
pub struct WorkspaceModel {
    snapshot: Option<Snapshot>,
    notice: Option<WorkspaceNotice>,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkspaceNotice {
    Action(String),
    Unavailable(String),
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

impl WorkspaceModel {
    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| match notice {
            WorkspaceNotice::Action(text) | WorkspaceNotice::Unavailable(text) => text.as_str(),
        })
    }

    /// Apply a response and report `(view changed, workspace changed)`.
    pub fn apply(&mut self, response: WorkspaceResponse) -> (bool, bool) {
        match response {
            WorkspaceResponse::Snapshot(snapshot) => {
                let snapshot_changed = self.snapshot.as_ref() != Some(&snapshot);
                let workspace_changed = self.snapshot.as_ref().is_none_or(|current| {
                    current.active_tab != snapshot.active_tab
                        || current.geometry != snapshot.geometry
                        || current.entries != snapshot.entries
                        || current.tabs != snapshot.tabs
                });
                let notice_cleared = (workspace_changed
                    || matches!(self.notice, Some(WorkspaceNotice::Unavailable(_))))
                    && self.notice.take().is_some();
                self.snapshot = Some(snapshot);
                (snapshot_changed || notice_cleared, workspace_changed)
            }
            WorkspaceResponse::Failure(failure) => (
                self.set_notice(WorkspaceNotice::Action(bounded(format!(
                    "Eon workspace {}: {}",
                    failure.code, failure.detail
                )))),
                false,
            ),
        }
    }

    pub fn mark_unavailable(&mut self, detail: impl Into<String>) -> bool {
        let expired_quota = self.snapshot.as_mut().is_some_and(|snapshot| {
            let codex = snapshot.codex_quota.take().is_some();
            let claude = snapshot.claude_quota.take().is_some();
            codex || claude
        });
        self.set_notice(WorkspaceNotice::Unavailable(bounded(detail.into()))) || expired_quota
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
        tab.panes
            .iter()
            .find(|pane| pane.id == *selected)
            .map(|pane| (pane.endpoint.as_slice(), pane.live))
    }

    fn set_notice(&mut self, notice: WorkspaceNotice) -> bool {
        let changed = self.notice.as_ref() != Some(&notice);
        self.notice = Some(notice);
        changed
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

    pub fn apply(&mut self, message: ServerMessage) -> Result<Option<ClipboardEffect>, ModelError> {
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
                self.scene = Some(Scene::from_frame(frame));
                if !preview_compatible {
                    self.scroll_preview = None;
                }
                self.awaiting_current_frame = false;
            }
            ServerMessage::WheelOutcome(WheelOutcome::Viewport { frame, .. }) if attached => {
                let frame = self.reducer.push(*frame).map_err(ModelError::Frame)?;
                self.scene = Some(Scene::from_frame(frame));
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
                self.scene = Some(Scene::from_frame(frame));
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
            }
            ServerMessage::CopiedText { location, text } if attached => {
                self.clear_orbit_notice();
                return Ok(Some(ClipboardEffect::SelectionCopy { location, text }));
            }
            ServerMessage::ClipboardWrite { location, text } if attached => {
                return Ok(Some(ClipboardEffect::TerminalWrite { location, text }));
            }
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
    }

    pub fn prepare_reconnect(&mut self) {
        self.reducer = FrameReducer::default();
        self.connection = ConnectionState::Connecting;
        self.awaiting_current_frame = true;
        self.scroll_preview = None;
        self.notices.clear();
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
    use eon_workspace_protocol::v9::{
        self as workspace, CodexQuota, CodexQuotaState, Failure, Pane, PopupEntry, Shortcut, Tab,
    };

    #[test]
    fn quota_refresh_and_workspace_failures_preserve_structure() {
        let snapshot = Snapshot {
            active_tab: "t1".into(),
            tabs: vec![
                Tab {
                    pending: false,
                    selected_popup: None,
                    popups: Vec::new(),
                    id: "t1".into(),
                    directory: b"/tmp/eon".to_vec(),
                    selected_pane: Some("pane-1".into()),
                    panes: vec![Pane {
                        id: "pane-1".into(),
                        session: "session-1".into(),
                        endpoint: b"/run/eon/orbit.sock".to_vec(),
                        live: true,
                    }],
                },
                Tab {
                    pending: false,
                    selected_popup: None,
                    popups: Vec::new(),
                    id: "t2".into(),
                    directory: b"/tmp/nova".to_vec(),
                    selected_pane: Some("pane-2".into()),
                    panes: vec![Pane {
                        id: "pane-2".into(),
                        session: "session-2".into(),
                        endpoint: b"/run/eon/nova.sock".to_vec(),
                        live: true,
                    }],
                },
            ],
            geometry: eon_workspace_protocol::v9::PopupGeometry {
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

        assert!(model.mark_unavailable("Cannot connect to Eon"));
        assert!(model.snapshot().unwrap().codex_quota.is_none());
        assert!(model.snapshot().unwrap().claude_quota.is_none());
        assert!(!model.mark_unavailable("Cannot connect to Eon"));
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
        offline.tabs[0].panes[0].live = false;
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
            panes: vec![],
            popups: vec![Popup {
                id: format!("u{number}"),
                entry: "project".into(),
                session: format!("directory-picker-{number}"),
                endpoint: format!("/run/eon/k{number}.sock").into_bytes(),
            }],
        };
        let mut snapshot = Snapshot {
            active_tab: "t2".into(),
            geometry: workspace::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "project".into(),
                label: "Project".into(),
                shortcut: Shortcut {
                    modifiers: workspace::ALT,
                    key: "KeyZ".into(),
                },
            }],
            tabs: vec![pending(1), pending(2)],
            codex_quota: None,
            claude_quota: None,
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
        snapshot.active_tab = "t2".into();
        model.apply(WorkspaceResponse::Snapshot(snapshot));
        assert_eq!(
            model.active_attachment(),
            Some((&b"/run/eon/k2.sock"[..], true))
        );
    }
}
