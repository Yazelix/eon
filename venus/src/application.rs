use crate::{Result, launch::LaunchArguments};
use accesskit_winit::{Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent};
use eon_workspace_protocol::v19::{
    Action as WorkspaceAction, Direction as WorkspaceDirection, PopupEntry, Sidebar,
    WorkspaceAction as CommonAction,
};
use orbit_protocol::session::{
    self, ClientMessage, ClipboardLocation, FailureCode, ScrollOutcome, SelectionAction,
    ServerMessage, SurfaceSize, WheelOutcome,
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
    Accessibility, AccessibilityTarget, BarItem, CellMetrics, ClipboardEffect, Color,
    ConnectionState, CpuTimes, CpuUse, DialogKind, DialogScene, EonBar, FontSettings, FontSetup,
    Hyperlink, InputState, LocalClock, LocalNoticeSource, MAX_LINK_BYTES, MetadataEvent,
    MetadataTransport, ModelError, PaneMetadata, PresentOutcome, RamUse, Renderer, Scene,
    SceneRect, SessionModel, ShortcutHintsScene, SidebarView, Transport, TransportEvent,
    WorkspaceEvent, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceHit, WorkspaceModel,
    WorkspaceScene, WorkspaceTransport, active_popup, attention_only_change, focused_terminal,
    validate_copy_uri, validate_open_uri,
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
    NativeShortcutAction, NativeShortcutTrigger, copy_paste_modifiers, detach_chord,
    native_key_shortcut, native_key_shortcut_for, native_pointer_shortcut, paste_shortcut,
    popup_shortcut, shortcut_groups, shortcut_hints_for, sideways_move_opens_column,
    tab_shortcut_index, workspace_shortcut,
};

const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// A working badge's spinner turns at most 5 times a second (VEN-C21, AP-PERF-001).
const SPINNER_INTERVAL: Duration = Duration::from_millis(200);
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
    Metadata,
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

struct MetadataObserver {
    transport: MetadataTransport,
    metadata: PaneMetadata,
}

#[derive(Debug, Default)]
struct ShortcutViewerState {
    open: bool,
    scroll: f32,
}

