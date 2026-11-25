//! The Rune candidate host for Venus's own `Application` (eon-yp6vg.7.9): supervised startup,
//! ready-v1, presentation control (Present/EOF), detach and Quit through VEN-C8, and focus
//! reports run the shipping policy unchanged, with Rune as the native host. Only the
//! non-default `candidate-rune` feature builds it.
//!
//! How Rune's callbacks reach the policy:
//! - `on_window_geometry` (output entry, applied size and scale) is the admission fact winit's
//!   `current_monitor` is, then `Resized` and the scale change, as winit delivers them;
//! - `on_window_event` sees each raw size and scale first, in the same dispatch as input: a
//!   change withdraws input at once. A draw of the Orbit grid for the applied geometry
//!   re-admits it;
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
//! workspace chrome or dialogs (eon-yp6vg.8-.16) or a platform accessibility adapter
//! (VEN-C1's Rune-hosted gap).
//! The host admits input from completed frames and treats GPU device loss as terminal.
//! Since dialogs are not drawn, closing a
//! busy window opens a Quit prompt nobody can see or answer: that window then ends only through
//! the `eon` CLI or the supervisor's EOF.

use super::*;
use crate::rune_candidate::{
    Failure, Fit, SlotGeometry,
    rune_host::{Drawn, FrameTarget, draw_slot, grid_view, hosted_renderer, preflight, resize_to},
};
use NativeClipboard::{Both, Standard};
use eon_workspace_protocol::SESSION_CLEANUP_TIMEOUT;
use rune::{
    App, AppHandle, AppSpec, CloseResponse, Compositor, Frame, FrameOutcome, FrameSkip,
    QuitResponse, Tx, WindowGeometry, WindowHandle,
    clipboard::plain::{self, Outcome, Selection},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
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
    pub(super) withdrawn: Cell<bool>,
    redraw: Cell<bool>,
    title: RefCell<Option<String>>,
    shown_title: RefCell<String>,
    ime_area: Cell<Option<PhysicalCaretArea>>,
    reading_paste: Cell<bool>,
    paste_target: Cell<Option<(u64, u64)>>,
    clipboard_writes: Cell<usize>,
}

impl RuneWindow {
    pub(super) fn sync_ime(&self, allowed: bool, area: Option<PhysicalCaretArea>) {
        self.ime_area.set(allowed.then(|| {
            area.unwrap_or((PhysicalPosition::new(0.0, 0.0), PhysicalSize::new(1.0, 1.0)))
        }));
    }

