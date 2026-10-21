#![forbid(unsafe_code)]

//! Native presentation and interaction for one authoritative Orbit session.

/// Maximum OSC 8 target size accepted by Venus link actions.
pub const MAX_LINK_BYTES: usize = 4096;

mod accessibility;
mod input;
mod model;
mod render;
mod scene;
mod transport;

pub use accessibility::{Accessibility, AccessibilityTarget, Activation};
pub use input::InputState;
pub use model::{
    ClipboardEffect, ConnectionState, LocalNoticeSource, ModelError, SessionModel, WorkspaceModel,
    active_popup,
};
pub use render::{CellMetrics, FontSettings, FontSetup, PresentOutcome, RenderError, Renderer};
pub use scene::{
    Color, DrawCell, DrawCursor, DrawRow, DrawStyle, GlyphRun, Hyperlink, LocalClock, PaneMetadata,
    Scene, ScenePreview, SceneRect, ShortcutGroup, ShortcutHint, ShortcutHintsScene, ShortcutRow,
    ShortcutViewerScene, WorkspaceColumn, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit,
    WorkspacePane, WorkspaceScene, WorkspaceTab, workspace_column_width,
};
pub use transport::{
    MetadataEvent, MetadataTransport, SendError, Transport, TransportEvent, WorkspaceEvent,
    WorkspaceTransport, read_workspace_response,
};
