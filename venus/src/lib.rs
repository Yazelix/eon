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
    Renderer, read_eon_icon,
};
pub use scene::{
    AttentionFilter, AttentionFilterChip, BarItem, CellText, Color, DialogKind, DialogScene,
    DialogText, DrawCell, DrawCursor, DrawRow, DrawStyle, EonBar, GlyphRun, HeaderText,
    HintSection, Hyperlink, LocalClock, PaneHeaderAction, Scene, SceneGraphics, ScenePlacement,
    ScenePreview, SceneRect, ShortcutGroup, ShortcutHint, ShortcutHintsScene, ShortcutRow,
    SidebarRow, SidebarView, WindowMenuAction, WorkspaceColumn, WorkspaceFocus,
    WorkspaceHeaderControl, WorkspaceHit, WorkspacePane, WorkspaceScene, WorkspaceTab,
    attention_inbox_hints, shown_workspace, validate_copy_uri, validate_open_uri, window_summary,
    workspace_column_width, wrapped_lines,
};
pub(crate) use system::Load;
pub use system::{CpuReadings, CpuTimes, CpuUse, DiskUse, LoadLevels, RamUse};
pub use transport::{
    BenchTrace, Outgoing, SendError, Transport, TransportEvent, WorkspaceEvent, WorkspaceTransport,
    bench_trace, bench_tracing, read_workspace_response,
};
