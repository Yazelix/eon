use super::*;
use crate::application::tests::busy_window_snapshot;
use crate::rune_candidate::fixture_messages;
use eon_workspace_protocol::v50::Response;
use std::{
    ffi::OsString,
    io::Read,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    sync::mpsc,
};
use winit::{
    event::{DeviceId, TouchPhase},
    keyboard::{KeyCode, PhysicalKey},
};

/// A lane that records what it is asked, in order; `quit` is the app-level end.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<&'static str>>>);

impl Recorder {
    fn push(&self, call: &'static str) {
        self.0.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

impl Lane for Recorder {
    fn focus(&self) {
        self.push("focus");
    }

    fn close(&self) {
        self.push("close");
    }
}

fn new_control(recorder: &Recorder) -> Control<Recorder> {
    let quit = recorder.clone();
    Control::with_focus(true, move || quit.push("quit"))
}

/// Feeds `input` to the production presentation control reader through `control`.
fn feed(control: &Control<Recorder>, input: &[u8]) {
    run_presentation_control(input, |event| control.control(event));
}

#[test]
fn present_present_eof_reach_one_lane_in_order() {
    let recorder = Recorder::default();
    let control = new_control(&recorder);
    control.bind(recorder.clone());
    control.admit(true);
    feed(&control, b"present\npresent\n");
    assert_eq!(recorder.calls(), ["focus", "focus", "close"]);
}

#[test]
fn present_never_activates_where_focus_is_more_than_attention() {
    // macOS and Windows: Rune's focus activates and unminimizes, which VEN-C14 rules out.
    let recorder = Recorder::default();
    let quit = recorder.clone();
    let control = Control::with_focus(false, move || quit.push("quit"));
    control.bind(recorder.clone());
    control.admit(true);
    feed(&control, b"present\n");
    assert_eq!(recorder.calls(), ["close"]);
    assert_eq!(FOCUS_IS_ATTENTION, cfg!(target_os = "linux"));
}

#[test]
fn an_unadmitted_or_partial_present_never_focuses_and_eof_still_closes() {
    let recorder = Recorder::default();
    let control = new_control(&recorder);
    control.bind(recorder.clone());
    // A focused window, or none yet: Present asks nothing (VEN-C14).
    control.admit(false);
    feed(&control, b"present\n");
    assert_eq!(recorder.calls(), ["close"]);

    let recorder = Recorder::default();
    let control = new_control(&recorder);
    control.bind(recorder.clone());
    control.admit(true);
    feed(&control, b"present\npres");
    assert_eq!(
        recorder.calls(),
        ["focus", "close"],
        "a partial command is EOF"
    );
}

#[test]
fn eof_before_the_window_exists_ends_the_app() {
    let recorder = Recorder::default();
    let control = new_control(&recorder);
    control.admit(true);
    feed(&control, b"present\n");
    assert_eq!(recorder.calls(), ["quit"]);
}

#[test]
fn venus_follows_presents_with_its_own_exit_on_the_same_lane() {
    // A healthy Venus exit follows the control thread's Presents on the same lane.
    let recorder = Recorder::default();
    let control = new_control(&recorder);
    control.bind(recorder.clone());
    control.admit(true);
    assert!(control.control(UserEvent::Present));
    control.exit();
    assert_eq!(recorder.calls(), ["focus", "close"]);
}

/// A Rune host on a GPU-less device, whose first frame has made Venus's window.
fn host(arguments: &[&str], admission: &str, quit: impl Fn() + Send + Sync + 'static) -> Host {
    let arguments = crate::launch::launch_arguments(
        arguments.iter().map(OsString::from),
        Some(admission.into()),
    )
    .unwrap();
    let native = App::new(AppSpec::new(
        "test",
        (960.0, 600.0),
        || unreachable!(),
        |_, _| grid_view(),
    ));
    let mut host = Host {
        app: Application::new(arguments, EventSink::Host(Arc::new(|_| true))),
        handle: native.handle(),
        control: Arc::new(Control::new(quit)),
        failure: Failure::default(),
        failed: false,
        frames: Rc::default(),
        fonts: None,
        early: Early::default(),
        geometry: None,
        raw: None,
        identity: None,
        slot: None,
        moved: None,
        closing: false,
    };
    let fonts = FontSetup::with_directories(&FontSettings::default(), None).unwrap();
    let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
    let renderer = Renderer::hosted(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        (960, 600),
        1.0,
        fonts,
    )
    .unwrap();
    host.create_window(renderer, SlotGeometry::new([0.0, 0.0, 960.0, 600.0], 1.0));
    host
}

/// A workspace window running g7 with p1 busy, before admission, so no attachment starts.
fn busy_host(quit: impl Fn() + Send + Sync + 'static) -> Host {
    let mut host = host(&["--workspace", "eon.sock"], "stdin-ready-focus-v2", quit);
    host.app
        .handle_workspace(WorkspaceEvent::Response(Response::Snapshot(
            busy_window_snapshot(b"p1.sock".to_vec()),
        )));
    host
}

fn quit_prompt_open(app: &Application) -> bool {
    app.close_prompt
        .as_ref()
        .is_some_and(|prompt| matches!(prompt.target, PromptTarget::QuitWindow))
}

fn escape(app: &mut Application) {
    for state in [ElementState::Pressed, ElementState::Released] {
        app.handle_shortcut_key(&ShortcutKey {
            physical_key: PhysicalKey::Code(KeyCode::Escape),
            state,
            repeat: false,
        });
    }
}

/// VEN-C8 through the Rune close-request hook: every native request defers to Venus; a close
/// request opens the quit prompt, and so does the next one after Escape; while a Stop is in
/// flight it does nothing. True when `close` met every step.
fn second_quit_is_kept(mut close: impl FnMut(&mut Host, &mut Tx) -> CloseResponse) -> bool {
    let ended = Recorder::default();
    let mut host = busy_host({
        let ended = ended.clone();
        move || ended.push("quit")
    });
    let mut tx = Tx::default();
    let first = close(&mut host, &mut tx) == CloseResponse::Defer && quit_prompt_open(&host.app);
    escape(&mut host.app);
    let cancelled = !quit_prompt_open(&host.app);
    let second = close(&mut host, &mut tx) == CloseResponse::Defer && quit_prompt_open(&host.app);
    escape(&mut host.app);
    host.app.quit = QuitProgress::Stopping(1);
    let held = close(&mut host, &mut tx) == CloseResponse::Defer
        && !quit_prompt_open(&host.app)
        && host.app.quit == QuitProgress::Stopping(1);
    first && cancelled && second && held && ended.calls().is_empty()
}

#[test]
fn a_second_close_request_quits_again_and_a_coalesced_mapping_fails() {
    assert!(second_quit_is_kept(on_close_request));
    // The naive mapping merges every later request into the first.
    let mut asked = false;
    assert!(!second_quit_is_kept(|host, tx| {
        if std::mem::replace(&mut asked, true) {
            CloseResponse::Defer
        } else {
            on_close_request(host, tx)
        }
    }));
}

#[test]
fn macos_quit_defers_to_the_same_close_policy() {
    let mut host = busy_host(|| {});
    let mut tx = Tx::default();
    assert_eq!(on_quit_request(&mut host, &mut tx), QuitResponse::Defer);
    assert!(quit_prompt_open(&host.app));
}

#[test]
fn a_close_request_without_a_running_window_detaches_on_the_lane() {
    let ended = Recorder::default();
    let mut host = host(&["--workspace", "eon.sock"], "stdin-ready-focus-v2", {
        let ended = ended.clone();
        move || ended.push("quit")
    });
    let mut tx = Tx::default();
    assert_eq!(on_close_request(&mut host, &mut tx), CloseResponse::Defer);
    // No window handle in this test, so the lane's end is the app quit.
    assert_eq!(ended.calls(), ["quit"]);
    assert_eq!(host.app.quit, QuitProgress::Idle);
}

#[test]
fn window_loss_exits_without_quit_or_a_second_close() {
    let ended = Recorder::default();
    let mut host = busy_host({
        let ended = ended.clone();
        move || ended.push("quit")
    });
    on_close(&mut host, &mut Tx::default());
    assert!(!quit_prompt_open(&host.app), "window loss is not Quit");
    assert_eq!(host.app.quit, QuitProgress::Idle);
    assert!(ended.calls().is_empty(), "Rune is already closing it");
}

#[test]
fn ready_waits_for_the_window_to_enter_an_output() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let geometry = |has_output| Geometry {
        size: (960, 600),
        scale: 1.0,
        has_output,
    };
    host.apply_geometry(geometry(false));
    assert!(!host.app.window.as_ref().unwrap().native.has_output());
    host.app.end_turn();
    assert!(host.app.initial_scale_pending, "no output, no readiness");
    host.apply_geometry(geometry(true));
    assert!(host.app.window.as_ref().unwrap().native.has_output());
    host.app.end_turn();
    assert!(
        !host.app.initial_scale_pending,
        "admitted once in an output"
    );
    assert!(host.app.fatal_error.is_none());
}

#[test]
fn facts_before_the_first_frame_apply_once_the_window_exists() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let window = host.app.window.take().unwrap();
    on_focus(&mut host, true, &mut Tx::default());
    host.apply_geometry(Geometry {
        size: (960, 600),
        scale: 1.0,
        has_output: true,
    });
    assert!(!host.app.window_focused);
    let WindowState { renderer, .. } = window;
    host.create_window(renderer, SlotGeometry::new([0.0, 0.0, 960.0, 600.0], 1.0));
    assert!(host.app.window_focused);
    assert!(host.app.window.as_ref().unwrap().native.has_output());
}

