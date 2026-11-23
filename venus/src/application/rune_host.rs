//! The Rune candidate host for Venus's own `Application` (eon-yp6vg.7.9): supervised startup,
//! ready-v1, presentation control (Present/EOF), detach and Quit through VEN-C8, and focus
//! reports run the shipping policy unchanged, with Rune as the native host. Only the
//! non-default `candidate-rune` feature builds it.
//!
//! How Rune's callbacks reach the policy:
//! - `on_window_geometry` (output entry, applied size and scale) is the admission fact winit's
//!   `current_monitor` is, then `Resized` and the scale change, as winit delivers them;
//! - `on_focus`, `on_visibility` are `Focused` and `Occluded`;
//! - a native close request (and macOS Quit) is `CloseRequested`: Venus decides (detach, or quit
//!   with consent) and Rune always defers, so no native callback stops Orbit or exits by itself;
//! - `on_close` (the window going) is `Destroyed`: exit, never Quit;
//! - `AppSpec::poll`, which runs at every frame and wake, ends the turn as `about_to_wait` does,
//!   and asks a wake only for the next deadline, so Venus sleeps at rest;
//! - presentation control runs on its own thread and calls the session window's handle
//!   itself: Present is `WindowHandle::focus` and EOF `WindowHandle::close` on the same handle,
//!   whose requests run in order, each stamped as it is sent, under `App::attention_wait`
//!   (VEN-C14). Rune's `focus` only requests attention on Wayland, Eon's Linux backend.
//!   Elsewhere it activates and unminimizes, so VEN-C14 requires Venus to refuse Present
//!   ([`FOCUS_IS_ATTENTION`]) until Rune has an attention-only
//!   request (eon-yp6vg.57 and .54 bring the candidate to macOS).
//!
//! Limits kept for later beads: the hosted renderer draws the active terminal only, without
//! workspace chrome or dialogs (eon-yp6vg.8-.16); no keyboard, pointer or IME input
//! (eon-yp6vg.7.2); no platform accessibility adapter (VEN-C1's Rune-hosted gap); a new GPU
//! device or target format is fatal (eon-yp6vg.7.10). Since dialogs are not drawn, closing a
//! busy window opens a Quit prompt nobody can see or answer: that window then ends only through
//! the `eon` CLI or the supervisor's EOF.

use super::*;
use crate::rune_candidate::{
    Admission, Failure, Fit, SlotGeometry, admit,
    rune_host::{draw_slot, grid_view, hosted_renderer, refused_device, resize_to, slot_geometry},
};
use eon_workspace_protocol::SESSION_CLEANUP_TIMEOUT;
use rune::{
    App, AppHandle, AppSpec, CloseResponse, Compositor, Frame, QuitResponse, Tx, WindowGeometry,
    WindowHandle, compose::DeviceKey,
};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// Rune delivers only its own window's events; Venus's handlers take a window ID.
fn window_id() -> WindowId {
    WindowId::from(0)
}

/// Venus's side of the Rune window. Rune takes title and redraw only from a callback's `Tx`, so
/// Application's requests wait here until the callback returns.
#[derive(Default)]
pub(super) struct RuneWindow {
    output: Cell<bool>,
    redraw: Cell<bool>,
    title: RefCell<Option<String>>,
    shown_title: RefCell<String>,
}

impl RuneWindow {
    pub(super) fn has_output(&self) -> bool {
        self.output.get()
    }

    pub(super) fn request_redraw(&self) {
        self.redraw.set(true);
    }

    pub(super) fn set_title(&self, title: &str) {
        if *self.shown_title.borrow() != title {
            *self.title.borrow_mut() = Some(title.to_owned());
        }
    }

    /// The title to give the window now, if it changed.
    fn take_title(&self) -> Option<String> {
        let title = self.title.take()?;
        *self.shown_title.borrow_mut() = title.clone();
        Some(title)
    }
}

