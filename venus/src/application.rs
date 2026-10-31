use crate::{Result, launch::LaunchArguments};
use accesskit_winit::{Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent};
use eon_workspace_protocol::v26::{
    Action as WorkspaceAction, Direction as WorkspaceDirection, PopupEntry, Sidebar,
    WorkspaceAction as CommonAction,
};
use orbit_protocol::{
    PlacementState,
    session::{
        self, ClientMessage, ClipboardLocation, FailureCode, ScrollOutcome, SelectionAction,
        ServerMessage, SurfaceSize, WheelOutcome,
    },
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    io::{self, Read, Write},
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};
#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorIcon, UserAttentionType, Window, WindowAttributes, WindowId},
};
use yazelix_venus::{
    Accessibility, AccessibilityTarget, AttentionFilter, BarItem, CellMetrics, ClipboardEffect,
    Color, ConnectionState, CpuTimes, CpuUse, DialogKind, DialogScene, EonBar, FontSettings,
    FontSetup, HeaderText, Hyperlink, ImageKey, ImageResync, InputState, LocalClock,
    LocalNoticeSource, MAX_LINK_BYTES, ModelEffect, ModelError, PaddingColor, PresentOutcome,
    RamUse, Renderer, Scene, SceneRect, SessionModel, ShortcutHintsScene, SidebarView, Transport,
    TransportEvent, WorkspaceEvent, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit,
    WorkspaceModel, WorkspaceScene, WorkspaceTransport, active_popup, attention_only_change,
    focused_terminal, labels_only_change, shown_workspace, validate_copy_uri, validate_open_uri,
};

mod geometry;
mod scroll;
mod shortcuts;

use geometry::{
    MIN_COLUMN_CELLS, admits_column, initial_window_size, rect_screen, surface_size,
    terminal_screen,
};

use scroll::{PendingViewportRequest, SelectionAutoscroll, TerminalScroll, finish_scroll_failure};

use shortcuts::{
    NativeShortcutAction, NativeShortcutTrigger, WORKSPACES_ENTRY, copy_paste_modifiers,
    detach_chord, invoke_popup, native_key_shortcut, native_key_shortcut_for,
    native_pointer_shortcut, paste_shortcut, popup_shortcut, shortcut_groups, shortcut_hints_for,
    sideways_move_opens_column, tab_shortcut_target, workspace_picker_tooltip, workspace_shortcut,
};

const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// A working badge steps one braille frame at most 10 times a second (VEN-C21, AP-PERF-001).
const SPINNER_INTERVAL: Duration = Duration::from_millis(100);
const MINUTE: Duration = Duration::from_secs(60);
const READING_INTERVAL: Duration = Duration::from_secs(2);
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_DEFERRED_SELECTION_MESSAGES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePlatform {
    Linux,
    Macos,
}

const NATIVE_PLATFORM: NativePlatform = if cfg!(target_os = "macos") {
    NativePlatform::Macos
} else {
    NativePlatform::Linux
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeClipboard {
    Standard,
    Primary,
    Both,
}

struct OrbitRetry {
    deadline: Option<Instant>,
    next_delay: Duration,
}

impl Default for OrbitRetry {
    fn default() -> Self {
        Self {
            deadline: None,
            next_delay: INITIAL_RETRY_DELAY,
        }
    }
}

impl OrbitRetry {
    fn schedule(&mut self, now: Instant) {
        if self.deadline.is_none() {
            self.deadline = Some(now + self.next_delay);
            self.next_delay = self.next_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
        }
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PresentationIdentity {
    generation: u64,
    revision: Option<u64>,
}

#[derive(Debug, Default)]
struct PresentationState {
    generation: u64,
    /// Counts invalidations; output revisions keep it, so a link target survives them.
    epoch: u64,
    presented: Option<PresentationIdentity>,
}

impl PresentationState {
    fn candidate(&self, revision: Option<u64>) -> PresentationIdentity {
        PresentationIdentity {
            generation: self.generation,
            revision,
        }
    }

    fn content_changed(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("presentation generation exhausted");
    }

    fn invalidate(&mut self) {
        self.content_changed();
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("presentation epoch exhausted");
        self.unpublish();
    }

    fn publish(&mut self, candidate: PresentationIdentity) {
        if candidate.generation == self.generation {
            self.presented = Some(candidate);
        }
    }

    fn unpublish(&mut self) {
        self.presented = None;
    }

    fn has_presented_geometry(&self) -> bool {
        self.presented.is_some()
    }

    fn presented_revision(&self) -> Option<u64> {
        self.presented.and_then(|identity| identity.revision)
    }

    fn is_current(&self, identity: PresentationIdentity) -> bool {
        identity.revision.is_some()
            && identity.generation == self.generation
            && self.presented == Some(identity)
    }
}

fn link_hint(uri: &str, entries: &[PopupEntry]) -> String {
    link_hint_for(NATIVE_PLATFORM, uri, entries)
}

fn link_hint_for(platform: NativePlatform, uri: &str, entries: &[PopupEntry]) -> String {
    let open = format!(
        "{}\n{} Open",
        uri.escape_default(),
        NativeShortcutTrigger::OpenLink.display_label_for(platform),
    );
    let copy = PhysicalKey::Code(KeyCode::KeyC);
    if native_key_shortcut_for(platform, copy, copy_paste_modifiers(platform), entries)
        .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Copy)
    {
        format!(
            "{open} · {} Copy",
            NativeShortcutTrigger::Copy.display_label_for(platform)
        )
    } else {
        open
    }
}

fn link_copy_shortcut(
    key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
    selected: bool,
    current: bool,
) -> bool {
    native_key_shortcut(key, modifiers, entries)
        .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Copy)
        && !selected
        && current
}

/// A link target: its URI over the same cells within one presentation epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkFocus {
    epoch: u64,
    row: u16,
    column: u16,
    columns: u16,
    uri: String,
}

impl LinkFocus {
    fn find<'a>(&self, scene: &'a Scene, epoch: u64) -> Option<Hyperlink<'a>> {
        scene.hyperlink_at(self.row, self.column).filter(|link| {
            epoch == self.epoch
                && (link.row, link.column, link.columns) == (self.row, self.column, self.columns)
                && link.uri == self.uri
        })
    }
}

/// A captured link press; once its target leaves the scene it only consumes the release.
#[derive(Clone)]
enum LinkPress {
    Live(LinkFocus),
    Retired,
}

#[derive(Default)]
struct LinkInteraction {
    focus: Option<LinkFocus>,
    /// The target highlighted in the last presented frame.
    shown: Option<LinkFocus>,
    unshifted: bool,
    pointer_inside: bool,
    pressed: Option<LinkPress>,
    notice: Option<LinkNotice>,
}

/// How long a link success notice shows without input (VEN-C5).
const LINK_SUCCESS_NOTICE: Duration = Duration::from_secs(2);

struct LinkNotice {
    text: String,
    /// When a success notice clears itself; other notices wait for input.
    expires: Option<Instant>,
    /// The workspace notice serial when it appeared; a later workspace notice outranks it.
    workspace_serial: u64,
}

impl LinkInteraction {
    fn show_notice(&mut self, text: &str, success: bool, now: Instant, workspace_serial: u64) {
        self.notice = Some(LinkNotice {
            text: text.into(),
            expires: success.then(|| now + LINK_SUCCESS_NOTICE),
            workspace_serial,
        });
    }

    fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| notice.text.as_str())
    }

    fn notice_deadline(&self) -> Option<Instant> {
        self.notice.as_ref()?.expires
    }

    /// Clears a success notice at its deadline, returning whether it cleared.
    fn expire_notice(&mut self, now: Instant) -> bool {
        self.notice_deadline()
            .is_some_and(|deadline| now >= deadline)
            && self.notice.take().is_some()
    }
}

struct LinkOpener {
    child: Child,
    deadline: Instant,
}