#[test]
fn the_control_thread_presents_only_while_venus_admits_it() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let mut tx = Tx::default();
    host.finish(&mut tx);
    assert!(
        host.control.admitted.load(Ordering::Acquire),
        "shown, unfocused"
    );
    on_focus(&mut host, true, &mut tx);
    assert!(!host.control.admitted.load(Ordering::Acquire), "focused");
}

#[test]
fn titles_reach_rune_once_per_change() {
    let window = RuneWindow::default();
    window.set_title("a");
    assert_eq!(window.take_title().as_deref(), Some("a"));
    window.set_title("a");
    assert_eq!(window.take_title(), None);
    window.set_title("b");
    assert_eq!(window.take_title().as_deref(), Some("b"));
}

#[test]
fn a_near_slot_draws_the_prepared_grid_pixel_exact_and_fixtures_draw_only_exact_slots() {
    let prepared = SlotGeometry::new([10.0, 0.0, 978.0, 1080.0], 1.0);
    let near = SlotGeometry::new([10.0, 0.0, 977.0, 1080.0], 1.0);
    let target = [1000, 1080];
    // The prepared size, not the slot's: a 978-pixel grid is never squeezed into 977.
    assert_eq!(
        prepared.viewport(near, Fit::Near, [0.0, 0.0], target),
        Some([10.0, 0.0, 978.0, 1080.0])
    );
    assert_eq!(
        prepared.viewport(near, Fit::Near, [4.0, 0.0], target),
        Some([6.0, 0.0, 978.0, 1080.0])
    );
    // Rounded a pixel past the far edge, as the separately rounded prepared size can be on
    // every frame (a 987-pixel slot at 13 with a 988-pixel grid in 1000): it shifts back a
    // pixel, never counting as a move, which three frames running would end the window.
    assert_eq!(
        prepared.viewport(near, Fit::Near, [0.0, 0.0], [987, 1080]),
        Some([9.0, 0.0, 978.0, 1080.0])
    );
    let rounded = SlotGeometry::new([13.0, 0.0, 988.0, 1080.0], 1.25);
    let snapped = SlotGeometry::new([13.0, 0.0, 987.0, 1080.0], 1.25);
    assert_eq!(
        rounded.viewport(snapped, Fit::Near, [0.0, 0.0], [1000, 1080]),
        Some([12.0, 0.0, 988.0, 1080.0])
    );
    let shifted = SlotGeometry::new([12.0, 0.0, 988.0, 1080.0], 1.25);
    assert_eq!(
        rounded.viewport(shifted, Fit::Near, [0.0, 0.0], [1000, 1080]),
        Some([12.0, 0.0, 988.0, 1080.0])
    );
    // A real resize leaves the prepared grid outside: a move, and the next frame prepares anew.
    let wide = SlotGeometry::new([0.0, 0.0, 978.0, 1080.0], 1.0);
    let narrowed = SlotGeometry::new([0.0, 0.0, 977.0, 1080.0], 1.0);
    assert_eq!(
        wide.viewport(narrowed, Fit::Near, [0.0, 0.0], [977, 1080]),
        None
    );
    // The fixtures never shift.
    assert_eq!(
        rounded.viewport(snapped, Fit::Exact, [0.0, 0.0], [1000, 1080]),
        None
    );
    // The fixtures' window never resizes: any move is refused.
    assert_eq!(
        prepared.viewport(near, Fit::Exact, [0.0, 0.0], target),
        None
    );
    assert_eq!(
        prepared.viewport(prepared, Fit::Exact, [0.0, 0.0], target),
        Some([10.0, 0.0, 978.0, 1080.0])
    );
}