/// One window's ordered native request lane: Rune's `WindowHandle`.
/// Whether Rune's `WindowHandle::focus` only requests attention here, as Present must (VEN-C14:
/// no direct focus or unminimize). It does on Wayland, Eon's Linux backend. On macOS and Windows
/// it activates the window.
pub(super) const FOCUS_IS_ATTENTION: bool = cfg!(target_os = "linux");

pub(super) trait Lane: Send + Sync + 'static {
    fn focus(&self);
    fn close(&self);
}

impl Lane for WindowHandle {
    fn focus(&self) {
        WindowHandle::focus(self);
    }

    fn close(&self) {
        WindowHandle::close(self);
    }
}

/// Presentation control on the Rune host, shared by the control thread and the UI thread.
/// Present and EOF are sent on the session window's lane from the control thread, so each is
/// stamped when it is read and none waits for the UI thread (VEN-C14's bound).
pub(super) struct Control<L> {
    lane: Mutex<Option<L>>,
    /// Whether the lane's `focus` only requests attention, so Present may send it.
    focus_is_attention: bool,
    /// `present_admitted` for the window now, kept by the UI thread after every callback.
    admitted: AtomicBool,
    /// Ends the app before any window exists.
    quit: Box<dyn Fn() + Send + Sync>,
}

impl<L: Lane> Control<L> {
    pub(super) fn new(quit: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_focus(FOCUS_IS_ATTENTION, quit)
    }

    pub(super) fn with_focus(
        focus_is_attention: bool,
        quit: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            lane: Mutex::new(None),
            focus_is_attention,
            admitted: AtomicBool::new(false),
            quit: Box::new(quit),
        }
    }

    fn lane(&self) -> std::sync::MutexGuard<'_, Option<L>> {
        self.lane.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The session window's lane, from Rune's `on_window`. Requests sent on a Rune handle before
    /// its window exists are queued and run in order when it does.
    pub(super) fn bind(&self, lane: L) {
        *self.lane() = Some(lane);
    }

    pub(super) fn admit(&self, admitted: bool) {
        self.admitted.store(admitted, Ordering::Release);
    }

    /// A presentation control event, on the control thread; false ends the reader.
    pub(super) fn control(&self, event: UserEvent) -> bool {
        match event {
            UserEvent::Present => {
                if self.admitted.load(Ordering::Acquire) {
                    self.present();
                }
                true
            }
            UserEvent::Exit => {
                self.exit();
                false
            }
            _ => true,
        }
    }

    /// Asks for attention on the lane, where its `focus` asks for nothing more.
    fn present(&self) {
        if !self.focus_is_attention {
            report("Present refused: this host has no attention-only request");
        } else if let Some(lane) = &*self.lane() {
            lane.focus();
        }
    }

    /// Closes the session window on its lane, after every Present sent before it. Before the
    /// window exists, no Present can have been sent, and the app ends at once.
    pub(super) fn exit(&self) {
        match &*self.lane() {
            Some(lane) => lane.close(),
            None => (self.quit)(),
        }
    }
}

/// The last native geometry Rune reported.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    size: (u32, u32),
    scale: f64,
    has_output: bool,
}

impl From<WindowGeometry> for Geometry {
    fn from(geometry: WindowGeometry) -> Self {
        Self {
            size: geometry.physical_size,
            scale: geometry.scale_factor,
            has_output: geometry.has_output,
        }
    }
}

/// Native facts that arrive before the first frame makes Venus's window, kept latest-only and
/// applied once it exists.
#[derive(Default)]
struct Early {
    geometry: Option<Geometry>,
    focused: Option<bool>,
    visible: Option<bool>,
}

/// The Rune app's state: Venus's `Application` and the host's own bookkeeping.
pub(super) struct Host {
    app: Application,
    control: Arc<Control<WindowHandle>>,
    failure: Failure,
    /// Consumed by the first frame's hosted renderer.
    fonts: Option<FontSetup>,
    early: Early,
    geometry: Option<Geometry>,
    identity: Option<(DeviceKey, wgpu::TextureFormat)>,
    /// The slot's last move between preparing and drawing, and how many frames in a row it
    /// repeated.
    moved: Option<((SlotGeometry, SlotGeometry), u32)>,
    /// Rune is closing the window: an exit needs no second close.
    closing: bool,
}