fn link_command(platform: NativePlatform, uri: &str) -> Command {
    let mut command = match platform {
        NativePlatform::Linux => {
            let mut command = Command::new("gio");
            command.args(["open", "--", uri]);
            command
        }
        NativePlatform::Macos => {
            let mut command = Command::new("/usr/bin/open");
            command.args(["-u", uri]);
            command
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn link_spawn_error(platform: NativePlatform) -> &'static str {
    match platform {
        NativePlatform::Linux => "Cannot open link: install GIO (gio) on the desktop host PATH.",
        NativePlatform::Macos => "Cannot open link: /usr/bin/open is unavailable.",
    }
}

fn run_new_window(executable: &Path) -> std::result::Result<(), String> {
    let output = Command::new(executable)
        .args(["window", "new"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Cannot open a new Eon window: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        Err(format!(
            "Cannot open a new Eon window: {}",
            if detail.is_empty() {
                output.status.to_string()
            } else {
                detail.chars().take(500).collect()
            }
        ))
    }
}

impl LinkOpener {
    fn start(uri: &str) -> std::result::Result<Self, &'static str> {
        validate_open_uri(uri)?;
        let child = link_command(NATIVE_PLATFORM, uri)
            .spawn()
            .map_err(|_| link_spawn_error(NATIVE_PLATFORM))?;
        Ok(Self {
            child,
            deadline: Instant::now() + Duration::from_secs(10),
        })
    }

    fn poll(&mut self, now: Instant) -> Option<std::result::Result<&'static str, &'static str>> {
        match self.child.try_wait() {
            Ok(Some(status)) if status.success() => Some(Ok("Link handed to the desktop handler.")),
            Ok(Some(_)) => Some(Err(
                "Could not open link. Check the desktop's HTTP/HTTPS handler.",
            )),
            Err(_) => Some(Err("Could not observe the native link dispatcher.")),
            Ok(None) if now >= self.deadline => Some(Err(
                "Link dispatcher timed out. The handler may already have opened.",
            )),
            Ok(None) => None,
        }
    }
}

impl Drop for LinkOpener {
    fn drop(&mut self) {
        // Native dispatchers hand off to a separate handler; retire only our child.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
enum SelectionGate {
    #[default]
    Ready,
    AwaitingFinish,
    AwaitingPresentation(u64),
}

impl SelectionGate {
    fn finish_sent(&mut self) {
        *self = Self::AwaitingFinish;
    }

    fn finish_received(&mut self, frame_revision: u64) {
        if matches!(self, Self::AwaitingFinish) {
            *self = Self::AwaitingPresentation(frame_revision);
        }
    }

    fn admits(&mut self, presented_revision: u64) -> bool {
        if matches!(self, Self::AwaitingPresentation(required) if presented_revision >= *required) {
            *self = Self::Ready;
        }
        matches!(self, Self::Ready)
    }
}

#[derive(Debug)]
enum UserEvent {
    AccessKit(AccessKitEvent),
    Exit,
    Present,
    Transport,
    Workspace,
    NewWindowFailed(String),
}

impl From<AccessKitEvent> for UserEvent {
    fn from(value: AccessKitEvent) -> Self {
        Self::AccessKit(value)
    }
}

struct WindowState {
    renderer: Renderer,
    scale_factor: f64,
    adapter: accesskit_winit::Adapter,
    accessibility: Accessibility,
    window: Arc<Window>,
}

#[derive(Clone, Copy)]
enum HeaderPress {
    Control(WorkspaceHeaderControl),
    Drag,
}

#[derive(Debug, Default)]
struct ShortcutViewerState {
    open: bool,
    scroll: f32,
}

#[derive(Debug, Default)]
struct AttentionInboxState {
    open: bool,
    filter: AttentionFilter,
    highlighted: Option<(String, String)>,
    scroll: f32,
}

impl AttentionInboxState {
    fn entries<'a>(
        &self,
        snapshot: &'a eon_workspace_protocol::v26::Snapshot,
    ) -> impl Iterator<Item = &'a eon_workspace_protocol::v26::QueueEntry> {
        let filter = self.filter;
        snapshot
            .attention_queue
            .iter()
            .filter(move |entry| filter.matches(entry.attention.state))
    }

    fn index(&self, snapshot: &eon_workspace_protocol::v26::Snapshot) -> Option<usize> {
        let (tab, target) = self.highlighted.as_ref()?;
        self.entries(snapshot)
            .position(|entry| entry.tab == *tab && entry.target == *target)
    }

    fn select(&mut self, snapshot: &eon_workspace_protocol::v26::Snapshot, index: usize) {
        let entries: Vec<_> = self.entries(snapshot).collect();
        self.highlighted = entries
            .get(index.min(entries.len().saturating_sub(1)))
            .map(|entry| (entry.tab.clone(), entry.target.clone()));
    }

    fn set_filter(
        &mut self,
        snapshot: &eon_workspace_protocol::v26::Snapshot,
        filter: AttentionFilter,
    ) {
        self.filter = filter;
        self.scroll = 0.0;
        self.select(snapshot, 0);
    }

    fn move_highlight(
        &mut self,
        snapshot: &eon_workspace_protocol::v26::Snapshot,
        backwards: bool,
    ) {
        let index = self.index(snapshot).unwrap_or(0);
        self.select(
            snapshot,
            if backwards {
                index.saturating_sub(1)
            } else {
                index + 1
            },
        );
    }

    fn reconcile(
        &mut self,
        snapshot: &eon_workspace_protocol::v26::Snapshot,
        previous_index: usize,
    ) {
        if self.index(snapshot).is_none() {
            self.select(snapshot, previous_index);
        }
    }

    fn refresh(
        &mut self,
        snapshot: &eon_workspace_protocol::v26::Snapshot,
        previous_index: usize,
        presentation: &mut PresentationState,
    ) -> bool {
        if !self.open {
            return false;
        }
        self.reconcile(snapshot, previous_index);
        presentation.invalidate();
        true
    }

    fn activation(&self, presentation: &PresentationState) -> Option<WorkspaceAction> {
        if !self.open || !presentation.has_presented_geometry() {
            return None;
        }
        let (tab, target) = self.highlighted.clone()?;
        Some(WorkspaceAction::FocusAttention { tab, target })
    }

    fn target(&self) -> Option<(&str, &str)> {
        self.highlighted
            .as_ref()
            .map(|(tab, target)| (tab.as_str(), target.as_str()))
    }
}

/// A `FocusAttention` sent from a sidebar or inbox row. Keyboard focus stays where it was until a snapshot
/// shows its target focused, then moves to the terminal; if the jump's own accepted answer only
/// selected the target's tab, it moves to the tab strip instead.
struct AttentionJump {
    tab: String,
    target: String,
    /// The transport's ticket for the jump's `FocusAttention`.
    ticket: u64,
}

/// A final close the prompt holds until Enter sends it.
#[derive(Debug)]
struct ClosePrompt {
    tab: String,
    action: WorkspaceAction,
    scroll: f32,
}

impl ClosePrompt {
    /// Whether the snapshot still holds the tab, and the pane, that the close names.
    fn names_live_target(&self, snapshot: &eon_workspace_protocol::v26::Snapshot) -> bool {
        snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == self.tab)
            .is_some_and(|tab| match &self.action {
                WorkspaceAction::ClosePane { pane } => tab.panes().any(|live| live.id == *pane),
                _ => true,
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ShortcutViewerCommand {
    Toggle,
    Close,
    ScrollLines(f32),
    ScrollPages(f32),
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AttentionInboxCommand {
    Toggle,
    Close,
    Filter(bool),
    Move(bool),
    Activate,
    Scroll(ShortcutViewerCommand),
}

/// One Orbit terminal this window presents: its attachment, model and viewport state.
struct TerminalView {
    orbit_socket: Option<PathBuf>,
    transport: Option<Transport>,
    model: SessionModel,
    deferred_selection: VecDeque<ClientMessage>,
    selection_gate: SelectionGate,
    last_resize: Option<SurfaceSize>,
    active_endpoint: Option<Vec<u8>>,
    active_endpoint_live: bool,
    orbit_retry: OrbitRetry,
    retry_suppressed: bool,
    scrollback_press: Option<PresentationIdentity>,
    terminal_scroll: TerminalScroll,
    selection_scroll: SelectionAutoscroll,
    pending_viewport: Option<PendingViewportRequest>,
    pending_return_to_live: bool,
    /// Set by an image resync: the next attachment's pending entries show the marker at once,
    /// until its frame has none pending (VEN-C31).
    images_resyncing: bool,
    refusal_resync: RefusalResync,
}

/// VEN-C31's one resync for refused images, across attachments. A reattach spends it; it returns
/// once the terminal's placements change or, after the next refusal, the budget gains room.
#[derive(Default)]
struct RefusalResync(Option<SpentResync>);

struct SpentResync {
    /// The placement target sizes when the resync was spent; identities change with each
    /// attachment.
    targets: Vec<SceneRect>,
    /// Resident image bytes when the frame next lacked a refused resource.
    resident: Option<u64>,
}

impl RefusalResync {
    /// Whether the refusal a frame `lacks` reattaches now, given the frame's placement `targets`
    /// and the window's `resident` image bytes.
    fn admit(
        &mut self,
        lacks: Option<ImageResync>,
        targets: Vec<SceneRect>,
        resident: u64,
    ) -> bool {
        if self.0.as_ref().is_some_and(|spent| {
            spent.targets != targets || spent.resident.is_some_and(|spent| resident < spent)
        }) {
            self.0 = None;
        }
        match (&mut self.0, lacks) {
            (_, None) => false,
            (Some(spent), Some(_)) => {
                spent.resident.get_or_insert(resident);
                false
            }
            (None, Some(lacks)) => {
                let resync = lacks == ImageResync::Refused;
                if resync {
                    self.0 = Some(SpentResync {
                        targets,
                        resident: None,
                    });
                }
                resync
            }
        }
    }
}

impl TerminalView {
    fn new(orbit_socket: Option<PathBuf>) -> Self {
        Self {
            orbit_socket,
            transport: None,
            model: SessionModel::new(),
            deferred_selection: VecDeque::new(),
            selection_gate: SelectionGate::default(),
            last_resize: None,
            active_endpoint: None,
            active_endpoint_live: false,
            orbit_retry: OrbitRetry::default(),
            retry_suppressed: false,
            scrollback_press: None,
            terminal_scroll: TerminalScroll::default(),
            selection_scroll: SelectionAutoscroll::default(),
            pending_viewport: None,
            pending_return_to_live: false,
            images_resyncing: false,
            refusal_resync: RefusalResync::default(),
        }
    }

    /// Whether this terminal reattaches now to recover its images (VEN-C31): whenever its
    /// attached frame references a resource the budget freed, and for refusals as
    /// [`RefusalResync`] allows.
    fn wants_image_resync(
        &mut self,
        renderer: &Renderer,
        referenced: &HashSet<ImageKey>,
        resident: u64,
    ) -> bool {
        let Some(graphics) = self.model.scene().map(|scene| &scene.graphics) else {
            return false;
        };
        let lacks = self
            .model
            .image_attachment()
            .filter(|attachment| *attachment == graphics.attachment)
            .and_then(|attachment| {
                renderer.image_resync(attachment, &graphics.placements, referenced)
            });
        // Sizes only: rows that scroll move every target without changing what is shown.
        let targets = graphics
            .placements
            .iter()
            .map(|placement| SceneRect {
                left: 0.0,
                top: 0.0,
                ..placement.target
            })
            .collect();
        lacks == Some(ImageResync::Freed) || self.refusal_resync.admit(lacks, targets, resident)
    }
}

/// Every resource that an attached terminal's current frame references, which the budget never
/// frees (VEN-C31).
fn image_references<'a>(views: impl IntoIterator<Item = &'a TerminalView>) -> HashSet<ImageKey> {
    views
        .into_iter()
        .filter_map(|view| {
            let attachment = view.model.image_attachment()?;
            let scene = view
                .model
                .scene()
                .filter(|scene| scene.graphics.attachment == attachment)?;
            Some(
                scene
                    .graphics
                    .placements
                    .iter()
                    .map(move |placement| ImageKey {
                        attachment,
                        resource: placement.resource,
                    }),
            )
        })
        .flatten()
        .collect()
}

struct Application {
    #[cfg_attr(
        target_os = "macos",
        expect(dead_code, reason = "macOS application identity is not mapped yet")
    )]
    application_id: String,
    workspace_socket: Option<PathBuf>,
    new_window_executable: Option<PathBuf>,
    supervised: bool,
    startup_admission: bool,
    decorations: bool,
    pane_frames: bool,
    focus_color: Color,
    border_width: u8,
    focus_border_width: u8,
    /// Whether working badges may turn; Eon's `--animate-working-badge` (VEN-C21).
    animate_working_badge: bool,
    eon_version: Option<String>,
    bar_items: Vec<BarItem>,
    background_opacity: f32,
    padding_color: PaddingColor,
    background_blur: bool,
    cursor_tail: Option<(Color, f32)>,
    fonts: FontSettings,
    columns: Option<u16>,
    rows: Option<u16>,
    fatal_error: Option<String>,
    initial_scale_pending: bool,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<WindowState>,
    /// The terminal that receives keyboard and pointer input: the selected column's,
    /// a popup's, or the standalone session's.
    terminal: TerminalView,
    /// Display-only terminals of the active tab's other columns, one per expanded live pane.
    column_views: Vec<TerminalView>,
    /// The selected terminal's presented identity, which pairs its input; column output
    /// repaints without changing it.
    presentation: PresentationState,
    links: LinkInteraction,
    workspace_transport: Option<WorkspaceTransport>,
    workspace_model: WorkspaceModel,
    input: InputState,
    input_epoch: Instant,
    link_opener: Option<LinkOpener>,
    render_notice: Option<String>,
    blink_visible: bool,
    next_blink: Option<Instant>,
    next_spinner: Option<Instant>,
    next_animation: Option<Instant>,
    /// The image marker deadline a redraw was last requested for, so a redraw that never derives
    /// the next one, as after a renderer failure, wakes Venus once rather than on every turn.
    marker_wake: Option<Instant>,
    clock: Option<LocalClock>,
    next_clock: Option<Instant>,
    /// The previous CPU reading and when it was taken.
    cpu_times: Option<(Instant, CpuTimes)>,
    cpu: Option<CpuUse>,
    ram: Option<RamUse>,
    next_reading: Option<Instant>,
    window_focused: bool,
    window_occluded: bool,
    workspace_focus: WorkspaceFocus,
    /// The agent sidebar row that holds keyboard focus while `workspace_focus` is `Sidebar`.
    sidebar_row: Option<String>,
    /// A sidebar row's jump awaiting its result, which focus follows (VEN-C30).
    attention_jump: Option<AttentionJump>,
    /// When a sidebar row's age next changes its label.
    next_age: Option<Instant>,
    header_press: Option<HeaderPress>,
    shortcut_viewer: ShortcutViewerState,
    attention_inbox: AttentionInboxState,
    warning_close_last_tab: bool,
    close_prompt: Option<ClosePrompt>,
    hints_visible: bool,
    hints: Option<ShortcutHintsScene>,
    zoom: Option<ZoomTarget>,
    tab_texts: HashMap<String, (String, f32)>,
    quota_widths: HashMap<String, f32>,
    /// Fitted header texts, the workspace label's name, by their text.
    fitted_texts: HashMap<String, (String, f32)>,
    tab_scroll: f32,
    pane_scroll: f32,
    cursor: PhysicalPosition<f64>,
}

impl Application {
    fn new(arguments: LaunchArguments, proxy: EventLoopProxy<UserEvent>) -> Self {
        let LaunchArguments {
            application_id,
            orbit_socket,
            workspace_socket,
            supervised,
            startup_admission,
            decorations,
            pane_frames,
            focus_color,
            border_width,
            focus_border_width,
            warning_close_last_tab,
            animate_working_badge,
            eon_version,
            bar_items,
            background_opacity,
            padding_color,
            background_blur,
            cursor_tail,
            fonts,
            columns,
            rows,
        } = arguments;
        let initial_scale_pending = startup_admission
            || columns.is_some()
            || rows.is_some()
            || fonts != FontSettings::default();
        Self {
            application_id,
            terminal: TerminalView::new(orbit_socket),
            column_views: Vec::new(),
            presentation: PresentationState::default(),
            links: LinkInteraction::default(),
            workspace_socket,
            new_window_executable: std::env::var_os("EON_NEW_WINDOW_EXECUTABLE")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            supervised,
            startup_admission,
            decorations,
            pane_frames,
            focus_color,
            border_width,
            focus_border_width,
            animate_working_badge,
            eon_version,
            bar_items,
            background_opacity,
            padding_color,
            background_blur,
            cursor_tail,
            fonts,
            columns,
            rows,
            proxy,
            fatal_error: None,
            initial_scale_pending,
            window: None,
            workspace_transport: None,
            workspace_model: WorkspaceModel::default(),
            input: InputState::default(),
            input_epoch: Instant::now(),
            link_opener: None,
            render_notice: None,
            blink_visible: true,
            next_blink: None,
            next_spinner: None,
            marker_wake: None,
            clock: None,
            next_clock: None,
            cpu_times: None,
            cpu: None,
            ram: None,
            next_reading: None,
            next_animation: None,
            window_focused: false,
            window_occluded: false,
            workspace_focus: WorkspaceFocus::Terminal,
            sidebar_row: None,
            attention_jump: None,
            next_age: None,
            header_press: None,
            shortcut_viewer: ShortcutViewerState::default(),
            attention_inbox: AttentionInboxState::default(),
            warning_close_last_tab,
            close_prompt: None,
            hints_visible: true,
            hints: None,
            zoom: None,
            tab_texts: HashMap::new(),
            quota_widths: HashMap::new(),
            fitted_texts: HashMap::new(),
            tab_scroll: 0.0,
            pane_scroll: 0.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result {
        // The Darwin package wrapper sets this list; Linux gets its fonts from FONTCONFIG_FILE.
        let fonts = FontSetup::with_directories(
            &self.fonts,
            std::env::var_os("EON_VENUS_FONT_DIRS").as_deref(),
        )?;
        let initial = initial_window_size(
            self.columns,
            self.rows,
            fonts.metrics(1.0),
            1.0,
            self.workspace_model.snapshot(),
        )?;
        let attributes = window_attributes(
            self.decorations,
            self.background_opacity,
            self.background_blur,
        )
        .with_inner_size(initial.to_logical::<f64>(1.0));
        #[cfg(target_os = "linux")]
        let attributes = attributes.with_name(self.application_id.as_str(), "yazelix-venus");
        let window = Arc::new(event_loop.create_window(attributes)?);
        if self.initial_scale_pending && window.scale_factor() != 1.0 {
            let initial = initial_window_size(
                self.columns,
                self.rows,
                fonts.metrics(window.scale_factor()),
                window.scale_factor(),
                self.workspace_model.snapshot(),
            )?;
            let _ = window.request_inner_size(initial);
        }
        let accessibility = Accessibility::new(window.inner_size());
        let adapter = accesskit_winit::Adapter::with_mixed_handlers(
            event_loop,
            &window,
            accessibility.activation(),
            self.proxy.clone(),
        );
        let renderer = pollster::block_on(Renderer::new(
            Arc::clone(&window),
            event_loop,
            self.background_opacity,
            self.padding_color,
            self.cursor_tail,
            self.pane_frames,
            self.focus_color,
            self.border_width,
            self.focus_border_width,
            self.animate_working_badge,
            fonts,
        ))?;
        window.set_visible(true);
        let scale_factor = window.scale_factor();
        report(format_args!("window scale factor {scale_factor}"));

        self.window = Some(WindowState {
            renderer,
            scale_factor,
            adapter,
            accessibility,
            window,
        });
        if !self.initial_scale_pending {
            self.start_initial_attachment();
        } else {
            self.reveal_workspace_selection();
        }
        self.refresh_client_view();
        Ok(())
    }

    fn start_initial_attachment(&mut self) {
        if self.workspace_transport.is_none()
            && let Some(socket) = self.workspace_socket.clone()
        {
            self.start_workspace(socket);
        }
        self.reveal_workspace_selection();
        if self.workspace_socket.is_some() {
            if let Some((endpoint, live)) = self.workspace_model.active_attachment() {
                self.set_orbit_attachment(Some(endpoint.to_vec()), live);
                self.reconcile_column_views();
            } else if self.workspace_model.snapshot().is_some() {
                self.set_workspace_focus(WorkspaceFocus::Tabs);
            }
        } else {
            self.start_orbit(
                self.terminal
                    .orbit_socket
                    .clone()
                    .expect("standalone launch has an Orbit socket"),
            );
        }
    }

    fn start_workspace(&mut self, socket: PathBuf) {
        let proxy = self.proxy.clone();
        self.workspace_transport = Some(WorkspaceTransport::start(socket, move || {
            let _ = proxy.send_event(UserEvent::Workspace);
        }));
    }

    fn start_orbit(&mut self, socket: PathBuf) {
        self.terminal.orbit_socket = Some(socket.clone());
        self.terminal.transport = Some(orbit_transport(&self.proxy, socket));
    }

    fn reset_cursor_animation(&mut self) {
        self.next_animation = None;
        if let Some(state) = &mut self.window {
            state.renderer.reset_cursor_animation();
        }
    }

    fn cancel_terminal_scroll(&mut self) {
        let redraw = self
            .terminal
            .terminal_scroll
            .active(self.terminal.pending_viewport);
        self.terminal
            .terminal_scroll
            .cancel(&mut self.terminal.pending_viewport);
        if !redraw {
            return;
        }
        if let Some(state) = &mut self.window {
            state.renderer.reset_cursor_animation();
            state.window.request_redraw();
        }
    }

    fn drive_terminal_scroll(&mut self) {
        if self.terminal.pending_viewport == Some(PendingViewportRequest::SelectionTick) {
            return;
        }
        let Some(frame_revision) = self.terminal.model.scene().map(|scene| scene.revision) else {
            self.terminal.terminal_scroll.reset();
            self.terminal.pending_viewport = None;
            return;
        };
        let Some(metrics) = self.window.as_ref().map(|state| state.renderer.metrics()) else {
            return;
        };
        let (lines, request) = {
            let preview = self.terminal.model.scroll_preview();
            (
                self.terminal.terminal_scroll.resolve_preview(preview),
                self.terminal.terminal_scroll.next_request(
                    self.terminal.pending_viewport,
                    frame_revision,
                    preview,
                    f64::from(metrics.height),
                    self.terminal
                        .model
                        .preview_may_be_stale()
                        .then(Instant::now),
                ),
            )
        };
        if let Some(lines) = lines {
            for message in self.input.wheel(
                MouseScrollDelta::LineDelta(0.0, lines as f32),
                metrics.width,
                metrics.height,
            ) {
                if !self.send(message) {
                    break;
                }
            }
        }
        if let Some(message) = request
            && !self.send(message)
        {
            self.terminal.terminal_scroll.reset();
        }
        if !self
            .terminal
            .terminal_scroll
            .active(self.terminal.pending_viewport)
        {
            self.terminal.model.clear_viewport_preview();
        }
    }

    fn drive_selection_autoscroll(&mut self, now: Instant) {
        if self
            .terminal
            .selection_scroll
            .next
            .is_none_or(|deadline| now < deadline)
        {
            return;
        }
        let position = (self.window_focused
            && !self.window_occluded
            && !self.dialog_open()
            && self.terminal.model.is_attached()
            && self.terminal.deferred_selection.is_empty()
            && matches!(self.terminal.selection_gate, SelectionGate::Ready)
            && self.terminal.pending_viewport.is_none()
            && self.presentation.presented_revision()
                == self.terminal.model.scene().map(|scene| scene.revision))
        .then(|| {
            self.window.as_ref().and_then(|state| {
                self.terminal_size()
                    .and_then(|screen| surface_size(screen, state.renderer.metrics()))
            })
        })
        .flatten()
        .filter(|size| self.terminal.last_resize == Some(*size))
        .and_then(|size| self.input.selection_at_top(size));
        let Some(position) = position else {
            self.terminal.selection_scroll.next = None;
            return;
        };
        if self
            .terminal
            .selection_scroll
            .take_due(now, self.terminal.pending_viewport)
            && !self.send(ClientMessage::Selection(SelectionAction::AutoscrollUp {
                position,
            }))
        {
            self.terminal.selection_scroll.fail();
        }
    }

    fn workspace_scene(&self) -> Option<WorkspaceScene> {
        let state = self.window.as_ref()?;
        self.workspace_model.snapshot().map(|snapshot| {
            let picker = workspace_picker_tooltip(&snapshot.entries);
            WorkspaceScene::from_snapshot_with_zoom(
                snapshot,
                EonBar {
                    items: &self.bar_items,
                    version: self.eon_version.as_deref(),
                    cpu: self.cpu.as_ref(),
                    ram: self.ram.as_ref(),
                    clock: self.clock.as_ref(),
                    workspace_picker: picker.as_deref(),
                },
                state.renderer.size(),
                state.renderer.metrics(),
                self.tab_scroll,
                self.pane_scroll,
                // Replays the last measurement, which every change to these texts rebuilds.
                |kind, label| match kind {
                    HeaderText::Tab(id) => self.tab_texts[id].clone(),
                    HeaderText::Fact => (label.to_owned(), self.quota_widths[label]),
                    HeaderText::Fit(_) => self.fitted_texts[label].clone(),
                },
                self.hints.clone(),
                self.zoomed(),
                sidebar_view(self.workspace_focus, self.sidebar_row.as_deref()),
            )
        })
    }

    /// Whether the selected pane and the active tab's layout still match the zoom target.
    fn zoomed(&self) -> bool {
        self.zoom.is_some() && self.workspace_model.snapshot().and_then(zoom_target) == self.zoom
    }

    fn popup_entries(&self) -> &[PopupEntry] {
        self.workspace_model
            .snapshot()
            .map_or(&[][..], |snapshot| snapshot.entries.as_slice())
    }

    /// Whether a dialog takes input from the workspace and terminal.
    fn dialog_open(&self) -> bool {
        self.shortcut_viewer.open || self.close_prompt.is_some() || self.attention_inbox.open
    }

    /// The open dialog: the close prompt, else the shortcut viewer.
    fn dialog_scene(&self) -> Option<DialogScene> {
        if self.attention_inbox.open {
            let state = self.window.as_ref()?;
            return Some(DialogScene::attention_inbox(
                self.workspace_model.snapshot()?,
                self.attention_inbox.filter,
                self.attention_inbox.target(),
                state.renderer.size(),
                state.renderer.metrics(),
                self.attention_inbox.scroll,
                sidebar_view(self.workspace_focus, None).now,
            ));
        }
        let Some(prompt) = &self.close_prompt else {
            return self.shortcut_viewer_scene();
        };
        let state = self.window.as_ref()?;
        let snapshot = self.workspace_model.snapshot()?;
        let tab = snapshot.tabs.iter().find(|tab| tab.id == prompt.tab)?;
        // Worded from the current snapshot, so an open prompt follows the workspace count.
        Some(DialogScene::close_last_tab(
            tab,
            &snapshot.entries,
            detach_chord(&snapshot.entries),
            close_prompt_workspace(snapshot),
            state.renderer.size(),
            state.renderer.metrics(),
            prompt.scroll,
        ))
    }

    fn shortcut_viewer_scene(&self) -> Option<DialogScene> {
        let state = self.window.as_ref()?;
        self.shortcut_viewer.open.then(|| {
            DialogScene::new(
                DialogKind::Shortcuts,
                shortcut_groups(
                    self.popup_entries(),
                    self.workspace_model
                        .snapshot()
                        .map_or(Sidebar::Off, |snapshot| snapshot.sidebar),
                ),
                state.renderer.size(),
                state.renderer.metrics(),
                self.shortcut_viewer.scroll,
            )
        })
    }

    fn set_shortcut_viewer(&mut self, open: bool) {
        if self.shortcut_viewer.open == open || open && self.workspace_model.snapshot().is_none() {
            return;
        }
        self.change_dialog(|app| {
            app.shortcut_viewer.open = open;
            app.shortcut_viewer.scroll = 0.0;
        });
    }

    fn set_attention_inbox(&mut self, open: bool) {
        if self.attention_inbox.open == open || open && self.workspace_model.snapshot().is_none() {
            return;
        }
        self.change_dialog(|app| {
            app.attention_inbox = AttentionInboxState {
                open,
                ..AttentionInboxState::default()
            };
            if open && let Some(snapshot) = app.workspace_model.snapshot() {
                app.attention_inbox.select(snapshot, 0);
            }
            app.presentation.invalidate();
        });
    }

    fn reveal_inbox_highlight(&mut self) {
        if !self.attention_inbox.open {
            return;
        }
        if let Some(dialog) = self.dialog_scene() {
            let scroll = dialog.attention_highlight_scroll();
            if self.attention_inbox.scroll != scroll {
                self.attention_inbox.scroll = scroll;
                self.presentation.invalidate();
            }
        }
    }

    fn filter_attention_inbox(&mut self, filter: AttentionFilter) {
        if let Some(snapshot) = self.workspace_model.snapshot() {
            self.attention_inbox.set_filter(snapshot, filter);
            self.presentation.invalidate();
            self.refresh_client_view();
        }
    }

    fn activate_inbox_row(&mut self, tab: String, target: String) {
        if !self.presentation.has_presented_geometry() {
            return;
        }
        self.set_attention_inbox(false);
        self.focus_attention(tab, target);
    }

    fn handle_attention_inbox_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        if !self.attention_inbox.open && self.dialog_open() {
            return false;
        }
        let command = match event.physical_key {
            PhysicalKey::Code(code) => attention_inbox_command(
                self.attention_inbox.open,
                code,
                self.input.modifiers(),
                event.repeat,
                self.popup_entries(),
            ),
            PhysicalKey::Unidentified(_) => None,
        };
        let captures = self.attention_inbox.open
            || command.is_some() && self.workspace_model.snapshot().is_some();
        if !self
            .input
            .consumes_shortcut(event.physical_key, event.state, event.repeat, captures)
        {
            return false;
        }
        if event.state == ElementState::Pressed {
            match command {
                Some(AttentionInboxCommand::Toggle) => {
                    self.set_attention_inbox(!self.attention_inbox.open)
                }
                Some(AttentionInboxCommand::Close) => self.set_attention_inbox(false),
                Some(AttentionInboxCommand::Filter(backwards)) => {
                    self.filter_attention_inbox(self.attention_inbox.filter.step(backwards))
                }
                Some(AttentionInboxCommand::Move(backwards)) => {
                    if let Some(snapshot) = self.workspace_model.snapshot() {
                        self.attention_inbox.move_highlight(snapshot, backwards);
                        self.reveal_inbox_highlight();
                        self.refresh_client_view();
                    }
                }
                Some(AttentionInboxCommand::Activate) => {
                    if let Some(WorkspaceAction::FocusAttention { tab, target }) =
                        self.attention_inbox.activation(&self.presentation)
                    {
                        self.activate_inbox_row(tab, target);
                    }
                }
                Some(AttentionInboxCommand::Scroll(command)) => self.scroll_dialog_by(command),
                None => {}
            }
        }
        true
    }

    /// Opens or closes a dialog, reporting the terminal's focus change with it.
    fn change_dialog(&mut self, change: impl FnOnce(&mut Self)) {
        let was_terminal_focused =
            terminal_focused(self.window_focused, self.workspace_focus) && !self.dialog_open();
        self.cancel_terminal_scroll();
        self.cancel_pointer_sequence();
        change(self);
        self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll);
        let is_terminal_focused =
            terminal_focused(self.window_focused, self.workspace_focus) && !self.dialog_open();
        let message = self.input.terminal_focus(is_terminal_focused);
        if was_terminal_focused != is_terminal_focused {
            self.send(message);
        }
        self.refresh_client_view();
    }

    /// Alt+Shift+D detaches from any workspace state, as a compositor close request does.
    fn handle_detach_key(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
    ) -> bool {
        let detaches = self.workspace_model.snapshot().is_some()
            && native_key_shortcut(
                event.physical_key,
                self.input.modifiers(),
                self.popup_entries(),
            )
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Detach);
        // Any other key, including a repeat another handler captured, passes on.
        if !detaches
            || !self
                .input
                .consumes_shortcut(event.physical_key, event.state, event.repeat, true)
        {
            return false;
        }
        if shortcut_is_ready(event.state, event.repeat) {
            event_loop.exit();
        }
        true
    }

    /// While the close prompt is open, Enter sends its close, Escape dismisses it, the
    /// viewer's navigation keys scroll it, and other keys reach nothing.
    fn handle_close_prompt_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        if self.close_prompt.is_none()
            || !self
                .input
                .consumes_shortcut(event.physical_key, event.state, event.repeat, true)
        {
            return false;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return true;
        };
        if event.state != ElementState::Pressed {
            return true;
        }
        let modifiers = self.input.modifiers();
        match close_prompt_answer(code, modifiers).filter(|_| !event.repeat) {
            Some(quit) => {
                let mut prompt = None;
                self.change_dialog(|app| prompt = app.close_prompt.take());
                if quit && let Some(prompt) = prompt {
                    self.send_close(prompt);
                }
            }
            None => {
                if let Some(command) = shortcut_viewer_command(true, code, modifiers, event.repeat)
                {
                    self.scroll_dialog_by(command);
                }
            }
        }
        true
    }

    /// Sends the prompt's close. A tab created meanwhile is active, and Eon closes only in
    /// the active tab, so the named tab is focused first.
    fn send_close(&mut self, prompt: ClosePrompt) {
        let inactive = self
            .workspace_model
            .snapshot()
            .is_some_and(|snapshot| snapshot.active_tab != prompt.tab);
        if !inactive
            || self.queue_workspace(WorkspaceAction::Workspace(CommonAction::FocusId(
                prompt.tab,
            )))
        {
            self.queue_workspace(prompt.action);
        }
    }

    fn handle_shortcut_viewer_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        if !self.shortcut_viewer.open && self.workspace_model.snapshot().is_none() {
            return false;
        }
        if native_key_shortcut(
            event.physical_key,
            self.input.modifiers(),
            self.popup_entries(),
        )
        .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::NewWindow)
        {
            return false;
        }
        let command = match event.physical_key {
            PhysicalKey::Code(code) => shortcut_viewer_command(
                self.shortcut_viewer.open,
                code,
                self.input.modifiers(),
                event.repeat,
            ),
            PhysicalKey::Unidentified(_) => None,
        };
        if !self.input.consumes_shortcut(
            event.physical_key,
            event.state,
            event.repeat,
            self.shortcut_viewer.open || command.is_some(),
        ) {
            return false;
        }
        if event.state == ElementState::Pressed {
            match command {
                Some(ShortcutViewerCommand::Toggle) => {
                    self.set_shortcut_viewer(!self.shortcut_viewer.open)
                }
                Some(ShortcutViewerCommand::Close) => self.set_shortcut_viewer(false),
                Some(command) => self.scroll_dialog_by(command),
                None => {}
            }
        }
        true
    }

    /// The open dialog's scroll: the close prompt's, else the shortcut viewer's.
    fn dialog_scroll(&mut self) -> &mut f32 {
        if self.attention_inbox.open {
            return &mut self.attention_inbox.scroll;
        }
        match &mut self.close_prompt {
            Some(prompt) => &mut prompt.scroll,
            None => &mut self.shortcut_viewer.scroll,
        }
    }

    fn scroll_dialog_by(&mut self, command: ShortcutViewerCommand) {
        match command {
            ShortcutViewerCommand::ScrollLines(lines) => {
                let height = self
                    .window
                    .as_ref()
                    .map_or(0.0, |state| state.renderer.metrics().height);
                self.scroll_dialog(lines * height)
            }
            ShortcutViewerCommand::ScrollPages(pages) => {
                let height = self
                    .dialog_scene()
                    .map_or(0.0, |dialog| dialog.content.height);
                self.scroll_dialog(pages * height)
            }
            ShortcutViewerCommand::Start => {
                self.scroll_dialog(-f32::MAX);
            }
            ShortcutViewerCommand::End => {
                self.scroll_dialog(f32::MAX);
            }
            ShortcutViewerCommand::Toggle | ShortcutViewerCommand::Close => {}
        }
    }

    fn scroll_dialog(&mut self, pixels: f32) {
        if !pixels.is_finite() {
            return;
        }
        let Some(dialog) = self.dialog_scene() else {
            return;
        };
        let scroll = (dialog.scroll + pixels).clamp(0.0, dialog.max_scroll);
        if *self.dialog_scroll() != scroll {
            *self.dialog_scroll() = scroll;
            if self.attention_inbox.open {
                if let Some(target) = self.dialog_scene().and_then(|dialog| {
                    dialog
                        .attention_scrolled_target()
                        .map(|(tab, target)| (tab.to_owned(), target.to_owned()))
                }) {
                    self.attention_inbox.highlighted = Some(target);
                }
                self.presentation.invalidate();
            }
            self.refresh_client_view();
        }
    }

    fn terminal_size(&self) -> Option<PhysicalSize<u32>> {
        if self.workspace_model.snapshot().is_some()
            && self.workspace_model.active_attachment().is_none()
        {
            return None;
        }
        let state = self.window.as_ref()?;
        Some(terminal_screen(
            self.workspace_scene().as_ref(),
            state.renderer.size(),
        ))
    }

    fn scrollback_label(&self, workspace: Option<&WorkspaceScene>) -> Option<String> {
        let state = self.window.as_ref()?;
        let scene = self.terminal.model.scene()?;
        let size = surface_size(
            terminal_screen(workspace, state.renderer.size()),
            state.renderer.metrics(),
        )?;
        if scene.columns != size.cols || scene.rows != size.rows {
            return None;
        }
        self.terminal.model.scrollback_label()
    }

    fn visible_scrollback_label(&self, workspace: Option<&WorkspaceScene>) -> Option<String> {
        self.scrollback_label(workspace).filter(|_| {
            workspace.is_some_and(|workspace| workspace.popup_label().is_none())
                || (!self.input.pointer_busy()
                    && self.links.focus.is_none()
                    && !self
                        .terminal
                        .model
                        .scene()
                        .is_some_and(Scene::has_selected_content))
        })
    }

    fn scrollback_rect(&self, workspace: Option<&WorkspaceScene>) -> Option<SceneRect> {
        self.visible_scrollback_label(workspace)?;
        if !self
            .presentation
            .is_current(self.presentation_candidate(workspace))
        {
            return None;
        }
        self.window.as_ref()?.renderer.presented_scrollback_rect()
    }

    fn presentation_candidate(&self, workspace: Option<&WorkspaceScene>) -> PresentationIdentity {
        let revision = self
            .terminal
            .model
            .scene()
            .filter(|_| {
                self.terminal.model.is_attached() && !self.terminal.model.awaiting_current_frame()
            })
            .and_then(|scene| {
                let state = self.window.as_ref()?;
                let screen = terminal_screen(workspace, state.renderer.size());
                surface_size(screen, state.renderer.metrics()).map(|_| scene.revision)
            });
        self.presentation.candidate(revision)
    }

    fn rebuild_workspace_measurements(
        &mut self,
        tab_scroll: f32,
        pane_scroll: f32,
    ) -> Option<WorkspaceScene> {
        let zoom = self.zoomed();
        let state = self.window.as_mut()?;
        let snapshot = self.workspace_model.snapshot()?;
        let zoom = WorkspaceScene::zoom_visible(snapshot, zoom);
        self.hints = self.hints_visible.then(|| {
            ShortcutHintsScene::new(
                if self.attention_inbox.open {
                    yazelix_venus::attention_inbox_hints()
                } else {
                    shortcut_hints_for(
                        NATIVE_PLATFORM,
                        self.workspace_focus,
                        &snapshot.entries,
                        snapshot.sidebar,
                    )
                },
                state.renderer.size(),
                state.renderer.metrics(),
                self.workspace_focus,
                |text| state.renderer.measure_header_text(text).1,
            )
        });
        self.tab_texts.clear();
        self.quota_widths.clear();
        self.fitted_texts.clear();
        let picker = workspace_picker_tooltip(&snapshot.entries);
        Some(WorkspaceScene::from_snapshot_with_zoom(
            snapshot,
            EonBar {
                items: &self.bar_items,
                version: self.eon_version.as_deref(),
                cpu: self.cpu.as_ref(),
                ram: self.ram.as_ref(),
                clock: self.clock.as_ref(),
                workspace_picker: picker.as_deref(),
            },
            state.renderer.size(),
            state.renderer.metrics(),
            tab_scroll,
            pane_scroll,
            |kind, label| match kind {
                HeaderText::Tab(id) => {
                    let fitted = state
                        .renderer
                        .fit_tab_text(label, zoom && id == snapshot.active_tab);
                    self.tab_texts.insert(id.to_owned(), fitted.clone());
                    fitted
                }
                HeaderText::Fact => {
                    let fitted = state.renderer.measure_header_text(label);
                    self.quota_widths.insert(label.to_owned(), fitted.1);
                    fitted
                }
                HeaderText::Fit(width) => {
                    let fitted = state.renderer.fit_header_text(label, width);
                    self.fitted_texts.insert(label.to_owned(), fitted.clone());
                    fitted
                }
            },
            self.hints.clone(),
            zoom,
            sidebar_view(self.workspace_focus, self.sidebar_row.as_deref()),
        ))
    }

    fn reveal_workspace_selection(&mut self) {
        let Some(scene) = self.rebuild_workspace_measurements(0.0, 0.0) else {
            return;
        };
        self.tab_scroll = scene.active_tab_scroll();
        self.pane_scroll = scene.selected_pane_scroll();
    }

    /// Puts keyboard focus on the sidebar row whose target is `target`, while it shows.
    fn focus_sidebar_row(&mut self, target: String) {
        if self
            .workspace_scene()
            .is_none_or(|scene| scene.sidebar_row(&target).is_none())
        {
            return;
        }
        if self.workspace_focus != WorkspaceFocus::Sidebar {
            self.sidebar_row = Some(target);
            self.set_workspace_focus(WorkspaceFocus::Sidebar);
        } else if self.sidebar_row.as_ref() != Some(&target) {
            self.sidebar_row = Some(target);
            // Revealing the row can scroll every row, and rows are hit targets.
            self.presentation.invalidate();
            self.refresh_client_view();
        }
    }

    /// Sends `FocusAttention` for a sidebar row; focus follows the result (`AttentionJump`).
    fn activate_sidebar_row(&mut self, target: &str) {
        if !self.presentation.has_presented_geometry() {
            return;
        }
        let Some(tab) = self
            .workspace_scene()
            .and_then(|scene| scene.sidebar_row(target).map(|row| row.tab.clone()))
        else {
            return;
        };
        self.focus_attention(tab, target.to_owned());
    }

    fn focus_attention(&mut self, tab: String, target: String) {
        if let Some(ticket) = self.queue_workspace_ticket(WorkspaceAction::FocusAttention {
            tab: tab.clone(),
            target: target.clone(),
        }) {
            self.attention_jump = Some(AttentionJump {
                tab,
                target,
                ticket,
            });
        }
    }

    /// Settles a pending row jump: on any snapshot that focuses its exact target, or on its own
    /// answer. A rejected or lost answer leaves focus on the row, beside its notice.
    fn settle_attention_jump(&mut self, answer: Option<(u64, bool)>) {
        let Some(jump) = &self.attention_jump else {
            return;
        };
        let answered = answer.filter(|(ticket, _)| *ticket == jump.ticket);
        let focus = match answered {
            Some((_, false)) => None,
            _ => self
                .workspace_model
                .snapshot()
                .and_then(|snapshot| jump_focus(jump, snapshot, answered.is_some())),
        };
        if answered.is_some() || focus.is_some() {
            self.attention_jump = None;
        }
        if let Some(focus) = focus {
            self.set_workspace_focus(focus);
        }
    }

    /// Returns keyboard focus to the terminal when the sidebar holds it but closes, hides for
    /// width or loses the focused row, and to the tabs when the workspace label holds it but
    /// hides or turns inert.
    fn retain_chrome_focus(&mut self) {
        if self.workspace_focus == WorkspaceFocus::Sidebar
            && !self.sidebar_row.as_deref().is_some_and(|row| {
                self.workspace_scene()
                    .is_some_and(|scene| scene.sidebar_row(row).is_some())
            })
        {
            self.set_workspace_focus(WorkspaceFocus::Terminal);
        }
        let label = WorkspaceHeaderControl::Workspaces;
        if self.workspace_focus == WorkspaceFocus::Header(label)
            && !self
                .workspace_scene()
                .is_some_and(|scene| scene.has_control(label))
        {
            self.set_workspace_focus(WorkspaceFocus::Tabs);
        }
    }

    fn set_workspace_focus(&mut self, focus: WorkspaceFocus) {
        let focus = if focus == WorkspaceFocus::Terminal
            && self.workspace_model.snapshot().is_some()
            && self.workspace_model.active_attachment().is_none()
        {
            WorkspaceFocus::Tabs
        } else {
            focus
        };
        if self.workspace_focus == focus {
            return;
        }
        // Focus left for somewhere other than the jump's row, so the jump no longer decides it.
        if focus != WorkspaceFocus::Sidebar {
            self.attention_jump = None;
        }
        let was_focused =
            terminal_focused(self.window_focused, self.workspace_focus) && !self.dialog_open();
        self.cancel_terminal_scroll();
        let was = self.workspace_focus;
        self.workspace_focus = focus;
        self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll);
        // Focus changes hints and controls, not the terminal's presented geometry; entering or
        // leaving the sidebar can scroll its rows, which are hit targets.
        if focus == WorkspaceFocus::Sidebar || was == WorkspaceFocus::Sidebar {
            self.presentation.invalidate();
        } else {
            self.presentation.content_changed();
        }
        let is_focused =
            terminal_focused(self.window_focused, self.workspace_focus) && !self.dialog_open();
        if was_focused != is_focused {
            if !is_focused {
                self.cancel_pointer_sequence();
            }
            let message = self.input.terminal_focus(is_focused);
            self.send(message);
        }
        self.refresh_client_view();
    }

    fn toggle_hints(&mut self) {
        self.cancel_terminal_scroll();
        self.cancel_pointer_sequence();
        self.hints_visible = !self.hints_visible;
        self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll);
        self.presentation.invalidate();
        self.send_resize();
        // The hints line changes the bar's inset, which can hide the workspace label.
        self.retain_chrome_focus();
        self.refresh_client_view();
    }

    /// Show the selected pane alone across the tab, or restore its columns. Hidden
    /// attachments keep their last size.
    fn toggle_zoom(&mut self) {
        let Some(snapshot) = self.workspace_model.snapshot() else {
            return;
        };
        let has_columns = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab)
            .is_some_and(|tab| !tab.columns.is_empty());
        if active_popup(snapshot).is_some() || !has_columns {
            return;
        }
        let Some(target) = zoom_target(snapshot) else {
            if self
                .workspace_model
                .reject_action("Nothing to zoom: this tab has one pane.")
            {
                self.refresh_client_view();
            }
            return;
        };
        self.zoom = (!self.zoomed()).then_some(target);
        self.cancel_terminal_scroll();
        self.cancel_pointer_sequence();
        if let Some(scene) = self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll)
            && self.zoom.is_none()
        {
            self.pane_scroll = scene.selected_pane_scroll();
        }
        self.presentation.invalidate();
        self.send_resize();
        self.refresh_client_view();
    }

    fn set_orbit_attachment(&mut self, endpoint: Option<Vec<u8>>, live: bool) {
        let same_endpoint = self.terminal.active_endpoint == endpoint;
        // A terminal that stays visible in another column keeps its view; one that Orbit
        // still owes an ordered viewport response reattaches after the retry delay, giving
        // Orbit time to release this attachment first.
        let owes_viewport = self.terminal.pending_viewport.is_some();
        let keep_outgoing = !same_endpoint
            && self.terminal.transport.is_some()
            && self
                .terminal
                .active_endpoint
                .as_deref()
                .is_some_and(|outgoing| {
                    self.workspace_model
                        .column_attachments()
                        .iter()
                        .any(|&(_, column, live)| live && column == outgoing)
                });
        if keep_outgoing
            && pointer_sequence_needs_cancel(
                &self.terminal.selection_gate,
                self.input.is_selecting(),
                &self.terminal.deferred_selection,
            )
        {
            self.send(ClientMessage::Selection(SelectionAction::Cancel));
        }
        self.terminal.terminal_scroll.reset();
        self.terminal.selection_scroll.reset();
        self.terminal.pending_viewport = None;
        self.terminal.pending_return_to_live = false;
        self.terminal.deferred_selection.clear();
        self.terminal.selection_gate = SelectionGate::Ready;
        if let Some(message) = self.input.retire_orbit_generation() {
            self.send(message);
        }
        if !same_endpoint
            && self.workspace_socket.is_some()
            && self.terminal.active_endpoint_live
            && live
            && endpoint.is_some()
            && (self.terminal.model.scene().is_none()
                || (self.terminal.model.is_attached()
                    && !self.terminal.model.awaiting_current_frame()))
        {
            self.next_animation = None;
            if let Some(state) = &mut self.window {
                state.renderer.suspend_cursor_animation();
            }
        } else {
            self.reset_cursor_animation();
        }
        if !same_endpoint {
            let incoming = endpoint.as_deref().filter(|_| live).and_then(|endpoint| {
                let index = self.column_views.iter().position(|view| {
                    view.transport.is_some() && view.active_endpoint.as_deref() == Some(endpoint)
                })?;
                Some(self.column_views.swap_remove(index))
            });
            let swapped = incoming.is_some();
            let mut outgoing = std::mem::replace(
                &mut self.terminal,
                incoming.unwrap_or_else(|| TerminalView::new(None)),
            );
            if keep_outgoing {
                outgoing.model.clear_viewport_preview();
                outgoing.scrollback_press = None;
                if owes_viewport {
                    outgoing.transport = None;
                    outgoing.model.prepare_reconnect();
                    outgoing.last_resize = None;
                    outgoing.orbit_retry.schedule(Instant::now());
                }
                self.column_views.push(outgoing);
            }
            if swapped {
                self.terminal.model.clear_viewport_preview();
                self.presentation.invalidate();
                if self.terminal.model.is_attached()
                    && let Some(message) = self.input.latest_focus()
                {
                    self.send(message);
                }
                return;
            }
        }
        self.terminal.active_endpoint = endpoint.clone();
        self.terminal.active_endpoint_live = live;
        self.terminal.transport = None;
        self.terminal.orbit_retry.reset();
        self.terminal.retry_suppressed = !live;
        if same_endpoint {
            self.terminal.model.prepare_reconnect();
        } else {
            self.terminal.model = SessionModel::new();
        }
        if !live && endpoint.is_some() {
            self.terminal
                .model
                .mark_lost("The selected Eon pane is offline");
        }
        self.terminal.last_resize = None;
        self.presentation.invalidate();
        if let Some(endpoint) = endpoint.filter(|_| live) {
            self.start_orbit(PathBuf::from(OsString::from_vec(endpoint)));
        } else if self.terminal.active_endpoint.is_none() {
            self.terminal.orbit_socket = None;
        }
    }

    /// Uploads or refuses a completed resource, or retires one, before the next message is
    /// handled (VEN-C31). A refusal leaves the marker over its entries and the renderer working.
    fn apply_image_effect(&mut self, effect: ModelEffect) {
        match effect {
            ModelEffect::Resource(resource) => {
                let referenced = self.sweep_images();
                if let Some(state) = &mut self.window
                    && let Err(error) = state.renderer.upload_image(resource, &referenced)
                {
                    report(format!("image not shown: {error}"));
                }
            }
            ModelEffect::Retired(key) => {
                if let Some(state) = &mut self.window {
                    state.renderer.retire_image(key);
                }
                // An unreferenced texture goes now, not at the next redraw.
                self.sweep_images();
            }
            ModelEffect::Clipboard(_) => {}
        }
    }

    /// Before each redraw, reattaches the terminals whose frames want their images back, then
    /// frees the resources that no attachment or current frame keeps (VEN-C31).
    fn sync_images(&mut self) {
        let Some(state) = &self.window else {
            return;
        };
        let referenced =
            image_references(std::iter::once(&self.terminal).chain(&self.column_views));
        let resident = state.renderer.image_resident();
        let resyncs = std::iter::once(&mut self.terminal)
            .chain(&mut self.column_views)
            .enumerate()
            .filter_map(|(index, view)| {
                view.wants_image_resync(&state.renderer, &referenced, resident)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in resyncs {
            match index.checked_sub(1) {
                None => self.resync_terminal_images(),
                Some(column) => self.resync_column_images(column),
            }
        }
        let views = std::iter::once(&mut self.terminal).chain(&mut self.column_views);
        for view in views {
            let attachment = view.model.image_attachment();
            if view.model.scene().is_some_and(|scene| {
                Some(scene.graphics.attachment) == attachment
                    && !scene
                        .graphics
                        .placements
                        .iter()
                        .any(|placement| placement.state == PlacementState::Pending)
            }) {
                view.images_resyncing = false;
            }
        }
        self.sweep_images();
    }

    /// Frees the resources that no attachment or current frame keeps, also while redraws are
    /// withheld, and returns the resources current frames reference (VEN-C31).
    fn sweep_images(&mut self) -> HashSet<ImageKey> {
        let views = || std::iter::once(&self.terminal).chain(&self.column_views);
        let referenced = image_references(views());
        let live = views()
            .filter_map(|view| view.model.image_attachment())
            .collect();
        let at_once = views()
            .filter(|view| view.images_resyncing)
            .filter_map(|view| view.model.image_attachment())
            .collect();
        if let Some(state) = &mut self.window {
            state.renderer.sweep_images(live, at_once, &referenced);
        }
        referenced
    }

    /// Reattaches the selected terminal to recover its images (VEN-C31). Until the new
    /// attachment's first frame it shows what a reconnect shows, with markers over its images.
    fn resync_terminal_images(&mut self) {
        let Some(socket) = self.terminal.orbit_socket.clone() else {
            return;
        };
        if let Some(message) = self.input.retire_orbit_generation() {
            self.send(message);
        }
        self.terminal.terminal_scroll.reset();
        self.terminal.selection_scroll.reset();
        self.terminal.pending_viewport = None;
        self.terminal.pending_return_to_live = false;
        self.terminal.deferred_selection.clear();
        self.terminal.selection_gate = SelectionGate::Ready;
        self.reset_cursor_animation();
        self.terminal.transport = None;
        self.terminal.orbit_retry.reset();
        self.terminal.retry_suppressed = false;
        self.terminal.model.prepare_reconnect();
        self.terminal.images_resyncing = true;
        self.terminal.last_resize = None;
        self.presentation.invalidate();
        self.start_orbit(socket);
        self.refresh_client_view();
    }

    /// Reattaches another column's terminal to recover its images (VEN-C31).
    fn resync_column_images(&mut self, index: usize) {
        let view = &mut self.column_views[index];
        let Some(socket) = view.orbit_socket.clone() else {
            return;
        };
        view.transport = None;
        view.model.prepare_reconnect();
        view.images_resyncing = true;
        view.retry_suppressed = false;
        view.last_resize = None;
        view.orbit_retry.reset();
        view.transport = Some(orbit_transport(&self.proxy, socket));
    }

    fn retry_allowed(&self) -> bool {
        retry_is_allowed(
            self.workspace_socket.is_some(),
            self.workspace_model.active_attachment(),
            self.terminal.active_endpoint.as_deref(),
        )
    }

    fn retry_orbit(&mut self, now: Instant) {
        if !self.terminal.orbit_retry.take_due(now) {
            return;
        }
        if self.terminal.transport.is_some() || !self.retry_allowed() {
            self.terminal.orbit_retry.reset();
            return;
        }
        self.terminal.model.prepare_reconnect();
        self.terminal.terminal_scroll.reset();
        self.terminal.pending_viewport = None;
        self.reset_cursor_animation();
        self.terminal.retry_suppressed = false;
        self.terminal.last_resize = None;
        self.presentation.invalidate();
        self.start_orbit(
            self.terminal
                .orbit_socket
                .clone()
                .expect("Orbit retry has an attachment socket"),
        );
        self.refresh_client_view();
    }

    /// Attach each other visible column's live expanded pane and release views no column shows.
    fn reconcile_column_views(&mut self) {
        let selected = self.terminal.active_endpoint.as_deref();
        let wanted = self
            .workspace_model
            .column_attachments()
            .into_iter()
            .filter(|&(_, endpoint, live)| live && Some(endpoint) != selected)
            .map(|(_, endpoint, _)| endpoint.to_vec())
            .collect::<Vec<_>>();
        let before = self.column_views.len();
        self.column_views.retain(|view| {
            view.active_endpoint
                .as_ref()
                .is_some_and(|endpoint| wanted.contains(endpoint))
        });
        let mut changed = before != self.column_views.len();
        for endpoint in wanted {
            if self
                .column_views
                .iter()
                .any(|view| view.active_endpoint.as_ref() == Some(&endpoint))
            {
                continue;
            }
            let socket = PathBuf::from(OsString::from_vec(endpoint.clone()));
            let mut view = TerminalView::new(Some(socket.clone()));
            view.transport = Some(orbit_transport(&self.proxy, socket));
            view.active_endpoint = Some(endpoint);
            view.active_endpoint_live = true;
            self.column_views.push(view);
            changed = true;
        }
        if changed {
            self.presentation.invalidate();
        }
        // Every caller may switch the selected endpoint just before, so this frees the images of
        // the terminals either change leaves, even while redraws are withheld (VEN-C31).
        self.sweep_images();
    }

    fn handle_column_transport(&mut self, index: usize, event: TransportEvent) {
        let retryable_busy = managed_busy_is_retryable(self.supervised, &event);
        let retryable_event = retryable_busy || matches!(&event, TransportEvent::RetryableLoss(_));
        let view = &mut self.column_views[index];
        let was_attached = view.model.is_attached();
        let image_attachment = view.model.image_attachment();
        let mut schedule_retry = false;
        let mut image_effect = None;
        match event {
            TransportEvent::Server(ServerMessage::Busy) if retryable_busy => {
                schedule_retry = apply_retryable_loss(
                    &mut view.model,
                    "Orbit already has a departing presentation client; retrying".into(),
                    view.retry_suppressed,
                );
            }
            TransportEvent::Server(message) => {
                if server_failure_suppresses_retry(&message) {
                    view.retry_suppressed = true;
                }
                match view.model.apply(message) {
                    Ok(Some(ModelEffect::Clipboard(effect))) => {
                        write_clipboard_effect(&mut view.model, effect);
                    }
                    Ok(Some(effect)) => image_effect = Some(effect),
                    Ok(None) => {}
                    Err(error) => view.model.mark_lost(error.to_string()),
                }
            }
            TransportEvent::Incompatible { version } => view.model.mark_incompatible(version),
            TransportEvent::InvalidInput(detail) => view.model.set_venus_notice(
                LocalNoticeSource::Input,
                format!("Venus could not encode input: {detail}"),
            ),
            TransportEvent::RetryableLoss(detail) => {
                schedule_retry =
                    apply_retryable_loss(&mut view.model, detail, view.retry_suppressed);
            }
            TransportEvent::Lost(detail) => view.model.mark_lost(detail),
        }
        if retryable_event || view.model.is_terminal() {
            view.transport = None;
            if schedule_retry {
                view.orbit_retry.schedule(Instant::now());
            } else {
                view.orbit_retry.reset();
            }
        }
        let attached = !was_attached && view.model.is_attached();
        if attached {
            view.orbit_retry.reset();
            view.last_resize = None;
            // Orbit forwards focus to applications that request it; this column is unfocused.
            if let Some(transport) = &view.transport
                && let Err(error) = transport.send(ClientMessage::Focus(session::FocusEvent::Lost))
            {
                view.model
                    .set_venus_notice(LocalNoticeSource::Queue, error.to_string());
            }
        }
        // Column frames repaint through the renderer's per-column revisions; the selected
        // terminal's presentation identity, which pairs its input, stays unchanged.
        if attached {
            self.send_resize();
        }
        if let Some(effect) = image_effect {
            self.apply_image_effect(effect);
        }
        // Detach, `Exited`, loss and reconnect end the attachment's images (VEN-C31).
        if self.column_views[index].model.image_attachment() != image_attachment {
            self.sweep_images();
        }
        if let Some(state) = &self.window {
            state.window.request_redraw();
        }
    }

    fn retry_column_views(&mut self, now: Instant) {
        for view in &mut self.column_views {
            if !view.orbit_retry.take_due(now) {
                continue;
            }
            if view.transport.is_some() {
                view.orbit_retry.reset();
                continue;
            }
            view.model.prepare_reconnect();
            view.retry_suppressed = false;
            view.last_resize = None;
            view.transport = Some(orbit_transport(
                &self.proxy,
                view.orbit_socket
                    .clone()
                    .expect("a column view has an attachment socket"),
            ));
        }
    }

    /// Size every other column's attached terminal to its own column.
    fn send_column_resizes(&mut self) {
        let Some(metrics) = self.window.as_ref().map(|state| state.renderer.metrics()) else {
            return;
        };
        let Some(workspace) = self.workspace_scene() else {
            return;
        };
        let attachments = self.workspace_model.column_attachments();
        for column in &workspace.other_columns {
            let Some(view) = attachments
                .iter()
                .find(|&&(index, _, _)| index == column.index)
                .and_then(|&(_, endpoint, _)| {
                    self.column_views
                        .iter_mut()
                        .find(|view| view.active_endpoint.as_deref() == Some(endpoint))
                })
            else {
                continue;
            };
            let Some(size) = surface_size(rect_screen(column.terminal), metrics) else {
                view.model.set_venus_notice(
                    LocalNoticeSource::Resize,
                    "Window dimensions are outside Orbit's accepted surface range",
                );
                continue;
            };
            if view.last_resize == Some(size) {
                view.model.clear_venus_notice(LocalNoticeSource::Resize);
                continue;
            }
            let Some(transport) = view.transport.as_ref().filter(|_| view.model.is_attached())
            else {
                continue;
            };
            match transport.send(ClientMessage::Resize(size)) {
                Ok(()) => {
                    view.last_resize = Some(size);
                    view.model.clear_venus_notice(LocalNoticeSource::Resize);
                    view.model.clear_venus_notice(LocalNoticeSource::Queue);
                }
                Err(error) => view
                    .model
                    .set_venus_notice(LocalNoticeSource::Queue, error.to_string()),
            }
        }
    }

    fn handle_workspace(&mut self, event: WorkspaceEvent) {
        if let WorkspaceEvent::Answered { ticket, accepted } = event {
            // A refused action's notice is new even when it repeats a poll's or an earlier one.
            let renewed = !accepted && self.workspace_model.renew_notice();
            if self.attention_jump.is_some() {
                self.settle_attention_jump(Some((ticket, accepted)));
                self.refresh_client_view();
            } else if renewed {
                self.refresh_client_view();
            }
            return;
        }
        let inbox_index = self
            .workspace_model
            .snapshot()
            .and_then(|snapshot| self.attention_inbox.index(snapshot))
            .unwrap_or(0);
        let (
            received_snapshot,
            snapshot_changed,
            attention_only,
            badge_only,
            labels_only,
            sidebar_toggled,
        ) = match &event {
            WorkspaceEvent::Response(eon_workspace_protocol::v26::Response::Snapshot(snapshot)) => {
                let current = self.workspace_model.snapshot();
                let attention_only =
                    current.is_some_and(|current| attention_only_change(current, snapshot));
                (
                    true,
                    current != Some(snapshot),
                    attention_only,
                    // Another workspace's attention moves no tab, pane or sidebar row (VEN-C21).
                    attention_only && current.is_some_and(|current| current.tabs == snapshot.tabs),
                    current.is_some_and(|current| labels_only_change(current, snapshot)),
                    current.is_some_and(|current| current.sidebar != snapshot.sidebar),
                )
            }
            _ => (false, false, false, false, false, false),
        };
        let unavailable = matches!(&event, WorkspaceEvent::Unavailable(_));
        let quota_expired = unavailable
            && self.workspace_model.snapshot().is_some_and(|snapshot| {
                snapshot.codex_quota.is_some() || snapshot.claude_quota.is_some()
            });
        let (mut view_changed, workspace_changed) = match event {
            WorkspaceEvent::Response(response) => self.workspace_model.apply(response),
            WorkspaceEvent::Unavailable(detail) => {
                (self.workspace_model.mark_unavailable(detail, false), false)
            }
            WorkspaceEvent::Answered { .. } => unreachable!("answers return above"),
        };
        if received_snapshot
            && snapshot_changed
            && let Some(snapshot) = self.workspace_model.snapshot()
        {
            view_changed |=
                self.attention_inbox
                    .refresh(snapshot, inbox_index, &mut self.presentation);
            self.reveal_inbox_highlight();
        }
        // Accepted selection or layout changes end zoom; covering popups leave its target intact.
        if self.zoom.is_some() && !self.zoomed() {
            self.zoom = None;
        }
        if self.window.is_none() || self.initial_scale_pending {
            if snapshot_changed && view_changed {
                self.reveal_workspace_selection();
            }
            self.close_stale_prompt(unavailable);
            return;
        }
        if unavailable && quota_expired {
            self.presentation.invalidate();
        }
        // Showing or hiding the sidebar moves every column, as the hints and zoom toggles do.
        if sidebar_toggled {
            self.cancel_pointer_sequence();
        }
        if workspace_changed {
            self.cancel_terminal_scroll();
            self.reveal_workspace_selection();
            self.presentation.invalidate();
            let attachment = self.workspace_model.active_attachment();
            let popup_visible = self
                .workspace_model
                .snapshot()
                .and_then(active_popup)
                .is_some();
            if attachment
                != self
                    .terminal
                    .active_endpoint
                    .as_deref()
                    .map(|endpoint| (endpoint, self.terminal.active_endpoint_live))
            {
                let (endpoint, live) = attachment.map_or((None, false), |(endpoint, live)| {
                    (Some(endpoint.to_vec()), live)
                });
                self.set_orbit_attachment(endpoint, live);
                // A pending row jump decides focus itself, below.
                if popup_visible && self.attention_jump.is_none() {
                    self.set_workspace_focus(WorkspaceFocus::Terminal);
                }
            }
            self.reconcile_column_views();
            self.send_resize();
        } else if snapshot_changed && view_changed {
            let Some(scene) =
                self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll)
            else {
                return;
            };
            self.tab_scroll = scene.tab_scroll();
            self.pane_scroll = scene.pane_scroll();
            // Attention moves no presented geometry outside a visible sidebar, so a held link
            // press or a shortcut waiting on the presented scene survives it; the sidebar's rows
            // are hit targets, so a refresh that moves them retires presses on them. The bar
            // label's badge moves no row.
            if labels_only || badge_only || attention_only && scene.sidebar().is_none() {
                self.presentation.content_changed();
            } else {
                self.presentation.invalidate();
                // Showing or hiding the sidebar resizes the columns; other refreshes leave
                // every size as it was, so nothing is sent.
                self.send_resize();
            }
        }
        self.close_stale_prompt(unavailable);
        if received_snapshot {
            self.settle_attention_jump(None);
        } else if unavailable {
            // Workspace loss ends a pending jump with focus on its row, beside the notice.
            self.attention_jump = None;
        }
        self.retain_chrome_focus();
        if self.workspace_model.snapshot().is_some()
            && self.workspace_model.active_attachment().is_none()
        {
            self.set_workspace_focus(WorkspaceFocus::Tabs);
        }
        if view_changed {
            self.refresh_client_view();
        }
    }

    /// Closes the prompt with its target or the workspace connection. Closing it refreshes the
    /// view, so it runs once the scene's texts are measured.
    fn close_stale_prompt(&mut self, unavailable: bool) {
        if self.close_prompt.as_ref().is_some_and(|prompt| {
            unavailable
                || !self
                    .workspace_model
                    .snapshot()
                    .is_some_and(|snapshot| prompt.names_live_target(snapshot))
        }) {
            self.change_dialog(|app| app.close_prompt = None);
        }
    }

    fn send_workspace(&mut self, action: CommonAction) -> bool {
        self.presentation.has_presented_geometry()
            && self.queue_workspace(WorkspaceAction::Workspace(action))
    }

    fn activate_workspace_header(&mut self, control: WorkspaceHeaderControl) {
        let Some(snapshot) = self.workspace_model.snapshot() else {
            return;
        };
        match workspace_header_action(control, &snapshot.active_tab) {
            WorkspaceHeaderAction::Workspace(action) => {
                self.request_workspace(WorkspaceAction::Workspace(action));
            }
            WorkspaceHeaderAction::ToggleShortcuts => {
                self.set_shortcut_viewer(!self.shortcut_viewer.open);
            }
            // The workspace label opens the picker as the picker's key does from the chrome:
            // every activation first focuses the label, so it focuses the picker, never toggles.
            WorkspaceHeaderAction::PickWorkspace => {
                if let Some(action) = invoke_popup(snapshot, WORKSPACES_ENTRY, false)
                    && self.request_workspace(action)
                {
                    self.set_workspace_focus(WorkspaceFocus::Terminal);
                }
            }
        }
    }

    /// Whether Venus lets a column request for the active tab reach Eon.
    fn fits_another_column(&self) -> bool {
        let (Some(state), Some(snapshot), Some(scene)) = (
            &self.window,
            self.workspace_model.snapshot(),
            self.workspace_scene(),
        ) else {
            return false;
        };
        let window = state.renderer.size();
        snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab)
            .is_some_and(|tab| {
                admits_column(
                    PhysicalSize::new(
                        scene.content_width(window.width as f32) as u32,
                        window.height,
                    ),
                    tab.columns.len(),
                    state.renderer.metrics(),
                )
            })
    }

    /// Queues a presented action, unless it would close the final tab while the warning is
    /// on: then the close prompt holds it, and nothing is queued yet.
    fn request_workspace(&mut self, action: WorkspaceAction) -> bool {
        if !self.presentation.has_presented_geometry() {
            return false;
        }
        if self.warning_close_last_tab
            && let Some(tab) = self
                .workspace_model
                .snapshot()
                .and_then(|snapshot| final_close_tab(snapshot, &action))
        {
            self.change_dialog(|app| {
                app.close_prompt = Some(ClosePrompt {
                    tab,
                    action,
                    scroll: 0.0,
                })
            });
            return false;
        }
        self.queue_workspace(action)
    }

    fn queue_workspace(&mut self, action: WorkspaceAction) -> bool {
        self.queue_workspace_ticket(action).is_some()
    }

    /// Queues `action`, returning the ticket its answer carries.
    fn queue_workspace_ticket(&mut self, action: WorkspaceAction) -> Option<u64> {
        let transport = self.workspace_transport.as_ref()?;
        match transport.send(action) {
            Ok(ticket) => Some(ticket),
            Err(error) => {
                let quota_expired = self.workspace_model.snapshot().is_some_and(|snapshot| {
                    snapshot.codex_quota.is_some() || snapshot.claude_quota.is_some()
                });
                if self
                    .workspace_model
                    .mark_unavailable(format!("Cannot queue Eon workspace action: {error}"), true)
                {
                    if quota_expired {
                        self.presentation.invalidate();
                    }
                    self.refresh_client_view();
                }
                None
            }
        }
    }

    fn open_new_window(&mut self) {
        let Some(executable) = self.new_window_executable.clone() else {
            self.terminal.model.set_venus_notice(
                LocalNoticeSource::Input,
                "Cannot open a new Eon window: no Eon executable was supplied.",
            );
            self.refresh_client_view();
            return;
        };
        let proxy = self.proxy.clone();
        let result = thread::Builder::new()
            .name("venus-new-eon-window".into())
            .spawn(move || {
                if let Err(failure) = run_new_window(&executable) {
                    let _ = proxy.send_event(UserEvent::NewWindowFailed(failure));
                }
            });
        if let Err(error) = result {
            self.terminal.model.set_venus_notice(
                LocalNoticeSource::Input,
                format!("Cannot start a new Eon window request: {error}"),
            );
            self.refresh_client_view();
        }
    }

    fn handle_workspace_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        let Some(snapshot) = self.workspace_model.snapshot() else {
            return false;
        };
        let focus = self.workspace_focus;
        let code = match event.physical_key {
            PhysicalKey::Code(code) => code,
            PhysicalKey::Unidentified(_) => return focus != WorkspaceFocus::Terminal,
        };
        let modifiers = self.input.modifiers();
        let fixed = native_key_shortcut(event.physical_key, modifiers, &snapshot.entries);
        let has_terminal = self.workspace_model.active_attachment().is_some();
        let cycles_focus =
            fixed.is_some_and(|shortcut| shortcut.action == NativeShortcutAction::CycleFocus);
        let opens_window =
            fixed.is_some_and(|shortcut| shortcut.action == NativeShortcutAction::NewWindow);
        let toggles_hints =
            fixed.is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ToggleHints);
        let toggles_zoom =
            fixed.is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ToggleZoom);
        // Alt+W stays an Eon chord even when the active tab has no pane to close.
        let closes_pane =
            fixed.is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ClosePane);
        let returns_to_terminal = fixed
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ReturnToTerminal)
            && focus != WorkspaceFocus::Terminal;
        let active_tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == snapshot.active_tab);
        let tab_target = tab_shortcut_target(code, modifiers, snapshot);
        let action = match tab_target {
            Some(tab) => {
                tab.map(|tab| WorkspaceAction::Workspace(CommonAction::FocusId(tab.id.clone())))
            }
            None => popup_shortcut(
                code,
                modifiers,
                snapshot,
                has_terminal && terminal_focused(self.window_focused, focus),
            )
            .or_else(|| {
                let selected_pane = active_tab.and_then(|tab| tab.selected_pane.as_deref());
                workspace_shortcut(
                    code,
                    modifiers,
                    &snapshot.entries,
                    &snapshot.active_tab,
                    selected_pane,
                    snapshot.sidebar,
                )
            }),
        };
        let opens_column = match &action {
            Some(WorkspaceAction::CreateColumn) => true,
            Some(WorkspaceAction::MovePaneSideways(side)) => {
                active_tab.is_some_and(|tab| sideways_move_opens_column(tab, *side))
            }
            _ => false,
        };
        let has_panes = active_popup(snapshot).is_none()
            && active_tab.is_some_and(|tab| !tab.columns.is_empty());
        if self.input.consumes_shortcut(
            event.physical_key,
            event.state,
            event.repeat,
            opens_window
                || toggles_hints
                || toggles_zoom
                || cycles_focus
                || returns_to_terminal
                || tab_target.is_some()
                || closes_pane
                || action.is_some(),
        ) {
            if event.state == ElementState::Pressed {
                if opens_window && !event.repeat {
                    self.open_new_window();
                } else if toggles_hints && !event.repeat {
                    self.toggle_hints();
                } else if toggles_zoom && !event.repeat {
                    self.toggle_zoom();
                } else if cycles_focus && !event.repeat {
                    // F6 visits the sidebar only while it shows rows to focus, returning to the
                    // row it last focused while that row remains.
                    let row = self.workspace_scene().and_then(|scene| {
                        self.sidebar_row
                            .as_deref()
                            .and_then(|row| scene.sidebar_row(row))
                            .or_else(|| scene.sidebar_rows().first())
                            .map(|row| row.target.clone())
                    });
                    match (cycle_focus(focus, row.is_some(), has_panes), row) {
                        (WorkspaceFocus::Sidebar, Some(row)) => self.focus_sidebar_row(row),
                        (next, _) => self.set_workspace_focus(next),
                    }
                } else if returns_to_terminal {
                    self.set_workspace_focus(WorkspaceFocus::Terminal);
                } else if let Some(action) = action
                    .filter(|action| sends_workspace_shortcut(action, event.state, event.repeat))
                    && self.presentation.has_presented_geometry()
                {
                    if opens_column && !self.fits_another_column() {
                        if self.workspace_model.reject_action(format!(
                            "No room for another column: each needs at least \
                             {MIN_COLUMN_CELLS} terminal columns."
                        )) {
                            self.refresh_client_view();
                        }
                        return true;
                    }
                    let popup = matches!(action, WorkspaceAction::InvokePopup { .. });
                    if self.request_workspace(action) && popup {
                        self.set_workspace_focus(WorkspaceFocus::Terminal);
                    }
                }
            }
            return true;
        }
        if focus == WorkspaceFocus::Terminal {
            return false;
        }
        if event.state == ElementState::Pressed {
            if !event.repeat
                && code == KeyCode::Tab
                && (modifiers == session::Modifiers::empty()
                    || modifiers == session::Modifiers::SHIFT)
                && let Some(next) = {
                    let scene = self.workspace_scene();
                    let shows = |control| {
                        scene
                            .as_ref()
                            .is_some_and(|scene| scene.has_control(control))
                    };
                    cycle_header_focus(
                        focus,
                        modifiers == session::Modifiers::SHIFT,
                        shows(WorkspaceHeaderControl::Workspaces),
                        shows(WorkspaceHeaderControl::Shortcuts),
                    )
                }
            {
                self.set_workspace_focus(next);
                return true;
            }
            if !event.repeat
                && matches!(code, KeyCode::Enter | KeyCode::Space)
                && modifiers == session::Modifiers::empty()
            {
                if let WorkspaceFocus::Header(control) = focus {
                    self.activate_workspace_header(control);
                    return true;
                }
                if focus == WorkspaceFocus::Sidebar
                    && let Some(row) = self.sidebar_row.clone()
                {
                    self.activate_sidebar_row(&row);
                    return true;
                }
            }
            if focus == WorkspaceFocus::Sidebar
                && let Some(NativeShortcutAction::TraverseChrome(
                    direction @ (WorkspaceDirection::Up | WorkspaceDirection::Down),
                )) = fixed.map(|shortcut| shortcut.action)
            {
                let down = direction == WorkspaceDirection::Down;
                if let Some(row) = self.workspace_scene().and_then(|scene| {
                    step_sidebar_row(scene.sidebar_rows(), self.sidebar_row.as_deref(), down)
                        .map(str::to_owned)
                }) {
                    self.focus_sidebar_row(row);
                }
                return true;
            }
            let direction = match (focus, fixed.map(|shortcut| shortcut.action)) {
                (
                    WorkspaceFocus::Tabs,
                    Some(NativeShortcutAction::TraverseChrome(
                        direction @ (WorkspaceDirection::Left | WorkspaceDirection::Right),
                    )),
                ) => Some(direction),
                (
                    WorkspaceFocus::Panes,
                    Some(NativeShortcutAction::TraverseChrome(
                        direction @ (WorkspaceDirection::Up | WorkspaceDirection::Down),
                    )),
                ) => Some(direction),
                _ => None,
            };
            if let Some(direction) = direction {
                self.send_workspace(CommonAction::Focus(direction));
            }
        }
        true
    }

    fn scroll_workspace(&mut self, delta: MouseScrollDelta, tabs: bool, metrics: CellMetrics) {
        let (horizontal, vertical) = match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                (x * metrics.width * 3.0, y * metrics.height * 3.0)
            }
            MouseScrollDelta::PixelDelta(position) => (position.x as f32, position.y as f32),
        };
        if !horizontal.is_finite() || !vertical.is_finite() {
            return;
        }
        let Some(scene) = self.workspace_scene() else {
            return;
        };
        self.input.reset_scroll();
        let changed = if tabs {
            let movement = if horizontal == 0.0 {
                vertical
            } else {
                horizontal
            };
            let next = (scene.tab_scroll() - movement).clamp(0.0, scene.tab_scroll_limit());
            let changed = self.tab_scroll != next;
            self.tab_scroll = next;
            changed
        } else {
            let next = (scene.pane_scroll() - vertical).clamp(0.0, scene.pane_scroll_limit());
            let changed = self.pane_scroll != next;
            self.pane_scroll = next;
            changed
        };
        if changed {
            self.presentation.invalidate();
            self.refresh_client_view();
        }
    }

    fn handle_transport(&mut self, event: TransportEvent) {
        let retryable_busy = managed_busy_is_retryable(self.supervised, &event);
        let retryable_event = retryable_busy || matches!(&event, TransportEvent::RetryableLoss(_));
        let image_attachment = self.terminal.model.image_attachment();
        let mut schedule_retry = false;
        match event {
            TransportEvent::Server(ServerMessage::Busy) if retryable_busy => {
                schedule_retry = apply_retryable_loss(
                    &mut self.terminal.model,
                    "Orbit already has a departing presentation client; retrying".into(),
                    self.terminal.retry_suppressed,
                );
            }
            TransportEvent::Server(message) => {
                let selection_finished = match &message {
                    ServerMessage::SelectionFinished { frame_revision } => Some(*frame_revision),
                    _ => None,
                };
                let preview_response = match &message {
                    ServerMessage::VerticalPreview(preview) => Some(preview.direction),
                    _ => None,
                };
                let batch_response = match &message {
                    ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                        requested_rows,
                        applied_rows,
                        ..
                    }) => Some((*requested_rows, *applied_rows)),
                    ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned {
                        requested_rows,
                    }) => Some((*requested_rows, 0)),
                    _ => None,
                };
                let terminal_owned_batch = matches!(
                    &message,
                    ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { .. })
                );
                let server_failure = matches!(&message, ServerMessage::Failure(_));
                let selection_scroll_failure = server_failure
                    && self.terminal.pending_viewport
                        == Some(PendingViewportRequest::SelectionTick);
                let expected_selection_rejection = selection_scroll_failure
                    && matches!(&message, ServerMessage::Failure(failure) if failure.code == FailureCode::InvalidInput);
                let plain_frame = matches!(&message, ServerMessage::Frame(_));
                if server_failure_suppresses_retry(&message) {
                    self.terminal.retry_suppressed = true;
                }
                let was_attached = self.terminal.model.is_attached();
                let awaiting_frame = self.terminal.model.awaiting_current_frame();
                let previous_geometry = self
                    .terminal
                    .model
                    .scene()
                    .map(|scene| (scene.columns, scene.rows, scene.screen));
                let frame = matches!(
                    &message,
                    ServerMessage::Frame(_)
                        | ServerMessage::WheelOutcome(WheelOutcome::Viewport { .. })
                        | ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { .. })
                );
                let cell_height = self
                    .window
                    .as_ref()
                    .map_or(0.0, |state| f64::from(state.renderer.metrics().height));
                let batch_accepted = if self.terminal.pending_viewport
                    == Some(PendingViewportRequest::SelectionTick)
                {
                    match &message {
                        ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                            requested_rows,
                            applied_rows,
                            frame,
                            ..
                        }) => {
                            *requested_rows == -1
                                && self.terminal.selection_scroll.accept(
                                    &mut self.terminal.pending_viewport,
                                    *applied_rows,
                                    frame.revision,
                                )
                        }
                        ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { .. }) => false,
                        _ => true,
                    }
                } else {
                    batch_response.is_none_or(|(requested, applied)| {
                        self.terminal.terminal_scroll.accept_batch(
                            &mut self.terminal.pending_viewport,
                            requested,
                            applied,
                            cell_height,
                        )
                    })
                };
                let result = if !batch_accepted {
                    Err(ModelError::UnexpectedMessage)
                } else if expected_selection_rejection {
                    // ORBS v14 rejects terminal-routed synthetic ticks as invalid input.
                    self.terminal.model.clear_viewport_preview();
                    Ok(None)
                } else {
                    self.terminal.model.apply(message)
                };
                let accepted_frame = frame && result.is_ok();
                let accepted = result.is_ok();
                match result {
                    Ok(Some(ModelEffect::Clipboard(effect))) => self.write_clipboard(effect),
                    Ok(Some(effect)) => self.apply_image_effect(effect),
                    Ok(None) => {}
                    Err(error) => self.terminal.model.mark_lost(error.to_string()),
                }
                if accepted {
                    if let Some(frame_revision) = selection_finished {
                        self.terminal.selection_gate.finish_received(frame_revision);
                        if let Some(presented_revision) = self.presented_revision() {
                            self.flush_deferred_selection(presented_revision);
                        }
                    }
                    if server_failure {
                        finish_scroll_failure(
                            &mut self.terminal.pending_viewport,
                            &mut self.terminal.terminal_scroll,
                            &mut self.terminal.selection_scroll,
                            expected_selection_rejection,
                        );
                    } else if let Some(direction) = preview_response {
                        self.terminal.terminal_scroll.preview_arrived(direction);
                    }
                    if terminal_owned_batch {
                        self.cancel_terminal_scroll();
                    } else if frame {
                        self.terminal.terminal_scroll.frame_arrived(Instant::now());
                    }
                }
                if server_failure
                    && !selection_scroll_failure
                    && (self.has_pointer_sequence()
                        || !matches!(self.terminal.selection_gate, SelectionGate::Ready))
                {
                    self.cancel_pointer_sequence();
                    self.terminal.selection_gate = SelectionGate::Ready;
                }
                if accepted_frame {
                    if awaiting_frame {
                        self.next_animation = None;
                        if let Some(state) = &mut self.window {
                            state.renderer.attach_cursor_animation();
                        }
                    }
                    let geometry = self
                        .terminal
                        .model
                        .scene()
                        .map(|scene| (scene.columns, scene.rows, scene.screen));
                    if plain_frame && geometry == previous_geometry {
                        // Output dirties the renderer, not the last presented input geometry.
                        self.presentation.content_changed();
                    } else {
                        self.presentation.invalidate();
                    }
                }
                if !was_attached && self.terminal.model.is_attached() {
                    self.terminal.orbit_retry.reset();
                    self.terminal.deferred_selection.clear();
                    self.terminal.selection_gate = SelectionGate::Ready;
                    self.terminal.selection_scroll.reset();
                    self.terminal.pending_viewport = None;
                    self.input.retire_orbit_generation();
                    self.send_resize();
                    if let Some(message) = self.input.latest_focus() {
                        self.send(message);
                    }
                }
            }
            TransportEvent::Incompatible { version } => {
                self.terminal.model.mark_incompatible(version)
            }
            TransportEvent::InvalidInput(detail) => {
                self.terminal.terminal_scroll.reset();
                if matches!(
                    self.terminal.pending_viewport,
                    Some(PendingViewportRequest::Scroll { .. })
                ) {
                    self.terminal.pending_viewport = None;
                }
                self.terminal.pending_return_to_live = false;
                self.terminal.model.set_venus_notice(
                    LocalNoticeSource::Input,
                    format!("Venus could not encode input: {detail}"),
                );
            }
            TransportEvent::RetryableLoss(detail) => {
                schedule_retry = apply_retryable_loss(
                    &mut self.terminal.model,
                    detail,
                    self.terminal.retry_suppressed,
                );
            }
            TransportEvent::Lost(detail) => self.terminal.model.mark_lost(detail),
        }
        if retryable_event || self.terminal.model.is_terminal() {
            self.terminal.terminal_scroll.reset();
            self.terminal.selection_scroll.reset();
            self.terminal.pending_viewport = None;
            self.terminal.pending_return_to_live = false;
            self.terminal.deferred_selection.clear();
            self.terminal.selection_gate = SelectionGate::Ready;
            self.input.retire_orbit_generation();
            self.reset_cursor_animation();
            self.terminal.transport = None;
            self.presentation.invalidate();
            if schedule_retry && self.retry_allowed() {
                self.terminal.orbit_retry.schedule(Instant::now());
            } else {
                self.terminal.orbit_retry.reset();
            }
        }
        if self.terminal.pending_return_to_live
            && self.terminal.model.is_attached()
            && self.terminal.transport.is_some()
            && self.terminal.pending_viewport.is_none()
        {
            self.terminal.pending_return_to_live = false;
            self.send(ClientMessage::ReturnToLive);
        }
        // Detach, `Exited`, loss and reconnect end the attachment's images (VEN-C31).
        if self.terminal.model.image_attachment() != image_attachment {
            self.sweep_images();
        }
        self.refresh_client_view();
    }

    fn link_scene(&self) -> Option<(&Scene, PresentationIdentity)> {
        let workspace = self.workspace_scene();
        let identity = self.presentation_candidate(workspace.as_ref());
        (self.link_surface_available(self.links.unshifted)
            && self.presentation.is_current(identity))
        .then(|| {
            (
                self.terminal
                    .model
                    .scene()
                    .expect("presented revision has a scene"),
                identity,
            )
        })
    }

    fn link_surface_available(&self, unshifted: bool) -> bool {
        self.window_focused
            && !self.window_occluded
            && !self.dialog_open()
            && self.workspace_focus == WorkspaceFocus::Terminal
            && !self
                .terminal
                .terminal_scroll
                .active(self.terminal.pending_viewport)
            && unshifted
    }

    /// The latest scene with links on screen or due in the next frame, which output may replace.
    fn visible_link_scene(&self, unshifted: bool) -> Option<&Scene> {
        let workspace = self.workspace_scene();
        (self.link_surface_available(unshifted)
            && self
                .presentation_candidate(workspace.as_ref())
                .revision
                .is_some())
        .then(|| self.terminal.model.scene())
        .flatten()
    }

    fn visible_link(&self, unshifted: bool) -> Option<Hyperlink<'_>> {
        let scene = self.visible_link_scene(unshifted)?;
        self.links
            .focus
            .as_ref()?
            .find(scene, self.presentation.epoch)
    }

    fn focused_link(&self) -> Option<Hyperlink<'_>> {
        self.visible_link(self.links.unshifted)
    }

    /// Whether a target the latest scene holds is on screen: that scene is presented, or the
    /// presented frame highlighted the target.
    fn presented(&self, target: &LinkFocus) -> bool {
        self.link_scene().is_some() || self.links.shown.as_ref() == Some(target)
    }

    /// The focused target, while presented and still in the latest scene.
    fn current_link(&self) -> Option<Hyperlink<'_>> {
        let link = self.focused_link()?;
        self.presented(self.links.focus.as_ref()?).then_some(link)
    }

    fn link_viewport(&self) -> Option<(SceneRect, SceneRect)> {
        let state = self.window.as_ref()?;
        if let Some(workspace) = self.workspace_scene() {
            Some((workspace.terminal, workspace.visible_terminal()?))
        } else {
            let rect = SceneRect {
                width: state.renderer.size().width as f32,
                height: state.renderer.size().height as f32,
                ..SceneRect::default()
            };
            Some((rect, rect))
        }
    }

    fn pointer_link(&self, unshifted: bool) -> Option<LinkFocus> {
        if !self.links.pointer_inside || self.input.pointer_busy() {
            return None;
        }
        let scene = self.visible_link_scene(unshifted)?;
        let state = self.window.as_ref()?;
        let workspace = self.workspace_scene();
        let (origin, visible) = self.link_viewport()?;
        let x = self.cursor.x as f32;
        let y = self.cursor.y as f32;
        if !visible.contains(x, y) {
            return None;
        }
        let metrics = state.renderer.metrics();
        if !self.status_for(unshifted).is_empty()
            && state
                .renderer
                .notice_rect(workspace.as_ref())
                .contains(x, y)
        {
            return None;
        }
        let column = ((x - origin.left - metrics.padding) / metrics.width).floor();
        let row = ((y - origin.top - metrics.padding) / metrics.height).floor();
        if column < 0.0
            || row < 0.0
            || column >= f32::from(scene.columns)
            || row >= f32::from(scene.rows)
        {
            return None;
        }
        let link = scene.hyperlink_at(row as u16, column as u16)?;
        Some(LinkFocus {
            epoch: self.presentation.epoch,
            row: link.row,
            column: link.column,
            columns: link.columns,
            uri: link.uri.into_owned(),
        })
    }

    /// Re-derives the hovered target from the visible scene, returning whether it changed.
    fn hover_link(&mut self, unshifted: bool) -> bool {
        if matches!(self.links.pressed, Some(LinkPress::Live(_))) {
            return false;
        }
        let focus = self.pointer_link(unshifted);
        if self.links.focus == focus {
            return false;
        }
        self.links.focus = focus;
        self.links.notice = None;
        true
    }

    fn inspect_pointer_link(&mut self) {
        if self.hover_link(self.links.unshifted) {
            self.refresh_client_view();
        }
    }

    fn link_status(&self, unshifted: bool) -> Option<String> {
        if let Some(notice) = self.links.notice() {
            return Some(notice.to_owned());
        }
        let link = self.visible_link(unshifted)?;
        if link.uri.len() > MAX_LINK_BYTES {
            return Some(format!(
                "Link target exceeds {MAX_LINK_BYTES} bytes; opening and copying are disabled."
            ));
        }
        Some(link_hint(&link.uri, self.popup_entries()))
    }

    fn handle_link_copy(&mut self, key: PhysicalKey, state: ElementState, repeat: bool) -> bool {
        let modifiers = self.input.modifiers();
        let selected = self
            .terminal
            .model
            .scene()
            .is_some_and(Scene::has_selected_content);
        let current = self.current_link().is_some();
        let recognized =
            link_copy_shortcut(key, modifiers, self.popup_entries(), selected, current);
        if !self.input.consumes_shortcut(key, state, repeat, recognized) {
            return false;
        }
        if shortcut_is_ready(state, repeat) {
            self.activate_link(true);
            self.refresh_client_view();
        }
        true
    }

    fn activate_link(&mut self, copy: bool) {
        let Some(uri) = self.current_link().map(|link| link.uri.into_owned()) else {
            return;
        };
        self.activate_uri(uri, copy);
    }

    fn activate_accessible_link(&mut self, row: u16, column: u16, copy: bool) {
        let Some(uri) = self
            .link_scene()
            .and_then(|(scene, _)| scene.hyperlink_at(row, column))
            .map(|link| link.uri.into_owned())
        else {
            return;
        };
        self.activate_uri(uri, copy);
        self.refresh_client_view();
    }

    fn activate_uri(&mut self, uri: String, copy: bool) {
        let result = if copy {
            validate_copy_uri(&uri).and_then(|()| {
                write_native_clipboard(NativeClipboard::Standard, uri)
                    .map(|()| "Link copied.")
                    .map_err(|_| "Could not copy the link to the native clipboard.")
            })
        } else if self.link_opener.is_some() {
            Err("A link dispatch is already pending.")
        } else {
            LinkOpener::start(&uri).map(|opener| {
                self.link_opener = Some(opener);
                "Opening link…"
            })
        };
        let success = copy && result.is_ok();
        self.links.show_notice(
            result.unwrap_or_else(|error| error),
            success,
            Instant::now(),
            self.workspace_model.notice_serial(),
        );
    }

    fn handle_link_button(&mut self, state: ElementState, button: MouseButton) -> bool {
        let opens_link = native_pointer_shortcut(button, self.input.modifiers())
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::OpenLink);
        if button != MouseButton::Left {
            return false;
        }
        if state == ElementState::Released
            && let Some(pressed) = self.links.pressed.take()
        {
            let released = self.pointer_link(self.links.unshifted);
            let activate = opens_link
                && matches!(&pressed, LinkPress::Live(target)
                    if released.as_ref() == Some(target) && self.presented(target));
            self.links.focus = released;
            if activate {
                self.activate_link(false);
            } else {
                self.links.notice = None;
            }
            self.refresh_client_view();
            return true;
        }
        if state == ElementState::Pressed {
            self.links.pressed = None;
            if opens_link
                && let Some(focus) = self
                    .pointer_link(self.links.unshifted)
                    .filter(|focus| self.presented(focus))
            {
                self.links.focus = Some(focus.clone());
                self.links.pressed = Some(LinkPress::Live(focus));
                self.refresh_client_view();
                return true;
            }
        }
        false
    }

    fn refresh_client_view(&mut self) {
        if !self.link_surface_available(self.links.unshifted) {
            self.links.pressed = None;
        }
        if self.links.focus.is_some() && self.focused_link().is_none() {
            self.links.focus = None;
        }
        if let Some(LinkPress::Live(target)) = &self.links.pressed
            && self
                .visible_link_scene(self.links.unshifted)
                .and_then(|scene| target.find(scene, self.presentation.epoch))
                .is_none()
        {
            self.links.pressed = Some(LinkPress::Retired);
        }
        let link_generation = self.link_scene().map(|(_, identity)| identity.generation);
        let status = self.status();
        let workspace = self.workspace_scene();
        // Wake when a visible row's age next changes its label; every change to the rows or
        // their visibility refreshes the view. Ages are not part of the presented content's
        // identity, so a refresh at or past the deadline marks the content changed.
        let now = Instant::now();
        if self.next_age.is_some_and(|due| now >= due) {
            self.presentation.content_changed();
        }
        let dialog = self.dialog_scene();
        self.next_age = workspace
            .as_ref()
            .and_then(WorkspaceScene::sidebar_age_wait)
            .into_iter()
            .chain(dialog.as_ref().and_then(DialogScene::attention_age_wait))
            .min()
            .map(|wait| now + Duration::from_millis(wait));
        let scrollback_label = self.scrollback_label(workspace.as_ref());
        let scrollback_rect = self.scrollback_rect(workspace.as_ref());
        let workspace_focus = self.workspace_focus;
        let ime_allowed = dialog.is_none()
            && ime_allowed(
                self.window_focused,
                workspace_focus,
                self.terminal.model.is_attached(),
            );
        let Some(state) = &mut self.window else {
            return;
        };
        state.window.set_title(window_title(
            self.terminal
                .model
                .scene()
                .map(|scene| scene.title.as_str()),
            self.render_notice.as_deref(),
        ));
        if let Some(scene) = self.terminal.model.scene()
            && let Some(cursor) = scene.cursor
        {
            let metrics = state.renderer.metrics();
            let origin = workspace.as_ref().map_or((0.0, 0.0), |workspace| {
                (workspace.terminal.left, workspace.terminal.top)
            });
            let left =
                origin.0 + metrics.padding + f32::from(cursor.leading_column()) * metrics.width;
            let top = origin.1 + metrics.padding + f32::from(cursor.row) * metrics.height;
            state.window.set_ime_cursor_area(
                PhysicalPosition::new(f64::from(left), f64::from(top)),
                PhysicalSize::new(f64::from(metrics.width * 2.0), f64::from(metrics.height)),
            );
        }
        state.window.set_ime_allowed(ime_allowed);
        state.window.set_cursor(
            if dialog.is_none()
                && (self.links.focus.is_some()
                    || scrollback_rect.is_some_and(|rect| {
                        self.links.pointer_inside
                            && rect.contains(self.cursor.x as f32, self.cursor.y as f32)
                    }))
            {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            },
        );
        state.accessibility.update(
            &mut state.adapter,
            self.terminal.model.scene(),
            workspace.as_ref(),
            dialog.as_ref(),
            workspace_focus,
            &status,
            scrollback_label.as_deref(),
            scrollback_rect,
            state.renderer.size(),
            state.renderer.metrics(),
            link_generation,
        );
        state.window.request_redraw();
    }

    fn send(&mut self, message: ClientMessage) -> bool {
        let pending = match &message {
            ClientMessage::ScrollVertical { rows, .. } => Some(PendingViewportRequest::Scroll {
                rows: *rows,
                cancelled: false,
            }),
            ClientMessage::Selection(SelectionAction::AutoscrollUp { .. }) => {
                Some(PendingViewportRequest::SelectionTick)
            }
            _ => None,
        };
        if pending.is_some() && self.terminal.pending_viewport.is_some() {
            return false;
        }
        let resize = match &message {
            ClientMessage::Resize(size) => Some(*size),
            _ => None,
        };
        if matches!(
            &message,
            ClientMessage::Mouse(_)
                | ClientMessage::Selection(
                    SelectionAction::Begin { .. }
                        | SelectionAction::Update { .. }
                        | SelectionAction::Finish { .. }
                )
        ) && let Some(size) = self.terminal_size().and_then(|screen| {
            self.window
                .as_ref()
                .and_then(|state| surface_size(screen, state.renderer.metrics()))
        }) && self.terminal.last_resize != Some(size)
            && (!self.send(ClientMessage::Resize(size)) || !can_follow_implicit_resize(&message))
        {
            return false;
        }
        let notice_source = if resize.is_some() {
            LocalNoticeSource::Resize
        } else {
            LocalNoticeSource::Input
        };
        let dismisses_clipboard = dismisses_clipboard_notice(notice_source, &message);
        if !self.terminal.model.is_attached() {
            return false;
        }
        let Some(transport) = &self.terminal.transport else {
            return false;
        };
        if message == ClientMessage::ReturnToLive && self.terminal.pending_viewport.is_some() {
            self.terminal.pending_return_to_live = true;
            return true;
        }
        if let Err(error) = transport.send(message) {
            self.terminal
                .model
                .set_venus_notice(LocalNoticeSource::Queue, error.to_string());
            self.refresh_client_view();
            return false;
        }
        if let Some(pending) = pending {
            self.terminal.pending_viewport = Some(pending);
        }
        if let Some(size) = resize {
            self.terminal.last_resize = Some(size);
            self.terminal.deferred_selection.clear();
            self.input.cancel_selection();
            self.terminal.selection_scroll.cancel();
        }
        let queue_recovered = self
            .terminal
            .model
            .clear_venus_notice(LocalNoticeSource::Queue);
        let clipboard_cleared = dismisses_clipboard
            && self
                .terminal
                .model
                .clear_venus_notice(LocalNoticeSource::Clipboard);
        let attention_cleared = dismisses_clipboard && self.workspace_model.dismiss_no_attention();
        if self.terminal.model.clear_venus_notice(notice_source)
            || queue_recovered
            || clipboard_cleared
            || attention_cleared
        {
            self.refresh_client_view();
        }
        true
    }

    fn has_pointer_sequence(&self) -> bool {
        self.input.is_selecting()
            || self.terminal.deferred_selection.iter().any(|message| {
                matches!(
                    message,
                    ClientMessage::Selection(
                        SelectionAction::Begin { .. }
                            | SelectionAction::Update { .. }
                            | SelectionAction::Finish { .. }
                    )
                )
            })
    }

    fn cancel_pointer_sequence(&mut self) {
        self.terminal.selection_scroll.cancel();
        self.header_press = None;
        self.terminal.scrollback_press = None;
        let cancel_server = pointer_sequence_needs_cancel(
            &self.terminal.selection_gate,
            self.input.is_selecting(),
            &self.terminal.deferred_selection,
        );
        self.terminal.deferred_selection.clear();
        self.input.cancel_selection();
        if cancel_server {
            self.send(ClientMessage::Selection(SelectionAction::Cancel));
        }
    }

    fn send_selection(&mut self, message: ClientMessage) {
        if self.send(message.clone()) {
            self.input.commit_selection(&message);
            if matches!(
                message,
                ClientMessage::Selection(SelectionAction::Finish { .. })
            ) {
                self.terminal.selection_gate.finish_sent();
            }
        } else {
            self.cancel_pointer_sequence();
        }
    }

    fn send_or_defer_selection(&mut self, message: ClientMessage, presentation_current: bool) {
        if selection_presentation_is_ready(&message, presentation_current)
            && self.terminal.deferred_selection.is_empty()
            && matches!(self.terminal.selection_gate, SelectionGate::Ready)
        {
            self.send_selection(message);
        } else if queue_deferred_selection(&mut self.terminal.deferred_selection, message.clone()) {
            self.input.commit_selection(&message);
        } else {
            self.cancel_pointer_sequence();
            self.terminal.model.set_venus_notice(
                LocalNoticeSource::Input,
                "Venus selection input exceeded its bounded capacity",
            );
            self.refresh_client_view();
        }
    }

    fn flush_deferred_selection(&mut self, frame_revision: u64) {
        if !self.terminal.selection_gate.admits(frame_revision) {
            return;
        }
        let batch = take_deferred_selection(&mut self.terminal.deferred_selection, frame_revision);
        let mut server_gesture = matches!(
            batch.first(),
            Some(ClientMessage::Selection(
                SelectionAction::Update { .. } | SelectionAction::Finish { .. }
            ))
        );
        for message in batch {
            let begins = matches!(
                message,
                ClientMessage::Selection(SelectionAction::Begin { .. })
            );
            let finished = matches!(
                message,
                ClientMessage::Selection(SelectionAction::Finish { .. })
            );
            if !self.send(message) {
                self.terminal.deferred_selection.clear();
                self.input.cancel_selection();
                if server_gesture {
                    self.send(ClientMessage::Selection(SelectionAction::Cancel));
                }
                return;
            }
            if finished {
                self.terminal.selection_gate.finish_sent();
            }
            server_gesture |= begins;
        }
    }

    fn presented_revision(&self) -> Option<u64> {
        self.presentation.presented_revision()
    }

    fn write_clipboard(&mut self, effect: ClipboardEffect) {
        write_clipboard_effect(&mut self.terminal.model, effect);
    }

    fn paste_clipboard(&mut self) {
        if !self.terminal.model.is_attached() {
            return;
        }
        let result = read_native_clipboard().and_then(clipboard_paste_message);
        match result {
            Ok(message) => {
                self.send(message);
            }
            Err(error) => {
                self.terminal
                    .model
                    .set_venus_notice(LocalNoticeSource::Clipboard, clipboard_paste_notice(error));
                self.refresh_client_view();
            }
        }
    }

    fn send_resize(&mut self) {
        self.send_column_resizes();
        let Some(state) = &self.window else {
            return;
        };
        let Some(screen) = self.terminal_size() else {
            return;
        };
        let Some(size) = surface_size(screen, state.renderer.metrics()) else {
            self.terminal.model.set_venus_notice(
                LocalNoticeSource::Resize,
                "Window dimensions are outside Orbit's accepted surface range",
            );
            return;
        };
        if self.terminal.last_resize == Some(size) {
            self.terminal
                .model
                .clear_venus_notice(LocalNoticeSource::Resize);
            return;
        }
        self.send(ClientMessage::Resize(size));
    }

    fn status(&self) -> String {
        self.status_for(self.links.unshifted)
    }

    fn status_for(&self, unshifted: bool) -> String {
        if let Some(notice) = &self.render_notice {
            return notice.clone();
        }
        let links_available =
            self.terminal.model.is_attached() && !self.terminal.model.awaiting_current_frame();
        // The newer of a link result and a workspace notice shows. With no result, a
        // hovered link's hint outranks the workspace notice, which returns when it leaves.
        let link_first = match &self.links.notice {
            Some(notice) => notice.workspace_serial == self.workspace_model.notice_serial(),
            None => self.visible_link(unshifted).is_some(),
        };
        if links_available
            && link_first
            && self.terminal.model.notice().is_none()
            && let Some(status) = self.link_status(unshifted)
        {
            return status;
        }
        if let Some(notice) = self.workspace_model.notice() {
            return notice.to_owned();
        }
        if let Some(notice) = status_notice(
            self.terminal.model.notice(),
            links_available.then(|| self.links.notice()).flatten(),
        ) {
            return notice.to_owned();
        }
        if links_available && let Some(status) = self.link_status(unshifted) {
            return status;
        }
        if self.initial_scale_pending {
            return "Opening terminal.".into();
        }
        if let Some(socket) = &self.workspace_socket
            && self.workspace_model.snapshot().is_none()
        {
            return format!("Connecting to Eon workspace at {}", socket.display());
        }
        match self.terminal.model.connection() {
            ConnectionState::Connecting | ConnectionState::Attached
                if self.workspace_model.snapshot().is_some() =>
            {
                String::new()
            }
            ConnectionState::Connecting => {
                format!(
                    "Connecting to Orbit at {}",
                    self.terminal
                        .orbit_socket
                        .as_ref()
                        .expect("connecting Orbit has an attachment socket")
                        .display()
                )
            }
            ConnectionState::Attached if self.terminal.model.awaiting_current_frame() => {
                "Attached to Orbit. Waiting for its current frame.".into()
            }
            ConnectionState::Attached => String::new(),
            ConnectionState::Busy => "Orbit already has an attached presentation client.".into(),
            ConnectionState::Incompatible { version } => format!(
                "Orbit uses local-session revision {version}; Venus requires revision {}.",
                session::VERSION
            ),
            ConnectionState::Lost { detail } => format!("Orbit connection lost: {detail}"),
            ConnectionState::Exited { code } => {
                format!("The authoritative Orbit process exited with status {code}.")
            }
        }
    }

    fn render(&mut self) {
        self.sync_images();
        if self.window_focused && !self.window_occluded {
            self.terminal.terminal_scroll.advance(Instant::now());
            self.drive_terminal_scroll();
        }
        if !cursor_animation_allowed(self.window_focused, self.window_occluded)
            || (self.terminal.model.scene().is_some()
                && (!self.terminal.model.is_attached()
                    || self.terminal.model.awaiting_current_frame()))
        {
            self.reset_cursor_animation();
        }
        let scroll_offset = self.window.as_ref().map_or(0.0, |state| {
            self.terminal.terminal_scroll.offset(
                self.terminal.model.scroll_preview(),
                f64::from(state.renderer.metrics().height),
            )
        });
        let unshifted = scroll_offset == 0.0;
        // The frame drawn now carries its own hover, so output does not blink the highlight.
        let hover_changed = self.hover_link(unshifted);
        let shown = self.visible_link(unshifted).and(self.links.focus.clone());
        let owned_status;
        let status = if let Some(notice) = self.render_notice.as_deref() {
            notice
        } else {
            owned_status = self.status_for(unshifted);
            owned_status.as_str()
        };
        let preedit = self.input.preedit();
        let workspace = self.workspace_scene();
        let dialog = self.dialog_scene();
        let workspace_focus = self.workspace_focus;
        let candidate = self.presentation_candidate(workspace.as_ref());
        let kinetic_active = self.terminal.terminal_scroll.velocity != 0.0;
        let link_generation = (candidate.revision.is_some()
            && self.link_surface_available(unshifted))
        .then_some(candidate.generation);
        let scrollback_label = self.visible_scrollback_label(workspace.as_ref());
        let prior_scrollback_target = self.scrollback_rect(workspace.as_ref());
        let highlighted_link = shown.as_ref().map(|link| (link.row, link.column));
        let hovered_hit = (dialog.is_none()
            && self.window_focused
            && self.links.pointer_inside
            && !self.input.pointer_busy())
        .then(|| {
            workspace
                .as_ref()
                .and_then(|scene| scene.hit_test(self.cursor.x as f32, self.cursor.y as f32))
        })
        .flatten();
        let hovered_header = match hovered_hit {
            Some(WorkspaceHit::Tab(id)) => Some((WorkspaceFocus::Tabs, id.to_owned())),
            Some(WorkspaceHit::Pane(id)) => Some((WorkspaceFocus::Panes, id.to_owned())),
            Some(WorkspaceHit::SidebarRow(id)) => Some((WorkspaceFocus::Sidebar, id.to_owned())),
            Some(WorkspaceHit::Control(control)) => {
                Some((WorkspaceFocus::Header(control), String::new()))
            }
            _ => None,
        };
        let quota_hovered = if let Some(WorkspaceHit::Quota(index)) = hovered_hit {
            Some(index)
        } else {
            None
        };
        let version_hovered = matches!(hovered_hit, Some(WorkspaceHit::Version));
        let cpu_hovered = matches!(hovered_hit, Some(WorkspaceHit::Cpu));
        let ram_hovered = matches!(hovered_hit, Some(WorkspaceHit::Ram));
        let clock_hovered = matches!(hovered_hit, Some(WorkspaceHit::Clock));
        let mut refresh = false;
        let mut presented = false;
        let Some(state) = &mut self.window else {
            return;
        };
        state.renderer.set_hyperlink(highlighted_link);
        state
            .renderer
            .set_scrollback_label(scrollback_label.clone());
        state.renderer.set_hovered_header(hovered_header);
        state.renderer.set_quota_hovered(quota_hovered);
        state.renderer.set_version_hovered(version_hovered);
        state.renderer.set_cpu_hovered(cpu_hovered);
        state.renderer.set_ram_hovered(ram_hovered);
        state.renderer.set_clock_hovered(clock_hovered);
        state.renderer.set_pressed_header(match self.header_press {
            Some(HeaderPress::Control(control)) => Some(control),
            _ => None,
        });
        match state.renderer.render(
            self.terminal.model.scene(),
            self.terminal.model.scroll_preview(),
            scroll_offset,
            workspace.as_ref(),
            &column_scenes(
                &self.workspace_model,
                &self.column_views,
                workspace.as_ref(),
            ),
            dialog.as_ref(),
            workspace_focus,
            status,
            self.blink_visible,
            preedit,
            candidate.generation,
        ) {
            Ok(PresentOutcome::Presented) => {
                presented = true;
                refresh = self.render_notice.take().is_some()
                    || state.renderer.presented_scrollback_rect() != prior_scrollback_target;
                self.presentation.publish(candidate);
                if let Some(revision) = candidate.revision {
                    self.terminal.selection_scroll.presented(
                        revision,
                        Instant::now(),
                        self.terminal.pending_viewport,
                    );
                }
                self.links.unshifted = unshifted;
                self.links.shown = shown;
                state.accessibility.present_links(
                    &mut state.adapter,
                    self.terminal.model.scene(),
                    link_generation,
                );
                if kinetic_active {
                    state.window.request_redraw();
                }
                if let Some(revision) = candidate.revision {
                    self.flush_deferred_selection(revision);
                }
            }
            Ok(PresentOutcome::Deferred) => {}
            Ok(PresentOutcome::Occluded) => {
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                self.links.unshifted = false;
            }
            Ok(PresentOutcome::Recovered) => {
                self.links.unshifted = false;
                self.presentation.invalidate();
                state.window.request_redraw();
            }
            Err(error) => {
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                if let Some(notice) = record_render_failure(
                    &mut self.render_notice,
                    &mut self.presentation,
                    error,
                    |title| state.window.set_title(title),
                    |message| report(message),
                ) {
                    state.accessibility.update(
                        &mut state.adapter,
                        self.terminal.model.scene(),
                        workspace.as_ref(),
                        dialog.as_ref(),
                        workspace_focus,
                        notice,
                        scrollback_label.as_deref(),
                        None,
                        state.renderer.size(),
                        state.renderer.metrics(),
                        None,
                    );
                }
            }
        }
        if refresh || hover_changed {
            self.refresh_client_view();
        }
        if presented {
            self.inspect_pointer_link();
        }
    }
}