#[test]
fn a_slot_within_a_device_pixel_is_the_prepared_one() {
    let prepared = SlotGeometry::new([0.0, 0.0, 978.0, 1080.0], 1.0);
    assert!(SlotGeometry::new([0.0, 0.0, 977.0, 1080.0], 1.0).same_slot(prepared));
    assert!(!SlotGeometry::new([0.0, 0.0, 960.0, 1080.0], 1.0).same_slot(prepared));
    assert!(!SlotGeometry::new([0.0, 0.0, 978.0, 1080.0], 2.0).same_slot(prepared));
}

#[test]
fn requests_drained_during_a_user_event_reach_the_host_once() {
    let ended = Recorder::default();
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", {
        let ended = ended.clone();
        move || ended.push("quit")
    });
    // Application's drain hands out what is pending; the host's callback must not put it back,
    // or the drain never ends (a workspace catch-up with a popup shown drains this way).
    host.app.native_requests.redraw = true;
    host.app.native_requests.exit = true;
    let mut requested = NativeRequests::default();
    host.app.apply_native_requests(collect(&mut requested));
    assert!(host.app.native_requests.take().is_none());
    assert_eq!(requested.take(), Some(NativeRequest::Redraw));
    assert_eq!(requested.take(), Some(NativeRequest::Exit));
    assert_eq!(requested.take(), None);
    host.user_event(UserEvent::Exit, &mut Tx::default());
    assert_eq!(ended.calls(), ["quit"], "Exit reaches the lane once");
}

#[test]
fn a_failed_first_frame_exits_with_its_error_instead_of_spinning() {
    let ended = Recorder::default();
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", {
        let ended = ended.clone();
        move || ended.push("quit")
    });
    host.app.window = None;
    host.fail("no slot".into());
    assert_eq!(
        ended.calls(),
        ["quit"],
        "the exit is asked for from the frame"
    );
    assert_eq!(host.failure.get().as_deref(), Some("no slot"));
    assert!(
        !poll(&mut host, 0.0, &mut Tx::default()),
        "no busy wake after a failed frame"
    );
}

#[test]
fn present_is_admitted_before_ready_v1_can_be_read() {
    // A supervisor may present the moment ready-v1 arrives, so the window is admitted as soon as
    // it exists, before the turn that writes ready-v1.
    let host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    assert!(host.app.initial_scale_pending, "ready-v1 not yet written");
    assert!(host.control.admitted.load(Ordering::Acquire));
}

#[test]
fn poll_sleeps_at_rest_and_wakes_for_a_redraw() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(Geometry {
        size: (960, 600),
        scale: 1.0,
        has_output: true,
    });
    let mut tx = Tx::default();
    let rested = (0..4).any(|_| !poll(&mut host, 0.0, &mut tx));
    assert!(rested, "an admitted, drawn window rests");
    host.rune_window().unwrap().request_redraw();
    assert!(
        poll(&mut host, 0.0, &mut tx),
        "a redraw request wakes the view"
    );
}

#[test]
fn a_scale_change_at_the_same_size_resizes_the_grid() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let geometry = |scale| Geometry {
        size: (960, 600),
        scale,
        has_output: true,
    };
    host.apply_geometry(geometry(1.0));
    host.app.end_turn();
    assert!(!host.app.initial_scale_pending);
    let before = host.app.window.as_ref().unwrap().renderer.metrics();
    host.apply_geometry(geometry(2.0));
    let state = host.app.window.as_ref().unwrap();
    assert_eq!(state.scale_factor, 2.0);
    assert_ne!(
        state.renderer.metrics(),
        before,
        "cell metrics follow the new scale"
    );
}

#[test]
fn output_loss_withdraws_input_and_regain_redraws_at_the_same_size() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let on_output = Geometry {
        size: (960, 600),
        scale: 1.0,
        has_output: true,
    };
    host.apply_geometry(on_output);
    host.finish(&mut Tx::default());
    host.frames
        .staged
        .set(Some(host.app.presentation.candidate(Some(1))));
    assert!(host.frames.report(FrameOutcome::Drawn, None));
    host.receive_frame(&mut Tx::default());
    assert!(host.app.presentation.has_presented_geometry());

    host.apply_geometry(Geometry {
        has_output: false,
        ..on_output
    });
    assert!(!host.app.presentation.has_presented_geometry());
    host.finish(&mut Tx::default());
    host.apply_geometry(on_output);
    assert!(
        host.finish(&mut Tx::default()),
        "output entry needs a fresh draw"
    );
    assert!(!host.app.presentation.has_presented_geometry());
}