impl Host {
    fn rune_window(&self) -> Option<&RuneWindow> {
        match &self.app.window.as_ref()?.native {
            NativeWindow::Rune(window) => Some(window),
            NativeWindow::Winit { .. } => None,
        }
    }

    fn apply(&mut self, request: NativeRequest) {
        match request {
            // Rune makes its window when it runs.
            NativeRequest::Resume => {}
            // Present comes from the control thread, never from Application's queue here.
            NativeRequest::Present => {}
            NativeRequest::Redraw => {
                if let Some(window) = self.rune_window() {
                    window.request_redraw();
                }
            }
            NativeRequest::Exit if self.closing => {}
            NativeRequest::Exit => self.control.exit(),
        }
    }

    /// The end of every Rune callback: applies Venus's native requests, records a fatal error
    /// for the exit status, keeps the control thread's Present admission current, and gives the
    /// window its title. True when the view must redraw.
    fn finish(&mut self, tx: &mut Tx) -> bool {
        if let Some(error) = &self.app.fatal_error {
            self.failure.set(error.clone());
        }
        while let Some(request) = self.app.native_requests.take() {
            self.apply(request);
        }
        self.admit_present();
        let Some(window) = self.rune_window() else {
            return false;
        };
        if let Some(title) = window.take_title() {
            tx.set_title(title);
        }
        window.redraw.replace(false)
    }

    /// Keeps the control thread's Present admission current: shown and unfocused (VEN-C14). It
    /// must be current before ready-v1 is written, since a supervisor may present at once.
    fn admit_present(&self) {
        self.control.admit(present_admitted(
            self.app.window_focused,
            self.app.window.is_some(),
        ));
    }

    /// `finish` for a callback whose state change Rune assumes unless told otherwise.
    fn finish_live(&mut self, tx: &mut Tx) {
        if !self.finish(tx) {
            tx.live_only();
        }
    }

    fn window_event(&mut self, event: WindowEvent) {
        self.app.handle_window_event(window_id(), event);
    }

    /// Applies one native geometry report: the output fact, then a scale change and a resize as
    /// winit orders them. The first report after the window exists compares against scale 1,
    /// the scale the initial size was chosen at (VEN-C19).
    fn apply_geometry(&mut self, geometry: Geometry) {
        let Some(window) = self.rune_window() else {
            self.early.geometry = Some(geometry);
            return;
        };
        window.output.set(geometry.has_output);
        let (scale_changed, size_changed) = match self.geometry {
            None => (geometry.scale != 1.0, true),
            Some(last) => (last.scale != geometry.scale, last.size != geometry.size),
        };
        self.geometry = Some(geometry);
        if scale_changed {
            let lane = self.control.lane().clone();
            let scale = geometry.scale;
            self.app.change_scale(scale, |size| {
                let lane = lane.ok_or("the Rune window has no handle yet")?;
                lane.resize_instant((
                    (f64::from(size.width) / scale) as f32,
                    (f64::from(size.height) / scale) as f32,
                ));
                Ok(())
            });
        }
        // A scale change resizes too, at the same physical size: the renderer and the Orbit grid
        // follow the new cell metrics.
        if (size_changed || scale_changed) && self.app.fatal_error.is_none() {
            let (width, height) = geometry.size;
            self.window_event(WindowEvent::Resized(PhysicalSize::new(width, height)));
        }
    }

    fn user_event(&mut self, event: UserEvent, tx: &mut Tx) {
        // Requests the event's own catch-up drains go to a queue of their own: put back into
        // Application's, the drain loop that hands them out would never end.
        let mut requested = NativeRequests::default();
        self.app.handle_user_event(event, collect(&mut requested));
        while let Some(request) = requested.take() {
            self.apply(request);
        }
        self.finish_live(tx);
    }