fn window_attributes(
    decorations: bool,
    background_opacity: f32,
    background_blur: bool,
) -> WindowAttributes {
    Window::default_attributes()
        .with_title("Venus")
        .with_visible(false)
        .with_inner_size(LogicalSize::new(960.0, 600.0))
        .with_decorations(decorations)
        .with_transparent(background_opacity < 1.0)
        .with_blur(background_blur)
}

impl ApplicationHandler<UserEvent> for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && self.initial_scale_pending
            && self.workspace_model.snapshot().is_none()
            && let Some(socket) = self.workspace_socket.clone()
        {
            if self.workspace_transport.is_none() {
                self.start_workspace(socket);
            }
            return;
        }
        if self.window.is_none()
            && let Err(error) = self.create_window(event_loop)
        {
            self.fatal_error = Some(error.to_string());
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let workspace = self.workspace_scene();
        let workspace_focus = self.workspace_focus;
        let dialog_open = self.dialog_open();
        let candidate = self.presentation_candidate(workspace.as_ref());
        let presentation_current = self.presentation.has_presented_geometry();
        let presented_revision = self.presentation.presented_revision();
        let scrollback_rect = self.scrollback_rect(workspace.as_ref());
        let Some(state) = &mut self.window else {
            return;
        };
        if state.window.id() != window_id {
            return;
        }
        state.adapter.process_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.next_animation = None;
                self.input.reset_scroll();
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                state.renderer.resize(size, state.scale_factor);
                self.cancel_pointer_sequence();
                self.reveal_workspace_selection();
                if self.attention_inbox.open {
                    self.reveal_inbox_highlight();
                }
                self.presentation.invalidate();
                self.send_resize();
                // A sidebar or workspace label hidden for width gives up its focus.
                self.retain_chrome_focus();
                self.refresh_client_view();
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                mut inner_size_writer,
            } => {
                if self.initial_scale_pending {
                    let initial = initial_window_size(
                        self.columns,
                        self.rows,
                        state.renderer.metrics_at_scale(scale_factor),
                        scale_factor,
                        self.workspace_model.snapshot(),
                    );
                    match initial.and_then(|size| {
                        inner_size_writer
                            .request_inner_size(size)
                            .map_err(Into::into)
                    }) {
                        Ok(()) => {}
                        Err(error) => {
                            self.fatal_error = Some(error.to_string());
                            event_loop.exit();
                            return;
                        }
                    }
                }
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                state.scale_factor = scale_factor;
                report(format_args!("window scale factor {scale_factor}"));
                self.presentation.invalidate();
                self.cancel_pointer_sequence();
            }
            WindowEvent::Occluded(occluded) => {
                self.window_occluded = occluded;
                self.next_animation = None;
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                state.renderer.reset_cursor_animation();
                if occluded {
                    self.presentation.invalidate();
                    self.cancel_pointer_sequence();
                } else {
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.cancel_terminal_scroll();
                self.input.set_modifiers(modifiers.state());
                if !dialog_open {
                    self.inspect_pointer_link();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.cancel_terminal_scroll();
                let Some(event) = self.input.resolve_key(event) else {
                    return;
                };
                let modifiers = self.input.modifiers();
                let pastes = paste_shortcut(event.physical_key, modifiers, self.popup_entries());
                let copies =
                    native_key_shortcut(event.physical_key, modifiers, self.popup_entries())
                        .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Copy);
                if self
                    .input
                    .suppresses_retired_key(event.physical_key, event.state, event.repeat)
                    || self.handle_attention_inbox_key(&event)
                    || self.handle_detach_key(event_loop, &event)
                    || self.handle_close_prompt_key(&event)
                    || self.handle_shortcut_viewer_key(&event)
                    || self.handle_link_copy(event.physical_key, event.state, event.repeat)
                    || self.handle_workspace_key(&event)
                {
                } else if self.input.consumes_shortcut(
                    event.physical_key,
                    event.state,
                    event.repeat,
                    pastes,
                ) {
                    if shortcut_is_ready(event.state, event.repeat) {
                        self.paste_clipboard();
                    }
                } else if self.input.consumes_shortcut(
                    event.physical_key,
                    event.state,
                    event.repeat,
                    copies,
                ) {
                    if copy_is_ready(self.input.is_selecting(), event.state, event.repeat) {
                        self.send_or_defer_selection(
                            ClientMessage::Selection(SelectionAction::Copy),
                            presentation_current,
                        );
                    }
                } else if let Some(message) = self.input.key(&event)
                    && (self.send(message) || event.state == ElementState::Released)
                {
                    self.input.commit_key(event.physical_key, event.state);
                }
            }
            WindowEvent::Ime(event) => {
                self.cancel_terminal_scroll();
                let allowed = !dialog_open
                    && ime_allowed(
                        self.window_focused,
                        workspace_focus,
                        self.terminal.model.is_attached(),
                    );
                if ime_reaches_terminal(allowed, &event)
                    && let Some(message) =
                        native_ime_message(&mut self.input, &mut self.terminal.model, event)
                {
                    self.send(message);
                }
                self.refresh_client_view();
            }
            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                self.next_animation = None;
                self.terminal
                    .terminal_scroll
                    .cancel(&mut self.terminal.pending_viewport);
                state.renderer.reset_cursor_animation();
                if !focused {
                    self.links.focus = None;
                    self.cancel_pointer_sequence();
                }
                let message = self.input.native_focus(
                    focused,
                    terminal_focused(focused, workspace_focus) && !dialog_open,
                );
                self.send(message);
                self.refresh_client_view();
            }
            WindowEvent::CursorMoved { position, .. } => {
                if dialog_open {
                    self.terminal.selection_scroll.cancel();
                    self.cursor = position;
                    self.links.pointer_inside = true;
                    return;
                }
                let header_at = |position: PhysicalPosition<f64>| {
                    workspace.as_ref().and_then(|scene| {
                        match scene.hit_test(position.x as f32, position.y as f32) {
                            hit @ Some(
                                WorkspaceHit::Tab(_)
                                | WorkspaceHit::Pane(_)
                                | WorkspaceHit::Quota(_)
                                | WorkspaceHit::Version
                                | WorkspaceHit::Cpu
                                | WorkspaceHit::Ram
                                | WorkspaceHit::Clock
                                | WorkspaceHit::Control(_),
                            ) => hit,
                            _ => None,
                        }
                    })
                };
                if header_at(self.cursor) != header_at(position)
                    || (!self.links.pointer_inside && header_at(position).is_some())
                {
                    state.window.request_redraw();
                }
                let was_scrollback = scrollback_rect
                    .is_some_and(|rect| rect.contains(self.cursor.x as f32, self.cursor.y as f32));
                let over_scrollback = scrollback_rect
                    .is_some_and(|rect| rect.contains(position.x as f32, position.y as f32));
                self.cursor = position;
                self.links.pointer_inside = true;
                if over_scrollback {
                    self.terminal.selection_scroll.edge(
                        false,
                        Instant::now(),
                        self.terminal.pending_viewport,
                    );
                    self.links.focus = None;
                    if was_scrollback != over_scrollback {
                        self.refresh_client_view();
                    }
                    return;
                }
                if self.links.pressed.is_some()
                    || self.header_press.is_some()
                    || self.terminal.scrollback_press.is_some()
                {
                    self.terminal.selection_scroll.edge(
                        false,
                        Instant::now(),
                        self.terminal.pending_viewport,
                    );
                    if was_scrollback != over_scrollback {
                        self.refresh_client_view();
                    }
                    return;
                }
                let motion = move_terminal_pointer(&mut self.input, position, workspace.as_ref());
                if self.input.is_selecting() {
                    let screen = terminal_screen(workspace.as_ref(), state.renderer.size());
                    let size = surface_size(screen, state.renderer.metrics());
                    if let Some(message) = size.and_then(|size| self.input.selection_motion(size)) {
                        self.send_or_defer_selection(message, presentation_current);
                    }
                    self.terminal.selection_scroll.edge(
                        workspace.as_ref().is_none_or(|workspace| {
                            matches!(
                                workspace.hit_test(position.x as f32, position.y as f32),
                                Some(WorkspaceHit::Terminal)
                            )
                        }) && size
                            .and_then(|size| self.input.selection_at_top(size))
                            .is_some(),
                        Instant::now(),
                        self.terminal.pending_viewport,
                    );
                } else if presented_revision.is_some()
                    && workspace.as_ref().is_none_or(|workspace| {
                        matches!(
                            workspace.hit_test(position.x as f32, position.y as f32),
                            Some(WorkspaceHit::Terminal)
                        )
                    })
                    && let Some(message) = motion
                {
                    self.send(message);
                }
                if !self.input.is_selecting() {
                    self.terminal.selection_scroll.edge(
                        false,
                        Instant::now(),
                        self.terminal.pending_viewport,
                    );
                }
                self.inspect_pointer_link();
                if was_scrollback != over_scrollback {
                    self.refresh_client_view();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.links.pointer_inside = false;
                self.terminal.scrollback_press = None;
                state.window.request_redraw();
                self.cancel_terminal_scroll();
                self.cancel_pointer_sequence();
                self.links.focus = None;
                self.refresh_client_view();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } => {
                if button == MouseButton::Left {
                    self.terminal.selection_scroll.cancel();
                }
                if dialog_open {
                    if self.attention_inbox.open
                        && presentation_current
                        && button == MouseButton::Left
                        && button_state == ElementState::Pressed
                        && let Some(dialog) = self.dialog_scene()
                    {
                        let x = self.cursor.x as f32;
                        let y = self.cursor.y as f32;
                        if let Some(filter) = dialog.attention_filter_at(x, y) {
                            self.filter_attention_inbox(filter);
                        } else if let Some((tab, target)) = dialog.attention_target_at(x, y) {
                            self.activate_inbox_row(tab.to_owned(), target.to_owned());
                        }
                    }
                    return;
                }
                if button_state == ElementState::Pressed {
                    self.terminal
                        .terminal_scroll
                        .cancel(&mut self.terminal.pending_viewport);
                    let navigation = matches!(
                        workspace.as_ref().and_then(|scene| {
                            scene.hit_test(self.cursor.x as f32, self.cursor.y as f32)
                        }),
                        Some(
                            WorkspaceHit::Tab(_)
                                | WorkspaceHit::Pane(_)
                                | WorkspaceHit::ColumnTerminal(_)
                                | WorkspaceHit::Control(_)
                        )
                    );
                    if !navigation {
                        state.renderer.reset_cursor_animation();
                    }
                    state.window.request_redraw();
                }
                let renderer_size = state.renderer.size();
                let metrics = state.renderer.metrics();
                let window = Arc::clone(&state.window);
                if button == MouseButton::Left && button_state == ElementState::Pressed {
                    self.header_press = None;
                }
                let scrollback_target = scrollback_rect
                    .filter(|rect| {
                        self.links.pointer_inside
                            && rect.contains(self.cursor.x as f32, self.cursor.y as f32)
                    })
                    .map(|_| candidate);
                if let Some(activate) = scrollback_button(
                    &mut self.terminal.scrollback_press,
                    button_state,
                    button,
                    scrollback_target,
                ) {
                    if activate {
                        self.send(ClientMessage::ReturnToLive);
                    } else if button_state == ElementState::Pressed {
                        self.links.pressed = None;
                    }
                    self.refresh_client_view();
                    return;
                }
                if self.handle_link_button(button_state, button) {
                    return;
                }
                if button_state == ElementState::Pressed {
                    self.links.focus = None;
                    self.links.notice = None;
                    self.refresh_client_view();
                }
                let hit = workspace.as_ref().and_then(|workspace| {
                    workspace.hit_test(self.cursor.x as f32, self.cursor.y as f32)
                });
                if button == MouseButton::Left
                    && button_state == ElementState::Released
                    && let Some(press) = self.header_press.take()
                {
                    let activate = match press {
                        HeaderPress::Control(control)
                            if presentation_current
                                && hit == Some(WorkspaceHit::Control(control)) =>
                        {
                            Some(control)
                        }
                        HeaderPress::Control(_) | HeaderPress::Drag => None,
                    };
                    self.refresh_client_view();
                    if let Some(control) = activate {
                        self.activate_workspace_header(control);
                    }
                    return;
                }
                if presentation_current
                    && button_state == ElementState::Pressed
                    && button == MouseButton::Left
                {
                    let target = match hit {
                        Some(WorkspaceHit::Tab(id)) => Some((WorkspaceFocus::Tabs, id.to_owned())),
                        Some(WorkspaceHit::Pane(id)) => {
                            Some((WorkspaceFocus::Panes, id.to_owned()))
                        }
                        Some(WorkspaceHit::ColumnTerminal(id)) => {
                            Some((WorkspaceFocus::Terminal, id.to_owned()))
                        }
                        Some(WorkspaceHit::Terminal) => {
                            self.set_workspace_focus(WorkspaceFocus::Terminal);
                            None
                        }
                        Some(WorkspaceHit::SidebarRow(id)) => {
                            let id = id.to_owned();
                            // Jump from the presented rows, then hold focus on the row.
                            self.activate_sidebar_row(&id);
                            self.focus_sidebar_row(id);
                            return;
                        }
                        Some(WorkspaceHit::Control(control)) => {
                            self.set_workspace_focus(WorkspaceFocus::Header(control));
                            self.header_press = Some(HeaderPress::Control(control));
                            self.refresh_client_view();
                            return;
                        }
                        Some(WorkspaceHit::Drag) => {
                            self.header_press = Some(HeaderPress::Drag);
                            if let Err(error) = window.drag_window() {
                                report(format!("cannot drag window: {error}"));
                            }
                            return;
                        }
                        Some(
                            WorkspaceHit::Quota(_)
                            | WorkspaceHit::Version
                            | WorkspaceHit::Cpu
                            | WorkspaceHit::Ram
                            | WorkspaceHit::Clock,
                        ) => None,
                        None => None,
                    };
                    if let Some((focus, id)) = target {
                        if self.send_workspace(CommonAction::FocusId(id)) {
                            self.set_workspace_focus(focus);
                        }
                        return;
                    }
                }
                if !button_reaches_terminal(
                    workspace.is_some(),
                    matches!(hit, Some(WorkspaceHit::Terminal)),
                    button_state,
                    presentation_current
                        || button == MouseButton::Left && candidate.revision.is_some(),
                ) {
                    return;
                }
                let _ = move_terminal_pointer(&mut self.input, self.cursor, workspace.as_ref());
                let screen = terminal_screen(workspace.as_ref(), renderer_size);
                let size = surface_size(screen, metrics);
                let selection = size.and_then(|size| {
                    self.input.selection_button(
                        button_state,
                        button,
                        size,
                        presented_revision.or(candidate.revision),
                        u64::try_from(self.input_epoch.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    )
                });
                if let Some(message) = selection {
                    self.send_or_defer_selection(message, presentation_current);
                } else if let Some(message) =
                    self.input
                        .mouse_button(button_state, button, presented_revision.is_some())
                    && (self.send(message) || button_state == ElementState::Released)
                {
                    self.input.commit_mouse_button(button_state, button);
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                if dialog_open {
                    let metrics = state.renderer.metrics();
                    let pixels = match delta {
                        MouseScrollDelta::LineDelta(_, vertical) => {
                            -vertical * metrics.height * 3.0
                        }
                        MouseScrollDelta::PixelDelta(position) => -position.y as f32,
                    };
                    self.scroll_dialog(pixels);
                    return;
                }
                if self.input.is_selecting() {
                    return;
                }
                self.links.pressed = None;
                self.header_press = None;
                self.terminal.scrollback_press = None;
                let metrics = state.renderer.metrics();
                let hit = workspace.as_ref().and_then(|workspace| {
                    workspace.hit_test(self.cursor.x as f32, self.cursor.y as f32)
                });
                if !matches!(hit, Some(WorkspaceHit::Terminal)) && workspace.is_some() {
                    let scrolling = self
                        .terminal
                        .terminal_scroll
                        .active(self.terminal.pending_viewport);
                    self.terminal
                        .terminal_scroll
                        .cancel(&mut self.terminal.pending_viewport);
                    if scrolling {
                        state.renderer.reset_cursor_animation();
                    }
                    state.window.request_redraw();
                }
                if workspace.as_ref().is_some_and(|scene| {
                    scene
                        .tab_viewport
                        .contains(self.cursor.x as f32, self.cursor.y as f32)
                }) {
                    if presentation_current {
                        self.scroll_workspace(delta, true, metrics);
                    }
                    return;
                }
                // Header scrolling belongs to the selected column; other columns' headers ignore the wheel.
                if let (Some(WorkspaceHit::Pane(id)), Some(scene)) = (hit, workspace.as_ref())
                    && scene.panes.iter().any(|pane| pane.id == id)
                {
                    if presentation_current {
                        self.scroll_workspace(delta, false, metrics);
                    }
                    return;
                }
                if workspace.is_some() && !matches!(hit, Some(WorkspaceHit::Terminal)) {
                    return;
                }
                if presented_revision.is_none() {
                    return;
                }
                let _ = move_terminal_pointer(&mut self.input, self.cursor, workspace.as_ref());
                let (horizontal, vertical) = match delta {
                    MouseScrollDelta::LineDelta(horizontal, vertical) => (
                        MouseScrollDelta::LineDelta(horizontal, 0.0),
                        f64::from(vertical),
                    ),
                    MouseScrollDelta::PixelDelta(position) => (
                        MouseScrollDelta::PixelDelta(PhysicalPosition::new(position.x, 0.0)),
                        position.y,
                    ),
                };
                for message in self.input.wheel(horizontal, metrics.width, metrics.height) {
                    if !self.send(message) {
                        break;
                    }
                }
                match delta {
                    MouseScrollDelta::LineDelta(_, _) => self
                        .terminal
                        .terminal_scroll
                        .push_lines(vertical, f64::from(metrics.height)),
                    MouseScrollDelta::PixelDelta(_) => self.terminal.terminal_scroll.push_pixels(
                        vertical,
                        phase,
                        Instant::now(),
                        f64::from(metrics.height),
                    ),
                }
                self.drive_terminal_scroll();
                if let Some(state) = &self.window {
                    state.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::NewWindowFailed(detail) => {
                self.terminal
                    .model
                    .set_venus_notice(LocalNoticeSource::Input, detail);
                self.refresh_client_view();
            }
            UserEvent::Exit => event_loop.exit(),
            UserEvent::Present => {
                if !self.window_focused
                    && let Some(state) = &self.window
                {
                    state
                        .window
                        .request_user_attention(Some(UserAttentionType::Informational));
                }
            }
            UserEvent::Transport => {
                let events = self
                    .terminal
                    .transport
                    .as_ref()
                    .map_or_else(Vec::new, Transport::drain_events);
                for event in events {
                    self.handle_transport(event);
                }
                for index in 0..self.column_views.len() {
                    let events = self.column_views[index]
                        .transport
                        .as_ref()
                        .map_or_else(Vec::new, Transport::drain_events);
                    for event in events {
                        self.handle_column_transport(index, event);
                    }
                }
                self.drive_terminal_scroll();
            }
            UserEvent::Workspace => {
                let events = self
                    .workspace_transport
                    .as_ref()
                    .map_or_else(Vec::new, WorkspaceTransport::drain_events);
                for event in events {
                    // A close that stopped the last terminal ended the
                    // generation; exit as on control loss.
                    if matches!(
                        event,
                        WorkspaceEvent::Response(eon_workspace_protocol::v26::Response::Ended(_))
                    ) {
                        event_loop.exit();
                        return;
                    }
                    self.handle_workspace(event);
                }
                if self.window.is_none() {
                    if self.workspace_model.snapshot().is_some() {
                        self.resumed(event_loop);
                    } else if let Some(notice) = self.workspace_model.notice() {
                        self.fatal_error = Some(notice.to_owned());
                        event_loop.exit();
                    }
                }
            }
            UserEvent::AccessKit(event) => {
                let Some(state) = &mut self.window else {
                    return;
                };
                if state.window.id() != event.window_id {
                    return;
                }
                match event.window_event {
                    AccessKitWindowEvent::InitialTreeRequested => self.refresh_client_view(),
                    AccessKitWindowEvent::ActionRequested(request) => {
                        let target = state
                            .accessibility
                            .action_target(request.target_node, request.action);
                        match target {
                            Some(AccessibilityTarget::InboxFilter(filter)) => {
                                self.filter_attention_inbox(filter)
                            }
                            Some(AccessibilityTarget::InboxRow {
                                tab,
                                target,
                                activate,
                            }) => {
                                if activate {
                                    self.activate_inbox_row(tab, target);
                                } else {
                                    self.attention_inbox.highlighted = Some((tab, target));
                                    self.reveal_inbox_highlight();
                                    self.refresh_client_view();
                                }
                            }
                            Some(AccessibilityTarget::Link { row, column, copy }) => {
                                self.activate_accessible_link(row, column, copy);
                            }
                            Some(AccessibilityTarget::ReturnToLive)
                                if self
                                    .scrollback_rect(self.workspace_scene().as_ref())
                                    .is_some() =>
                            {
                                self.send(ClientMessage::ReturnToLive);
                            }
                            Some(AccessibilityTarget::ReturnToLive) => {}
                            Some(AccessibilityTarget::HeaderControl { control, activate }) => {
                                self.set_workspace_focus(WorkspaceFocus::Header(control));
                                if activate {
                                    self.activate_workspace_header(control);
                                }
                            }
                            Some(AccessibilityTarget::SidebarRow { target, activate }) => {
                                if activate {
                                    self.activate_sidebar_row(&target);
                                }
                                self.focus_sidebar_row(target);
                            }
                            Some(target) => {
                                if let Some(focus) =
                                    accessibility_workspace_focus(target, |action| {
                                        self.queue_workspace(WorkspaceAction::Workspace(action))
                                    })
                                {
                                    self.set_workspace_focus(focus);
                                }
                            }
                            None => {}
                        }
                    }
                    AccessKitWindowEvent::AccessibilityDeactivated => {}
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Initial Wayland scale can arrive after the first buffer. Once the
        // window has entered an output, later scale/size changes belong to the user.
        if self.initial_scale_pending
            && self.fatal_error.is_none()
            && self
                .window
                .as_ref()
                .is_some_and(|state| state.window.current_monitor().is_some())
        {
            self.initial_scale_pending = false;
            if self.startup_admission
                && (self.workspace_socket.is_none()
                    || self.workspace_model.active_attachment().is_some())
                && self
                    .terminal_size()
                    .and_then(|size| {
                        surface_size(size, self.window.as_ref().unwrap().renderer.metrics())
                    })
                    .is_none()
            {
                self.fatal_error =
                    Some("initial native window leaves no valid terminal grid".into());
                event_loop.exit();
                return;
            }
            if self.startup_admission
                && let Err(error) = io::stdout()
                    .write_all(b"ready-v1")
                    .and_then(|()| io::stdout().flush())
            {
                self.fatal_error = Some(format!("Cannot report Venus startup readiness: {error}"));
                event_loop.exit();
                return;
            }
            self.start_initial_attachment();
            self.refresh_client_view();
        }
        let now = Instant::now();
        if let Some(result) = self
            .link_opener
            .as_mut()
            .and_then(|opener| opener.poll(now))
        {
            self.link_opener = None;
            self.links.show_notice(
                result.unwrap_or_else(|error| error),
                result.is_ok(),
                now,
                self.workspace_model.notice_serial(),
            );
            self.refresh_client_view();
        }
        if self.links.expire_notice(now) {
            self.refresh_client_view();
        }
        self.retry_orbit(now);
        self.retry_column_views(now);
        // Items Eon leaves out of the bar are never read and never wake Venus.
        let shown = |item| self.bar_items.contains(&item);
        let (show_clock, show_cpu, show_ram) = (
            shown(BarItem::Clock),
            shown(BarItem::Cpu),
            shown(BarItem::Ram),
        );
        if self.workspace_model.snapshot().is_some()
            && show_clock
            && self.next_clock.is_none_or(|due| now >= due)
        {
            let reading = read_local_clock();
            self.next_clock = Some(now + reading.as_ref().map_or(MINUTE, |(_, wait)| *wait));
            let clock = reading.map(|(clock, _)| clock);
            if clock != self.clock {
                self.clock = clock;
                self.refresh_client_view();
            }
        }
        if self.workspace_model.snapshot().is_some()
            && (show_cpu || show_ram)
            && self.next_reading.is_none_or(|due| now >= due)
        {
            self.next_reading = Some(now + READING_INTERVAL);
            let times = show_cpu.then(CpuTimes::read).flatten();
            // Each value compares readings about one interval apart, as the label says. After a
            // failed read or an event-loop stall, the next reading starts over.
            let cpu = self
                .cpu_times
                .filter(|(taken, _)| now.duration_since(*taken) < READING_INTERVAL * 3 / 2)
                .zip(times)
                .and_then(|((_, earlier), later)| CpuUse::between(earlier, later));
            self.cpu_times = times.map(|times| (now, times));
            let ram = show_ram.then(RamUse::read).flatten();
            if cpu != self.cpu || ram != self.ram {
                self.cpu = cpu;
                self.ram = ram;
                self.refresh_client_view();
            }
        }
        if self.next_age.is_some_and(|due| now >= due) {
            self.refresh_client_view();
        }
        self.drive_selection_autoscroll(now);
        if self.terminal.terminal_scroll.retry_preview(now) {
            self.drive_terminal_scroll();
        }
        let blinking = std::iter::once(&self.terminal)
            .chain(&self.column_views)
            .filter_map(|view| view.model.scene())
            .any(Scene::has_blinking_content);
        if update_blink(blinking, &mut self.blink_visible, &mut self.next_blink, now)
            && let Some(state) = &self.window
        {
            state.window.request_redraw();
        }
        let spinning = !self.window_occluded
            && self
                .window
                .as_ref()
                .is_some_and(|state| state.renderer.spinning());
        if update_spinner(spinning, &mut self.next_spinner, now)
            && let Some(state) = &mut self.window
        {
            state.renderer.turn_spinners();
            state.window.request_redraw();
        }
        // A pending image entry's marker comes due without any other change (VEN-C31).
        let next_marker = self
            .window
            .as_ref()
            .and_then(|state| state.renderer.next_marker_deadline());
        if next_marker.is_some_and(|due| now >= due && self.marker_wake != Some(due))
            && let Some(state) = &self.window
        {
            self.marker_wake = next_marker;
            state.window.request_redraw();
        }
        let animation_active = self
            .window
            .as_ref()
            .is_some_and(|state| state.renderer.cursor_animation_active());
        if update_animation_deadline(
            animation_active,
            cursor_animation_allowed(self.window_focused, self.window_occluded),
            &mut self.next_animation,
            now,
        ) && let Some(state) = &self.window
        {
            state.window.request_redraw();
        }
        event_loop.set_control_flow(
            [
                self.next_blink,
                self.next_spinner,
                self.next_animation,
                self.next_clock,
                self.next_age,
                self.next_reading,
                next_marker.filter(|due| *due > now),
                self.terminal.selection_scroll.next,
                self.terminal.terminal_scroll.preview_retry_at(),
                self.terminal.orbit_retry.deadline,
                self.column_views
                    .iter()
                    .filter_map(|view| view.orbit_retry.deadline)
                    .min(),
                // winit may suppress output-enter wakeups. Poll only until
                // this window completes its initial native size admission.
                (self.initial_scale_pending && self.window.is_some())
                    .then_some(now + ANIMATION_FRAME_INTERVAL),
                self.link_opener
                    .as_ref()
                    .map(|_| now + Duration::from_millis(50)),
                self.links.notice_deadline(),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}

/// Reads the local wall clock and the wait until its next minute. An unknown local time zone
/// yields no clock rather than UTC.
fn read_local_clock() -> Option<(LocalClock, Duration)> {
    let now = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::try_system().ok()?);
    let elapsed = Duration::from_secs(now.second().unsigned_abs().into())
        + Duration::from_nanos(now.subsec_nanosecond().unsigned_abs().into());
    Some((LocalClock::new(&now), MINUTE.saturating_sub(elapsed)))
}

fn update_blink(
    blinking: bool,
    visible: &mut bool,
    deadline: &mut Option<Instant>,
    now: Instant,
) -> bool {
    if !blinking {
        *visible = true;
        *deadline = None;
    } else if deadline.is_none() {
        *deadline = Some(now + BLINK_INTERVAL);
    } else if deadline.is_some_and(|next| now >= next) {
        *visible = !*visible;
        *deadline = Some(now + BLINK_INTERVAL);
        return true;
    }
    false
}

/// Whether the spinners turn now: once `SPINNER_INTERVAL` has passed since the previous turn
/// while a spinner shows. Ticking stops while none does.
fn update_spinner(spinning: bool, deadline: &mut Option<Instant>, now: Instant) -> bool {
    if !spinning {
        *deadline = None;
    } else if deadline.is_none() {
        *deadline = Some(now + SPINNER_INTERVAL);
    } else if deadline.is_some_and(|next| now >= next) {
        *deadline = Some(now + SPINNER_INTERVAL);
        return true;
    }
    false
}

fn cursor_animation_allowed(focused: bool, occluded: bool) -> bool {
    focused && !occluded
}

fn update_animation_deadline(
    active: bool,
    allowed: bool,
    deadline: &mut Option<Instant>,
    now: Instant,
) -> bool {
    if !active || !allowed {
        *deadline = None;
    } else if deadline.is_none() {
        *deadline = Some(now + ANIMATION_FRAME_INTERVAL);
    } else if deadline.is_some_and(|next| now >= next) {
        *deadline = Some(now + ANIMATION_FRAME_INTERVAL);
        return true;
    }
    false
}

fn retry_is_allowed(
    workspace: bool,
    selected: Option<(&[u8], bool)>,
    active: Option<&[u8]>,
) -> bool {
    !workspace || selected.is_some_and(|(endpoint, live)| live && active == Some(endpoint))
}

/// The selected pane and ordered layout; status and metadata changes do not end zoom.
#[derive(Debug, PartialEq, Eq)]
struct ZoomTarget {
    tab: String,
    pane: String,
    /// Each column's expanded pane and ordered pane IDs.
    columns: Vec<(String, Vec<String>)>,
}

fn zoom_target(snapshot: &eon_workspace_protocol::v26::Snapshot) -> Option<ZoomTarget> {
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)?;
    let selected = tab.selected_pane.as_ref()?;
    if tab.panes().nth(1).is_none()
        || !tab
            .columns
            .iter()
            .any(|column| column.expanded == *selected)
    {
        return None;
    }
    Some(ZoomTarget {
        tab: tab.id.clone(),
        pane: selected.clone(),
        columns: tab
            .columns
            .iter()
            .map(|column| {
                (
                    column.expanded.clone(),
                    column.panes.iter().map(|pane| pane.id.clone()).collect(),
                )
            })
            .collect(),
    })
}

/// Live panes of the active tab, except endpoints whose presentation attachment is not
/// established; none while a popup covers the tab.
fn server_failure_suppresses_retry(message: &ServerMessage) -> bool {
    matches!(
        message,
        ServerMessage::Failure(failure)
            if matches!(failure.code, FailureCode::Protocol | FailureCode::Terminal)
    )
}

fn managed_busy_is_retryable(supervised: bool, event: &TransportEvent) -> bool {
    supervised && matches!(event, TransportEvent::Server(ServerMessage::Busy))
}

fn apply_retryable_loss(model: &mut SessionModel, detail: String, retry_suppressed: bool) -> bool {
    let schedule_retry = !model.is_terminal() && !retry_suppressed;
    model.mark_lost_preserving_constraining_notice(detail);
    schedule_retry
}

fn window_title<'a>(scene_title: Option<&'a str>, render_notice: Option<&'a str>) -> &'a str {
    render_notice
        .or_else(|| scene_title.filter(|title| !title.is_empty()))
        .unwrap_or("Venus")
}

fn report(message: impl std::fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "venus: {message}");
}

fn record_render_failure<'a>(
    render_notice: &'a mut Option<String>,
    presentation: &mut PresentationState,
    error: impl std::fmt::Display,
    set_title: impl FnOnce(&str),
    report: impl FnOnce(&str),
) -> Option<&'a str> {
    presentation.invalidate();
    if render_notice.is_some() {
        return None;
    }
    let notice = format!("Venus renderer failure: {error}");
    set_title(&notice);
    report(&notice);
    Some(render_notice.insert(notice).as_str())
}

/// Each other column's latest scene, aligned with the workspace scene's `other_columns`.
fn column_scenes<'a>(
    workspace_model: &'a WorkspaceModel,
    views: &'a [TerminalView],
    workspace: Option<&WorkspaceScene>,
) -> Vec<Option<&'a Scene>> {
    let attachments = workspace_model.column_attachments();
    workspace.map_or_else(Vec::new, |workspace| {
        workspace
            .other_columns
            .iter()
            .map(|column| {
                let &(_, endpoint, _) = attachments
                    .iter()
                    .find(|&&(index, _, _)| index == column.index)?;
                views
                    .iter()
                    .find(|view| view.active_endpoint.as_deref() == Some(endpoint))?
                    .model
                    .scene()
            })
            .collect()
    })
}

fn orbit_transport(proxy: &EventLoopProxy<UserEvent>, socket: PathBuf) -> Transport {
    let proxy = proxy.clone();
    Transport::start(socket, move || {
        let _ = proxy.send_event(UserEvent::Transport);
    })
}

/// Write a terminal's clipboard effect natively and report the result on that terminal.
fn write_clipboard_effect(model: &mut SessionModel, effect: ClipboardEffect) {
    let clipboard = native_clipboard(&effect);
    let text = match effect {
        ClipboardEffect::SelectionCopy { text, .. }
        | ClipboardEffect::TerminalWrite { text, .. } => text,
    };
    let result = write_native_clipboard(clipboard, text);
    model.set_venus_notice(
        LocalNoticeSource::Clipboard,
        clipboard_notice(clipboard, result),
    );
}

fn can_follow_implicit_resize(message: &ClientMessage) -> bool {
    matches!(message, ClientMessage::Mouse(_))
}

fn shortcut_is_ready(state: ElementState, repeat: bool) -> bool {
    state == ElementState::Pressed && !repeat
}

fn copy_is_ready(selecting: bool, state: ElementState, repeat: bool) -> bool {
    !selecting && shortcut_is_ready(state, repeat)
}

fn selection_presentation_is_ready(message: &ClientMessage, current: bool) -> bool {
    current || matches!(message, ClientMessage::Selection(SelectionAction::Copy))
}

fn dismisses_clipboard_notice(source: LocalNoticeSource, message: &ClientMessage) -> bool {
    source == LocalNoticeSource::Input
        && !matches!(
            message,
            ClientMessage::Key(session::KeyEvent {
                action: session::KeyAction::Release,
                ..
            })
        )
}

fn status_notice<'a>(model: Option<&'a str>, link: Option<&'a str>) -> Option<&'a str> {
    model.or(link)
}

fn move_terminal_pointer(
    input: &mut InputState,
    position: PhysicalPosition<f64>,
    workspace: Option<&WorkspaceScene>,
) -> Option<ClientMessage> {
    let (x, y) = workspace.map_or((position.x, position.y), |workspace| {
        (
            position.x - f64::from(workspace.terminal.left),
            position.y - f64::from(workspace.terminal.top),
        )
    });
    input.move_pointer(x, y)
}

fn button_reaches_terminal(
    workspace: bool,
    terminal_hit: bool,
    state: ElementState,
    presentation_current: bool,
) -> bool {
    state == ElementState::Released || presentation_current && (!workspace || terminal_hit)
}

fn scrollback_button(
    pressed: &mut Option<PresentationIdentity>,
    state: ElementState,
    button: MouseButton,
    target: Option<PresentationIdentity>,
) -> Option<bool> {
    if button != MouseButton::Left {
        return None;
    }
    if state == ElementState::Released {
        return pressed.take().map(|press| Some(press) == target);
    }
    if state == ElementState::Pressed {
        *pressed = target;
        return target.map(|_| false);
    }
    None
}

fn ime_allowed(
    window_focused: bool,
    workspace_focus: WorkspaceFocus,
    orbit_attached: bool,
) -> bool {
    orbit_attached && terminal_focused(window_focused, workspace_focus)
}

fn ime_reaches_terminal(allowed: bool, event: &Ime) -> bool {
    allowed || matches!(event, Ime::Disabled)
}

fn native_ime_message(
    input: &mut InputState,
    model: &mut SessionModel,
    event: Ime,
) -> Option<ClientMessage> {
    match input.ime(event) {
        Ok(message) => message,
        Err(detail) => {
            model.set_venus_notice(
                LocalNoticeSource::Input,
                format!("Venus could not encode input: {detail}"),
            );
            None
        }
    }
}

/// What the agent sidebar's rows need: the time their ages count to, and the focused row.
fn sidebar_view(focus: WorkspaceFocus, row: Option<&str>) -> SidebarView<'_> {
    SidebarView {
        now: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            }),
        focused: row.filter(|_| focus == WorkspaceFocus::Sidebar),
    }
}