#[test]
fn fatal_failure_withdraws_admission_and_releases_the_renderer() {
    let ended = Recorder::default();
    let mut host = busy_host({
        let ended = ended.clone();
        move || ended.push("quit")
    });
    host.app
        .presentation
        .publish(host.app.presentation.candidate(Some(1)));
    host.fail("first renderer error".into());
    host.fail("later error".into());
    assert_eq!(host.failure.get().as_deref(), Some("first renderer error"));
    assert!(!host.control.admitted.load(Ordering::Acquire));
    assert!(!host.app.presentation.has_presented_geometry());
    assert!(host.app.window.is_none());
    assert_eq!(host.app.quit, QuitProgress::Idle);
    assert_eq!(ended.calls(), ["quit"]);
}

const NATIVE: Geometry = Geometry {
    size: (960, 600),
    scale: 1.0,
    has_output: true,
};
const FULL: Geometry = Geometry {
    size: (1280, 800),
    ..NATIVE
};
const HALF: Geometry = Geometry {
    size: (640, 800),
    ..NATIVE
};
const PEER_TIMEOUT: Duration = Duration::from_secs(5);

fn native_slot() -> SlotGeometry {
    SlotGeometry::new([0.0, 0.0, 960.0, 600.0], 1.0)
}

fn stage(host: &mut Host, identity: PresentationIdentity) {
    stage_in(host, native_slot(), native_slot(), identity);
}

fn stage_in(
    host: &mut Host,
    prepared: SlotGeometry,
    drawn: SlotGeometry,
    identity: PresentationIdentity,
) {
    assert!(!host.finish_slot(
        Some((prepared, identity)),
        Drawn {
            hook_calls: 1,
            slot: Some(drawn),
            moved: None,
            refusal: None
        },
    ));
}

fn present(host: &mut Host, identity: PresentationIdentity) {
    present_at(host, native_slot(), identity);
}

fn present_at(host: &mut Host, slot: SlotGeometry, identity: PresentationIdentity) {
    present_in(host, slot, slot, identity);
}

fn present_in(
    host: &mut Host,
    prepared: SlotGeometry,
    drawn: SlotGeometry,
    identity: PresentationIdentity,
) {
    stage_in(host, prepared, drawn, identity);
    assert!(host.frames.report(FrameOutcome::Drawn, None));
    host.receive_frame(&mut Tx::default());
}

#[test]
fn skips_withdraw_only_the_input_their_reason_requires() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    let receipt = host.app.presentation.candidate(Some(1));
    present(&mut host, receipt);
    let copy = ClientMessage::Selection(SelectionAction::Copy);
    host.app.terminal.deferred_selection.push_back(copy.clone());

    for outcome in [FrameOutcome::Unchanged, FrameOutcome::Skipped] {
        stage(&mut host, receipt);
        assert!(!host.frames.report(outcome, Some(FrameSkip::Timeout)));
        assert!(host.frames.staged.get().is_none());
        assert!(host.app.presentation.is_current(receipt));
        assert_eq!(host.app.terminal.deferred_selection.len(), 1);
    }
    host.app.terminal.terminal_scroll.phase_active = true;
    host.app.terminal.pending_viewport = Some(PendingViewportRequest::Scroll {
        rows: 1,
        cancelled: false,
    });
    host.app.links.unshifted = true;
    host.app.next_animation = Some(Instant::now());
    assert!(
        host.frames
            .report(FrameOutcome::Skipped, Some(FrameSkip::Covered))
    );
    host.receive_frame(&mut Tx::default());
    assert!(!host.app.presentation.has_presented_geometry());
    assert_eq!(host.app.terminal.deferred_selection.len(), 1);
    assert!(!host.app.terminal.terminal_scroll.phase_active);
    assert_eq!(
        host.app.terminal.pending_viewport,
        Some(PendingViewportRequest::Scroll {
            rows: 1,
            cancelled: true
        })
    );
    assert!(!host.app.links.unshifted);
    assert!(host.app.next_animation.is_none());

    assert!(host.frames.report(FrameOutcome::Skipped, None));
    assert!(
        !host
            .frames
            .report(FrameOutcome::Skipped, Some(FrameSkip::Covered))
    );
    host.receive_frame(&mut Tx::default());
    assert!(
        host.app.terminal.deferred_selection.is_empty(),
        "the surface withdrawal wins"
    );
    let epoch = host.app.presentation.epoch;
    assert!(
        host.frames
            .report(FrameOutcome::Skipped, Some(FrameSkip::Surface))
    );
    host.receive_frame(&mut Tx::default());
    assert_eq!(
        host.app.presentation.epoch, epoch,
        "the same surface stays withdrawn"
    );
    present(&mut host, receipt);
    assert!(
        !host.app.presentation.has_presented_geometry(),
        "late old receipt"
    );
    let fresh = host.app.presentation.candidate(Some(1));
    present(&mut host, fresh);
    assert!(host.app.presentation.is_current(fresh));
}

