//! Native presentation and interaction for one authoritative Orbit session.

/// Maximum OSC 8 target size accepted by Venus link actions.
pub const MAX_LINK_BYTES: usize = 4096;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The `yazelix-venus` binary: parses Venus's launch arguments and the private presentation
/// control variable, then runs the native client on winit. The binary is only this call, so the
/// Rune candidate entry can run the same `Application` (eon-yp6vg.7.9).
#[doc(hidden)]
pub fn run_venus() -> Result {
    application::run(launch_arguments_from_env()?)
}

/// The Rune candidate entry (eon-yp6vg.7.9): the same launch arguments, startup admission,
/// presentation control and policy as `run_venus`, with Rune as the native host.
#[cfg(feature = "candidate-rune")]
#[doc(hidden)]
pub fn run_rune_candidate() -> Result {
    application::rune_host::run(launch_arguments_from_env()?)
}

fn launch_arguments_from_env() -> Result<launch::LaunchArguments> {
    launch::launch_arguments(
        std::env::args_os().skip(1),
        std::env::var_os("EON_VENUS_PRESENTATION_CONTROL"),
    )
}

mod accessibility;
mod application;
mod contour;
mod input;
mod launch;
mod model;
mod native_command;
mod render;
#[cfg(feature = "candidate-rune")]
pub mod rune_candidate;
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
pub use native_command::{WindowCommand, external_command};
pub use render::{
    CellMetrics, FontSettings, FontSetup, FrameTimings, ImageResync, PaddingColor, Palette,
    PaletteContrast, PaletteMode, PanelHeading, PresentOutcome, RenderError, RenderTimings,
    Renderer, contrast_ratio, read_eon_icon,
};
pub use scene::{
    AttentionFilter, AttentionFilterChip, BarItem, CellText, Color, DialogKind, DialogScene,
    DialogText, DrawCell, DrawCursor, DrawRow, DrawStyle, EonBar, GlyphRun, HeaderText,
    HeadingAction, HintSection, Hyperlink, LocalClock, PaneHeaderAction, ReleaseStage, Scene,
    SceneGraphics, ScenePlacement, ScenePreview, SceneRect, ShortcutGroup, ShortcutHint,
    ShortcutHintsScene, ShortcutRow, SidebarRow, SidebarView, WindowMenuAction, WorkspaceColumn,
    WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit, WorkspacePane, WorkspaceScene,
    WorkspaceTab, attention_inbox_hints, shown_workspace, validate_copy_uri, validate_open_uri,
    window_summary, workspace_column_width, wrapped_lines,
};
pub(crate) use system::Load;
pub use system::{
    CpuReadings, CpuTimes, CpuUse, DiskUse, GpuDevice, GpuStatus, GpuUse, LoadLevels, RamUse,
};
pub use transport::{
    BenchTrace, Outgoing, SendError, Transport, TransportEvent, WorkspaceEvent, WorkspaceTransport,
    bench_trace, bench_tracing, frame_elapsed, frame_time, read_workspace_response,
};