/// F6's next region: Terminal, Sidebar while it has rows, Tabs, then Panes while the active tab
/// shows any.
fn cycle_focus(focus: WorkspaceFocus, sidebar_rows: bool, has_panes: bool) -> WorkspaceFocus {
    match focus {
        WorkspaceFocus::Terminal if sidebar_rows => WorkspaceFocus::Sidebar,
        WorkspaceFocus::Terminal | WorkspaceFocus::Sidebar => WorkspaceFocus::Tabs,
        WorkspaceFocus::Tabs if has_panes => WorkspaceFocus::Panes,
        _ => WorkspaceFocus::Terminal,
    }
}

/// The sidebar row one below or above `current`; none past either end or without a current row.
fn step_sidebar_row<'a>(
    rows: &'a [yazelix_venus::SidebarRow],
    current: Option<&str>,
    down: bool,
) -> Option<&'a str> {
    let index = rows
        .iter()
        .position(|row| Some(row.target.as_str()) == current)?;
    let next = if down {
        index + 1
    } else {
        index.checked_sub(1)?
    };
    rows.get(next).map(|row| row.target.as_str())
}

/// Where a row jump moves focus once `snapshot` is current: the terminal when it focuses the
/// jump's exact target; after the jump's own accepted `answer`, the tab strip when only the
/// target's tab is selected; otherwise nowhere yet.
fn jump_focus(
    jump: &AttentionJump,
    snapshot: &eon_workspace_protocol::v26::Snapshot,
    answer: bool,
) -> Option<WorkspaceFocus> {
    if focused_terminal(snapshot) == Some(jump.target.as_str()) {
        Some(WorkspaceFocus::Terminal)
    } else if answer && snapshot.active_tab == jump.tab {
        // The covered-target rule only selected the tab.
        Some(WorkspaceFocus::Tabs)
    } else {
        None
    }
}