#[test]
fn resize_and_occlusion_supersede_queued_frames_and_moved_slots_never_admit() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    let initial = host.app.presentation.candidate(Some(1));
    stage(&mut host, initial);
    assert!(host.frames.report(FrameOutcome::Drawn, None));
    host.apply_geometry(Geometry {
        size: (800, 600),
        ..NATIVE
    });
    host.receive_frame(&mut Tx::default());
    assert!(!host.app.presentation.has_presented_geometry());
    let changed = host.app.presentation.candidate(Some(2));
    stage(&mut host, changed);
    assert!(
        host.frames.staged.get().is_none(),
        "a provisional slot is too wide"
    );

    host.apply_geometry(NATIVE);
    let current = host.app.presentation.candidate(Some(2));
    stage(&mut host, current);
    assert!(host.frames.report(FrameOutcome::Drawn, None));
    on_visibility(&mut host, false, &mut Tx::default());
    host.receive_frame(&mut Tx::default());
    assert!(!host.app.presentation.has_presented_geometry());
    on_visibility(&mut host, true, &mut Tx::default());
    let current = host.app.presentation.candidate(Some(2));
    present(&mut host, current);
    assert!(host.app.presentation.is_current(current));

    for _ in 1..MOVED_FRAMES {
        assert!(host.finish_slot(
            Some((native_slot(), current)),
            Drawn {
                hook_calls: 1,
                slot: None,
                refusal: None,
                moved: Some(SlotGeometry::new([0.0, 0.0, 800.0, 600.0], 1.0)),
            },
        ));
        assert!(!host.app.presentation.has_presented_geometry());
    }
    assert!(!host.finish_slot(
        Some((native_slot(), current)),
        Drawn {
            hook_calls: 1,
            slot: None,
            refusal: None,
            moved: Some(SlotGeometry::new([0.0, 0.0, 800.0, 600.0], 1.0)),
        },
    ));
    assert!(host.failed);
}

/// A canonical Orbit peer on a private socket with bounded waits for the candidate's drains.
struct Peer {
    listener: UnixListener,
    socket: PathBuf,
    wakes: mpsc::Receiver<()>,
}

impl Peer {
    fn new(host: &mut Host, name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vr-{:x}-{name}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let socket = root.join("o");
        let listener = UnixListener::bind(&socket).unwrap_or_else(|error| {
            let _ = std::fs::remove_dir(&root);
            panic!("bind private Orbit peer: {error}");
        });
        listener.set_nonblocking(true).unwrap();
        let (wake, wakes) = mpsc::sync_channel(1);
        host.app.transport_wake = Arc::new(move || {
            let _ = wake.try_send(());
        });
        host.app.start_orbit(socket.clone());
        Self {
            listener,
            socket,
            wakes,
        }
    }

    fn accept(&self) -> UnixStream {
        let deadline = Instant::now() + PEER_TIMEOUT;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    // macOS inherits nonblocking mode from the listener.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(PEER_TIMEOUT)).unwrap();
                    assert_eq!(Self::read(&stream), ClientMessage::Hello);
                    return stream;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2));
                }
                other => panic!("Orbit client did not connect: {other:?}"),
            }
        }
    }

    fn read(mut stream: &UnixStream) -> ClientMessage {
        let mut packet = vec![0; session::HEADER_BYTES];
        stream.read_exact(&mut packet).unwrap();
        packet.resize(session::client_message_len(&packet).unwrap().unwrap(), 0);
        stream
            .read_exact(&mut packet[session::HEADER_BYTES..])
            .unwrap();
        session::decode_client_message(&packet).unwrap()
    }

    fn send(
        &self,
        mut stream: &UnixStream,
        host: &mut Host,
        messages: impl IntoIterator<Item = ServerMessage>,
        settled: impl Fn(&Host) -> bool,
    ) {
        for message in messages {
            stream
                .write_all(&session::encode_server_message(&message).unwrap())
                .unwrap();
        }
        let deadline = Instant::now() + PEER_TIMEOUT;
        while !settled(host) {
            self.wakes
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            host.user_event(UserEvent::Transport, &mut Tx::default());
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir(self.socket.parent().unwrap());
    }
}

fn grid_at(host: &Host, geometry: Geometry) -> SurfaceSize {
    let metrics = host
        .app
        .window
        .as_ref()
        .unwrap()
        .renderer
        .metrics_at_scale(geometry.scale);
    surface_size(PhysicalSize::new(geometry.size.0, geometry.size.1), metrics).unwrap()
}

fn grid_fixture(step: usize, size: SurfaceSize) -> Vec<ServerMessage> {
    let mut messages = fixture_messages(step);
    if let Some(ServerMessage::Frame(frame)) = messages.last_mut() {
        let (cols, rows) = (size.cols, size.rows);
        let blank = frame.rows.last().unwrap().cells.last().unwrap().clone();
        for row in &mut frame.rows {
            row.cells.resize(usize::from(cols), blank.clone());
        }
        let blank_row = frame.rows.last().unwrap().clone();
        frame.rows.resize(usize::from(rows), blank_row);
        frame.dimensions = orbit_protocol::Dimensions { cols, rows };
    }
    messages
}

fn attach_fixture(peer: &Peer, stream: &UnixStream, host: &mut Host, geometry: Geometry) {
    let frames = grid_fixture(0, grid_at(host, geometry));
    peer.send(stream, host, frames, |host| {
        host.app.terminal.model.scene().is_some()
    });
}

fn selection_begin(frame_revision: u64) -> ClientMessage {
    ClientMessage::Selection(SelectionAction::Begin {
        frame_revision,
        position: session::SelectionPosition { x: 0.0, y: 0.0 },
        time_ns: 0,
        modifiers: session::Modifiers::empty(),
    })
}