/// A `FocusAttention` sent from a sidebar row. Keyboard focus stays on the row until a snapshot
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
    fn names_live_target(&self, snapshot: &eon_workspace_protocol::v19::Snapshot) -> bool {
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
        }
    }
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
    eon_version: Option<String>,
    bar_items: Vec<BarItem>,
    background_opacity: f32,
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
    metadata_observers: HashMap<Vec<u8>, MetadataObserver>,
    workspace_model: WorkspaceModel,
    input: InputState,
    input_epoch: Instant,
    link_opener: Option<LinkOpener>,
    render_notice: Option<String>,
    blink_visible: bool,
    next_blink: Option<Instant>,
    next_spinner: Option<Instant>,
    next_animation: Option<Instant>,
    clock: Option<LocalClock>,
    next_clock: Option<Instant>,
    /// The previous `/proc/stat` reading and when it was taken.
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
    warning_close_last_tab: bool,
    close_prompt: Option<ClosePrompt>,
    hints_visible: bool,
    hints: Option<ShortcutHintsScene>,
    zoom: Option<ZoomTarget>,
    tab_texts: HashMap<String, (String, f32)>,
    quota_widths: HashMap<String, f32>,
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
            eon_version,
            bar_items,
            background_opacity,
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
            eon_version,
            bar_items,
            background_opacity,
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
            metadata_observers: HashMap::new(),
            workspace_model: WorkspaceModel::default(),
            input: InputState::default(),
            input_epoch: Instant::now(),
            link_opener: None,
            render_notice: None,
            blink_visible: true,
            next_blink: None,
            next_spinner: None,
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
            warning_close_last_tab,
            close_prompt: None,
            hints_visible: true,
            hints: None,
            zoom: None,
            tab_texts: HashMap::new(),
            quota_widths: HashMap::new(),
            tab_scroll: 0.0,
            pane_scroll: 0.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let fonts = FontSetup::new(&self.fonts)?;
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
            self.cursor_tail,
            self.pane_frames,
            self.focus_color,
            self.border_width,
            self.focus_border_width,
            fonts,
        ))?;
        window.set_visible(true);
        let scale_factor = window.scale_factor();

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
            WorkspaceScene::from_snapshot_with_zoom(
                snapshot,
                EonBar {
                    items: &self.bar_items,
                    version: self.eon_version.as_deref(),
                    cpu: self.cpu.as_ref(),
                    ram: self.ram.as_ref(),
                    clock: self.clock.as_ref(),
                },
                state.renderer.size(),
                state.renderer.metrics(),
                self.tab_scroll,
                self.pane_scroll,
                |endpoint| {
                    self.metadata_observers
                        .get(endpoint)
                        .map(|observer| &observer.metadata)
                },
                |id, label| {
                    id.map_or_else(
                        || (label.to_owned(), self.quota_widths[label]),
                        |id| self.tab_texts[id].clone(),
                    )
                },
                self.hints.clone(),
                self.zoomed(),
                sidebar_view(self.workspace_focus, self.sidebar_row.as_deref()),
            )
        })
    }

    /// Whether the zoom still names the active tab, its column count and its selected column.
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
        self.shortcut_viewer.open || self.close_prompt.is_some()
    }

    /// The open dialog: the close prompt, else the shortcut viewer.
    fn dialog_scene(&self) -> Option<DialogScene> {
        let Some(prompt) = &self.close_prompt else {
            return self.shortcut_viewer_scene();
        };
        let state = self.window.as_ref()?;
        let snapshot = self.workspace_model.snapshot()?;
        let tab = snapshot.tabs.iter().find(|tab| tab.id == prompt.tab)?;
        Some(DialogScene::close_last_tab(
            tab,
            &snapshot.entries,
            |endpoint| {
                self.metadata_observers
                    .get(endpoint)
                    .map(|observer| &observer.metadata)
            },
            detach_chord(&snapshot.entries),
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

    /// Opens or closes a dialog, reporting the terminal's focus change with it.
    fn change_dialog(&mut self, change: impl FnOnce(&mut Self)) {
        let was_terminal_focused =
            terminal_focused(self.window_focused, self.workspace_focus) && !self.dialog_open();
        self.cancel_terminal_scroll();
        self.cancel_pointer_sequence();
        change(self);
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
                *self.dialog_scroll() = 0.0;
                self.refresh_client_view();
            }
            ShortcutViewerCommand::End => {
                if let Some(dialog) = self.dialog_scene() {
                    *self.dialog_scroll() = dialog.max_scroll;
                    self.refresh_client_view();
                }
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
        self.hints = self.hints_visible.then(|| {
            ShortcutHintsScene::new(
                shortcut_hints_for(
                    NATIVE_PLATFORM,
                    self.workspace_focus,
                    &snapshot.entries,
                    snapshot.sidebar,
                ),
                state.renderer.size(),
                state.renderer.metrics(),
                self.workspace_focus,
                |text| state.renderer.measure_header_text(text).1,
            )
        });
        self.tab_texts.clear();
        self.quota_widths.clear();
        Some(WorkspaceScene::from_snapshot_with_zoom(
            snapshot,
            EonBar {
                items: &self.bar_items,
                version: self.eon_version.as_deref(),
                cpu: self.cpu.as_ref(),
                ram: self.ram.as_ref(),
                clock: self.clock.as_ref(),
            },
            state.renderer.size(),
            state.renderer.metrics(),
            tab_scroll,
            pane_scroll,
            |_| None,
            |id, label| {
                let fitted = if id.is_some() {
                    state.renderer.fit_tab_text(label)
                } else {
                    state.renderer.measure_header_text(label)
                };
                if let Some(id) = id {
                    self.tab_texts.insert(id.to_owned(), fitted.clone());
                } else {
                    self.quota_widths.insert(label.to_owned(), fitted.1);
                }
                fitted
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
        let Some(tab) = self
            .workspace_scene()
            .and_then(|scene| scene.sidebar_row(target).map(|row| row.tab.clone()))
        else {
            return;
        };
        if !self.presentation.has_presented_geometry() {
            return;
        }
        if let Some(ticket) = self.queue_workspace_ticket(WorkspaceAction::FocusAttention {
            tab: tab.clone(),
            target: target.to_owned(),
        }) {
            self.attention_jump = Some(AttentionJump {
                tab,
                target: target.to_owned(),
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
    /// width or loses the focused row.
    fn retain_sidebar_focus(&mut self) {
        if self.workspace_focus == WorkspaceFocus::Sidebar
            && !self.sidebar_row.as_deref().is_some_and(|row| {
                self.workspace_scene()
                    .is_some_and(|scene| scene.sidebar_row(row).is_some())
            })
        {
            self.set_workspace_focus(WorkspaceFocus::Terminal);
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
        self.refresh_client_view();
    }

    /// Zoom the active tab's selected column across the tab, or restore its columns. Other
    /// columns keep their attachments at their column size while hidden.
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
                .reject_action("Nothing to zoom: this tab has one column.")
            {
                self.refresh_client_view();
            }
            return;
        };
        self.zoom = (!self.zoomed()).then_some(target);
        self.cancel_terminal_scroll();
        self.cancel_pointer_sequence();
        self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll);
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
    }

    fn handle_column_transport(&mut self, index: usize, event: TransportEvent) {
        let retryable_busy = managed_busy_is_retryable(self.supervised, &event);
        let retryable_event = retryable_busy || matches!(&event, TransportEvent::RetryableLoss(_));
        let view = &mut self.column_views[index];
        let was_attached = view.model.is_attached();
        let mut schedule_retry = false;
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
                    Ok(Some(effect)) => write_clipboard_effect(&mut view.model, effect),
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
            self.reconcile_metadata_observers();
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
        let (received_snapshot, snapshot_changed, attention_only, sidebar_toggled) = match &event {
            WorkspaceEvent::Response(eon_workspace_protocol::v19::Response::Snapshot(snapshot)) => {
                let current = self.workspace_model.snapshot();
                (
                    true,
                    current != Some(snapshot),
                    current.is_some_and(|current| attention_only_change(current, snapshot)),
                    current.is_some_and(|current| current.sidebar != snapshot.sidebar),
                )
            }
            _ => (false, false, false, false),
        };
        let unavailable = matches!(&event, WorkspaceEvent::Unavailable(_));
        let quota_expired = unavailable
            && self.workspace_model.snapshot().is_some_and(|snapshot| {
                snapshot.codex_quota.is_some() || snapshot.claude_quota.is_some()
            });
        let (view_changed, workspace_changed) = match event {
            WorkspaceEvent::Response(response) => self.workspace_model.apply(response),
            WorkspaceEvent::Unavailable(detail) => {
                (self.workspace_model.mark_unavailable(detail, false), false)
            }
            WorkspaceEvent::Answered { .. } => unreachable!("answers return above"),
        };
        // Any change of the active tab, its column count or its selected column ends zoom.
        if self.zoom.is_some() && !self.zoomed() {
            self.zoom = None;
        }
        // The prompt closes with its target or the workspace connection.
        if self.close_prompt.as_ref().is_some_and(|prompt| {
            unavailable
                || !self
                    .workspace_model
                    .snapshot()
                    .is_some_and(|snapshot| prompt.names_live_target(snapshot))
        }) {
            self.change_dialog(|app| app.close_prompt = None);
        }
        if self.window.is_none() || self.initial_scale_pending {
            if snapshot_changed {
                self.reveal_workspace_selection();
            }
            return;
        }
        if unavailable {
            let metadata_changed = !self.metadata_observers.is_empty();
            self.metadata_observers.clear();
            if quota_expired || metadata_changed {
                self.presentation.invalidate();
            }
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
        } else if snapshot_changed {
            let Some(scene) =
                self.rebuild_workspace_measurements(self.tab_scroll, self.pane_scroll)
            else {
                return;
            };
            self.tab_scroll = scene.tab_scroll();
            self.pane_scroll = scene.pane_scroll();
            // Attention moves no presented geometry outside a visible sidebar, so a held link
            // press or a shortcut waiting on the presented scene survives it; the sidebar's rows
            // are hit targets, so a refresh that moves them retires presses on them.
            if attention_only && scene.sidebar().is_none() {
                self.presentation.content_changed();
            } else {
                self.presentation.invalidate();
                // Showing or hiding the sidebar resizes the columns; other refreshes leave
                // every size as it was, so nothing is sent.
                self.send_resize();
            }
        }
        if received_snapshot {
            self.settle_attention_jump(None);
        } else if unavailable {
            // Workspace loss ends a pending jump with focus on its row, beside the notice.
            self.attention_jump = None;
        }
        self.retain_sidebar_focus();
        if self.workspace_model.snapshot().is_some()
            && self.workspace_model.active_attachment().is_none()
        {
            self.set_workspace_focus(WorkspaceFocus::Tabs);
        }
        if received_snapshot {
            self.reconcile_metadata_observers();
        }
        if view_changed {
            self.refresh_client_view();
        }
    }

    fn reconcile_metadata_observers(&mut self) {
        let endpoints = self
            .workspace_model
            .snapshot()
            .map_or_else(HashSet::new, |snapshot| {
                let unattached = std::iter::once(&self.terminal)
                    .chain(&self.column_views)
                    .filter(|view| !view.model.is_attached())
                    .filter_map(|view| view.active_endpoint.as_deref())
                    .collect::<Vec<_>>();
                visible_metadata_endpoints(snapshot, &unattached)
            });
        let before = self.metadata_observers.len();
        self.metadata_observers
            .retain(|endpoint, _| endpoints.contains(endpoint));
        let retained = self.metadata_observers.len();
        for endpoint in endpoints {
            let socket = PathBuf::from(OsString::from_vec(endpoint.clone()));
            let proxy = self.proxy.clone();
            self.metadata_observers
                .entry(endpoint)
                .or_insert_with(|| MetadataObserver {
                    transport: MetadataTransport::start(socket, move || {
                        let _ = proxy.send_event(UserEvent::Metadata);
                    }),
                    metadata: PaneMetadata::Connecting,
                });
        }
        if before != retained || retained != self.metadata_observers.len() {
            self.presentation.invalidate();
        }
    }

    fn handle_metadata(&mut self) {
        let mut changed = false;
        for observer in self.metadata_observers.values_mut() {
            let Some(event) = observer.transport.drain_event() else {
                continue;
            };
            let metadata = match event {
                MetadataEvent::Metadata(metadata) => PaneMetadata::Available {
                    working_directory: metadata.working_directory,
                },
                MetadataEvent::Unavailable => PaneMetadata::Unavailable,
            };
            if observer.metadata != metadata {
                observer.metadata = metadata;
                changed = true;
            }
        }
        if changed {
            self.presentation.invalidate();
            self.refresh_client_view();
        }
    }

    fn send_workspace(&mut self, action: CommonAction) -> bool {
        self.presentation.has_presented_geometry()
            && self.queue_workspace(WorkspaceAction::Workspace(action))
    }

    fn activate_workspace_header(&mut self, control: WorkspaceHeaderControl) {
        let Some(active_tab) = self
            .workspace_model
            .snapshot()
            .map(|snapshot| snapshot.active_tab.clone())
        else {
            return;
        };
        match workspace_header_action(control, &active_tab) {
            WorkspaceHeaderAction::Workspace(action) => {
                self.request_workspace(WorkspaceAction::Workspace(action));
            }
            WorkspaceHeaderAction::ToggleShortcuts => {
                self.set_shortcut_viewer(!self.shortcut_viewer.open);
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
        let tab_index = tab_shortcut_index(code, modifiers);
        let action = match tab_index {
            Some(index) => snapshot
                .tabs
                .get(index)
                .map(|tab| WorkspaceAction::Workspace(CommonAction::FocusId(tab.id.clone()))),
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
                || tab_index.is_some()
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
                && let Some(next) = cycle_header_focus(
                    focus,
                    modifiers == session::Modifiers::SHIFT,
                    self.workspace_scene()
                        .is_some_and(|scene| scene.has_control(WorkspaceHeaderControl::Shortcuts)),
                )
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
                    ServerMessage::VerticalPreview(preview) => {
                        Some((preview.frame_revision, preview.direction))
                    }
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
                    // ORBS v13 rejects terminal-routed synthetic ticks as invalid input.
                    self.terminal.model.clear_viewport_preview();
                    Ok(None)
                } else {
                    self.terminal.model.apply(message)
                };
                let accepted_frame = frame && result.is_ok();
                let accepted = result.is_ok();
                match result {
                    Ok(Some(effect)) => self.write_clipboard(effect),
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
                    } else if let Some((revision, direction)) = preview_response {
                        self.terminal
                            .terminal_scroll
                            .preview_arrived(revision, direction);
                    }
                    if terminal_owned_batch {
                        self.cancel_terminal_scroll();
                    } else if plain_frame {
                        self.terminal.terminal_scroll.rebase();
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
                    self.reconcile_metadata_observers();
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
        self.next_age = workspace
            .as_ref()
            .and_then(WorkspaceScene::sidebar_age_wait)
            .map(|wait| now + Duration::from_millis(wait));
        let dialog = self.dialog_scene();
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
                self.presentation.invalidate();
                self.send_resize();
                // A sidebar hidden for width gives its focus back to the terminal.
                self.retain_sidebar_focus();
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
            UserEvent::Metadata => self.handle_metadata(),
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
                        WorkspaceEvent::Response(eon_workspace_protocol::v19::Response::Ended(_))
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
                self.terminal.selection_scroll.next,
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

/// The active tab, its column count and its selected column: what a zoom names. A tab with
/// fewer than two columns has nothing to zoom.
type ZoomTarget = (String, usize, usize);

fn zoom_target(snapshot: &eon_workspace_protocol::v19::Snapshot) -> Option<ZoomTarget> {
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)?;
    let selected = tab.selected_pane.as_ref()?;
    let column = tab
        .columns
        .iter()
        .position(|column| column.expanded == *selected)?;
    (tab.columns.len() > 1).then(|| (tab.id.clone(), tab.columns.len(), column))
}

/// Live panes of the active tab, except endpoints whose presentation attachment is not
/// established; none while a popup covers the tab.
fn visible_metadata_endpoints(
    snapshot: &eon_workspace_protocol::v19::Snapshot,
    unattached: &[&[u8]],
) -> HashSet<Vec<u8>> {
    if active_popup(snapshot).is_some() {
        return HashSet::new();
    }
    snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)
        .into_iter()
        .flat_map(|tab| tab.panes())
        .filter(|pane| pane.live && !unattached.contains(&pane.endpoint.as_slice()))
        .map(|pane| pane.endpoint.clone())
        .collect()
}

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
    snapshot: &eon_workspace_protocol::v19::Snapshot,
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
        | AccessibilityTarget::SidebarRow { .. }
        | AccessibilityTarget::Link { .. }
        | AccessibilityTarget::ReturnToLive => {
            return None;
        }
    };
    queue(CommonAction::FocusId(id)).then_some(focus)
}

/// The tab a close would end the window with: the only tab, closed whole or through its
/// last pane. Runtime refuses a pane close under a popup, so that close is not asked about.
fn final_close_tab(
    snapshot: &eon_workspace_protocol::v19::Snapshot,
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

/// The close prompt's answer to a key: Enter quits, Escape cancels.
fn close_prompt_answer(code: KeyCode, modifiers: session::Modifiers) -> Option<bool> {
    match code {
        _ if modifiers != session::Modifiers::empty() => None,
        KeyCode::Enter => Some(true),
        KeyCode::Escape => Some(false),
        _ => None,
    }
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
}

fn workspace_header_action(
    control: WorkspaceHeaderControl,
    active_tab: &str,
) -> WorkspaceHeaderAction {
    match control {
        WorkspaceHeaderControl::NewTab => WorkspaceHeaderAction::Workspace(CommonAction::CreateTab),
        WorkspaceHeaderControl::Shortcuts => WorkspaceHeaderAction::ToggleShortcuts,
        WorkspaceHeaderControl::CloseTab => {
            WorkspaceHeaderAction::Workspace(CommonAction::CloseTab {
                tab: active_tab.into(),
            })
        }
    }
}

fn cycle_header_focus(
    focus: WorkspaceFocus,
    backwards: bool,
    shortcuts_visible: bool,
) -> Option<WorkspaceFocus> {
    use WorkspaceHeaderControl::{CloseTab, NewTab, Shortcuts};

    Some(match (focus, backwards) {
        (WorkspaceFocus::Tabs, false) => WorkspaceFocus::Header(NewTab),
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
        (WorkspaceFocus::Header(NewTab), true) => WorkspaceFocus::Tabs,
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
        if !matches!(response, eon_workspace_protocol::v19::Response::Snapshot(_)) {
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
mod tests {
    use super::shortcuts::{
        ALT_SHIFT, CTRL_ALT, CTRL_SHIFT, FIXED_SHORTCUTS, copy_paste_modifiers,
        native_key_shortcut_for, native_pointer_shortcut_for, shortcut_groups_for,
    };
    use super::*;
    use crate::launch;
    use eon_workspace_protocol::v19::{
        InvokeIntent, Pane, Popup, PopupEntry, Shortcut, Side, Snapshot, Tab,
    };

    pub(super) fn show_test_popup(snapshot: &mut Snapshot) {
        snapshot.entries = vec![PopupEntry {
            id: "project".into(),
            label: "Project".into(),
            shortcut: Shortcut {
                modifiers: eon_workspace_protocol::v19::ALT,
                key: "KeyZ".into(),
            },
        }];
        snapshot.tabs[0].popups = vec![Popup {
            id: "u1".into(),
            entry: "project".into(),
            session: "popup-session".into(),
            endpoint: b"/tmp/popup.sock".to_vec(),
            attention: None,
        }];
        snapshot.tabs[0].selected_popup = Some("u1".into());
    }

    #[test]
    fn close_prompt_asks_only_for_a_window_ending_close() {
        use orbit_protocol::session::Modifiers;

        let pane = |id: &str| Pane {
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: id.as_bytes().to_vec(),
            live: true,
            attention: None,
        };
        let tab = |id: &str, panes: &[&str]| Tab {
            id: id.into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: panes.first().map(|pane| (*pane).into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v19::Column {
                panes: panes.iter().map(|id| pane(id)).collect(),
                expanded: panes
                    .first()
                    .map_or_else(String::new, |pane| (*pane).into()),
            }],
            popups: vec![],
            attention_rollup: None,
        };
        let snapshot = |tabs| Snapshot {
            active_tab: "t1".into(),
            geometry: eon_workspace_protocol::v19::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![],
            tabs,
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };
        let close_tab =
            |id: &str| WorkspaceAction::Workspace(CommonAction::CloseTab { tab: id.into() });
        let close_pane = |id: &str| WorkspaceAction::ClosePane { pane: id.into() };

        let one = snapshot(vec![tab("t1", &["p1"])]);
        assert_eq!(final_close_tab(&one, &close_tab("t1")), Some("t1".into()));
        assert_eq!(final_close_tab(&one, &close_pane("p1")), Some("t1".into()));
        assert_eq!(final_close_tab(&one, &close_tab("t2")), None);
        assert_eq!(
            final_close_tab(
                &one,
                &WorkspaceAction::Workspace(CommonAction::Focus(WorkspaceDirection::Left))
            ),
            None
        );
        // Runtime refuses a pane close under a popup, so it is not asked about.
        let mut covered = one.clone();
        show_test_popup(&mut covered);
        assert_eq!(final_close_tab(&covered, &close_pane("p1")), None);
        assert_eq!(
            final_close_tab(&covered, &close_tab("t1")),
            Some("t1".into())
        );
        let stacked = snapshot(vec![tab("t1", &["p1", "p2"])]);
        assert_eq!(final_close_tab(&stacked, &close_pane("p1")), None);
        let two = snapshot(vec![tab("t1", &["p1"]), tab("t2", &["p2"])]);
        assert_eq!(final_close_tab(&two, &close_tab("t1")), None);
        assert_eq!(final_close_tab(&two, &close_pane("p1")), None);

        // The prompt keeps its close while the named tab and pane live, even when a tab appears.
        let prompt = |action| ClosePrompt {
            tab: "t1".into(),
            action,
            scroll: 0.0,
        };
        assert!(prompt(close_tab("t1")).names_live_target(&two));
        assert!(prompt(close_pane("p1")).names_live_target(&two));
        assert!(!prompt(close_pane("p1")).names_live_target(&snapshot(vec![tab("t1", &["p2"])])));
        assert!(!prompt(close_tab("t1")).names_live_target(&snapshot(vec![tab("t2", &["p2"])])));

        assert_eq!(
            close_prompt_answer(KeyCode::Enter, Modifiers::empty()),
            Some(true)
        );
        assert_eq!(
            close_prompt_answer(KeyCode::Escape, Modifiers::empty()),
            Some(false)
        );
        assert_eq!(close_prompt_answer(KeyCode::Enter, Modifiers::SHIFT), None);
        assert_eq!(close_prompt_answer(KeyCode::KeyY, Modifiers::empty()), None);

        // Alt+Shift+D detaches unless a configured popup takes the key.
        let detach = PhysicalKey::Code(KeyCode::KeyD);
        let files = [PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: Shortcut {
                modifiers: eon_workspace_protocol::v19::ALT | eon_workspace_protocol::v19::SHIFT,
                key: "KeyD".into(),
            },
        }];
        assert_eq!(
            native_key_shortcut_for(NATIVE_PLATFORM, detach, ALT_SHIFT, &[])
                .map(|shortcut| shortcut.action),
            Some(NativeShortcutAction::Detach)
        );
        assert!(native_key_shortcut_for(NATIVE_PLATFORM, detach, ALT_SHIFT, &files).is_none());
        assert!(detach_chord(&[]).is_some());
        assert_eq!(detach_chord(&files), None);
    }

    #[test]
    fn zoom_names_the_tab_column_count_and_selected_column() {
        let pane = |id: &str| Pane {
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: id.as_bytes().to_vec(),
            live: true,
            attention: None,
        };
        let column = |panes: &[&str]| eon_workspace_protocol::v19::Column {
            panes: panes.iter().map(|id| pane(id)).collect(),
            expanded: panes[0].into(),
        };
        let tab = |id: &str, selected: &str, columns| Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: id.into(),
            directory: b"/tmp/eon".to_vec(),
            selected_pane: Some(selected.into()),
            columns,
            attention_rollup: None,
        };
        let snapshot = |active: &str, tabs| Snapshot {
            active_tab: active.into(),
            tabs,
            geometry: eon_workspace_protocol::v19::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };
        let two = |selected| {
            snapshot(
                "t1",
                vec![
                    tab("t1", selected, vec![column(&["p1"]), column(&["p2"])]),
                    tab("t2", "p9", vec![column(&["p8"]), column(&["p9"])]),
                ],
            )
        };
        let zoom = zoom_target(&two("p2"));
        assert_eq!(zoom, Some(("t1".into(), 2, 1)));
        // A new pane in the zoomed column, or a popup over it, keeps the zoom.
        let mut grown = two("p2");
        grown.tabs[0].columns[1].panes.push(pane("p3"));
        assert_eq!(zoom_target(&grown), zoom);
        let mut covered = two("p2");
        covered.tabs[0].selected_popup = Some("u1".into());
        assert_eq!(zoom_target(&covered), zoom);
        // Another selected column, column count or active tab ends it.
        assert_ne!(zoom_target(&two("p1")), zoom);
        let mut wider = two("p2");
        wider.tabs[0].columns.push(column(&["p4"]));
        assert_ne!(zoom_target(&wider), zoom);
        let mut other_tab = two("p2");
        other_tab.active_tab = "t2".into();
        assert_ne!(zoom_target(&other_tab), zoom);
        // One column has nothing to zoom.
        assert_eq!(
            zoom_target(&snapshot(
                "t1",
                vec![tab("t1", "p1", vec![column(&["p1"])])]
            )),
            None
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display"]
    fn typography_without_grid_waits_for_and_validates_workspace_geometry() {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        struct Probe(Application);
        impl ApplicationHandler<UserEvent> for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.0.resumed(event_loop);
                assert!(self.0.window.is_none(), "wait for workspace geometry");
                self.0
                    .workspace_model
                    .apply(eon_workspace_protocol::v19::Response::Snapshot(Snapshot {
                        active_tab: "t1".into(),
                        geometry: eon_workspace_protocol::v19::PopupGeometry {
                            side_margin: 8.0,
                            vertical_margin: 4.0,
                        },
                        entries: Vec::new(),
                        tabs: vec![Tab {
                            pending: false,
                            selected_popup: None,
                            popups: Vec::new(),
                            id: "t1".into(),
                            directory: b"/tmp".to_vec(),
                            selected_pane: Some("p1".into()),
                            columns: vec![eon_workspace_protocol::v19::Column {
                                panes: vec![Pane {
                                    id: "p1".into(),
                                    session: "s1".into(),
                                    endpoint: b"/unused-orbit.sock".to_vec(),
                                    live: true,
                                    attention: None,
                                }],
                                expanded: "p1".into(),
                            }],
                            attention_rollup: None,
                        }],
                        codex_quota: None,
                        claude_quota: None,
                        sidebar: eon_workspace_protocol::v19::Sidebar::Open,
                    }));
                self.0.resumed(event_loop);
                assert!(self.0.window.is_none());
                assert!(self.0.terminal.transport.is_none());
                assert!(
                    self.0
                        .fatal_error
                        .as_deref()
                        .is_some_and(|error| { error.contains("initial terminal dimensions") })
                );
            }
            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let arguments = launch::launch_arguments(
            ["--font-size", "96", "--line-height", "3", "--workspace"]
                .into_iter()
                .map(OsString::from)
                .chain([std::env::temp_dir()
                    .join("venus-unavailable-typography-workspace.sock")
                    .into_os_string()]),
            None,
        )
        .unwrap();
        let mut probe = Probe(Application::new(arguments, event_loop.create_proxy()));
        event_loop.run_app(&mut probe).unwrap();
        assert!(probe.0.fatal_error.is_some());
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer; use fractional output scale"]
    fn native_initial_grid_survives_compositor_scale_admission() {
        native_startup(true);
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer; use fractional output scale"]
    fn native_font_only_startup_completes_without_input() {
        native_startup(false);
    }

    #[cfg(target_os = "linux")]
    fn native_startup(explicit_grid: bool) {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        struct Probe {
            app: Application,
            explicit_grid: bool,
            checked: bool,
        }
        impl ApplicationHandler<UserEvent> for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.app.resumed(event_loop);
            }
            fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
                assert!(
                    !matches!(event, UserEvent::Exit),
                    "native startup must complete without a test timer or user input"
                );
                self.app.user_event(event_loop, event);
            }
            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                id: WindowId,
                event: WindowEvent,
            ) {
                if self.app.initial_scale_pending {
                    assert!(
                        self.app.terminal.transport.is_none(),
                        "attach only after native size admission"
                    );
                }
                self.app.window_event(event_loop, id, event);
            }
            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                self.app.about_to_wait(event_loop);
                if self.app.initial_scale_pending {
                    return;
                }
                assert!(self.app.terminal.transport.is_some());
                let state = self
                    .app
                    .window
                    .as_ref()
                    .expect("the configured native window opens");
                let size = surface_size(state.renderer.size(), state.renderer.metrics()).unwrap();
                if self.explicit_grid {
                    assert_eq!((size.cols, size.rows), (100, 30));
                } else {
                    assert_eq!(
                        state.renderer.size(),
                        LogicalSize::new(960.0, 600.0).to_physical::<u32>(state.scale_factor)
                    );
                }
                self.checked = true;
                event_loop.exit();
            }
        }
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let arguments = launch::launch_arguments(
            [
                "--application-id",
                "venus-typography-proof",
                "--no-decorations",
                "--font-size",
                "20",
                "--line-height",
                "1.5",
            ]
            .into_iter()
            .chain(
                explicit_grid
                    .then_some(["--columns", "100", "--rows", "30"])
                    .into_iter()
                    .flatten(),
            )
            .map(OsString::from)
            .chain([std::env::temp_dir()
                .join("venus-unavailable-typography.sock")
                .into_os_string()]),
            None,
        )
        .unwrap();
        let mut probe = Probe {
            app: Application::new(arguments, event_loop.create_proxy()),
            explicit_grid,
            checked: false,
        };
        let proxy = event_loop.create_proxy();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let watchdog = thread::spawn(move || {
            if stopped.recv_timeout(Duration::from_secs(5))
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                let _ = proxy.send_event(UserEvent::Exit);
            }
        });
        event_loop.run_app(&mut probe).unwrap();
        let _ = stop.send(());
        watchdog.join().unwrap();
        assert!(probe.checked);
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
    fn live_output_keeps_application_input_admitted_before_repaint() {
        use eon_workspace_protocol::v19 as workspace;
        use orbit_protocol::{
            Capabilities, Colors, Cursor, CursorShape, Dimensions, Frame, Rgb, Screen,
        };
        use std::{
            os::unix::net::{UnixListener, UnixStream},
            sync::{Barrier, mpsc},
        };
        use winit::{
            event::{DeviceId, TouchPhase},
            platform::wayland::EventLoopBuilderExtWayland,
        };

        fn frame(revision: u64, screen: Screen) -> TransportEvent {
            TransportEvent::Server(ServerMessage::Frame(Box::new(Frame {
                revision,
                dimensions: Dimensions { cols: 0, rows: 0 },
                screen,
                scroll_position: orbit_protocol::ScrollPosition::default(),
                title: format!("output {revision}"),
                working_directory: String::new(),
                capabilities: Capabilities {
                    hyperlinks: true,
                    kitty_graphics: false,
                },
                colors: Colors {
                    background: Rgb::BLACK,
                    foreground: Rgb::BLACK,
                    cursor: None,
                    palette: [Rgb::BLACK; orbit_protocol::PALETTE_LEN],
                },
                cursor: Cursor {
                    visible: false,
                    blinking: false,
                    password_input: false,
                    shape: CursorShape::Block,
                    viewport: None,
                },
                rows: Vec::new(),
            })))
        }

        fn linked_frame(revision: u64, uri: &str) -> TransportEvent {
            use orbit_protocol::{Cell, CellStyle, CellWidth, Row, StyleColor, Underline};
            let TransportEvent::Server(ServerMessage::Frame(mut frame)) =
                frame(revision, Screen::Alternate)
            else {
                unreachable!()
            };
            frame.dimensions = Dimensions { cols: 2, rows: 1 };
            let cell = Cell {
                width: CellWidth::Wide,
                text: "界".into(),
                hyperlink: uri.into(),
                style: CellStyle {
                    foreground: StyleColor::None,
                    background: StyleColor::None,
                    underline_color: StyleColor::None,
                    underline: Underline::None,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    inverse: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                    selected: false,
                    protected: false,
                },
            };
            frame.rows = vec![Row {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: vec![
                    cell.clone(),
                    Cell {
                        width: CellWidth::SpacerTail,
                        text: String::new(),
                        ..cell
                    },
                ],
            }];
            TransportEvent::Server(ServerMessage::Frame(frame))
        }

        struct Probe {
            app: Application,
            listener: UnixListener,
            stream: Option<UnixStream>,
            actions: mpsc::Receiver<WorkspaceAction>,
            checked: bool,
        }
        impl ApplicationHandler<UserEvent> for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.app.create_window(event_loop).unwrap();
                self.stream = Some(self.listener.accept().unwrap().0);
                self.app
                    .handle_transport(TransportEvent::Server(ServerMessage::Attached));
                assert!(
                    self.app.status().is_empty(),
                    "normal workspace attachment must not announce its first-frame wait"
                );
                self.app.handle_transport(frame(1, Screen::Primary));
            }

            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                id: WindowId,
                event: WindowEvent,
            ) {
                self.app.window_event(event_loop, id, event);
                if self.checked || self.app.presented_revision() != Some(1) {
                    return;
                }
                let app = &mut self.app;
                let device_id = DeviceId::dummy();
                let scene = app.workspace_scene().unwrap();
                let terminal = PhysicalPosition::new(
                    f64::from(scene.terminal.left + 30.0),
                    f64::from(scene.terminal.top + 30.0),
                );
                let tab = scene.tabs[1].rect;
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: terminal,
                    },
                );
                let generation = app.presentation.generation;
                app.handle_transport(frame(2, Screen::Primary));
                // Exercise the actual handlers in the output-to-presentation gap.
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 8.0)),
                        phase: TouchPhase::Started,
                    },
                );
                assert!(
                    app.terminal.terminal_scroll.phase_active,
                    "first wheel event was dropped after output"
                );
                app.handle_transport(frame(3, Screen::Primary));
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
                        phase: TouchPhase::Ended,
                    },
                );
                assert!(
                    !app.terminal.terminal_scroll.phase_active,
                    "gesture End was dropped after output"
                );
                assert!(app.send_workspace(CommonAction::Focus(WorkspaceDirection::Right)));
                assert_eq!(
                    self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                    WorkspaceAction::Workspace(CommonAction::Focus(WorkspaceDirection::Right))
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(
                            f64::from(tab.left + 2.0),
                            f64::from(tab.top + 2.0),
                        ),
                    },
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                    },
                );
                assert_eq!(
                    self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                    WorkspaceAction::Workspace(CommonAction::FocusId("t2".into()))
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: terminal,
                    },
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                    },
                );
                app.handle_transport(frame(4, Screen::Primary));
                assert!(
                    app.input.is_selecting(),
                    "selection did not start: {:?}, resize {:?}, terminal {:?}",
                    app.terminal.model.notice(),
                    app.terminal.last_resize,
                    app.terminal_size()
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(terminal.x + 20.0, terminal.y),
                    },
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Released,
                        button: MouseButton::Left,
                    },
                );
                assert!(
                    app.terminal.deferred_selection.is_empty(),
                    "live output deferred the gesture"
                );
                assert_eq!(app.terminal.selection_gate, SelectionGate::AwaitingFinish);
                assert!(
                    app.presentation.generation > generation,
                    "renderer cache must refresh"
                );
                assert_eq!(
                    app.presented_revision(),
                    Some(1),
                    "unpainted content is not presented"
                );
                app.handle_transport(TransportEvent::Server(ServerMessage::SelectionFinished {
                    frame_revision: 4,
                }));
                assert_eq!(
                    app.terminal.selection_gate,
                    SelectionGate::AwaitingPresentation(4)
                );
                app.render();
                assert_eq!(app.presented_revision(), Some(4));
                assert_eq!(app.terminal.selection_gate, SelectionGate::Ready);

                let unchanged = app.workspace_model.snapshot().unwrap().clone();
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Failure(
                    workspace::Failure {
                        code: "notice".into(),
                        detail: "temporary notice".into(),
                    },
                )));
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    unchanged,
                )));
                assert_eq!(
                    app.presented_revision(),
                    Some(4),
                    "an unchanged snapshot that retains a notice must keep presented geometry"
                );

                // A workspace poll can skip intermediate pane selections.
                // Extra headers must resize the retained attachment too.
                let previous_size = app.terminal.last_resize.unwrap();
                let mut snapshot = app.workspace_model.snapshot().unwrap().clone();
                app.terminal.active_endpoint =
                    Some(snapshot.tabs[0].columns[0].panes[0].endpoint.clone());
                app.terminal.active_endpoint_live = true;
                snapshot.tabs[0].columns[0].panes.push(Pane {
                    id: "p3".into(),
                    session: "s3".into(),
                    endpoint: b"/unused/p3.sock".to_vec(),
                    live: false,
                    attention: None,
                });
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    snapshot,
                )));
                let resized = surface_size(
                    app.terminal_size().unwrap(),
                    app.window.as_ref().unwrap().renderer.metrics(),
                )
                .unwrap();
                assert!(resized.rows < previous_size.rows);

                let stream = self.stream.as_mut().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut messages = Vec::new();
                loop {
                    let mut header = [0; session::HEADER_BYTES];
                    match stream.read_exact(&mut header) {
                        Ok(()) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) =>
                        {
                            break;
                        }
                        Err(error) => panic!("input transport failed: {error}"),
                    }
                    let mut bytes = Vec::from(header);
                    bytes.resize(session::client_message_len(&header).unwrap().unwrap(), 0);
                    stream
                        .read_exact(&mut bytes[session::HEADER_BYTES..])
                        .unwrap();
                    messages.push(session::decode_client_message(&bytes).unwrap());
                }
                assert!(
                    messages
                        .iter()
                        .any(|message| matches!(message, ClientMessage::PreviewVertical { .. }))
                );
                assert!(
                    messages.contains(&ClientMessage::Resize(resized)),
                    "changed pane geometry did not resize the retained attachment"
                );
                let selections: Vec<_> = messages
                    .into_iter()
                    .filter_map(|message| match message {
                        ClientMessage::Selection(action) => Some(action),
                        _ => None,
                    })
                    .collect();
                assert!(
                    matches!(
                        selections.as_slice(),
                        [
                            SelectionAction::Begin {
                                frame_revision: 1,
                                ..
                            },
                            SelectionAction::Update { .. },
                            SelectionAction::Finish { .. },
                        ]
                    ),
                    "unexpected selection wire sequence: {selections:?}"
                );

                // Geometry and attachment transitions still require a real presentation.
                app.handle_transport(frame(5, Screen::Alternate));
                assert_eq!(app.presented_revision(), None);
                assert!(!app.send_workspace(CommonAction::Focus(WorkspaceDirection::Right)));
                app.render();
                assert_eq!(app.presented_revision(), Some(5));
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::Resized(PhysicalSize::new(900, 600)),
                );
                assert_eq!(app.presented_revision(), None);
                app.render();
                app.window_focused = true;
                app.links.pointer_inside = true;
                app.handle_transport(linked_frame(6, "https://example.com/6"));
                let workspace = app.workspace_scene().unwrap();
                let metrics = app.window.as_ref().unwrap().renderer.metrics();
                app.cursor = PhysicalPosition::new(
                    f64::from(workspace.terminal.left + metrics.padding + metrics.width * 1.5),
                    f64::from(workspace.terminal.top + metrics.padding + metrics.height * 0.5),
                );
                app.input
                    .set_modifiers(winit::keyboard::ModifiersState::CONTROL);
                app.inspect_pointer_link();
                assert!(
                    app.focused_link().is_some() && app.current_link().is_none(),
                    "unpresented target is inert"
                );
                assert!(
                    !app.handle_link_button(ElementState::Pressed, MouseButton::Left),
                    "an unpresented target cannot capture a press"
                );
                let highlighted = |app: &Application| {
                    app.links.shown.as_ref().map(|link| (link.row, link.column))
                };
                app.render();
                assert_eq!(app.focused_link().unwrap().uri, "https://example.com/6");
                assert_eq!(highlighted(app), Some((0, 0)));
                // No notice disables links: the hover, hint and click below run
                // under a workspace notice, which the hovered link's hint outranks.
                app.workspace_model
                    .apply(workspace::Response::Failure(workspace::Failure {
                        code: "no-attention".into(),
                        detail: "No agent needs attention.".into(),
                    }));
                app.render();
                assert_eq!(highlighted(app), Some((0, 0)), "a notice keeps the hover");
                assert!(app.status().contains("https://example.com/6"));
                // Output that keeps the target over the same cells keeps its
                // hover, hint and captured click, frame after frame.
                let kept = "file:///kept-across-output";
                app.handle_transport(linked_frame(7, kept));
                app.render();
                assert!(app.handle_link_button(ElementState::Pressed, MouseButton::Left));
                for revision in 8..11 {
                    app.handle_transport(linked_frame(revision, kept));
                    assert!(app.status().contains(kept), "output must not hide the hint");
                    app.render();
                    assert_eq!(
                        highlighted(app),
                        Some((0, 0)),
                        "output must not blink the hover"
                    );
                }
                assert!(app.handle_link_button(ElementState::Released, MouseButton::Left));
                assert_eq!(
                    app.links.notice(),
                    Some("Only HTTP and HTTPS links can be opened. Copy other targets explicitly."),
                    "a click across output revisions must activate its kept target"
                );
                app.links.focus = None;
                app.links.notice = None;
                assert!(app.workspace_model.notice().is_some());
                app.activate_accessible_link(0, 0, false);
                let refused =
                    "Only HTTP and HTTPS links can be opened. Copy other targets explicitly.";
                assert_eq!(
                    app.status(),
                    refused,
                    "a link result outranks an older notice"
                );
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Failure(
                    workspace::Failure {
                        code: "no-attention".into(),
                        detail: "Nothing is waiting.".into(),
                    },
                )));
                assert_eq!(
                    app.status(),
                    "Nothing is waiting.",
                    "a newer notice outranks it"
                );
                assert!(app.workspace_model.dismiss_no_attention());
                assert_eq!(
                    app.status(),
                    refused,
                    "an older failure returns when the notice clears"
                );
                let now = Instant::now();
                app.links.show_notice(
                    "Link handed to the desktop handler.",
                    true,
                    now,
                    app.workspace_model.notice_serial(),
                );
                app.about_to_wait(event_loop);
                assert!(
                    matches!(
                        event_loop.control_flow(),
                        ControlFlow::WaitUntil(at) if at <= now + LINK_SUCCESS_NOTICE
                    ),
                    "a success notice schedules its own wakeup"
                );
                app.links.show_notice(
                    "Link handed to the desktop handler.",
                    true,
                    Instant::now() - LINK_SUCCESS_NOTICE,
                    app.workspace_model.notice_serial(),
                );
                app.about_to_wait(event_loop);
                assert!(
                    app.links.notice.is_none(),
                    "a success clears at its deadline"
                );
                app.inspect_pointer_link();
                // A target replaced by output retires its press for good, even
                // when it returns before the next frame.
                let press = |app: &mut Application, state| {
                    assert!(app.handle_link_button(state, MouseButton::Left));
                };
                press(app, ElementState::Pressed);
                app.handle_transport(linked_frame(11, "file:///replaced"));
                assert!(app.focused_link().is_none());
                app.handle_transport(linked_frame(12, kept));
                press(app, ElementState::Released);
                assert!(
                    app.link_opener.is_none() && app.links.notice.is_none(),
                    "a captured click cannot cross a frame replacement"
                );
                assert_eq!(app.focused_link().unwrap().uri, kept);
                // The hover follows a successor while the retired press is held.
                app.render();
                press(app, ElementState::Pressed);
                app.handle_transport(linked_frame(13, "file:///successor"));
                app.render();
                assert_eq!(app.focused_link().unwrap().uri, "file:///successor");
                assert_eq!(highlighted(app), Some((0, 0)));
                press(app, ElementState::Released);
                assert!(app.links.notice.is_none());
                // A presentation change retires a press over an unchanged target.
                press(app, ElementState::Pressed);
                app.presentation.invalidate();
                app.render();
                press(app, ElementState::Released);
                assert!(
                    app.links.notice.is_none(),
                    "a presentation change retires a captured press"
                );
                app.handle_transport(linked_frame(14, ""));
                app.render();
                assert_eq!(
                    highlighted(app),
                    None,
                    "an overwritten target loses its hover"
                );
                app.handle_transport(linked_frame(15, "file:///modifier-mismatch"));
                app.render();
                let link = app.links.focus.clone().map(LinkPress::Live);
                app.input
                    .set_modifiers(winit::keyboard::ModifiersState::empty());
                app.links.pressed = link.clone();
                assert!(app.handle_link_button(ElementState::Released, MouseButton::Left));
                assert!(
                    app.links.notice.is_none(),
                    "a link release without Ctrl must not activate its target"
                );
                app.links.pressed = link.clone();
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                        phase: TouchPhase::Moved,
                    },
                );
                assert!(
                    app.links.pressed.is_none(),
                    "scroll input must retire a captured link press immediately"
                );
                app.links.pressed = link.clone();
                app.cursor = PhysicalPosition::new(0.0, 0.0);
                assert!(!app.handle_link_button(ElementState::Pressed, MouseButton::Left));
                assert!(
                    app.links.pressed.is_none(),
                    "a fresh ordinary press must retire a missed link release"
                );
                app.links.pressed = link.clone();
                app.set_workspace_focus(WorkspaceFocus::Panes);
                assert!(
                    app.links.pressed.is_none(),
                    "leaving terminal focus must retire a captured link press"
                );
                app.set_workspace_focus(WorkspaceFocus::Terminal);
                let size = surface_size(
                    app.terminal_size().unwrap(),
                    app.window.as_ref().unwrap().renderer.metrics(),
                )
                .unwrap();
                let TransportEvent::Server(ServerMessage::Frame(mut scrolled)) =
                    linked_frame(16, "")
                else {
                    unreachable!()
                };
                let mut blank = scrolled.rows[0].cells[0].clone();
                blank.width = orbit_protocol::CellWidth::Narrow;
                blank.text.clear();
                blank.hyperlink.clear();
                scrolled.dimensions = Dimensions {
                    cols: size.cols,
                    rows: size.rows,
                };
                scrolled.screen = Screen::Primary;
                scrolled.scroll_position = orbit_protocol::ScrollPosition {
                    rows_from_live: 12,
                    history_rows: 12,
                };
                scrolled.rows = vec![
                    orbit_protocol::Row {
                        wrapped: false,
                        wrap_continuation: false,
                        kitty_virtual_placeholder: false,
                        cells: vec![blank; usize::from(size.cols)],
                    };
                    usize::from(size.rows)
                ];
                let mut tick_frame = scrolled.clone();
                tick_frame.revision = 17;
                tick_frame.scroll_position.rows_from_live = 13;
                tick_frame.scroll_position.history_rows = 13;
                app.handle_transport(TransportEvent::Server(ServerMessage::Frame(scrolled)));
                assert!(
                    app.scrollback_rect(app.workspace_scene().as_ref())
                        .is_none()
                );
                app.render();
                let pill = app
                    .scrollback_rect(app.workspace_scene().as_ref())
                    .expect("the presented scrollback pill is actionable");
                assert!(
                    app.send(ClientMessage::Selection(SelectionAction::AutoscrollUp {
                        position: session::SelectionPosition { x: 1.0, y: 1.0 },
                    }))
                );
                assert!(!app.send(ClientMessage::ScrollVertical {
                    frame_revision: 16,
                    rows: -1
                }));
                assert_eq!(
                    app.terminal.pending_viewport,
                    Some(PendingViewportRequest::SelectionTick)
                );
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(
                            f64::from(pill.left + 1.0),
                            f64::from(pill.top + pill.height / 2.0),
                        ),
                    },
                );
                for event in [
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                    },
                    WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                        phase: TouchPhase::Moved,
                    },
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Released,
                        button: MouseButton::Left,
                    },
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                    },
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Released,
                        button: MouseButton::Left,
                    },
                ] {
                    app.window_event(event_loop, id, event);
                }
                let stream = self.stream.as_mut().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_millis(300)))
                    .unwrap();
                let mut returns = || {
                    let mut count = 0;
                    loop {
                        let mut header = [0; session::HEADER_BYTES];
                        if stream.read_exact(&mut header).is_err() {
                            break;
                        }
                        let mut bytes = Vec::from(header);
                        bytes.resize(session::client_message_len(&header).unwrap().unwrap(), 0);
                        stream
                            .read_exact(&mut bytes[session::HEADER_BYTES..])
                            .unwrap();
                        count += usize::from(
                            session::decode_client_message(&bytes).unwrap()
                                == ClientMessage::ReturnToLive,
                        );
                    }
                    count
                };
                assert_eq!(returns(), 0, "return waits for the outstanding tick");
                let response = ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                    requested_rows: -1,
                    applied_rows: -1,
                    frame: tick_frame,
                    next: orbit_protocol::session::PreviewOutcome::Viewport {
                        cols: size.cols,
                        edge_reached: true,
                        rows: Vec::new(),
                    },
                });
                session::encode_server_message(&response).expect("canonical Orbit scroll response");
                app.handle_transport(TransportEvent::Server(response));
                assert!(
                    app.terminal.model.is_attached(),
                    "{:?}",
                    app.terminal.model.connection()
                );
                assert_eq!(
                    returns(),
                    1,
                    "the pill sends one semantic action after the tick"
                );
                assert!(app.send(ClientMessage::ScrollVertical {
                    frame_revision: 17,
                    rows: -1
                }));
                assert!(
                    !app.send(ClientMessage::Selection(SelectionAction::AutoscrollUp {
                        position: session::SelectionPosition { x: 1.0, y: 1.0 },
                    }))
                );
                assert!(app.send(ClientMessage::ReturnToLive));
                assert_eq!(returns(), 0, "return waits for an ordinary scroll batch");
                app.handle_transport(TransportEvent::Server(ServerMessage::Failure(
                    orbit_protocol::session::Failure {
                        code: FailureCode::InvalidInput,
                        detail: "deferred batch rejected".into(),
                    },
                )));
                assert!(
                    app.terminal.model.is_attached(),
                    "{:?}",
                    app.terminal.model.connection()
                );
                assert_eq!(returns(), 1, "return follows the rejected scroll batch");
                assert!(app.send(ClientMessage::ScrollVertical {
                    frame_revision: 17,
                    rows: -1
                }));
                assert!(
                    !app.send(ClientMessage::Selection(SelectionAction::AutoscrollUp {
                        position: session::SelectionPosition { x: 1.0, y: 1.0 },
                    }))
                );
                assert!(app.send(ClientMessage::ReturnToLive));
                assert_eq!(returns(), 0);
                app.handle_transport(TransportEvent::InvalidInput("bad input".into()));
                assert!(app.terminal.model.is_attached());
                assert_eq!(returns(), 0, "a local failure cancels the queued return");
                let original_transport = app.terminal.transport.take().unwrap();
                let socket = std::env::temp_dir()
                    .join(format!("venus-admission-{}.sock", std::process::id()));
                let listener = UnixListener::bind(&socket).unwrap();
                let resume_writer = Arc::new(Barrier::new(2));
                let writer_gate = Arc::clone(&resume_writer);
                let (paused, writer_paused) = mpsc::channel();
                app.terminal.transport = Some(Transport::start(socket.clone(), move || {
                    paused.send(()).unwrap();
                    writer_gate.wait();
                }));
                let (blocked_stream, _) = listener.accept().unwrap();
                let transport = app.terminal.transport.as_ref().unwrap();
                // Hold the writer in its encoding-error wake so Full cannot race a dequeue.
                transport
                    .send(ClientMessage::ScrollVertical {
                        frame_revision: 17,
                        rows: 0,
                    })
                    .unwrap();
                writer_paused.recv_timeout(Duration::from_secs(3)).unwrap();
                let mut admission = Ok(());
                for _ in 0..512 {
                    admission = transport.send(ClientMessage::ReturnToLive);
                    if admission.is_err() {
                        break;
                    }
                }
                assert_eq!(admission, Err(yazelix_venus::SendError::Full));
                let requests = [
                    ClientMessage::ScrollVertical {
                        frame_revision: 17,
                        rows: -1,
                    },
                    ClientMessage::Selection(SelectionAction::AutoscrollUp {
                        position: session::SelectionPosition { x: 1.0, y: 1.0 },
                    }),
                ];
                for request in &requests {
                    assert!(!app.send(request.clone()));
                    assert_eq!(app.terminal.pending_viewport, None);
                }
                resume_writer.wait();
                blocked_stream.shutdown(std::net::Shutdown::Both).unwrap();
                drop(blocked_stream);
                drop(listener);
                std::fs::remove_file(&socket).unwrap();
                app.terminal.transport = Some(Transport::start(socket, || {}));
                let deadline = Instant::now() + Duration::from_secs(3);
                while app
                    .terminal
                    .transport
                    .as_ref()
                    .unwrap()
                    .send(ClientMessage::ReturnToLive)
                    != Err(yazelix_venus::SendError::Closed)
                {
                    assert!(
                        Instant::now() < deadline,
                        "transport closes after connect failure"
                    );
                    thread::sleep(Duration::from_millis(1));
                }
                for request in requests {
                    assert!(!app.send(request));
                    assert_eq!(app.terminal.pending_viewport, None);
                }
                app.terminal.transport = Some(original_transport);
                let snapshot = app.workspace_model.snapshot().unwrap().clone();
                app.links.pressed = link.clone();
                app.handle_workspace(WorkspaceEvent::Unavailable("offline".into()));
                assert_eq!(app.presented_revision(), None);
                assert!(
                    !matches!(app.links.pressed, Some(LinkPress::Live(_))),
                    "the presentation change must retire its captured press"
                );
                app.workspace_model
                    .apply(workspace::Response::Snapshot(snapshot));
                app.refresh_client_view();

                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::Resized(PhysicalSize::new(100, 600)),
                );
                let scene = app.workspace_scene().unwrap();
                assert!(scene.tab_scroll_limit() > 0.0);
                for position in [
                    PhysicalPosition::new(10.0, 1.0),
                    PhysicalPosition::new(
                        f64::from(scene.tabs[0].rect.right() + 1.0),
                        f64::from(scene.tab_viewport.height / 2.0),
                    ),
                ] {
                    app.tab_scroll = 0.0;
                    app.presentation.invalidate();
                    app.render();
                    app.cursor = position;
                    assert_eq!(
                        app.workspace_scene()
                            .unwrap()
                            .hit_test(position.x as f32, position.y as f32),
                        None
                    );
                    app.window_event(
                        event_loop,
                        id,
                        WindowEvent::MouseWheel {
                            device_id,
                            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
                            phase: TouchPhase::Moved,
                        },
                    );
                    assert!(
                        app.tab_scroll > 0.0,
                        "wheel over tab-strip gaps must scroll tabs"
                    );
                }
                // A healthy workspace handoff keeps only cosmetic position,
                // including when several selections arrive before a frame.
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::Resized(PhysicalSize::new(960, 600)),
                );
                app.window_focused = true;
                let animation_active = |app: &Application| {
                    app.window
                        .as_ref()
                        .unwrap()
                        .renderer
                        .cursor_animation_active()
                };
                let root = self.listener.local_addr().unwrap();
                let root = root.as_pathname().unwrap().parent().unwrap();
                let sockets = [root.join("tail-a.sock"), root.join("tail-b.sock")];
                let listeners = sockets
                    .each_ref()
                    .map(|socket| UnixListener::bind(socket).unwrap());
                let mut snapshot = app.workspace_model.snapshot().unwrap().clone();
                for (tab, socket) in snapshot.tabs.iter_mut().zip(&sockets) {
                    tab.columns[0].panes[0].endpoint = socket.clone().into_os_string().into_vec();
                }
                app.workspace_socket = Some(root.join("workspace.sock"));
                app.terminal.active_endpoint_live = true;
                let cursor_frame = |revision, x| {
                    let TransportEvent::Server(ServerMessage::Frame(mut frame)) =
                        linked_frame(revision, "https://example.invalid/tail")
                    else {
                        unreachable!()
                    };
                    for cell in &mut frame.rows[0].cells {
                        cell.width = orbit_protocol::CellWidth::Narrow;
                        cell.text = "x".into();
                    }
                    frame.cursor.visible = true;
                    frame.cursor.viewport = Some(orbit_protocol::CursorViewport {
                        x,
                        y: 0,
                        at_wide_tail: false,
                    });
                    TransportEvent::Server(ServerMessage::Frame(frame))
                };
                app.handle_transport(cursor_frame(1024, 0));
                app.render();
                let mut streams = Vec::new();
                for index in [1, 0, 1, 0] {
                    snapshot.active_tab = snapshot.tabs[index].id.clone();
                    app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                        snapshot.clone(),
                    )));
                    streams.push(listeners[index].accept().unwrap().0);
                    assert!(app.terminal.model.scene().is_none());
                    app.render();
                    assert!(!animation_active(app));
                    assert!(app.next_animation.is_none());
                    app.handle_transport(TransportEvent::Server(ServerMessage::Attached));
                    app.render();
                    assert!(app.terminal.model.scene().is_none());
                    if streams.len() == 1 {
                        app.handle_transport(cursor_frame(1, 1));
                        app.render();
                        assert!(animation_active(app));
                        thread::sleep(Duration::from_millis(20));
                        app.render();
                    }
                }
                app.handle_transport(cursor_frame(1, 0));
                app.render();
                assert!(animation_active(app));
                app.handle_transport(TransportEvent::Lost("cursor proof disconnect".into()));
                assert!(!animation_active(app));
                assert!(app.next_animation.is_none());
                // Recovery may display the last complete Scene while waiting,
                // but its first fresh cursor must snap after a real failure.
                app.terminal.model.prepare_reconnect();
                app.render();
                app.handle_transport(TransportEvent::Server(ServerMessage::Attached));
                app.render();
                app.handle_transport(cursor_frame(1, 1));
                app.render();
                assert!(!animation_active(app));
                // A retained failure Scene is static even if its layout moves,
                // and cannot seed continuity into another attachment.
                app.handle_transport(TransportEvent::Lost("cursor layout disconnect".into()));
                app.render();
                let mut header = snapshot.tabs[0].columns[0].panes[0].clone();
                header.id = "offline-header".into();
                header.session = "offline-session".into();
                header.live = false;
                snapshot.tabs[0].columns[0].panes.insert(0, header);
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    snapshot.clone(),
                )));
                app.render();
                assert!(
                    !animation_active(app),
                    "a retained failure frame must not animate"
                );
                snapshot.active_tab = snapshot.tabs[1].id.clone();
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    snapshot,
                )));
                streams.push(listeners[1].accept().unwrap().0);
                app.handle_transport(TransportEvent::Server(ServerMessage::Attached));
                app.render();
                app.handle_transport(cursor_frame(1, 0));
                app.render();
                assert!(
                    !animation_active(app),
                    "a failed attachment must not seed a healthy handoff"
                );
                drop(streams);
                drop(listeners);
                for socket in sockets {
                    std::fs::remove_file(socket).unwrap();
                }
                app.set_orbit_attachment(Some(b"replacement".to_vec()), false);
                assert!(app.status().contains("selected Eon pane is offline"));
                assert_eq!(app.presented_revision(), None);
                assert!(app.pointer_link(app.links.unshifted).is_none());
                let mut empty = app.workspace_model.snapshot().unwrap().clone();
                empty.active_tab = empty.tabs[0].id.clone();
                show_test_popup(&mut empty);
                empty.tabs[0].columns.clear();
                empty.tabs[0].selected_pane = None;
                empty.tabs[0].selected_popup = None;
                app.handle_workspace(WorkspaceEvent::Response(
                    eon_workspace_protocol::v19::Response::Snapshot(empty),
                ));
                assert!(app.terminal.transport.is_none());
                assert!(app.terminal.model.scene().is_none());
                assert!(app.terminal_size().is_none());
                assert_eq!(app.workspace_focus, WorkspaceFocus::Tabs);
                app.set_workspace_focus(WorkspaceFocus::Terminal);
                assert_eq!(app.workspace_focus, WorkspaceFocus::Tabs);
                assert!(
                    app.status().is_empty(),
                    "an empty body has no resize failure"
                );
                self.checked = true;
                event_loop.exit();
            }
        }

        let root = std::env::temp_dir().join(format!("venus-live-input-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let orbit_socket = root.join("orbit.sock");
        let workspace_socket = root.join("workspace.sock");
        let listener = UnixListener::bind(&orbit_socket).unwrap();
        let workspace_listener = UnixListener::bind(&workspace_socket).unwrap();
        let snapshot = Snapshot {
            active_tab: "t1".into(),
            geometry: eon_workspace_protocol::v19::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: (1..=2)
                .map(|index| Tab {
                    pending: false,
                    selected_popup: None,
                    popups: Vec::new(),
                    id: format!("t{index}"),
                    directory: b"/tmp".to_vec(),
                    selected_pane: Some(format!("p{index}")),
                    columns: vec![eon_workspace_protocol::v19::Column {
                        panes: vec![Pane {
                            id: format!("p{index}"),
                            session: format!("s{index}"),
                            endpoint: root
                                .join(format!("pane-{index}.sock"))
                                .into_os_string()
                                .into_vec(),
                            live: true,
                            attention: None,
                        }],
                        expanded: format!("p{index}"),
                    }],
                    attention_rollup: None,
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };
        let response = workspace::Response::Snapshot(snapshot.clone());
        let (sender, actions) = mpsc::channel();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                loop {
                    let (mut stream, _) = workspace_listener.accept().unwrap();
                    let mut header = [0; workspace::HEADER_BYTES];
                    stream.read_exact(&mut header).unwrap();
                    let mut bytes = Vec::from(header);
                    bytes.resize(workspace::declared_message_len(&header).unwrap(), 0);
                    stream
                        .read_exact(&mut bytes[workspace::HEADER_BYTES..])
                        .unwrap();
                    let action = workspace::decode_request(&bytes).unwrap().action;
                    stream
                        .write_all(&workspace::encode_response(&response).unwrap())
                        .unwrap();
                    if action != WorkspaceAction::Workspace(CommonAction::Inspect) {
                        sender.send(action).unwrap();
                        break;
                    }
                }
            }
        });
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let mut app = Application::new(
            launch::launch_arguments(
                [
                    "--font-family",
                    "DejaVu Sans Mono",
                    "--font-size",
                    "20",
                    "--line-height",
                    "1.5",
                ]
                .into_iter()
                .map(OsString::from)
                .chain([orbit_socket.into_os_string()]),
                None,
            )
            .unwrap(),
            event_loop.create_proxy(),
        );
        // This input fixture supplies its own attachment and frames; native
        // startup admission has separate coverage.
        app.initial_scale_pending = false;
        assert!(app.status().starts_with("Connecting to Orbit"));
        app.workspace_model
            .apply(workspace::Response::Snapshot(snapshot));
        assert!(
            app.status().is_empty(),
            "normal workspace attachment must not announce its connection"
        );
        app.workspace_transport = Some(WorkspaceTransport::start(workspace_socket, || {}));
        let mut probe = Probe {
            app,
            listener,
            stream: None,
            actions,
            checked: false,
        };
        event_loop.run_app(&mut probe).unwrap();
        assert!(probe.checked);
        drop(probe);
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_endpoints_are_only_live_panes_in_the_active_tab() {
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
                    columns: vec![eon_workspace_protocol::v19::Column {
                        panes: vec![
                            Pane {
                                id: "pane-1".into(),
                                session: "session-1".into(),
                                endpoint: b"one".to_vec(),
                                live: true,
                                attention: None,
                            },
                            Pane {
                                id: "pane-2".into(),
                                session: "session-2".into(),
                                endpoint: b"offline".to_vec(),
                                live: false,
                                attention: None,
                            },
                        ],
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
                    selected_pane: Some("pane-3".into()),
                    columns: vec![eon_workspace_protocol::v19::Column {
                        panes: vec![Pane {
                            id: "pane-3".into(),
                            session: "session-3".into(),
                            endpoint: b"hidden".to_vec(),
                            live: true,
                            attention: None,
                        }],
                        expanded: "pane-3".into(),
                    }],
                    attention_rollup: None,
                },
            ],
            geometry: eon_workspace_protocol::v19::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };

        assert_eq!(
            visible_metadata_endpoints(&snapshot, &[b"one"]),
            HashSet::new()
        );
        assert_eq!(
            visible_metadata_endpoints(&snapshot, &[]),
            [b"one".to_vec()].into()
        );
        // Every column of the active tab is visible, so each has an observer.
        let mut columns = snapshot.clone();
        columns.tabs[0]
            .columns
            .push(eon_workspace_protocol::v19::Column {
                panes: vec![Pane {
                    id: "pane-4".into(),
                    session: "session-4".into(),
                    endpoint: b"beside".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "pane-4".into(),
            });
        assert_eq!(
            visible_metadata_endpoints(&columns, &[]),
            [b"one".to_vec(), b"beside".to_vec()].into()
        );
        // A column still attaching waits for its observer like the selected pane.
        assert_eq!(
            visible_metadata_endpoints(&columns, &[b"beside"]),
            [b"one".to_vec()].into()
        );

        let mut popup = snapshot;
        show_test_popup(&mut popup);
        assert_eq!(visible_metadata_endpoints(&popup, &[]), HashSet::new());

        let inactive_popup = Snapshot {
            active_tab: "t2".into(),
            ..popup
        };
        assert_eq!(
            visible_metadata_endpoints(&inactive_popup, &[]),
            [b"hidden".to_vec()].into()
        );
    }

    #[test]
    fn renderer_failure_uses_non_gpu_diagnostics_until_success() {
        let mut notice = None;
        let mut presentation = PresentationState::default();
        let candidate = presentation.candidate(Some(7));
        presentation.publish(candidate);
        let mut titles = Vec::new();
        let mut diagnostics = Vec::new();
        let mut alerts = 0;

        for error in ["the Venus glyph atlas is full", "a later renderer failure"] {
            if record_render_failure(
                &mut notice,
                &mut presentation,
                error,
                |title| titles.push(title.to_owned()),
                |message| diagnostics.push(format!("venus: {message}")),
            )
            .is_some()
            {
                alerts += 1;
            }
        }

        let failure = "Venus renderer failure: the Venus glyph atlas is full";
        assert!(!presentation.has_presented_geometry());
        assert_eq!(titles, [failure]);
        assert_eq!(diagnostics, [format!("venus: {failure}")]);
        assert_eq!(alerts, 1);
        assert_eq!(
            window_title(Some("Orbit title"), notice.as_deref()),
            failure
        );

        assert!(notice.take().is_some());
        assert_eq!(
            window_title(Some("Orbit title"), notice.as_deref()),
            "Orbit title"
        );
        assert_eq!(window_title(Some(""), None), "Venus");
    }

    #[test]
    fn launch_values_project_to_native_window_attributes() {
        let parse = |arguments: &[&str]| {
            launch::launch_arguments(arguments.iter().map(OsString::from), None).unwrap()
        };

        let default = parse(&[]);
        let attributes = window_attributes(
            default.decorations,
            default.background_opacity,
            default.background_blur,
        );
        assert!(attributes.decorations && !attributes.transparent && !attributes.blur);

        for value in ["0", "0.88", "1"] {
            let parsed = parse(&[
                "--no-decorations",
                "--background-opacity",
                value,
                "--background-blur",
                "--workspace",
                "eon.sock",
            ]);
            let attributes = window_attributes(
                parsed.decorations,
                parsed.background_opacity,
                parsed.background_blur,
            );
            assert!(attributes.blur);
            assert_eq!(attributes.transparent, value != "1");
        }
    }

    #[test]
    fn cursor_animation_deadline_stops_when_inactive_unfocused_or_occluded() {
        let now = Instant::now();
        let mut deadline = None;

        assert!(!update_animation_deadline(true, true, &mut deadline, now));
        assert_eq!(deadline, Some(now + ANIMATION_FRAME_INTERVAL));
        assert!(update_animation_deadline(
            true,
            true,
            &mut deadline,
            now + ANIMATION_FRAME_INTERVAL,
        ));
        assert_eq!(
            deadline,
            Some(now + ANIMATION_FRAME_INTERVAL.saturating_mul(2))
        );

        for (active, focused, occluded) in [
            (false, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            let allowed = cursor_animation_allowed(focused, occluded);
            assert!(!update_animation_deadline(
                active,
                allowed,
                &mut deadline,
                now,
            ));
            assert_eq!(deadline, None);
        }
    }

    #[test]
    fn presentation_control_emits_complete_commands_then_exit() {
        let mut events = Vec::new();

        run_presentation_control(&b"ignored\npresent\npart"[..], |event| {
            events.push(match event {
                UserEvent::Present => "present",
                UserEvent::Exit => "exit",
                _ => panic!("unexpected presentation control event"),
            });
            true
        });

        assert_eq!(events, ["present", "exit"]);
    }

    #[test]
    fn blinking_starts_with_a_complete_visible_phase_after_idle() {
        let now = Instant::now();
        let mut visible = false;
        let mut deadline = Some(now);

        assert!(!update_blink(false, &mut visible, &mut deadline, now));
        assert_eq!((visible, deadline), (true, None));
        assert!(!update_blink(true, &mut visible, &mut deadline, now));
        assert_eq!((visible, deadline), (true, Some(now + BLINK_INTERVAL)));
    }

    #[test]
    fn spinner_turns_at_most_five_times_a_second_and_only_while_shown() {
        let now = Instant::now();
        let mut deadline = None;

        assert!(!update_spinner(true, &mut deadline, now));
        let late = now + SPINNER_INTERVAL * 3;
        assert!(update_spinner(true, &mut deadline, late));
        assert_eq!(deadline, Some(late + SPINNER_INTERVAL));
        assert!(!update_spinner(
            true,
            &mut deadline,
            late + SPINNER_INTERVAL - Duration::from_millis(1)
        ));
        assert!(!update_spinner(false, &mut deadline, late));
        assert_eq!(deadline, None);
    }

    #[test]
    fn orbit_retry_schedule_is_bounded_and_resettable() {
        let now = Instant::now();
        let mut retry = OrbitRetry::default();

        for delay in [250, 500, 1_000, 2_000, 4_000, 5_000, 5_000] {
            retry.schedule(now);
            assert_eq!(retry.deadline, Some(now + Duration::from_millis(delay)));
            retry.schedule(now + Duration::from_millis(1));
            assert_eq!(retry.deadline, Some(now + Duration::from_millis(delay)));
            assert!(!retry.take_due(now + Duration::from_millis(delay - 1)));
            assert!(retry.take_due(now + Duration::from_millis(delay)));
        }

        retry.schedule(now);
        retry.reset();
        assert_eq!(retry.deadline, None);
        retry.schedule(now);
        assert_eq!(retry.deadline, Some(now + Duration::from_millis(250)));
    }

    #[test]
    fn workspace_recovery_requires_the_same_selected_live_endpoint() {
        let first = b"/run/eon/session-1.sock";
        let second = b"/run/eon/session-2.sock";

        assert!(retry_is_allowed(false, None, None));
        assert!(retry_is_allowed(true, Some((first, true)), Some(first)));
        assert!(!retry_is_allowed(true, Some((first, false)), Some(first)));
        assert!(!retry_is_allowed(true, Some((second, true)), Some(first)));
    }

    #[test]
    fn only_supervised_orbit_busy_is_retryable() {
        let busy = TransportEvent::Server(ServerMessage::Busy);

        assert!(managed_busy_is_retryable(true, &busy));
        assert!(!managed_busy_is_retryable(false, &busy));
        assert!(!managed_busy_is_retryable(
            true,
            &TransportEvent::Server(ServerMessage::Accepted),
        ));
    }

    #[test]
    fn protocol_and_terminal_failures_do_not_become_socket_retries() {
        use orbit_protocol::session::Failure;

        for (code, suppressed) in [
            (FailureCode::InvalidInput, false),
            (FailureCode::Protocol, true),
            (FailureCode::Terminal, true),
        ] {
            assert_eq!(
                server_failure_suppresses_retry(&ServerMessage::Failure(Failure {
                    code,
                    detail: "bounded".into(),
                })),
                suppressed
            );
        }
    }

    #[test]
    fn retryable_loss_records_state_independently_of_retry_policy() {
        use orbit_protocol::session::Failure;

        let attached = || {
            let mut model = SessionModel::new();
            model.apply(ServerMessage::Attached).unwrap();
            model
        };
        let fail = |model: &mut SessionModel, code, detail: &str| {
            model
                .apply(ServerMessage::Failure(Failure {
                    code,
                    detail: detail.into(),
                }))
                .unwrap();
        };
        let mut suppressed = attached();
        fail(&mut suppressed, FailureCode::Terminal, "request rejected");
        assert_eq!(
            suppressed.notice(),
            Some("Orbit terminal failure: request rejected")
        );
        suppressed.apply(ServerMessage::Accepted).unwrap();
        assert_eq!(suppressed.notice(), None);
        fail(
            &mut suppressed,
            FailureCode::InvalidInput,
            "later rejected input",
        );
        assert_eq!(
            suppressed.notice(),
            Some("Orbit rejected input: later rejected input")
        );

        assert!(!apply_retryable_loss(
            &mut suppressed,
            "Orbit closed the local session".into(),
            true,
        ));
        assert!(matches!(
            suppressed.connection(),
            ConnectionState::Lost { detail } if detail == "Orbit closed the local session"
        ));
        assert!(!suppressed.is_attached());
        assert_eq!(suppressed.notice(), None);

        let mut protocol_failure = attached();
        fail(
            &mut protocol_failure,
            FailureCode::Protocol,
            "invalid mouse coordinates",
        );
        protocol_failure.set_venus_notice(LocalNoticeSource::Input, "late local notice");
        assert!(!apply_retryable_loss(
            &mut protocol_failure,
            "Orbit closed the local session".into(),
            true,
        ));
        assert_eq!(
            protocol_failure.notice(),
            Some("Orbit protocol failure: invalid mouse coordinates")
        );
        assert!(matches!(
            protocol_failure.connection(),
            ConnectionState::Lost { .. }
        ));

        let mut eligible = attached();
        assert!(apply_retryable_loss(
            &mut eligible,
            "Cannot read from Orbit: connection reset".into(),
            false,
        ));
        assert!(matches!(
            eligible.connection(),
            ConnectionState::Lost { detail }
                if detail == "Cannot read from Orbit: connection reset"
        ));
    }

    #[test]
    fn stalled_link_dispatcher_is_bounded_and_reaped() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let now = Instant::now();
        let mut opener = LinkOpener {
            child,
            deadline: now + Duration::from_secs(10),
        };
        assert!(opener.poll(now).is_none());
        assert!(
            opener
                .poll(now + Duration::from_secs(10))
                .unwrap()
                .unwrap_err()
                .contains("timed out")
        );
        drop(opener);
        assert!(
            !PathBuf::from(format!("/proc/{pid}")).exists(),
            "dispatcher must be reaped"
        );
    }

    #[test]
    fn link_success_notice_clears_on_its_own_and_failures_persist() {
        let now = Instant::now();
        let mut opener = LinkOpener {
            child: Command::new("true").spawn().unwrap(),
            deadline: now + Duration::from_secs(10),
        };
        let handed = loop {
            if let Some(result) = opener.poll(now) {
                break result;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(handed, Ok("Link handed to the desktop handler."));
        let mut links = LinkInteraction::default();
        links.show_notice(handed.unwrap(), true, now, 0);
        let deadline = links.notice_deadline().unwrap();
        assert_eq!(deadline, now + LINK_SUCCESS_NOTICE);
        assert!(!links.expire_notice(deadline - Duration::from_millis(1)));
        assert!(
            links.expire_notice(deadline),
            "success clears with no input"
        );
        assert_eq!((links.notice(), links.notice_deadline()), (None, None));
        links.show_notice("Could not open link.", false, now, 0);
        assert!(!links.expire_notice(now + Duration::from_secs(3600)));
        assert_eq!(links.notice(), Some("Could not open link."));
    }

    #[test]
    fn hyperlink_actions_require_current_presentation_and_safe_exact_uri() {
        let mut presentation = PresentationState::default();
        let first = presentation.candidate(Some(7));
        assert!(!presentation.is_current(first));
        presentation.publish(first);
        assert!(presentation.is_current(first));
        presentation.content_changed();
        assert!(!presentation.is_current(first));
        let second = presentation.candidate(Some(8));
        assert!(!presentation.is_current(second));
        presentation.publish(second);
        assert!(presentation.is_current(second));
        presentation.unpublish();
        assert!(!presentation.is_current(second));
        presentation.invalidate();
        presentation.publish(first);
        assert!(!presentation.is_current(first));

        for uri in [
            "https://example.com/exact?x=%26&y=2#part",
            "HTTP://localhost:8080/",
            "https://[::1]:443/a",
        ] {
            assert_eq!(validate_open_uri(uri), Ok(()), "{uri}");
        }
        for uri in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https:///bad",
            "https://user@example.com/",
            "https://example.com\\@evil.test",
            "https://example.com/%zz",
            "https://example.com/%0",
            "https://example.com/\n",
            "https://example.com/\u{202e}",
            "https://[bad]/",
            "https://example.com:99999/",
            "https://-bad.example/",
            "https://example.com/ space",
        ] {
            assert!(validate_open_uri(uri).is_err(), "{uri:?}");
        }
        assert!(validate_open_uri(&format!("https://example.com/{}", "x".repeat(4096))).is_err());
        assert!(validate_copy_uri("file:///tmp/a").is_ok());
        assert!(validate_copy_uri("https://example.com/\n").is_err());
        let target = "https://example.com/\u{202e}abc";
        assert_eq!(
            link_hint_for(NativePlatform::Linux, target, &[]),
            "https://example.com/\\u{202e}abc\nCtrl+click Open · Ctrl+Shift+C Copy"
        );
        let copy_modifiers = copy_paste_modifiers(NATIVE_PLATFORM);
        assert!(link_copy_shortcut(
            PhysicalKey::Code(KeyCode::KeyC),
            copy_modifiers,
            &[],
            false,
            true,
        ));
        assert!(!link_copy_shortcut(
            PhysicalKey::Code(KeyCode::KeyC),
            copy_modifiers,
            &[],
            true,
            true,
        ));
        for key in [
            KeyCode::KeyO,
            KeyCode::Tab,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::Enter,
            KeyCode::NumpadEnter,
            KeyCode::Escape,
        ] {
            assert!(!link_copy_shortcut(
                PhysicalKey::Code(key),
                copy_modifiers,
                &[],
                false,
                true,
            ));
        }
    }

    #[test]
    fn attachment_revision_collision_waits_for_the_new_generation() {
        let mut presentation = PresentationState::default();
        let attachment_a = presentation.candidate(Some(1));
        presentation.publish(attachment_a);
        assert_eq!(presentation.presented_revision(), Some(1));

        presentation.invalidate();
        let attachment_b = presentation.candidate(Some(1));
        assert_ne!(attachment_b, attachment_a);
        assert_eq!(presentation.presented_revision(), None);

        presentation.publish(attachment_b);
        assert_eq!(presentation.presented_revision(), Some(1));
    }

    #[test]
    fn output_keeps_presented_geometry_until_a_structural_transition() {
        let mut presentation = PresentationState::default();
        let workspace_a = presentation.candidate(Some(7));
        presentation.publish(workspace_a);
        assert!(presentation.has_presented_geometry());

        presentation.content_changed();
        let output = presentation.candidate(Some(8));
        assert!(output.generation > workspace_a.generation);
        assert!(presentation.has_presented_geometry());
        assert_eq!(presentation.presented_revision(), Some(7));
        presentation.publish(workspace_a);
        assert_eq!(presentation.presented_revision(), Some(7));
        presentation.publish(output);
        assert_eq!(presentation.presented_revision(), Some(8));

        presentation.invalidate();
        let workspace_b = presentation.candidate(Some(7));
        assert!(!presentation.has_presented_geometry());
        presentation.publish(workspace_a);
        assert!(!presentation.has_presented_geometry());

        presentation.publish(workspace_b);
        assert!(presentation.has_presented_geometry());
        presentation.unpublish();
        assert!(!presentation.has_presented_geometry());

        presentation.publish(workspace_b);
        assert!(presentation.has_presented_geometry());
    }

    #[test]
    fn published_accessibility_focus_waits_for_eon_queue_admission() {
        for (target, admitted, expected_focus, expected_action) in [
            (
                AccessibilityTarget::Tab("t2".into()),
                true,
                Some(WorkspaceFocus::Tabs),
                CommonAction::FocusId("t2".into()),
            ),
            (
                AccessibilityTarget::Pane("pane-3".into()),
                false,
                None,
                CommonAction::FocusId("pane-3".into()),
            ),
        ] {
            let mut queued = None;
            let focus = accessibility_workspace_focus(target, |action| {
                queued = Some(action);
                admitted
            });
            assert_eq!((focus, queued), (expected_focus, Some(expected_action)));
        }
        assert_eq!(
            accessibility_workspace_focus(AccessibilityTarget::Terminal, |_| unreachable!()),
            Some(WorkspaceFocus::Terminal)
        );
    }

    #[test]
    fn workspace_chrome_preserves_terminal_release_pairing() {
        use ElementState::{Pressed, Released};

        for (workspace, terminal_hit, state, current, expected) in [
            (true, false, Pressed, true, false),
            (true, false, Released, false, true),
            (true, true, Pressed, true, true),
            (false, false, Pressed, true, true),
            (false, false, Pressed, false, false),
        ] {
            assert_eq!(
                button_reaches_terminal(workspace, terminal_hit, state, current),
                expected
            );
        }
    }

    #[test]
    fn scrollback_pill_activates_only_a_complete_left_click() {
        use ElementState::{Pressed, Released};
        let mut pressed = None;
        let current = Some(PresentationIdentity {
            generation: 1,
            revision: Some(4),
        });
        let replaced = Some(PresentationIdentity {
            generation: 2,
            revision: Some(5),
        });
        for (state, target, expected) in [
            (Pressed, None, None),
            (Released, current, None),
            (Pressed, current, Some(false)),
            (Released, None, Some(false)),
            (Pressed, current, Some(false)),
            (Released, replaced, Some(false)),
            (Pressed, current, Some(false)),
            (Released, current, Some(true)),
            (Released, current, None),
        ] {
            assert_eq!(
                scrollback_button(&mut pressed, state, MouseButton::Left, target),
                expected,
                "{state:?} {target:?}"
            );
        }
    }

    #[test]
    fn new_window_shortcut_invokes_the_supplied_eon_command() {
        use std::{fs, os::unix::fs::PermissionsExt, time::SystemTime};

        let shortcut = native_key_shortcut_for(
            NativePlatform::Linux,
            PhysicalKey::Code(KeyCode::KeyN),
            ALT_SHIFT,
            &[],
        )
        .unwrap();
        assert_eq!(shortcut.action, NativeShortcutAction::NewWindow);
        assert!(
            shortcut_groups_for(
                NativePlatform::Linux,
                &[],
                eon_workspace_protocol::v19::Sidebar::Open
            )
            .iter()
            .flat_map(|group| &group.rows)
            .any(|row| row.shortcut == "Alt+Shift+N" && row.action == "Open new Eon window")
        );

        let script = std::env::temp_dir().join(format!(
            "venus-new-window-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(
            &script,
            "#!/bin/sh\nprintf '%s|%s' \"$1\" \"$2\" >&2\nexit 7\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(run_new_window(&script).unwrap_err().contains("window|new"));
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(run_new_window(&script).is_ok());
        fs::remove_file(script).unwrap();
    }

    #[test]
    fn sidebar_rows_take_f6_focus_step_by_arrow_and_follow_their_jump() {
        use eon_workspace_protocol::v19::{Attention, AttentionState, Column, PopupGeometry};

        // F6 visits the sidebar between the terminal and the tabs, only while it has rows.
        assert_eq!(
            cycle_focus(WorkspaceFocus::Terminal, true, true),
            WorkspaceFocus::Sidebar
        );
        assert_eq!(
            cycle_focus(WorkspaceFocus::Sidebar, true, true),
            WorkspaceFocus::Tabs
        );
        assert_eq!(
            cycle_focus(WorkspaceFocus::Terminal, false, true),
            WorkspaceFocus::Tabs
        );
        assert_eq!(
            cycle_focus(WorkspaceFocus::Tabs, true, true),
            WorkspaceFocus::Panes
        );
        assert_eq!(
            cycle_focus(WorkspaceFocus::Tabs, true, false),
            WorkspaceFocus::Terminal
        );
        assert_eq!(
            cycle_focus(WorkspaceFocus::Panes, true, true),
            WorkspaceFocus::Terminal
        );

        let pane = |id: &str, attention: bool| Pane {
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: format!("/run/{id}.sock").into_bytes(),
            live: true,
            attention: attention.then_some(Attention {
                state: AttentionState::Unavailable,
                record: None,
            }),
        };
        let tab = |id: &str, panes: Vec<Pane>| Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: id.into(),
            directory: format!("/work/{id}").into_bytes(),
            selected_pane: Some(panes[0].id.clone()),
            attention_rollup: panes
                .iter()
                .any(|pane| pane.attention.is_some())
                .then_some(AttentionState::Unavailable),
            columns: vec![Column {
                expanded: panes[0].id.clone(),
                panes,
            }],
        };
        let mut snapshot = Snapshot {
            active_tab: "t1".into(),
            tabs: vec![
                tab("t1", vec![pane("p1", false), pane("p2", true)]),
                tab("t2", vec![pane("p3", true)]),
            ],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };
        let scene = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1400, 800),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        // Arrows step through the rows in order and stop at either end.
        let rows = scene.sidebar_rows();
        assert_eq!(step_sidebar_row(rows, Some("p2"), true), Some("p3"));
        assert_eq!(step_sidebar_row(rows, Some("p3"), false), Some("p2"));
        assert_eq!(step_sidebar_row(rows, Some("p3"), true), None);
        assert_eq!(step_sidebar_row(rows, Some("p2"), false), None);
        assert_eq!(step_sidebar_row(rows, None, true), None);

        // A jump focuses the terminal on any snapshot that focuses its exact target; only its
        // own answer moves focus to the tab strip, when that selected just the target's tab.
        let jump = AttentionJump {
            tab: "t2".into(),
            target: "p3".into(),
            ticket: 7,
        };
        assert_eq!(focused_terminal(&snapshot), Some("p1"));
        for answer in [false, true] {
            assert_eq!(jump_focus(&jump, &snapshot, answer), None);
        }
        snapshot.active_tab = "t2".into();
        for answer in [false, true] {
            assert_eq!(
                jump_focus(&jump, &snapshot, answer),
                Some(WorkspaceFocus::Terminal)
            );
        }
        show_test_popup(&mut snapshot);
        snapshot.tabs[1].popups.push(Popup {
            id: "u1".into(),
            entry: "project".into(),
            session: "session-u1".into(),
            endpoint: b"/run/u1.sock".to_vec(),
            attention: None,
        });
        snapshot.tabs[1].selected_popup = Some("u1".into());
        assert_eq!(focused_terminal(&snapshot), Some("u1"));
        assert_eq!(jump_focus(&jump, &snapshot, false), None);
        assert_eq!(
            jump_focus(&jump, &snapshot, true),
            Some(WorkspaceFocus::Tabs)
        );
    }

    #[test]
    fn eon_bar_controls_reuse_existing_actions_and_local_viewer() {
        assert_eq!(
            workspace_header_action(WorkspaceHeaderControl::NewTab, "t7"),
            WorkspaceHeaderAction::Workspace(CommonAction::CreateTab)
        );
        assert_eq!(
            workspace_header_action(WorkspaceHeaderControl::Shortcuts, "t7"),
            WorkspaceHeaderAction::ToggleShortcuts
        );
        assert_eq!(
            workspace_header_action(WorkspaceHeaderControl::CloseTab, "t7"),
            WorkspaceHeaderAction::Workspace(CommonAction::CloseTab { tab: "t7".into() })
        );
        assert_eq!(
            cycle_header_focus(WorkspaceFocus::Tabs, false, true),
            Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
        );
        assert_eq!(
            cycle_header_focus(
                WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
                true,
                true
            ),
            Some(WorkspaceFocus::Tabs)
        );
        assert_eq!(cycle_header_focus(WorkspaceFocus::Panes, false, true), None);
        assert_eq!(
            cycle_header_focus(
                WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
                false,
                false
            ),
            Some(WorkspaceFocus::Header(WorkspaceHeaderControl::CloseTab))
        );
        assert_eq!(
            cycle_header_focus(
                WorkspaceFocus::Header(WorkspaceHeaderControl::CloseTab),
                true,
                false
            ),
            Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
        );
    }

    #[test]
    fn shortcut_viewer_uses_live_rows_and_captures_its_local_sequence() {
        let toggle = PhysicalKey::Code(KeyCode::KeyB);
        assert_eq!(
            native_key_shortcut(toggle, ALT_SHIFT, &[]).map(|shortcut| shortcut.action),
            Some(NativeShortcutAction::ToggleHints)
        );
        use orbit_protocol::session::Modifiers;

        let entries = vec![PopupEntry {
            id: "project".into(),
            label: "Money ops".into(),
            shortcut: Shortcut {
                modifiers: eon_workspace_protocol::v19::ALT,
                key: "KeyZ".into(),
            },
        }];
        let groups = shortcut_groups_for(
            NativePlatform::Linux,
            &entries,
            eon_workspace_protocol::v19::Sidebar::Open,
        );
        let key_shortcut = |key, modifiers| {
            native_key_shortcut_for(NativePlatform::Linux, key, modifiers, &[]).unwrap()
        };
        let rows = groups
            .iter()
            .flat_map(|group| &group.rows)
            .map(|row| (row.shortcut.as_str(), row.action.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            groups
                .iter()
                .filter(|group| group.title != "Projects and tools")
                .map(|group| group.rows.len())
                .sum::<usize>(),
            FIXED_SHORTCUTS.len()
        );
        for shortcut in FIXED_SHORTCUTS {
            let display = shortcut.trigger.display_label_for(NativePlatform::Linux);
            assert_eq!(
                rows.iter()
                    .filter(|row| **row == (display.as_str(), shortcut.label))
                    .count(),
                1,
                "{}",
                display
            );
            match shortcut.trigger {
                NativeShortcutTrigger::Key(code, modifiers) => assert!(std::ptr::eq(
                    key_shortcut(PhysicalKey::Code(code), modifiers),
                    shortcut
                )),
                NativeShortcutTrigger::AltDigits => {
                    for code in [KeyCode::Digit1, KeyCode::Digit9, KeyCode::Digit0] {
                        assert!(std::ptr::eq(
                            key_shortcut(PhysicalKey::Code(code), Modifiers::ALT),
                            shortcut
                        ));
                    }
                }
                NativeShortcutTrigger::Copy => {
                    assert!(std::ptr::eq(
                        key_shortcut(PhysicalKey::Code(KeyCode::KeyC), CTRL_SHIFT),
                        shortcut
                    ));
                }
                NativeShortcutTrigger::Paste => {
                    for (code, modifiers) in [
                        (KeyCode::KeyV, CTRL_SHIFT),
                        (KeyCode::Paste, Modifiers::empty()),
                    ] {
                        assert!(std::ptr::eq(
                            key_shortcut(PhysicalKey::Code(code), modifiers),
                            shortcut
                        ));
                    }
                }
                NativeShortcutTrigger::OpenLink => assert!(std::ptr::eq(
                    native_pointer_shortcut_for(
                        NativePlatform::Linux,
                        MouseButton::Left,
                        Modifiers::CTRL,
                    )
                    .unwrap(),
                    shortcut
                )),
            }
        }
        assert_eq!(
            rows.iter()
                .filter(|(shortcut, _)| *shortcut == "Alt+/")
                .count(),
            1
        );
        assert!(rows.contains(&("Ctrl+click", "Open link")));
        assert!(rows.contains(&("Alt+Z", "Money ops")));
        assert!(
            shortcut_groups_for(
                NativePlatform::Linux,
                &[],
                eon_workspace_protocol::v19::Sidebar::Open
            )
            .iter()
            .flat_map(|group| &group.rows)
            .all(|row| row.action != "Money ops")
        );

        assert_eq!(
            shortcut_viewer_command(false, KeyCode::Slash, Modifiers::ALT, false),
            Some(ShortcutViewerCommand::Toggle)
        );
        assert_eq!(
            shortcut_viewer_command(false, KeyCode::Slash, Modifiers::ALT, true),
            None
        );
        assert_eq!(
            shortcut_viewer_command(true, KeyCode::Escape, Modifiers::empty(), false),
            Some(ShortcutViewerCommand::Close)
        );
        assert_eq!(
            shortcut_viewer_command(true, KeyCode::KeyA, Modifiers::empty(), false),
            None
        );
        assert_eq!(
            workspace_shortcut(
                KeyCode::Slash,
                Modifiers::ALT,
                &[],
                "t1",
                None,
                Sidebar::Open
            ),
            None
        );

        let mut input = InputState::default();
        let slash = PhysicalKey::Code(KeyCode::Slash);
        assert!(input.consumes_shortcut(slash, ElementState::Pressed, false, true));
        assert!(input.consumes_shortcut(slash, ElementState::Pressed, true, false));
        assert!(input.consumes_shortcut(slash, ElementState::Released, false, false));
    }

    #[test]
    fn macos_uses_command_for_standard_host_actions_only() {
        use orbit_protocol::session::Modifiers;

        let action = |key, modifiers| {
            native_key_shortcut_for(
                NativePlatform::Macos,
                PhysicalKey::Code(key),
                modifiers,
                &[],
            )
            .map(|shortcut| shortcut.action)
        };
        assert_eq!(
            action(KeyCode::KeyC, Modifiers::SUPER),
            Some(NativeShortcutAction::Copy)
        );
        assert_eq!(
            action(KeyCode::KeyV, Modifiers::SUPER),
            Some(NativeShortcutAction::Paste)
        );
        assert_eq!(action(KeyCode::KeyC, CTRL_SHIFT), None);
        assert_eq!(action(KeyCode::KeyV, CTRL_SHIFT), None);
        assert_eq!(
            action(KeyCode::KeyH, Modifiers::ALT),
            Some(NativeShortcutAction::Focus(WorkspaceDirection::Left))
        );
        assert_eq!(
            action(KeyCode::KeyH, CTRL_ALT),
            Some(NativeShortcutAction::MovePaneSideways(Side::Left))
        );
        assert_eq!(
            native_pointer_shortcut_for(
                NativePlatform::Macos,
                MouseButton::Left,
                Modifiers::SUPER,
            )
            .map(|shortcut| shortcut.action),
            Some(NativeShortcutAction::OpenLink)
        );
        assert!(
            native_pointer_shortcut_for(NativePlatform::Macos, MouseButton::Left, Modifiers::CTRL,)
                .is_none()
        );

        let rows = shortcut_groups_for(
            NativePlatform::Macos,
            &[],
            eon_workspace_protocol::v19::Sidebar::Open,
        )
        .into_iter()
        .flat_map(|group| group.rows)
        .map(|row| row.shortcut)
        .collect::<Vec<_>>();
        for label in [
            "Command+C",
            "Command+V or Paste",
            "Command+click",
            "Option+H",
            "Control+Option+H",
            "Option+/",
        ] {
            assert!(rows.iter().any(|row| row == label), "{label}");
        }
        assert_eq!(
            link_hint_for(NativePlatform::Macos, "https://example.com/", &[]),
            "https://example.com/\nCommand+click Open · Command+C Copy"
        );

        for (platform, program, arguments) in [
            (
                NativePlatform::Linux,
                "gio",
                vec!["open", "--", "https://example.com/"],
            ),
            (
                NativePlatform::Macos,
                "/usr/bin/open",
                vec!["-u", "https://example.com/"],
            ),
        ] {
            let command = link_command(platform, "https://example.com/");
            assert_eq!(command.get_program().to_str(), Some(program));
            assert_eq!(
                command
                    .get_args()
                    .map(|argument| argument.to_str().unwrap())
                    .collect::<Vec<_>>(),
                arguments
            );
        }
    }

    #[test]
    fn structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit() {
        let actions = [
            CommonAction::CreateTab,
            CommonAction::CreatePane { focus: true },
            CommonAction::Move(WorkspaceDirection::Left),
            CommonAction::CloseTab { tab: "t1".into() },
        ]
        .map(WorkspaceAction::Workspace);
        for action in actions.into_iter().chain([
            WorkspaceAction::CreateColumn,
            WorkspaceAction::ClosePane { pane: "p1".into() },
            WorkspaceAction::MovePaneSideways(Side::Right),
        ]) {
            assert!(sends_workspace_shortcut(
                &action,
                ElementState::Pressed,
                false
            ));
            assert!(!sends_workspace_shortcut(
                &action,
                ElementState::Pressed,
                true
            ));
            assert!(!sends_workspace_shortcut(
                &action,
                ElementState::Released,
                false
            ));
        }
        assert!(sends_workspace_shortcut(
            &WorkspaceAction::Workspace(CommonAction::Focus(WorkspaceDirection::Right)),
            ElementState::Pressed,
            true
        ));
        let mut snapshot = Snapshot {
            active_tab: "t1".into(),
            geometry: eon_workspace_protocol::v19::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: true,
                selected_pane: None,
                selected_popup: None,
                columns: Vec::new(),
                popups: vec![],
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v19::Sidebar::Open,
        };
        show_test_popup(&mut snapshot);
        for (focused, intent) in [(true, InvokeIntent::Toggle), (false, InvokeIntent::Focus)] {
            let action = popup_shortcut(
                KeyCode::KeyZ,
                orbit_protocol::session::Modifiers::ALT,
                &snapshot,
                focused,
            )
            .unwrap();
            assert_eq!(
                action,
                WorkspaceAction::InvokePopup {
                    tab: "t1".into(),
                    entry: "project".into(),
                    expected_instance: Some("u1".into()),
                    intent
                }
            );
            eon_workspace_protocol::v19::check_popup_action(&snapshot, &action).unwrap();
            assert!(!sends_workspace_shortcut(
                &action,
                ElementState::Pressed,
                true
            ));
        }
        for key in [KeyCode::Escape, KeyCode::Tab, KeyCode::Enter, KeyCode::KeyC] {
            assert!(
                popup_shortcut(
                    key,
                    orbit_protocol::session::Modifiers::empty(),
                    &snapshot,
                    true
                )
                .is_none()
            );
        }
    }

    #[test]
    fn ime_requires_current_attached_terminal_but_always_cleans_up() {
        assert!(!ime_allowed(false, WorkspaceFocus::Terminal, true));
        assert!(!ime_allowed(true, WorkspaceFocus::Panes, true));
        assert!(!ime_allowed(true, WorkspaceFocus::Terminal, false));
        assert!(ime_allowed(true, WorkspaceFocus::Terminal, true));
        assert!(ime_reaches_terminal(false, &Ime::Disabled));
        assert!(!ime_reaches_terminal(false, &Ime::Commit("ignored".into())));
        assert!(ime_reaches_terminal(true, &Ime::Commit("accepted".into())));
    }

    #[test]
    fn rejected_native_ime_commit_uses_the_input_notice_owner() {
        let mut input = InputState::default();
        let mut model = SessionModel::new();
        let rejected = "x".repeat(session::MAX_KEY_TEXT_BYTES + 1);

        assert_eq!(
            native_ime_message(&mut input, &mut model, Ime::Commit(rejected)),
            None
        );
        assert_eq!(
            model.notice(),
            Some(
                "Venus could not encode input: native input method commit is not accepted semantic key text"
            )
        );

        assert!(matches!(
            native_ime_message(&mut input, &mut model, Ime::Commit("界".into())),
            Some(ClientMessage::Key(_))
        ));
        assert!(model.clear_venus_notice(LocalNoticeSource::Input));
        assert_eq!(model.notice(), None);
    }

    #[test]
    fn orbit_focus_requires_window_and_terminal_focus() {
        assert!(terminal_focused(true, WorkspaceFocus::Terminal));
        assert!(!terminal_focused(false, WorkspaceFocus::Terminal));
        assert!(!terminal_focused(true, WorkspaceFocus::Tabs));
        assert!(!terminal_focused(true, WorkspaceFocus::Panes));
    }

    #[test]
    fn revision_bound_selection_waits_for_a_recovered_surface() {
        let mouse = ClientMessage::Mouse(orbit_protocol::session::MouseEvent {
            action: orbit_protocol::session::MouseAction::Motion,
            button: None,
            modifiers: orbit_protocol::session::Modifiers::empty(),
            x: 0.0,
            y: 0.0,
        });
        let selection = ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 7,
            position: orbit_protocol::session::SelectionPosition { x: 0.0, y: 0.0 },
            time_ns: 0,
            modifiers: orbit_protocol::session::Modifiers::empty(),
        });

        assert!(can_follow_implicit_resize(&mouse));
        assert!(!can_follow_implicit_resize(&selection));
    }

    #[test]
    fn copy_is_revision_free_but_waits_for_selection_finish() {
        use winit::event::ElementState::{Pressed, Released};

        let copy = ClientMessage::Selection(SelectionAction::Copy);
        assert!(selection_presentation_is_ready(&copy, false));
        assert!(!copy_is_ready(true, Pressed, false));
        assert!(copy_is_ready(false, Pressed, false));
        assert!(!copy_is_ready(false, Pressed, true));
        assert!(!copy_is_ready(false, Released, false));
    }

    #[test]
    fn key_release_does_not_dismiss_clipboard_result() {
        let key = |action| {
            ClientMessage::Key(session::KeyEvent {
                action,
                key: session::PhysicalKey::UNIDENTIFIED,
                modifiers: session::Modifiers::empty(),
                consumed_modifiers: session::Modifiers::empty(),
                composing: false,
                text: None,
                unshifted_codepoint: None,
            })
        };

        assert!(!dismisses_clipboard_notice(
            LocalNoticeSource::Resize,
            &key(session::KeyAction::Press)
        ));
        assert!(!dismisses_clipboard_notice(
            LocalNoticeSource::Input,
            &key(session::KeyAction::Release)
        ));
        assert!(dismisses_clipboard_notice(
            LocalNoticeSource::Input,
            &key(session::KeyAction::Press)
        ));
    }

    #[test]
    fn clipboard_failure_outranks_stale_link_notice() {
        assert_eq!(
            status_notice(Some("paste failed"), Some("link copied")),
            Some("paste failed")
        );
    }

    #[test]
    fn clipboard_results_are_attributed_without_copying_terminal_cells() {
        use NativeClipboard::{Both, Standard};

        assert_eq!(
            clipboard_notice(Standard, Ok::<(), &str>(())),
            "Text copied to the native clipboard."
        );
        assert_eq!(
            clipboard_notice(Both, Ok::<(), &str>(())),
            "Text copied to the native clipboard and primary selection."
        );
        assert_eq!(
            clipboard_notice(Standard, Err("display unavailable")),
            "Venus could not write the native clipboard: display unavailable"
        );
    }

    #[test]
    fn deferred_selection_preserves_order_bounds_and_cancel_ownership() {
        let begin = |revision| {
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: revision,
                position: orbit_protocol::session::SelectionPosition { x: 1.0, y: 2.0 },
                time_ns: 3,
                modifiers: orbit_protocol::session::Modifiers::empty(),
            })
        };
        let finish = ClientMessage::Selection(SelectionAction::Finish {
            position: orbit_protocol::session::SelectionPosition { x: 1.0, y: 2.0 },
            modifiers: orbit_protocol::session::Modifiers::empty(),
        });
        let mut pending = VecDeque::from([
            begin(1),
            finish.clone(),
            begin(1),
            finish.clone(),
            ClientMessage::Selection(SelectionAction::Copy),
        ]);

        let first = take_deferred_selection(&mut pending, 7);
        assert!(matches!(
            first.as_slice(),
            [
                ClientMessage::Selection(SelectionAction::Begin {
                    frame_revision: 7,
                    ..
                }),
                ClientMessage::Selection(SelectionAction::Finish { .. })
            ]
        ));
        assert!(matches!(
            pending.front(),
            Some(ClientMessage::Selection(SelectionAction::Begin { .. }))
        ));

        assert_eq!(take_deferred_selection(&mut pending, 8).len(), 2);
        assert_eq!(
            take_deferred_selection(&mut pending, 9),
            [ClientMessage::Selection(SelectionAction::Copy)]
        );
        assert!(pending.is_empty());

        pending.resize(
            MAX_DEFERRED_SELECTION_MESSAGES,
            ClientMessage::Selection(SelectionAction::Copy),
        );
        assert!(!queue_deferred_selection(
            &mut pending,
            ClientMessage::Selection(SelectionAction::Copy)
        ));

        let local_begin =
            VecDeque::from([ClientMessage::Selection(SelectionAction::Copy), begin(1)]);
        let server_finish = VecDeque::from([finish]);
        let ready = SelectionGate::Ready;
        let empty = VecDeque::new();
        assert!(!pointer_sequence_needs_cancel(&ready, true, &local_begin));
        assert!(pointer_sequence_needs_cancel(&ready, false, &server_finish));
        assert!(pointer_sequence_needs_cancel(
            &SelectionGate::AwaitingFinish,
            false,
            &empty
        ));
        assert!(pointer_sequence_needs_cancel(&ready, true, &empty));
        assert!(!pointer_sequence_needs_cancel(&ready, false, &empty));
    }

    #[test]
    fn selection_gate_waits_for_finish_and_its_authoritative_presentation() {
        let mut gate = SelectionGate::Ready;

        assert!(gate.admits(7));
        gate.finish_sent();
        assert!(!gate.admits(7));

        gate.finish_received(8);
        assert!(!gate.admits(7));
        assert!(gate.admits(8));

        gate.finish_sent();
        gate.finish_received(8);
        assert!(gate.admits(9));
    }

    #[test]
    fn native_clipboard_text_becomes_one_bounded_semantic_paste() {
        let text = "first\n界\0second";
        assert_eq!(
            clipboard_paste_message(text.as_bytes()),
            Ok(ClientMessage::Paste(text.as_bytes().to_vec()))
        );
        assert_eq!(
            clipboard_paste_message(&b""[..]).unwrap_err(),
            "The native clipboard contains no text"
        );
        let mut oversized = io::Cursor::new(vec![b'x'; session::MAX_PASTE_BYTES + 2]);
        assert_eq!(
            clipboard_paste_message(&mut oversized).unwrap_err(),
            format!(
                "Native clipboard text exceeds the {} byte paste limit",
                session::MAX_PASTE_BYTES
            )
        );
        assert_eq!(oversized.position(), session::MAX_PASTE_BYTES as u64 + 1);
        assert_eq!(
            clipboard_paste_message(&[0xff][..]).unwrap_err(),
            "stream did not contain valid UTF-8"
        );
        assert_eq!(
            clipboard_paste_notice("display unavailable"),
            "Venus could not paste from the native clipboard: display unavailable"
        );
    }

    #[test]
    fn clipboard_effects_preserve_native_targets() {
        use NativeClipboard::{Both, Primary, Standard};

        assert!(matches!(
            native_clipboard_for(
                NativePlatform::Linux,
                &ClipboardEffect::SelectionCopy {
                    location: ClipboardLocation::Selection,
                    text: String::new(),
                }
            ),
            Both
        ));
        assert!(matches!(
            native_clipboard_for(
                NativePlatform::Linux,
                &ClipboardEffect::SelectionCopy {
                    location: ClipboardLocation::Standard,
                    text: String::new(),
                }
            ),
            Standard
        ));
        assert!(matches!(
            native_clipboard_for(
                NativePlatform::Linux,
                &ClipboardEffect::TerminalWrite {
                    location: ClipboardLocation::Selection,
                    text: String::new(),
                }
            ),
            Primary
        ));
        for effect in [
            ClipboardEffect::SelectionCopy {
                location: ClipboardLocation::Selection,
                text: String::new(),
            },
            ClipboardEffect::SelectionCopy {
                location: ClipboardLocation::Primary,
                text: String::new(),
            },
            ClipboardEffect::TerminalWrite {
                location: ClipboardLocation::Standard,
                text: String::new(),
            },
        ] {
            assert_eq!(
                native_clipboard_for(NativePlatform::Macos, &effect),
                Standard
            );
        }
    }
}