fn terminal_focused(window_focused: bool, workspace_focus: WorkspaceFocus) -> bool {
    window_focused && workspace_focus == WorkspaceFocus::Terminal
}

fn accessibility_workspace_focus(
    target: AccessibilityTarget,
    queue: impl FnOnce(CommonAction) -> bool,
) -> Option<WorkspaceFocus> {
    let (focus, id) = match target {
        AccessibilityTarget::Terminal => return Some(WorkspaceFocus::Terminal),
        AccessibilityTarget::Tab(id) => (WorkspaceFocus::Tabs, id),
        AccessibilityTarget::Pane(id) => (WorkspaceFocus::Panes, id),
        AccessibilityTarget::HeaderControl { .. }
        | AccessibilityTarget::InboxFilter(_)
        | AccessibilityTarget::InboxRow { .. }
        | AccessibilityTarget::SidebarRow { .. }
        | AccessibilityTarget::Link { .. }
        | AccessibilityTarget::ReturnToLive => {
            return None;
        }
    };
    queue(CommonAction::FocusId(id)).then_some(focus)
}

/// The tab a final close names: the shown workspace's only tab, closed whole or through its
/// last pane. That close ends the window, or with other workspaces removes just this one
/// (EON-C33). Runtime refuses a pane close under a popup, so that close is not asked about.
fn final_close_tab(
    snapshot: &eon_workspace_protocol::v26::Snapshot,
    action: &WorkspaceAction,
) -> Option<String> {
    let [tab] = snapshot.tabs.as_slice() else {
        return None;
    };
    let last = match action {
        WorkspaceAction::Workspace(CommonAction::CloseTab { tab: id }) => *id == tab.id,
        WorkspaceAction::ClosePane { pane } => {
            let mut panes = tab.panes();
            tab.selected_popup.is_none()
                && panes.next().is_some_and(|only| only.id == *pane)
                && panes.next().is_none()
        }
        _ => false,
    };
    last.then(|| tab.id.clone())
}