#[test]
fn an_idle_drawn_receipt_flushes_the_new_revision_without_requesting_a_redraw() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    let peer = Peer::new(&mut host, "receipt");
    let private_root = peer.socket.parent().unwrap();
    assert_ne!(private_root, std::env::temp_dir());
    assert_eq!(
        private_root.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    let stream = peer.accept();
    attach_fixture(&peer, &stream, &mut host, NATIVE);
    let candidate = host.prepare_grid(native_slot()).unwrap();
    host.finish(&mut Tx::default());
    host.app
        .terminal
        .deferred_selection
        .push_back(selection_begin(0));
    stage(&mut host, candidate);
    assert!(
        !host.app.presentation.has_presented_geometry(),
        "encoding is too early"
    );
    assert!(host.frames.report(FrameOutcome::Drawn, None));
    assert!(
        !host.receive_frame(&mut Tx::default()),
        "quiet receipt update"
    );
    assert!(host.app.presentation.is_current(candidate));
    assert!(host.app.terminal.deferred_selection.is_empty());
    let deadline = Instant::now() + PEER_TIMEOUT;
    loop {
        let message = Peer::read(&stream);
        if let ClientMessage::Selection(_) = message {
            assert_eq!(message, selection_begin(1));
            break;
        }
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn another_withdrawal_discards_intervening_selection_without_sending_another_cancel() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    let peer = Peer::new(&mut host, "withdraw");
    let stream = peer.accept();
    attach_fixture(&peer, &stream, &mut host, NATIVE);
    let candidate = host.prepare_grid(native_slot()).unwrap();
    present(&mut host, candidate);
    host.app.terminal.selection_gate.finish_sent();
    assert!(host.raw_geometry(Some((800, 600)), None));
    assert!(
        !host.app.presentation.has_presented_geometry(),
        "a raw resize withdraws before the idle report"
    );

    for withdraw in [
        (|host: &mut Host| {
            host.apply_geometry(Geometry {
                size: (800, 600),
                ..NATIVE
            })
        }) as fn(&mut Host),
        |host| assert!(host.raw_geometry(Some((960, 600)), None)),
        // Back at the applied size before any frame: input queued meanwhile is still stale.
        |host| assert!(host.raw_geometry(Some((800, 600)), None)),
        |host| on_visibility(host, false, &mut Tx::default()),
        |host| {
            assert!(
                host.frames
                    .report(FrameOutcome::Skipped, Some(FrameSkip::Surface))
            );
            host.receive_frame(&mut Tx::default());
        },
    ] {
        host.app.send_or_defer_selection(selection_begin(1), false);
        assert!(host.app.input.is_selecting());
        withdraw(&mut host);
        assert!(host.app.terminal.deferred_selection.is_empty());
        assert!(!host.app.input.is_selecting());
    }
    on_visibility(&mut host, true, &mut Tx::default());
    host.apply_geometry(NATIVE);
    host.app.terminal.selection_gate = SelectionGate::Ready;
    let fresh = host.prepare_grid(native_slot()).unwrap();
    present(&mut host, fresh);
    assert!(host.app.send(ClientMessage::ReturnToLive));
    let mut selections = Vec::new();
    let deadline = Instant::now() + PEER_TIMEOUT;
    loop {
        match Peer::read(&stream) {
            ClientMessage::Selection(selection) => selections.push(selection),
            ClientMessage::ReturnToLive => break,
            _ => {}
        }
        assert!(Instant::now() < deadline);
    }
    assert_eq!(selections, [SelectionAction::Cancel]);
}

/// The resize and coordinate actions the peer reads up to a `ReturnToLive` marker.
fn actions_until_marker(host: &mut Host, stream: &UnixStream) -> Vec<ClientMessage> {
    assert!(host.app.send(ClientMessage::ReturnToLive));
    let mut actions = Vec::new();
    loop {
        match Peer::read(stream) {
            ClientMessage::ReturnToLive => return actions,
            message @ (ClientMessage::Resize(_)
            | ClientMessage::PreviewVertical { .. }
            | ClientMessage::ScrollVertical { .. }
            | ClientMessage::Mouse(_)
            | ClientMessage::Selection(_)) => actions.push(message),
            _ => {}
        }
    }
}

fn wheel(host: &mut Host) {
    host.window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
        phase: TouchPhase::Moved,
    });
}

fn move_to_terminal(host: &mut Host) {
    host.window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(120.0, 300.0),
    });
}

fn left(host: &mut Host, state: ElementState) {
    host.window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    });
}

/// Replays both intervals in the Step 2 native resize sentinel.
#[test]
fn a_raw_resize_withdraws_input_until_a_draw_of_the_applied_grid() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let full_slot = SlotGeometry::new([0.0, 0.0, 1280.0, 800.0], 1.0);
    let half_slot = SlotGeometry::new([0.0, 0.0, 640.0, 800.0], 1.0);
    host.apply_geometry(FULL);
    let peer = Peer::new(&mut host, "raw");
    let stream = peer.accept();
    attach_fixture(&peer, &stream, &mut host, FULL);
    let presented = host.prepare_grid(full_slot).unwrap();
    present_at(&mut host, full_slot, presented);
    actions_until_marker(&mut host, &stream);
    assert!(
        !host.raw_geometry(Some(FULL.size), None),
        "a configure repeat"
    );
    assert!(host.app.presentation.is_current(presented));
    let cell = f64::from(host.app.window.as_ref().unwrap().renderer.metrics().height);
    host.app.terminal.terminal_scroll.push_lines(1.0, cell);

    assert!(host.raw_geometry(Some(HALF.size), None));
    host.app.drive_terminal_scroll();
    wheel(&mut host);
    let early = host.prepare_grid(half_slot).unwrap();
    present_at(&mut host, half_slot, early);
    assert!(
        !host.app.presentation.has_presented_geometry(),
        "Drawn before the idle report"
    );
    wheel(&mut host);
    assert!(actions_until_marker(&mut host, &stream).is_empty());

    host.apply_geometry(HALF);
    let resized = grid_at(&host, HALF);
    assert_eq!((resized.screen_width, resized.screen_height), (640, 800));
    assert_eq!(
        actions_until_marker(&mut host, &stream),
        [ClientMessage::Resize(resized)]
    );
    let old_grid = host.prepare_grid(half_slot).unwrap();
    present_at(&mut host, half_slot, old_grid);
    wheel(&mut host);
    assert_eq!(
        actions_until_marker(&mut host, &stream),
        [],
        "the old grid must not admit wheel input after applied geometry"
    );
    assert!(!host.app.presentation.has_presented_geometry());
    move_to_terminal(&mut host);
    left(&mut host, ElementState::Pressed);
    left(&mut host, ElementState::Released);
    assert!(host.app.terminal.deferred_selection.is_empty());
    assert!(actions_until_marker(&mut host, &stream).is_empty());

    peer.send(&stream, &mut host, grid_fixture(1, resized), |host| {
        host.app.terminal.model.scene().unwrap().revision == 2
    });
    wheel(&mut host);
    assert!(actions_until_marker(&mut host, &stream).is_empty());
    let qualifying = host.prepare_grid(half_slot).unwrap();
    present_at(&mut host, half_slot, qualifying);
    assert!(host.app.presentation.is_current(qualifying));
    wheel(&mut host);
    assert_eq!(
        actions_until_marker(&mut host, &stream),
        [ClientMessage::PreviewVertical {
            frame_revision: 2,
            direction: session::VerticalDirection::Up,
        }]
    );
}