    pub(super) fn retire_paste(&self) {
        self.paste_target.set(None);
    }

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

fn ime_rect(area: PhysicalCaretArea, scale: f32) -> [f32; 4] {
    let (pos, size) = area;
    let scale = f64::from(scale);
    [pos.x, pos.y, size.width, size.height].map(|value| (value / scale) as f32)
}

#[derive(Debug)]
pub(super) enum Clipboard {
    Read(Outcome),
    Written(u64, NativeClipboard, bool),
}

fn paste_target(app: &Application) -> Option<(u64, u64)> {
    let attachment = app.terminal.model.image_attachment()?;
    (terminal_focused(app.window_focused, app.workspace_focus)
        && !app.dialog_open()
        && !app.input_locked())
    .then_some((app.attachments, attachment))
}

pub(super) fn read_clipboard(app: &mut Application) -> bool {
    let Some(NativeWindow::Rune(window)) = app.window.as_ref().map(|state| &state.native) else {
        return false;
    };
    let Some(target) = paste_target(app) else {
        return true;
    };
    if window.reading_paste.replace(true) {
        app.complete_paste(Err("A native paste is already pending".into()));
        return true;
    }
    window.paste_target.set(Some(target));
    let events = app.events.clone();
    let complete = move |outcome| {
        events.send(UserEvent::RuneClipboard(Clipboard::Read(outcome)));
    };
    plain::read_text(Selection::Regular, session::MAX_PASTE_BYTES, complete);
    true
}

// Bound memory while a native read is holding the shared worker. Each effect already has
// Orbit's byte cap and can offer at most two destinations.
const MAX_CLIPBOARD_WRITES: usize = 8;

impl RuneWindow {
    pub(super) fn write_clipboard(
        &self,
        events: &EventSink,
        model: &mut SessionModel,
        target: NativeClipboard,
        text: String,
    ) {
        let Some(attachment) = model.image_attachment() else {
            return;
        };
        let pending = self.clipboard_writes.get();
        if pending == MAX_CLIPBOARD_WRITES {
            model.set_venus_notice(
                LocalNoticeSource::Clipboard,
                clipboard_notice(target, Err("Too many native clipboard writes are pending")),
            );
            return;
        }
        self.clipboard_writes.set(pending + 1);
        let events = events.clone();
        let completed = move |success| {
            events.send(UserEvent::RuneClipboard(Clipboard::Written(
                attachment, target, success,
            )));
        };
        // Both requests are submitted before another effect can enqueue. The FIFO worker
        // completes the first before it can successfully write the second.
        let regular = (target == Both).then(|| {
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            plain::write_text(Selection::Regular, text.clone(), move |outcome| {
                let _ = send.try_send(outcome == Outcome::Written);
            });
            receive
        });
        let selection = if target == Standard {
            Selection::Regular
        } else {
            Selection::Primary
        };
        plain::write_text(selection, text, move |outcome| {
            completed(
                outcome == Outcome::Written
                    && regular.is_none_or(|result| result.try_recv() == Ok(true)),
            );
        });
    }
}

pub(super) fn complete_clipboard(app: &mut Application, result: Clipboard) {
    let Some(NativeWindow::Rune(window)) = app.window.as_ref().map(|state| &state.native) else {
        return;
    };
    match result {
        Clipboard::Read(outcome) => {
            window.reading_paste.set(false);
            let request = window.paste_target.take();
            if request.is_none() || request != paste_target(app) {
                return;
            }
            app.complete_paste(match outcome {
                Outcome::Text(text) => check_clipboard_paste_size(text.len())
                    .map(|()| ClientMessage::Paste(text.into_bytes())),
                Outcome::Empty | Outcome::NonText => Err(CLIPBOARD_EMPTY.into()),
                Outcome::TooLarge => Err(clipboard_paste_limit()),
                Outcome::InvalidUtf8 => Err(CLIPBOARD_INVALID_UTF8.into()),
                _ => Err("Native clipboard text is unavailable".into()),
            });
        }
        Clipboard::Written(attachment, target, success) => {
            let pending = &window.clipboard_writes;
            pending.set(pending.get().saturating_sub(1));
            if let Some(view) = std::iter::once(&mut app.terminal)
                .chain(&mut app.column_views)
                .find(|view| view.model.image_attachment() == Some(attachment))
            {
                let result = success.then_some(()).ok_or("Native clipboard write failed");
                let notice = clipboard_notice(target, result);
                view.model
                    .set_venus_notice(LocalNoticeSource::Clipboard, notice);
                app.refresh_client_view();
            }
        }
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

    fn retire(&self) {
        self.admit(false);
        self.lane().take();
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

fn window_slot((width, height): (u32, u32), scale: f32) -> SlotGeometry {
    SlotGeometry::new([0.0, 0.0, width as f32, height as f32], scale)
}

fn slot_for_target(slot: SlotGeometry, size: (u32, u32)) -> SlotGeometry {
    // The grid fills the window. Slot rounding must not move Orbit to a different grid.
    let target = window_slot(size, slot.scale());
    if slot.same_slot(target) { target } else { slot }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Completion {
    Drawn(Option<PresentationIdentity>),
    Covered,
    Surface,
}

/// The compositor and Rune's post-present hook run in order on the UI thread.
#[derive(Default)]
struct FrameHandoff {
    staged: Cell<Option<PresentationIdentity>>,
    completed: Cell<Option<Completion>>,
    queued: Cell<bool>,
}

impl FrameHandoff {
    fn report(&self, outcome: FrameOutcome, skip: Option<FrameSkip>) -> bool {
        let staged = self.staged.take();
        let completion = match (outcome, skip) {
            (FrameOutcome::Drawn, _) => Completion::Drawn(staged),
            (FrameOutcome::Unchanged, _) | (FrameOutcome::Skipped, Some(FrameSkip::Timeout)) => {
                return false;
            }
            (FrameOutcome::Skipped, Some(FrameSkip::Covered)) => Completion::Covered,
            _ => Completion::Surface,
        };
        // A less severe skip can arrive before a pending surface withdrawal is applied.
        if self.completed.get() != Some(Completion::Surface) {
            self.completed.set(Some(completion));
        }
        !self.queued.replace(true)
    }

    fn discard(&self) {
        self.staged.take();
        self.completed.take();
    }
}

/// Native facts that arrive before the first frame makes Venus's window, kept latest-only and
/// applied once it exists.
#[derive(Default)]
struct Early {
    geometry: Option<Geometry>,
    focused: Option<bool>,
    visible: Option<bool>,
    modifiers: Option<winit::event::Modifiers>,
}

/// The Rune app's state: Venus's `Application` and the host's own bookkeeping.
pub(super) struct Host {
    app: Application,
    handle: AppHandle<Self>,
    control: Arc<Control<WindowHandle>>,
    failure: Failure,
    failed: bool,
    frames: Rc<FrameHandoff>,
    /// Consumed by the first frame's hosted renderer.
    fonts: Option<FontSetup>,
    early: Early,
    geometry: Option<Geometry>,
    /// Rune's raw size and scale while they differ from `geometry`, before the idle report
    /// applies them: no frame re-admits input meanwhile.
    raw: Option<Geometry>,
    identity: Option<FrameTarget>,
    slot: Option<SlotGeometry>,
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

    fn finish(&mut self, tx: &mut Tx) -> bool {
        self.finish_with_deadline(tx, self.app.terminal.input_deadline())
    }

    /// The end of a Rune callback: applies Venus's native requests, records a fatal error
    /// for the exit status, keeps the control thread's Present admission current, and gives the
    /// window its title. True when the view must redraw.
    fn finish_with_deadline(&mut self, tx: &mut Tx, deadline: Option<Instant>) -> bool {
        if self.check_failure() {
            tx.quit();
            return false;
        }
        while let Some(request) = self.app.native_requests.take() {
            self.apply(request);
        }
        if let Some(deadline) = deadline {
            tx.wake_after(deadline.saturating_duration_since(Instant::now()));
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

    fn native_event(&mut self, event: &WindowEvent, tx: &mut Tx) -> bool {
        match event {
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::Ime(_)
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::ModifiersChanged(_) => {}
            _ => return false,
        }
        let synthetic = matches!(
            event,
            WindowEvent::KeyboardInput {
                is_synthetic: true,
                ..
            }
        );
        if !synthetic && self.begin() {
            if let WindowEvent::ModifiersChanged(modifiers) = event
                && self.app.window.is_none()
            {
                self.early.modifiers = Some(*modifiers);
            } else {
                self.window_event(event.clone());
            }
        }
        self.finish_live(tx);
        // Rune consumes input but keeps its own modifier bookkeeping even when this returns true.
        true
    }

    fn begin(&mut self) -> bool {
        if self.check_failure() {
            return false;
        }
        self.complete_frame();
        true
    }

    fn receive_frame(&mut self, tx: &mut Tx) -> bool {
        self.frames.queued.set(false);
        self.begin();
        self.finish(tx)
    }

    fn complete_frame(&mut self) {
        match self.frames.completed.take() {
            Some(Completion::Drawn(Some(candidate))) => {
                if self.app.window_occluded
                    || !self.geometry.is_some_and(|geometry| geometry.has_output)
                    || candidate.generation != self.app.presentation.generation
                {
                    return;
                }
                self.app.presentation.publish(candidate);
                if let Some(window) = self.rune_window() {
                    window.withdrawn.set(false);
                }
                if let Some(revision) = candidate.revision {
                    let now = Instant::now();
                    self.app.terminal.selection_presented(revision, now);
                    self.app.flush_deferred_selection(revision);
                    self.app.native_requests.redraw |= self.app.advance_terminal_scroll(now);
                }
            }
            Some(Completion::Surface | Completion::Drawn(None)) => self.withdraw_surface(),
            Some(Completion::Covered) => {
                self.app.presentation.unpublish();
                self.app.stop_covered_motion();
                self.app.reset_cursor_animation();
            }
            None => {}
        }
    }

    fn withdraw_surface(&mut self) {
        self.frames.discard();
        if self
            .rune_window()
            .is_some_and(|window| !window.withdrawn.get())
        {
            self.app.presentation.invalidate();
        }
        self.app.cancel_native_pointer_sequence();
        self.app.input.reset_scroll();
        self.app.cancel_terminal_scroll();
    }

    /// Applies one native geometry report: the output fact, then a scale change and a resize as
    /// winit orders them. The first report after the window exists compares against scale 1,
    /// the scale the initial size was chosen at (VEN-C19).
    fn apply_geometry(&mut self, geometry: Geometry) {
        let Some(window) = self.rune_window() else {
            self.early.geometry = Some(geometry);
            return;
        };
        let had_output = window.output.replace(geometry.has_output);
        let (scale_changed, size_changed) = match self.geometry {
            None => (geometry.scale != 1.0, true),
            Some(last) => (last.scale != geometry.scale, last.size != geometry.size),
        };
        self.geometry = Some(geometry);
        self.raw = None;
        if size_changed || scale_changed {
            self.frames.discard();
        } else if had_output && !geometry.has_output {
            self.withdraw_surface();
        } else {
            self.complete_frame();
        }
        if scale_changed {
            let lane = self.control.lane().clone();
            let scale = geometry.scale;
            self.app.apply_scale(scale, |size| {
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
        if (size_changed || scale_changed) && !self.check_failure() {
            let (width, height) = geometry.size;
            self.window_event(WindowEvent::Resized(PhysicalSize::new(width, height)));
        }
        if geometry.has_output
            && !had_output
            && let Some(window) = self.rune_window()
        {
            window.request_redraw();
        }
    }

    /// Rune's raw size or scale, before Rune applies it and before `on_window_geometry` reports
    /// it. Input admitted at the applied geometry stops at once. A draw can qualify after the
    /// window returns to that geometry or the idle report applies the new one, with the
    /// canonical Resize. True when state changed.
    fn raw_geometry(&mut self, size: Option<(u32, u32)>, scale: Option<f64>) -> bool {
        let Some(applied) = self.geometry else {
            return false;
        };
        let last = self.raw.unwrap_or(applied);
        let raw = Geometry {
            size: size.unwrap_or(last.size),
            scale: scale.unwrap_or(last.scale),
            ..last
        };
        if raw != applied {
            self.raw = Some(raw);
        } else if self.raw.take().is_none() {
            return false;
        } else if let Some(window) = self.rune_window() {
            // The idle report will see no change and Rune's own resize asks for no frame.
            window.request_redraw();
        }
        // Input queued since the last move used a geometry the window no longer has.
        self.withdraw_surface();
        true
    }

    fn user_event(&mut self, event: UserEvent, tx: &mut Tx) {
        if !self.begin() {
            self.finish_live(tx);
            return;
        }
        // Requests the event's own catch-up drains go to a queue of their own: put back into
        // Application's, the drain loop that hands them out would never end.
        let mut requested = NativeRequests::default();
        self.app.handle_user_event(event, collect(&mut requested));
        if !self.check_failure() {
            while let Some(request) = requested.take() {
                self.apply(request);
            }
        }
        self.finish_live(tx);
    }

    /// The first frame: Venus's window, with its hosted renderer on Rune's device, then the
    /// tail of `create_window` and the native facts that came first.
    fn create_window(&mut self, renderer: Renderer, geometry: SlotGeometry) {
        let scale_factor = f64::from(geometry.scale());
        report(format_args!("window scale factor {scale_factor}"));
        self.app.window = Some(WindowState {
            renderer,
            scale_factor,
            native: NativeWindow::Rune(RuneWindow::default()),
        });
        let early = std::mem::take(&mut self.early);
        if let Some(geometry) = early.geometry {
            self.apply_geometry(geometry);
        }
        if self.check_failure() {
            return;
        }
        if self.app.initial_scale_pending {
            self.app.reveal_workspace_selection();
        } else {
            self.app.start_initial_attachment();
        }
        self.app.refresh_client_view();
        if let Some(focused) = early.focused {
            self.window_event(WindowEvent::Focused(focused));
        }
        if let Some(modifiers) = early.modifiers {
            self.window_event(WindowEvent::ModifiersChanged(modifiers));
        }
        if let Some(visible) = early.visible {
            self.window_event(WindowEvent::Occluded(!visible));
        }
        self.admit_present();
    }

    fn check_failure(&mut self) -> bool {
        if !self.failed
            && let Some(message) = self
                .app
                .fatal_error
                .take()
                .or_else(|| self.handle.window_error())
        {
            self.latch_failure(message);
        }
        self.failed
    }

    fn latch_failure(&mut self, message: String) {
        self.failed = true;
        self.failure.set(message);
        self.control.retire();
        self.frames.discard();
        self.app.presentation.invalidate();
        self.app.terminal.deferred_selection.clear();
        self.app.input.cancel_selection();
        self.app.terminal.transport = None;
        self.app.column_views.clear();
        self.app.workspace_transport = None;
        self.app.window = None;
        self.app.native_requests = NativeRequests::default();
    }

    /// There is no Tx in a compositor callback. App quit also works if Rune
    /// has already removed the window and discarded its lane.
    fn fail(&mut self, message: String) {
        if !self.failed {
            self.latch_failure(message);
            (self.control.quit)();
        }
    }

    /// Admits the frame's device and format, makes the window on the first frame, and prepares
    /// the active terminal at the slot's geometry. None draws only Rune's clear.
    fn prepare(f: &mut Frame<'_, Self>) -> Option<(SlotGeometry, PresentationIdentity)> {
        let (geometry, target) = match preflight(f, f.rt.state.identity) {
            Ok(preflight) => preflight,
            Err(message) => {
                f.rt.state.fail(message);
                return None;
            }
        };
        let geometry = slot_for_target(geometry, f.size);
        if f.rt.state.identity.is_none() {
            let fonts = f.rt.state.fonts.take()?;
            match hosted_renderer(f, geometry, fonts) {
                Ok(renderer) => {
                    f.rt.state.identity = Some(target);
                    f.rt.state.create_window(renderer, geometry);
                }
                Err(error) => {
                    f.rt.state.fail(error);
                    return None;
                }
            }
        }
        let host = &mut f.rt.state;
        if host.check_failure() {
            (host.control.quit)();
            return None;
        }
        host.prepare_grid(geometry)
            .map(|candidate| (geometry, candidate))
    }

    fn prepare_grid(&mut self, geometry: SlotGeometry) -> Option<PresentationIdentity> {
        if self
            .slot
            .replace(geometry)
            .is_some_and(|last| last != geometry)
        {
            self.withdraw_surface();
        }
        let app = &mut self.app;
        // Against the renderer itself: a window Resized may have sized it since the last frame.
        let renderer = &mut app.window.as_mut()?.renderer;
        let (width, height) = geometry.size();
        if renderer.size() != PhysicalSize::new(width, height)
            || renderer.metrics() != renderer.metrics_at_scale(f64::from(geometry.scale()))
        {
            resize_to(renderer, geometry);
            app.workspace_update |= LAYOUT_WORKSPACE;
        }
        app.sync_images();
        app.prepare_workspace();
        let candidate = app.presentation_candidate(app.workspace_scene().as_deref());
        let scene = app.terminal.model.scene()?;
        let status = app.status_for(false, candidate);
        let renderer = &mut app.window.as_mut()?.renderer;
        let generation = candidate.generation;
        let preedit = app.input.preedit();
        if let Err(error) =
            renderer.prepare_hosted(scene, app.blink_visible, generation, &status, preedit)
        {
            self.fail(error.to_string());
            return None;
        }
        Some(candidate)
    }

    fn finish_slot(
        &mut self,
        prepared: Option<(SlotGeometry, PresentationIdentity)>,
        drawn: Drawn,
    ) -> bool {
        self.frames.staged.take();
        if let Some(refusal) = drawn.refusal {
            self.fail(refusal);
            return false;
        }
        // A resize between preparation and drawing needs another frame. Repeating the
        // same move means the slot cannot draw where the renderer prepared it.
        if let Some(((prepared, _), at)) = prepared.zip(drawn.moved) {
            self.withdraw_surface();
            let repeats = match self.moved {
                Some((last, count)) if last == (prepared, at) => count + 1,
                _ => 1,
            };
            self.moved = Some(((prepared, at), repeats));
            if repeats >= MOVED_FRAMES {
                self.fail(at.moved_from(prepared));
                return false;
            }
            return true;
        }
        self.moved = None;
        if let Some(((prepared, candidate), at)) = prepared.zip(drawn.slot)
            && self.raw.is_none()
            && self.geometry.is_some_and(|geometry| {
                geometry.has_output
                    && prepared == window_slot(geometry.size, geometry.scale as f32)
                    && at.same_slot(prepared)
            })
            && (candidate.revision.is_some()
                || self.app.terminal_scene_for_presentation().is_none())
        {
            self.frames.staged.set(Some(candidate));
        }
        false
    }
}

/// Draws Venus's window in Rune's frame pass.
struct HostCompositor;

/// Frames in a row a slot may draw away from where it was prepared before that is a fault.
const MOVED_FRAMES: u32 = 3;

impl Compositor<Host> for HostCompositor {
    fn render(&mut self, f: &mut Frame<'_, Host>) {
        if !f.rt.state.begin() {
            (f.rt.state.control.quit)();
            return;
        }
        let prepared = Host::prepare(f);
        if f.rt.state.failed {
            if f.rt.state.handle.window_error().is_none() {
                draw_slot(f, None, Fit::Near);
            }
            return;
        }
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
        if host.finish_slot(prepared, drawn) {
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
    if !host.check_failure() {
        host.apply_geometry(geometry.into());
    }
    host.finish(tx)
}

/// Structural events withdraw geometry before any pending receipt can be applied.
fn on_window_event(host: &mut Host, window: &Window, event: &WindowEvent, tx: &mut Tx) -> bool {
    let (size, scale) = match *event {
        WindowEvent::Resized(size) => (Some((size.width, size.height)), None),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => (None, Some(scale_factor)),
        // On macOS a drag across displays rescales the drawable before winit reports the scale.
        WindowEvent::Moved(_) => (None, Some(window.scale_factor())),
        _ => return host.native_event(event, tx),
    };
    if !host.check_failure() && !host.raw_geometry(size, scale) {
        return false;
    }
    host.finish(tx)
}

fn on_focus(host: &mut Host, focused: bool, tx: &mut Tx) {
    if host.begin() {
        if host.app.window.is_none() {
            host.early.focused = Some(focused);
            host.early.modifiers = host.early.modifiers.filter(|_| focused);
        } else {
            host.window_event(WindowEvent::Focused(focused));
        }
    }
    host.finish_live(tx);
}

fn on_visibility(host: &mut Host, visible: bool, tx: &mut Tx) {
    if !host.check_failure() {
        if !visible {
            host.frames.discard();
        } else {
            host.complete_frame();
        }
        if host.app.window.is_none() {
            host.early.visible = Some(visible);
        } else {
            host.window_event(WindowEvent::Occluded(!visible));
        }
    }
    host.finish_live(tx);
}

/// A compositor close request (or macOS Quit) is Venus's to decide under VEN-C8; Rune keeps the
/// window until Venus exits through its lane.
fn on_close_request(host: &mut Host, tx: &mut Tx) -> CloseResponse {
    if host.begin() {
        host.window_event(WindowEvent::CloseRequested);
    }
    host.finish_live(tx);
    CloseResponse::Defer
}

fn on_quit_request(host: &mut Host, tx: &mut Tx) -> QuitResponse {
    on_close_request(host, tx);
    QuitResponse::Defer
}

/// The window is going: exit, as winit's `Destroyed`; never a Quit, and no second close.
fn on_close(host: &mut Host, tx: &mut Tx) {
    host.closing = true;
    if !host.check_failure() {
        host.frames.discard();
        host.window_event(WindowEvent::Destroyed);
    }
    host.finish_live(tx);
}

/// Every frame and wake: the end of a native turn, as winit's `about_to_wait`.
fn poll(host: &mut Host, _clock: f64, tx: &mut Tx) -> bool {
    if !host.begin() {
        return host.finish(tx);
    }
    if host.app.window.is_none() {
        // The first frame makes the window; admission is sampled on the turn after it.
        tx.wake_after(Duration::ZERO);
        return true;
    }
    host.admit_present();
    let deadline = host.app.end_turn();
    host.finish_with_deadline(tx, deadline)
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
        if !matches!(response, eon_workspace_protocol::v50::Response::Snapshot(_)) {
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
        |host: &Host, cx| {
            let area = host.rune_window().and_then(|window| window.ime_area.get());
            grid_view().children(area.map(|area| {
                let [x, y, w, h] = ime_rect(area, cx.env().scale);
                rune::el().absolute(x, y).w(w).h(h).ime_cursor()
            }))
        },
    );
    spec.poll = Some(poll);
    let mut app = App::new(spec)
        .args(Vec::new())
        .srgb_blend(false)
        .terminal_gpu_loss()
        .attention_wait(SESSION_CLEANUP_TIMEOUT)
        .on_window_geometry(on_geometry)
        .on_window_event(on_window_event)
        .on_focus(on_focus)
        .on_visibility(on_visibility)
        .on_close_request(on_close_request)
        .on_quit_request(on_quit_request)
        .on_close(on_close)
        .compositor(HostCompositor);
    let _ = handle.set(app.handle());
    let frames = Rc::new(FrameHandoff::default());
    let receipt_handle = app.handle();
    app = app.on_frame_encoded({
        let frames = Rc::clone(&frames);
        move |frame| {
            if frames.report(frame.outcome(), frame.skipped()) {
                receipt_handle.update_if(Host::receive_frame);
            }
        }
    });
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
        handle: app.handle(),
        control,
        failure: failure.clone(),
        failed: false,
        frames,
        fonts: Some(fonts),
        early: Early::default(),
        geometry: None,
        raw: None,
        identity: None,
        slot: None,
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