/// The shown workspace's name or `wN`, which a final close removes while other workspaces
/// remain; `None` while that close ends the window.
fn close_prompt_workspace(snapshot: &eon_workspace_protocol::v26::Snapshot) -> Option<String> {
    shown_workspace(snapshot)
        .filter(|&(_, _, count)| count >= 2)
        .map(|(name, _, _)| name.to_owned())
}

/// The close prompt's answer to a key: Enter quits, Escape cancels.
fn close_prompt_answer(code: KeyCode, modifiers: session::Modifiers) -> Option<bool> {
    match code {
        _ if modifiers != session::Modifiers::empty() => None,
        KeyCode::Enter => Some(true),
        KeyCode::Escape => Some(false),
        _ => None,
    }
}

fn attention_inbox_command(
    open: bool,
    code: KeyCode,
    modifiers: session::Modifiers,
    repeat: bool,
    entries: &[PopupEntry],
) -> Option<AttentionInboxCommand> {
    if !repeat
        && native_key_shortcut(PhysicalKey::Code(code), modifiers, entries)
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ToggleInbox)
    {
        return Some(AttentionInboxCommand::Toggle);
    }
    if !open || modifiers != session::Modifiers::empty() {
        return None;
    }
    Some(match code {
        KeyCode::Escape if !repeat => AttentionInboxCommand::Close,
        KeyCode::ArrowLeft => AttentionInboxCommand::Filter(true),
        KeyCode::ArrowRight => AttentionInboxCommand::Filter(false),
        KeyCode::ArrowUp => AttentionInboxCommand::Move(true),
        KeyCode::ArrowDown => AttentionInboxCommand::Move(false),
        KeyCode::Enter | KeyCode::Space if !repeat => AttentionInboxCommand::Activate,
        _ => AttentionInboxCommand::Scroll(shortcut_viewer_command(true, code, modifiers, repeat)?),
    })
}