#[test]
fn a_near_slot_keeps_orbit_and_selection_on_the_applied_grid() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let current = grid_at(&host, NATIVE);
    let boundary = u32::from(current.cols) * current.cell_width + current.padding_left * 2;
    let rounded = Geometry {
        size: (boundary - 1, NATIVE.size.1),
        ..NATIVE
    };
    let applied = Geometry {
        size: (boundary, NATIVE.size.1),
        ..NATIVE
    };
    let measured = SlotGeometry::new([0.0, 0.0, rounded.size.0 as f32, 600.0], 1.0);
    host.apply_geometry(applied);
    let prepared = slot_for_target(measured, applied.size);
    assert!(host.prepare_grid(prepared).is_none());
    let peer = Peer::new(&mut host, "rounding");
    let stream = peer.accept();
    attach_fixture(&peer, &stream, &mut host, rounded);
    let required = grid_at(&host, applied);
    assert_eq!(
        actions_until_marker(&mut host, &stream),
        [ClientMessage::Resize(required)]
    );

    let candidate = host.prepare_grid(prepared).unwrap();
    present_in(&mut host, prepared, measured, candidate);
    assert_eq!(host.app.presented_revision(), None);

    peer.send(&stream, &mut host, grid_fixture(1, required), |host| {
        host.app.terminal.model.scene().unwrap().revision == 2
    });
    let candidate = host.prepare_grid(prepared).unwrap();
    present_in(&mut host, prepared, measured, candidate);
    assert!(host.app.presentation.is_current(candidate));
    host.app.cursor = PhysicalPosition::new(120.0, 300.0);
    left(&mut host, ElementState::Pressed);
    assert!(matches!(
        actions_until_marker(&mut host, &stream).as_slice(),
        [ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 2,
            ..
        })]
    ));
}

#[test]
fn compatible_output_keeps_the_displayed_selection_revision_until_raw_resize() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    let peer = Peer::new(&mut host, "compatible");
    let stream = peer.accept();
    attach_fixture(&peer, &stream, &mut host, NATIVE);
    let displayed = host.prepare_grid(native_slot()).unwrap();
    present(&mut host, displayed);
    actions_until_marker(&mut host, &stream);

    let size = grid_at(&host, NATIVE);
    peer.send(&stream, &mut host, grid_fixture(1, size), |host| {
        host.app.terminal.model.scene().unwrap().revision == 2
    });
    assert_eq!(host.app.presented_revision(), Some(1));
    move_to_terminal(&mut host);
    actions_until_marker(&mut host, &stream);
    left(&mut host, ElementState::Pressed);
    assert!(matches!(
        actions_until_marker(&mut host, &stream).as_slice(),
        [ClientMessage::Selection(SelectionAction::Begin {
            frame_revision: 1,
            ..
        })]
    ));

    assert!(host.raw_geometry(Some((959, 600)), None));
    assert!(host.raw_geometry(Some(NATIVE.size), None));
    let matching = host.prepare_grid(native_slot()).unwrap();
    present(&mut host, matching);
    left(&mut host, ElementState::Released);
    assert_eq!(
        actions_until_marker(&mut host, &stream),
        [ClientMessage::Selection(SelectionAction::Cancel)]
    );
}

#[test]
fn raw_geometry_holds_admission_through_one_pixel_and_returns_with_the_window() {
    fn present_current(host: &mut Host, slot: SlotGeometry) -> PresentationIdentity {
        let identity = host.app.presentation.candidate(Some(1));
        present_at(host, slot, identity);
        identity
    }
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    host.apply_geometry(NATIVE);
    present_current(&mut host, native_slot());
    // One pixel is within the tolerance of the slot a hosted grid draws at.
    assert!(host.raw_geometry(Some((959, 600)), None));
    present_current(&mut host, SlotGeometry::new([0.0, 0.0, 959.0, 600.0], 1.0));
    assert!(
        !host.app.presentation.has_presented_geometry(),
        "drawn before the idle report"
    );
    host.finish(&mut Tx::default());
    // Back within the burst: the idle report sees no change, so the host asks for the frame.
    assert!(host.raw_geometry(Some(NATIVE.size), None));
    assert!(host.finish(&mut Tx::default()), "a fresh draw");
    let back = present_current(&mut host, native_slot());
    assert!(host.app.presentation.is_current(back));

    assert!(host.raw_geometry(None, Some(2.0)));
    assert!(
        host.raw_geometry(Some(NATIVE.size), None),
        "a same-size Resized keeps the pending scale"
    );
    present_current(&mut host, native_slot());
    assert!(!host.app.presentation.has_presented_geometry());
    assert!(host.raw_geometry(None, Some(1.0)));
    let back = present_current(&mut host, native_slot());
    assert!(host.app.presentation.is_current(back));
}

