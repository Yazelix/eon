#![deny(unsafe_code)]

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
    ClipboardEffect, ConnectionState, ImageKey, ImageResource, LocalNoticeSource, ModelEffect,
    ModelError, SessionModel, WorkspaceModel, active_popup, attention_only_change,
    focused_terminal, labels_only_change,
};
pub use render::{
    CellMetrics, FontSettings, FontSetup, ImageResync, PaddingColor, PresentOutcome, RenderError,
    Renderer,
};
pub use scene::{
    BarItem, Color, DialogKind, DialogScene, DrawCell, DrawCursor, DrawRow, DrawStyle, EonBar,
    GlyphRun, HeaderText, Hyperlink, LocalClock, Scene, SceneGraphics, ScenePlacement,
    ScenePreview, SceneRect, ShortcutGroup, ShortcutHint, ShortcutHintsScene, ShortcutRow,
    SidebarRow, SidebarView, WorkspaceColumn, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit,
    WorkspacePane, WorkspaceScene, WorkspaceTab, shown_workspace, validate_copy_uri,
    validate_open_uri, workspace_column_width,
};
pub use system::{CpuTimes, CpuUse, RamUse};
pub use transport::{
    SendError, Transport, TransportEvent, WorkspaceEvent, WorkspaceTransport,
    read_workspace_response,
};