    /// The first frame: Venus's window, with its hosted renderer on Rune's device, then the
    /// tail of `create_window` and the native facts that came first.
    fn create_window(&mut self, renderer: Renderer, geometry: SlotGeometry) {
        let (width, height) = geometry.size();
        let accessibility = Accessibility::new(
            PhysicalSize::new(width, height),
            self.app.detect_local_hosts,
        );
        let scale_factor = f64::from(geometry.scale());
        report(format_args!("window scale factor {scale_factor}"));
        self.app.window = Some(WindowState {
            renderer,
            scale_factor,
            accessibility,
            native: NativeWindow::Rune(RuneWindow::default()),
        });
        if self.app.initial_scale_pending {
            self.app.reveal_workspace_selection();
        } else {
            self.app.start_initial_attachment();
        }
        self.app.refresh_client_view();
        let early = std::mem::take(&mut self.early);
        if let Some(geometry) = early.geometry {
            self.apply_geometry(geometry);
        }
        if let Some(focused) = early.focused {
            self.window_event(WindowEvent::Focused(focused));
        }
        if let Some(visible) = early.visible {
            self.window_event(WindowEvent::Occluded(!visible));
        }
        self.admit_present();
    }

    /// A fatal render or admission failure, from a frame: recorded for the exit status, and the
    /// exit asked for now, since no callback may follow a frame.
    fn fail(&mut self, message: String) {
        report(&message);
        let message = self.app.fatal_error.get_or_insert(message).clone();
        self.failure.set(message);
        self.apply(NativeRequest::Exit);
    }

    /// Admits the frame's device and format, makes the window on the first frame, and prepares
    /// the active terminal at the slot's geometry. None draws only Rune's clear.
    fn prepare(f: &mut Frame<'_, Self>) -> Option<(SlotGeometry, PresentationIdentity)> {
        let geometry = match slot_geometry(f) {
            Ok(geometry) => geometry,
            Err(message) => {
                f.rt.state.fail(message);
                return None;
            }
        };
        let identity = (f.device_key(), f.format);
        match admit(f.rt.state.identity.as_ref(), &identity) {
            Admission::Refuse => {
                let message = refused_device(f.format);
                f.rt.state.fail(message);
                return None;
            }
            Admission::Create => {
                let fonts = f.rt.state.fonts.take()?;
                match hosted_renderer(f, geometry, fonts) {
                    Ok(renderer) => {
                        f.rt.state.identity = Some(identity);
                        f.rt.state.create_window(renderer, geometry);
                    }
                    Err(error) => {
                        f.rt.state.fail(error);
                        return None;
                    }
                }
            }
            Admission::Draw => {}
        }
        let host = &mut f.rt.state;
        let app = &mut host.app;
        // Against the renderer itself: a window Resized may have sized it since the last frame.
        let renderer = &mut app.window.as_mut()?.renderer;
        let (width, height) = geometry.size();
        if renderer.size() != PhysicalSize::new(width, height)
            || renderer.metrics() != renderer.metrics_at_scale(f64::from(geometry.scale()))
        {
            resize_to(renderer, geometry);
        }
        app.sync_images();
        let candidate = app.presentation_candidate(app.workspace_scene().as_ref());
        let scene = app.terminal.model.scene()?;
        let renderer = &mut app.window.as_mut()?.renderer;
        if let Err(error) = renderer.prepare_hosted(scene, app.blink_visible, candidate.generation)
        {
            host.fail(error.to_string());
            return None;
        }
        Some((geometry, candidate))
    }
}

/// Draws Venus's window in Rune's frame pass.
struct HostCompositor;

/// Frames in a row a slot may draw away from where it was prepared before that is a fault.
const MOVED_FRAMES: u32 = 3;