fn shortcut_viewer_command(
    open: bool,
    code: KeyCode,
    modifiers: session::Modifiers,
    repeat: bool,
) -> Option<ShortcutViewerCommand> {
    // Eon reserves the viewer toggle, so no popup takes it.
    if !repeat
        && native_key_shortcut(PhysicalKey::Code(code), modifiers, &[])
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::ToggleViewer)
    {
        return Some(ShortcutViewerCommand::Toggle);
    }
    if !open || modifiers != session::Modifiers::empty() {
        return None;
    }
    Some(match code {
        KeyCode::Escape if !repeat => ShortcutViewerCommand::Close,
        KeyCode::ArrowUp => ShortcutViewerCommand::ScrollLines(-1.0),
        KeyCode::ArrowDown => ShortcutViewerCommand::ScrollLines(1.0),
        KeyCode::PageUp => ShortcutViewerCommand::ScrollPages(-1.0),
        KeyCode::PageDown => ShortcutViewerCommand::ScrollPages(1.0),
        KeyCode::Home => ShortcutViewerCommand::Start,
        KeyCode::End => ShortcutViewerCommand::End,
        _ => return None,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum WorkspaceHeaderAction {
    Workspace(CommonAction),
    ToggleShortcuts,
    PickWorkspace,
}

fn workspace_header_action(
    control: WorkspaceHeaderControl,
    active_tab: &str,
) -> WorkspaceHeaderAction {
    match control {
        WorkspaceHeaderControl::Workspaces => WorkspaceHeaderAction::PickWorkspace,
        WorkspaceHeaderControl::NewTab => WorkspaceHeaderAction::Workspace(CommonAction::CreateTab),
        WorkspaceHeaderControl::Shortcuts => WorkspaceHeaderAction::ToggleShortcuts,
        WorkspaceHeaderControl::CloseTab => {
            WorkspaceHeaderAction::Workspace(CommonAction::CloseTab {
                tab: active_tab.into(),
            })
        }
    }
}

/// Tab and Shift+Tab move from the tabs through the header controls: the workspace label while
/// it opens the picker, New tab, Shortcuts while it shows, then Close tab.
fn cycle_header_focus(
    focus: WorkspaceFocus,
    backwards: bool,
    workspace_label: bool,
    shortcuts_visible: bool,
) -> Option<WorkspaceFocus> {
    use WorkspaceHeaderControl::{CloseTab, NewTab, Shortcuts, Workspaces};

    Some(match (focus, backwards) {
        (WorkspaceFocus::Tabs, false) if workspace_label => WorkspaceFocus::Header(Workspaces),
        (WorkspaceFocus::Tabs | WorkspaceFocus::Header(Workspaces), false) => {
            WorkspaceFocus::Header(NewTab)
        }
        (WorkspaceFocus::Header(NewTab), false) => WorkspaceFocus::Header(if shortcuts_visible {
            Shortcuts
        } else {
            CloseTab
        }),
        (WorkspaceFocus::Header(Shortcuts), false) => WorkspaceFocus::Header(CloseTab),
        (WorkspaceFocus::Header(CloseTab), false) => WorkspaceFocus::Tabs,
        (WorkspaceFocus::Tabs, true) => WorkspaceFocus::Header(CloseTab),
        (WorkspaceFocus::Header(CloseTab), true) => {
            WorkspaceFocus::Header(if shortcuts_visible { Shortcuts } else { NewTab })
        }
        (WorkspaceFocus::Header(Shortcuts), true) => WorkspaceFocus::Header(NewTab),
        (WorkspaceFocus::Header(NewTab), true) if workspace_label => {
            WorkspaceFocus::Header(Workspaces)
        }
        (WorkspaceFocus::Header(NewTab | Workspaces), true) => WorkspaceFocus::Tabs,
        _ => return None,
    })
}

fn sends_workspace_shortcut(action: &WorkspaceAction, state: ElementState, repeat: bool) -> bool {
    state == ElementState::Pressed
        && (!repeat || matches!(action, WorkspaceAction::Workspace(CommonAction::Focus(_))))
}

fn clipboard_notice<E: std::fmt::Display>(
    clipboard: NativeClipboard,
    result: std::result::Result<(), E>,
) -> String {
    result.map_or_else(
        |error| format!("Venus could not write the native clipboard: {error}"),
        |()| match clipboard {
            NativeClipboard::Standard => "Text copied to the native clipboard.".into(),
            NativeClipboard::Primary => "Text copied to the native primary selection.".into(),
            NativeClipboard::Both => {
                "Text copied to the native clipboard and primary selection.".into()
            }
        },
    )
}

fn take_deferred_selection(
    pending: &mut VecDeque<ClientMessage>,
    frame_revision: u64,
) -> Vec<ClientMessage> {
    let mut batch = Vec::new();
    while let Some(mut message) = pending.pop_front() {
        if let ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: revision,
            ..
        }) = &mut message
        {
            if !batch.is_empty() {
                pending.push_front(message);
                break;
            }
            *revision = frame_revision;
        }
        let finished = matches!(
            message,
            ClientMessage::Selection(SelectionAction::Finish { .. } | SelectionAction::Copy)
        );
        batch.push(message);
        if finished {
            break;
        }
    }
    batch
}