#[test]
fn refused_image_reattach_presents_only_the_new_attachment_on_the_same_renderer() {
    let mut host = host(&["orbit.sock"], "stdin-ready-focus-v2", || {});
    let slot = SlotGeometry::new([0.0, 0.0, 1280.0, 800.0], 1.0);
    host.apply_geometry(FULL);
    let size = grid_at(&host, FULL);
    let peer = Peer::new(&mut host, "resync");
    let first = peer.accept();

    let mut upload = grid_fixture(4, size);
    let ServerMessage::Frame(mut image_frame) = upload.pop().unwrap() else {
        unreachable!()
    };
    let ServerMessage::ResourceAnnounce(announcement) = &upload[0] else {
        unreachable!()
    };
    let budget = u64::from(announcement.width) * u64::from(announcement.height) * 4;
    let window = host.app.window.as_mut().unwrap();
    window.renderer.set_test_image_budget(budget);
    let renderer = &window.renderer as *const Renderer;
    host.app.column_views.push(TerminalView::new(None));
    for message in grid_fixture(0, size)
        .into_iter()
        .chain(grid_fixture(4, size))
    {
        host.app
            .handle_column_transport(0, TransportEvent::Server(message));
    }
    let at_revision = |revision| {
        move |host: &Host| {
            host.app
                .terminal
                .model
                .scene()
                .is_some_and(|scene| scene.revision == revision)
        }
    };
    peer.send(
        &first,
        &mut host,
        grid_fixture(0, size)
            .into_iter()
            .chain(grid_fixture(4, size)),
        at_revision(5),
    );
    let old = host.prepare_grid(slot).unwrap();
    present_at(&mut host, slot, old);
    assert_eq!(host.app.presented_revision(), Some(5));
    let old_graphics = host.app.terminal.model.scene().unwrap().graphics.clone();
    let referenced =
        image_references(std::iter::once(&host.app.terminal).chain(&host.app.column_views));
    let image_renderer = &mut host.app.window.as_mut().unwrap().renderer;
    assert_eq!(image_renderer.image_resident(), budget);
    assert_eq!(
        image_renderer.image_resync(
            old_graphics.attachment,
            &old_graphics.placements,
            &referenced
        ),
        Some(ImageResync::NoRoom)
    );
    assert!(image_renderer.test_image_marker(&old_graphics));

    host.app.column_views.clear();
    host.app.sweep_images();
    assert_eq!(
        host.app.window.as_ref().unwrap().renderer.image_resident(),
        0
    );
    let retained = host.prepare_grid(slot).unwrap();
    assert!(host.app.terminal.images_resyncing);
    assert_eq!(
        retained.revision, None,
        "retained scenes cannot admit old input"
    );
    present_at(&mut host, slot, old);
    assert!(!host.app.presentation.has_presented_geometry());
    present_at(&mut host, slot, retained);
    assert_eq!(host.app.presented_revision(), None);

    let second = peer.accept();
    image_frame.revision = 6;
    image_frame.graphics.as_mut().unwrap().placements[0].state = PlacementState::Pending;
    peer.send(
        &second,
        &mut host,
        [
            ServerMessage::Attached,
            ServerMessage::Frame(image_frame.clone()),
        ],
        at_revision(6),
    );
    let fresh = host.prepare_grid(slot).unwrap();
    let pending = host.app.terminal.model.scene().unwrap().graphics.clone();
    assert_ne!(old_graphics.attachment, pending.attachment);
    assert!(
        host.app
            .window
            .as_mut()
            .unwrap()
            .renderer
            .test_image_marker(&pending)
    );
    assert_eq!(
        host.app
            .window
            .as_ref()
            .unwrap()
            .renderer
            .next_marker_deadline(),
        None
    );
    present_at(&mut host, slot, fresh);
    assert_eq!(host.app.presented_revision(), Some(6));

    image_frame.revision = 7;
    image_frame.graphics.as_mut().unwrap().placements[0].state = PlacementState::Ready;
    peer.send(
        &second,
        &mut host,
        upload
            .iter()
            .cloned()
            .chain([ServerMessage::Frame(image_frame.clone())]),
        at_revision(7),
    );
    let fresh = host.prepare_grid(slot).unwrap();
    present_at(&mut host, slot, fresh);
    assert_eq!(host.app.presented_revision(), Some(7));
    let replacement = host.app.terminal.model.scene().unwrap().graphics.clone();
    let current_renderer = &mut host.app.window.as_mut().unwrap().renderer;
    assert_eq!(current_renderer as *const Renderer, renderer);
    assert_eq!(current_renderer.image_resident(), budget);
    assert!(!current_renderer.test_image_marker(&replacement));

    // Refuse a second resource while the replacement is still referenced. When
    // the frame switches to it, the same target sizes and residency spend no further reattach.
    for message in &mut upload {
        match message {
            ServerMessage::ResourceAnnounce(message) => message.resource += 1,
            ServerMessage::ResourceChunk(message) => message.resource += 1,
            _ => unreachable!(),
        }
    }
    image_frame.revision = 8;
    image_frame.graphics.as_mut().unwrap().placements[0].resource += 1;
    peer.send(
        &second,
        &mut host,
        upload
            .into_iter()
            .chain([ServerMessage::Frame(image_frame)]),
        at_revision(8),
    );
    for _ in 0..2 {
        assert_eq!(host.prepare_grid(slot).unwrap().revision, Some(8));
        assert_eq!(
            host.app.terminal.model.image_attachment(),
            Some(pending.attachment)
        );
        assert_eq!(
            host.app.window.as_ref().unwrap().renderer.image_resident(),
            budget
        );
    }

    host.app.fatal_error = Some("terminal failure with a resident image".into());
    let turns = host.app.turns;
    host.user_event(UserEvent::Transport, &mut Tx::default());
    assert!(!poll(&mut host, 0.0, &mut Tx::default()));
    assert_eq!(
        host.app.turns, turns,
        "no deadline or retry work after failure"
    );
    assert!(host.app.terminal.transport.is_none());
    assert!(host.app.window.is_none());
    assert_eq!(host.app.presented_revision(), None);
    assert_eq!(host.app.quit, QuitProgress::Idle);
}