impl Compositor<Host> for HostCompositor {
    fn render(&mut self, f: &mut Frame<'_, Host>) {
        let prepared = Host::prepare(f);
        // The renderer leaves the state while Rune's pass borrows the frame.
        let mut window = f.rt.state.app.window.take();
        let drawn = draw_slot(
            f,
            window
                .as_ref()
                .zip(prepared)
                .map(|(window, (geometry, _))| (&window.renderer, geometry)),
            Fit::Near,
        );
        if let Some(window) = &mut window {
            window.renderer.end_frame_hosted();
        }
        let host = &mut f.rt.state;
        host.app.window = window;
        if let Some(refusal) = drawn.refusal {
            host.fail(refusal);
        }
        // A resize that lands between preparing and drawing skips this frame's grid and asks for
        // another, prepared where the slot now is. The same move frame after frame is a fault.
        let again = match prepared.zip(drawn.moved) {
            Some(((prepared, _), at)) => {
                let repeats = match host.moved {
                    Some((last, count)) if last == (prepared, at) => count + 1,
                    _ => 1,
                };
                host.moved = Some(((prepared, at), repeats));
                if repeats >= MOVED_FRAMES {
                    host.fail(at.moved_from(prepared));
                }
                repeats < MOVED_FRAMES
            }
            None => {
                host.moved = None;
                false
            }
        };
        if drawn.drew
            && let Some((_, candidate)) = prepared
        {
            host.app.presentation.publish(candidate);
            if let Some(revision) = candidate.revision {
                host.app.flush_deferred_selection(revision);
            }
        }
        if again {
            f.rt.invalidate();
        }
    }
}

/// The native callback for Application's own drains: requests go to `requested`, never back
/// into the queue being drained.
fn collect(requested: &mut NativeRequests) -> impl FnMut(&mut Application, NativeRequest) + '_ {
    |_, request| requested.request(request)
}

fn on_geometry(host: &mut Host, geometry: WindowGeometry, tx: &mut Tx) -> bool {
    host.apply_geometry(geometry.into());
    host.finish(tx)
}

fn on_focus(host: &mut Host, focused: bool, tx: &mut Tx) {
    if host.app.window.is_none() {
        host.early.focused = Some(focused);
    } else {
        host.window_event(WindowEvent::Focused(focused));
    }
    host.finish_live(tx);
}

fn on_visibility(host: &mut Host, visible: bool, tx: &mut Tx) {
    if host.app.window.is_none() {
        host.early.visible = Some(visible);
    } else {
        host.window_event(WindowEvent::Occluded(!visible));
    }
    host.finish_live(tx);
}

/// A compositor close request (or macOS Quit) is Venus's to decide under VEN-C8; Rune keeps the
/// window until Venus exits through its lane.
fn on_close_request(host: &mut Host, tx: &mut Tx) -> CloseResponse {
    host.window_event(WindowEvent::CloseRequested);
    host.finish_live(tx);
    CloseResponse::Defer
}

fn on_quit_request(host: &mut Host, tx: &mut Tx) -> QuitResponse {
    host.window_event(WindowEvent::CloseRequested);
    host.finish_live(tx);
    QuitResponse::Defer
}

/// The window is going: exit, as winit's `Destroyed`; never a Quit, and no second close.
fn on_close(host: &mut Host, tx: &mut Tx) {
    host.closing = true;
    host.window_event(WindowEvent::Destroyed);
    host.finish_live(tx);
}

/// Every frame and wake: the end of a native turn, as winit's `about_to_wait`.
fn poll(host: &mut Host, _clock: f64, tx: &mut Tx) -> bool {
    if host.app.window.is_none() {
        if host.app.fatal_error.is_some() {
            // The first frame failed: the exit is under way; nothing more to wake for.
            return host.finish(tx);
        }
        // The first frame makes the window; admission is sampled on the turn after it.
        tx.wake_after(Duration::ZERO);
        return true;
    }
    host.admit_present();
    if let Some(deadline) = host.app.end_turn() {
        tx.wake_after(deadline.saturating_duration_since(Instant::now()));
    }
    host.finish(tx)
}