fn queue_deferred_selection(pending: &mut VecDeque<ClientMessage>, message: ClientMessage) -> bool {
    if pending.len() >= MAX_DEFERRED_SELECTION_MESSAGES {
        return false;
    }
    pending.push_back(message);
    true
}

fn pointer_sequence_needs_cancel(
    gate: &SelectionGate,
    selecting: bool,
    pending: &VecDeque<ClientMessage>,
) -> bool {
    if matches!(gate, SelectionGate::AwaitingFinish) {
        return true;
    }
    for message in pending {
        match message {
            ClientMessage::Selection(SelectionAction::Begin { .. }) => return false,
            ClientMessage::Selection(
                SelectionAction::Update { .. } | SelectionAction::Finish { .. },
            ) => return true,
            _ => {}
        }
    }
    selecting
}

fn clipboard_paste_message(input: impl Read) -> std::result::Result<ClientMessage, String> {
    let mut text = Vec::new();
    input
        .take(session::MAX_PASTE_BYTES as u64 + 1)
        .read_to_end(&mut text)
        .map_err(|error| error.to_string())?;
    if text.is_empty() {
        return Err("The native clipboard contains no text".into());
    }
    if text.len() > session::MAX_PASTE_BYTES {
        return Err(format!(
            "Native clipboard text exceeds the {} byte paste limit",
            session::MAX_PASTE_BYTES
        ));
    }
    std::str::from_utf8(&text).map_err(|_| "stream did not contain valid UTF-8".to_string())?;
    Ok(ClientMessage::Paste(text))
}

fn clipboard_paste_notice(error: impl std::fmt::Display) -> String {
    format!("Venus could not paste from the native clipboard: {error}")
}

#[cfg(target_os = "linux")]
fn write_native_clipboard(
    clipboard: NativeClipboard,
    text: String,
) -> std::result::Result<(), wl_clipboard_rs::copy::Error> {
    use wl_clipboard_rs::copy::{ClipboardType, MimeType, Options, Source};

    let mut options = Options::new();
    options.clipboard(match clipboard {
        NativeClipboard::Standard => ClipboardType::Regular,
        NativeClipboard::Primary => ClipboardType::Primary,
        NativeClipboard::Both => ClipboardType::Both,
    });
    options.copy(Source::Bytes(text.into_bytes().into()), MimeType::Text)
}

#[cfg(target_os = "macos")]
fn write_native_clipboard(
    _: NativeClipboard,
    text: String,
) -> std::result::Result<(), arboard::Error> {
    arboard::Clipboard::new()?.set_text(text)
}

#[cfg(target_os = "linux")]
fn read_native_clipboard() -> std::result::Result<impl Read, String> {
    use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

    let (pipe, _) = paste::get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text)
        .map_err(|error| error.to_string())?;
    Ok(pipe)
}

#[cfg(target_os = "macos")]
fn read_native_clipboard() -> std::result::Result<io::Cursor<String>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .get_text()
        .map(io::Cursor::new)
        .map_err(|error| error.to_string())
}

fn native_clipboard(effect: &ClipboardEffect) -> NativeClipboard {
    native_clipboard_for(NATIVE_PLATFORM, effect)
}

fn native_clipboard_for(platform: NativePlatform, effect: &ClipboardEffect) -> NativeClipboard {
    if platform == NativePlatform::Macos {
        return NativeClipboard::Standard;
    }
    match effect {
        ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Selection,
            ..
        } => NativeClipboard::Both,
        ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Standard,
            ..
        }
        | ClipboardEffect::TerminalWrite {
            location: ClipboardLocation::Standard,
            ..
        } => NativeClipboard::Standard,
        ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Primary,
            ..
        }
        | ClipboardEffect::TerminalWrite {
            location: ClipboardLocation::Selection | ClipboardLocation::Primary,
            ..
        } => NativeClipboard::Primary,
    }
}

pub(super) fn run(arguments: LaunchArguments) -> Result {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|_| io::Error::other("Venus requires a native display"))?;
    let mut application = Application::new(arguments, event_loop.create_proxy());
    if application.startup_admission && application.workspace_socket.is_some() {
        let response = yazelix_venus::read_workspace_response(&mut io::stdin().lock())?;
        if !matches!(response, eon_workspace_protocol::v26::Response::Snapshot(_)) {
            return Err("Venus startup admission requires an Eon workspace snapshot".into());
        }
        application.workspace_model.apply(response);
    }
    if application.supervised {
        start_presentation_control(event_loop.create_proxy())?;
    }
    event_loop.run_app(&mut application)?;
    if let Some(error) = application.fatal_error {
        return Err(error.into());
    }
    Ok(())
}

fn start_presentation_control(proxy: EventLoopProxy<UserEvent>) -> Result {
    thread::Builder::new()
        .name("venus-presentation-control".into())
        .spawn(move || {
            run_presentation_control(io::stdin().lock(), |event| proxy.send_event(event).is_ok());
        })?;
    Ok(())
}

fn run_presentation_control(mut input: impl Read, mut send: impl FnMut(UserEvent) -> bool) {
    let mut message = [0; 8];
    while input.read_exact(&mut message).is_ok() {
        if message == *b"present\n" && !send(UserEvent::Present) {
            return;
        }
    }
    let _ = send(UserEvent::Exit);
}

#[cfg(test)]
mod tests;
