use super::*;
use crate::application::tests::busy_window_snapshot;
use eon_workspace_protocol::v49::Response;
use std::ffi::OsString;
use winit::keyboard::{KeyCode, PhysicalKey};

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
    Control::new(move || quit.push("quit"))
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
    // Ended, Stopped or a fatal error exits through the lane too, after the control thread's
    // Presents.
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
    let mut host = Host {
        app: Application::new(arguments, EventSink::Host(Arc::new(|_| true))),
        control: Arc::new(Control::new(quit)),
        failure: Failure::default(),
        fonts: None,
        early: Early::default(),
        geometry: None,
        identity: None,
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