/// Runs a launch through the Rune host. Errors are startup refusals and fatal window, render or
/// admission failures, as on winit.
pub(crate) fn run(arguments: LaunchArguments) -> Result {
    if arguments.supervised && arguments.workspace_socket.is_some() && !arguments.startup_admission
    {
        return Err(
            "the Rune candidate needs startup admission (stdin-ready-focus-v2) in an Eon \
                    workspace"
                .into(),
        );
    }
    #[cfg(target_os = "linux")]
    rune::tooling::set_app_id(Box::leak(arguments.application_id.clone().into_boxed_str()));
    let handle: Arc<OnceLock<AppHandle<Host>>> = Arc::default();
    let events = {
        let handle = Arc::clone(&handle);
        EventSink::Host(Arc::new(move |event| match handle.get() {
            Some(handle) => {
                handle.update(move |host: &mut Host, tx| host.user_event(event, tx));
                true
            }
            None => false,
        }))
    };
    let mut application = Application::new(arguments, events);
    if application.startup_admission && application.workspace_socket.is_some() {
        let response = crate::read_workspace_response(&mut io::stdin().lock())?;
        if !matches!(response, eon_workspace_protocol::v47::Response::Snapshot(_)) {
            return Err("Venus startup admission requires an Eon workspace snapshot".into());
        }
        application.workspace_model.apply(response);
    }
    // VEN-C19: fonts and the initial grid are known before the window exists.
    let fonts = FontSetup::with_directories(
        &application.fonts,
        std::env::var_os("EON_VENUS_FONT_DIRS").as_deref(),
    )?;
    let initial = initial_window_size(
        application.columns,
        application.rows,
        fonts.metrics(1.0),
        1.0,
        application.workspace_model.snapshot(),
    )?;
    let mut spec = AppSpec::new(
        "Venus",
        (initial.width as f32, initial.height as f32),
        || unreachable!("App::init supplies the state"),
        |_, _| grid_view(),
    );
    spec.poll = Some(poll);
    let mut app = App::new(spec)
        .args(Vec::new())
        .srgb_blend(false)
        .attention_wait(SESSION_CLEANUP_TIMEOUT)
        .on_window_geometry(on_geometry)
        .on_focus(on_focus)
        .on_visibility(on_visibility)
        .on_close_request(on_close_request)
        .on_quit_request(on_quit_request)
        .on_close(on_close)
        .compositor(HostCompositor);
    let _ = handle.set(app.handle());
    let control = {
        let quit = app.handle();
        Arc::new(Control::new(move || quit.quit()))
    };
    app = app.on_window({
        let control = Arc::clone(&control);
        move |window: &WindowHandle| control.bind(window.clone())
    });
    if application.supervised {
        let control = Arc::clone(&control);
        thread::Builder::new()
            .name("venus-presentation-control".into())
            .spawn(move || {
                run_presentation_control(io::stdin().lock(), |event| control.control(event));
            })?;
        if !application.startup_admission {
            application.focus_reporter = admit_presentation(
                &mut io::stdout(),
                false,
                application.workspace_socket.is_some(),
                application.window_focused,
            )?;
        }
    }
    let failure = Failure::default();
    let host = Host {
        app: application,
        control,
        failure: failure.clone(),
        fonts: Some(fonts),
        early: Early::default(),
        geometry: None,
        identity: None,
        moved: None,
        closing: false,
    };
    app = app.init(move || host);
    let _ = EXIT_FAILURE.set(failure.clone());
    rune::fast_exit(exit_status);
    app.try_run()?;
    failure.into_result()
}

/// The failure a fast exit reports: set once, by `run`.
static EXIT_FAILURE: OnceLock<Failure> = OnceLock::new();

/// Rune's fast exit, once the window's hooks (and its attention wait) have run: exit 1 with the
/// recorded fatal error, else Rune ends the process with 0. An orderly exit would drop the window
/// and GPU device first, and that teardown waits on the compositor: with it stopped, control EOF
/// would no longer exit within `SESSION_CLEANUP_TIMEOUT` (VEN-C14).
fn exit_status() {
    if let Some(message) = EXIT_FAILURE.get().and_then(Failure::get) {
        eprintln!("venus-rune-candidate: {message}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
