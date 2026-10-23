#![forbid(unsafe_code)]

//! Native presentation and interaction for one authoritative Orbit session.

/// Maximum OSC 8 target size accepted by Venus link actions.
pub const MAX_LINK_BYTES: usize = 4096;

mod accessibility;
mod input;
mod model;
mod render;
mod scene;
mod system;
mod transport;

pub use accessibility::{Accessibility, AccessibilityTarget, Activation};
pub use input::InputState;
pub use model::{
    ClipboardEffect, ConnectionState, LocalNoticeSource, ModelError, SessionModel, WorkspaceModel,
    active_popup, attention_only_change, focused_terminal,
};
pub use render::{CellMetrics, FontSettings, FontSetup, PresentOutcome, RenderError, Renderer};
pub use scene::{
    BarItem, Color, DialogKind, DialogScene, DrawCell, DrawCursor, DrawRow, DrawStyle, EonBar,
    GlyphRun, Hyperlink, LocalClock, PaneMetadata, Scene, ScenePreview, SceneRect, ShortcutGroup,
    ShortcutHint, ShortcutHintsScene, ShortcutRow, SidebarRow, SidebarView, WorkspaceColumn,
    WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit, WorkspacePane, WorkspaceScene,
    WorkspaceTab, validate_copy_uri, validate_open_uri, workspace_column_width,
};
pub use system::{CpuTimes, CpuUse, RamUse};
pub use transport::{
    MetadataEvent, MetadataTransport, SendError, Transport, TransportEvent, WorkspaceEvent,
    WorkspaceTransport, read_workspace_response,
};
