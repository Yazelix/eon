use super::*;
use crate::launch;
#[cfg(target_os = "linux")]
use eon_workspace_protocol::v41::{Pane, Tab};

#[test]
#[cfg(target_os = "linux")]
#[ignore = "starts its own isolated Wayland compositor; needs EON_VENUS_TEST_SWAY and EON_VENUS_TEST_LIBRARY_PATH"]
fn window_menu_application_routes_input_and_lifecycle() {
    use std::{fs, os::unix::fs::DirBuilderExt};
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    if std::env::var_os("EON_VENUS_TEST_MENU_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!("venus-menu-{}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let config = root.join("sway.conf");
        fs::write(&config, "output * resolution 800x600\nxwayland disable\n").unwrap();
        let mut compositor = Command::new(std::env::var_os("EON_VENUS_TEST_SWAY").unwrap())
            .args(["-c"])
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &root)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let socket = loop {
            if let Some(socket) = fs::read_dir(&root)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with("wayland-") && !name.ends_with(".lock")
                })
            {
                break socket.path();
            }
            assert!(
                Instant::now() < deadline,
                "private compositor did not start"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "application::tests::native::window_menu_application_routes_input_and_lifecycle",
                "--nocapture",
            ])
            .env("EON_VENUS_TEST_MENU_CHILD", "1")
            .env("XDG_RUNTIME_DIR", &root)
            .env("WAYLAND_DISPLAY", socket)
            .env(
                "LD_LIBRARY_PATH",
                std::env::var_os("EON_VENUS_TEST_LIBRARY_PATH").unwrap(),
            )
            .output()
            .unwrap();
        compositor.kill().unwrap();
        compositor.wait().unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    struct Probe(Application);
    impl ApplicationHandler<UserEvent> for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let app = &mut self.0;
            let mut snapshot = super::chrome::quit_snapshot();
            snapshot.busy = Some(super::chrome::busy_p1());
            let mut peer = snapshot.running.as_ref().unwrap().this.clone();
            peer.window = Some("01234567".into());
            let action = WindowMenuAction::peer(&peer);
            snapshot.running.as_mut().unwrap().others = Some(vec![peer]);
            app.workspace_model
                .apply(eon_workspace_protocol::v41::Response::Snapshot(snapshot));
            app.set_window_menu(true);
            app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Pressed,
                false,
                Some(WindowMenuAction::Quit),
            );
            assert_eq!(app.window_menu.pressed, Some(WindowMenuAction::Quit));
            assert!(app.handle_window_menu_key(
                event_loop,
                &ShortcutKey {
                    physical_key: PhysicalKey::Code(KeyCode::Escape),
                    state: ElementState::Pressed,
                    repeat: false,
                }
            ));
            assert!(!app.window_menu.open);
            assert!(app.window_menu.pressed.is_none());
            app.set_window_menu(true);
            app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Released,
                false,
                Some(WindowMenuAction::Quit),
            );
            assert!(app.close_prompt.is_none() && !event_loop.exiting());
            assert!(app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Pressed,
                true,
                None
            ));
            assert!(!app.window_menu.open);
            assert!(app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Released,
                false,
                None
            ));
            app.set_window_menu(true);
            app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Pressed,
                false,
                Some(WindowMenuAction::Quit),
            );
            assert!(app.close_prompt.is_none());
            app.handle_window_menu_button(
                event_loop,
                MouseButton::Left,
                ElementState::Released,
                false,
                Some(WindowMenuAction::Detach),
            );
            assert!(app.close_prompt.is_none() && !event_loop.exiting());
            app.activate_window_action(event_loop, WindowMenuAction::Quit);
            assert!(matches!(
                app.close_prompt.as_ref().map(|prompt| &prompt.target),
                Some(PromptTarget::QuitWindow)
            ));
            app.close_prompt = None;
            app.set_window_menu(true);
            app.activate_window_action(event_loop, action.clone());
            assert!(
                matches!(app.close_prompt.as_ref().map(|prompt| &prompt.target), Some(PromptTarget::QuitOtherWindow(target)) if *target == action)
            );
            app.close_prompt = None;
            app.set_window_menu(true);
            app.activate_window_action(event_loop, WindowMenuAction::Detach);
            assert!(event_loop.exiting());
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let arguments = launch::launch_arguments([OsString::from("/unused-orbit.sock")], None).unwrap();
    let mut probe = Probe(Application::new(arguments, event_loop.create_proxy()));
    event_loop.run_app(&mut probe).unwrap();
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
                .apply(eon_workspace_protocol::v41::Response::Snapshot(Snapshot {
                    workspace: "s1".into(),
                    active_workspace: "s1".into(),
                    workspaces: vec![eon_workspace_protocol::v41::WorkspaceInfo {
                        id: "s1".into(),
                        persistent_id: Some("s1".into()),
                        registration_error: None,
                        name: None,
                        tab_count: 1,
                        attention: None,
                        recency: 0,
                    }],
                    active_tab: "t1".into(),
                    geometry: eon_workspace_protocol::v41::PopupGeometry {
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
                        columns: vec![eon_workspace_protocol::v41::Column {
                            panes: vec![Pane {
                                label: "p1".into(),
                                pinned_name: None,
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
                    codex_quota_failure: None,
                    claude_quota_failure: None,
                    running: None,
                    busy: None,
                    sidebar: eon_workspace_protocol::v41::Sidebar::Open,
                    attention_queue: Vec::new(),
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
        fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
    use eon_workspace_protocol::v41 as workspace;
    use orbit_protocol::{Dimensions, Screen};
    use std::{
        os::unix::net::{UnixListener, UnixStream},
        sync::{Barrier, mpsc},
    };
    use winit::{
        event::{DeviceId, TouchPhase},
        platform::wayland::EventLoopBuilderExtWayland,
    };

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

        fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
                label: "p3".into(),
                pinned_name: None,
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
            let highlighted =
                |app: &Application| app.links.shown.as_ref().map(|link| (link.row, link.column));
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
            let refused = "Only HTTP and HTTPS links can be opened. Copy other targets explicitly.";
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
            // Metadata repaints bar and sidebar controls, not the terminal link beneath a
            // still pointer. Exercise pane and popup with the sidebar both visible and hidden.
            let original_snapshot = app.workspace_model.snapshot().unwrap().clone();
            let original_size = app.window.as_ref().unwrap().renderer.size();
            let original_cursor = app.cursor;
            app.window_event(
                event_loop,
                id,
                WindowEvent::Resized(PhysicalSize::new(2000, 900)),
            );
            for (popup, sidebar_open) in
                [(false, true), (true, true), (false, false), (true, false)]
            {
                let mut snapshot = original_snapshot.clone();
                snapshot.sidebar = if sidebar_open {
                    workspace::Sidebar::Open
                } else {
                    workspace::Sidebar::Closed
                };
                if popup {
                    show_test_popup(&mut snapshot);
                    snapshot.tabs[0].popups[0].endpoint =
                        app.terminal.active_endpoint.clone().unwrap();
                }
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    snapshot.clone(),
                )));
                let workspace = app.workspace_scene().unwrap();
                assert_eq!(workspace.sidebar().is_some(), sidebar_open);
                let metrics = app.window.as_ref().unwrap().renderer.metrics();
                app.cursor = PhysicalPosition::new(
                    f64::from(workspace.terminal.left + metrics.padding + metrics.width * 1.5),
                    f64::from(workspace.terminal.top + metrics.padding + metrics.height * 0.5),
                );
                app.render();
                let focus = app.links.focus.clone();
                use winit::keyboard::ModifiersState;
                for (modifiers, expected) in [
                    (ModifiersState::empty(), CursorIcon::Default),
                    (ModifiersState::CONTROL, CursorIcon::Pointer),
                    (ModifiersState::empty(), CursorIcon::Default),
                    (ModifiersState::SHIFT, CursorIcon::Default),
                    (
                        ModifiersState::CONTROL | ModifiersState::SHIFT,
                        CursorIcon::Default,
                    ),
                    (ModifiersState::CONTROL, CursorIcon::Pointer),
                ] {
                    // An expired age lets us observe refresh even when hover does not change.
                    app.next_age = Some(Instant::now());
                    let generation = app.presentation.generation;
                    app.window_event(
                        event_loop,
                        id,
                        WindowEvent::ModifiersChanged(modifiers.into()),
                    );
                    assert!(
                        app.presentation.generation > generation,
                        "modifiers must refresh the view"
                    );
                    assert_eq!(app.links.focus, focus, "modifiers keep the hovered target");
                    assert_eq!(app.pointer_icon(false, None), expected);
                }
                app.links.focus = None;
                assert_eq!(app.pointer_icon(false, None), CursorIcon::Default);
                app.links.focus = focus.clone();
                assert!(app.handle_link_button(ElementState::Pressed, MouseButton::Left));
                for progress in [1, 2, 3] {
                    snapshot.tabs[0].columns[0].panes[0].attention = Some(workspace::Attention {
                        state: workspace::AttentionState::Working,
                        record: Some(workspace::AttentionRecord {
                            progress: Some(progress),
                            app: "claude".into(),
                            name: String::new(),
                            title: String::new(),
                            message: String::new(),
                            kind: None,
                            changed_at: 0,
                            stale: false,
                            others: 0,
                        }),
                    });
                    app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                        snapshot.clone(),
                    )));
                    assert_eq!(app.presentation.has_presented_geometry(), !sidebar_open);
                    assert_eq!(app.links.focus, focus, "sidebar progress must keep hover");
                    assert!(matches!(app.links.pressed, Some(LinkPress::Live(_))));
                    assert_eq!(app.current_link().unwrap().uri, kept);
                    app.render();
                    assert_eq!(highlighted(app), Some((0, 0)));
                }
                let mut codex = snapshot.clone();
                codex.codex_quota = Some(workspace::CodexQuota {
                    state: workspace::CodexQuotaState::Fresh,
                    observed_at: 1,
                    windows: Vec::new(),
                });
                let mut claude = codex.clone();
                claude.claude_quota = Some(workspace::ClaudeQuota {
                    state: workspace::ClaudeQuotaState::Fresh,
                    observed_at: 1,
                    windows: Vec::new(),
                });
                let mut combined = claude.clone();
                combined.codex_quota.as_mut().unwrap().observed_at += 1;
                combined.claude_quota.as_mut().unwrap().observed_at += 1;
                combined.tabs[0].columns[0].panes[0]
                    .attention
                    .as_mut()
                    .unwrap()
                    .record
                    .as_mut()
                    .unwrap()
                    .progress = Some(4);
                for snapshot in [codex, claude, combined, snapshot] {
                    app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                        snapshot,
                    )));
                    assert!(
                        !app.presentation.has_presented_geometry(),
                        "quota controls wait for repaint"
                    );
                    assert_eq!(app.links.focus, focus, "quota refresh must keep hover");
                    assert!(matches!(app.links.pressed, Some(LinkPress::Live(_))));
                    assert_eq!(app.current_link().unwrap().uri, kept);
                    app.render();
                    assert_eq!(highlighted(app), Some((0, 0)));
                }
                app.links.notice = None;
                assert!(app.handle_link_button(ElementState::Released, MouseButton::Left));
                assert_eq!(
                    app.links.notice(),
                    Some(refused),
                    "the captured click must activate its kept target"
                );
            }
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                original_snapshot,
            )));
            app.cursor = original_cursor;
            app.window_event(event_loop, id, WindowEvent::Resized(original_size));
            app.render();
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
            let TransportEvent::Server(ServerMessage::Frame(mut scrolled)) = linked_frame(16, "")
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
            let socket =
                std::env::temp_dir().join(format!("venus-admission-{}.sock", std::process::id()));
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
            app.handle_workspace(WorkspaceEvent::Unavailable {
                notice: "Cannot reach Eon.".into(),
                detail: None,
            });
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
                eon_workspace_protocol::v41::Response::Snapshot(empty),
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v41::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 2,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v41::PopupGeometry {
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
                columns: vec![eon_workspace_protocol::v41::Column {
                    panes: vec![Pane {
                        label: format!("p{index}"),
                        pinned_name: None,
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
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v41::Sidebar::Open,
        attention_queue: Vec::new(),
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
                // Closing a subscription leaves Venus refreshing on its interval.
                if matches!(action, WorkspaceAction::Subscribe { .. }) {
                    continue;
                }
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

/// A writer that names each write as it begins, then holds it until the test
/// releases it.
struct GatedWriter {
    started: std::sync::mpsc::Sender<Vec<u8>>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Write for GatedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.started.send(bytes.to_vec()).unwrap();
        self.release.recv().unwrap();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn focus_writer_keeps_latest_without_blocking_native_reporting() {
    let mailbox = Arc::new(FocusMailbox::default());
    let reporter = FocusReporter {
        mailbox: Arc::clone(&mailbox),
    };
    let (started, writes) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let worker = {
        let mailbox = Arc::clone(&mailbox);
        thread::spawn(move || {
            send_focus(
                &mailbox,
                GatedWriter {
                    started,
                    release: held,
                },
            )
        })
    };
    let frame = || {
        writes
            .recv_timeout(Duration::from_secs(10))
            .expect("a focus write began")
    };

    reporter.report(true);
    assert_eq!(frame(), b"focus:1\n");
    // While that write is held, reports return at once and keep only the
    // latest focus.
    let (returned, reported) = std::sync::mpsc::channel();
    thread::spawn(move || {
        reporter.report(true);
        reporter.report(false);
        returned.send(reporter).unwrap();
    });
    let reporter = reported
        .recv_timeout(Duration::from_secs(10))
        .expect("a report waited for the held write");
    assert_eq!(mailbox.lock().focused, Some(false));
    release.send(()).unwrap();
    assert_eq!(frame(), b"focus:0\n");
    release.send(()).unwrap();

    // A repeated focus writes nothing once taken.
    reporter.report(false);
    while mailbox.lock().focused.is_some() {
        thread::yield_now();
    }
    reporter.report(true);
    assert_eq!(frame(), b"focus:1\n", "a repeated focus was written");
    release.send(()).unwrap();
    drop(reporter);
    worker.join().unwrap().unwrap();
}

/// A writer that is interrupted before each write and then takes at most
/// three bytes, sending what it holds after each.
struct ShortWriter {
    written: Vec<u8>,
    interrupted: bool,
    progress: std::sync::mpsc::Sender<Vec<u8>>,
}

impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.interrupted = !self.interrupted;
        if self.interrupted {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = bytes.len().min(3);
        self.written.extend_from_slice(&bytes[..count]);
        self.progress.send(self.written.clone()).unwrap();
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn interrupted_and_short_focus_writes_still_send_complete_frames() {
    let mailbox = Arc::new(FocusMailbox::default());
    let reporter = FocusReporter {
        mailbox: Arc::clone(&mailbox),
    };
    let (progress, written) = std::sync::mpsc::channel();
    let worker = {
        let mailbox = Arc::clone(&mailbox);
        thread::spawn(move || {
            send_focus(
                &mailbox,
                ShortWriter {
                    written: Vec::new(),
                    interrupted: false,
                    progress,
                },
            )
        })
    };
    let until = |length: usize| loop {
        let written = written
            .recv_timeout(Duration::from_secs(10))
            .expect("a focus write progressed");
        if written.len() >= length {
            return written;
        }
    };
    reporter.report(true);
    assert_eq!(until(8), b"focus:1\n");
    reporter.report(false);
    assert_eq!(until(16), b"focus:1\nfocus:0\n");
    drop(reporter);
    worker.join().unwrap().unwrap();
}

#[test]
fn a_failed_focus_report_closes_only_its_direction() {
    let (venus, mut eon) = UnixStream::pair().unwrap();
    // Fill the return direction so that the report's write times out.
    venus
        .set_write_timeout(Some(Duration::from_millis(1)))
        .unwrap();
    let mut filled = 0;
    while let Ok(count) = (&venus).write(&[0; 4096]) {
        filled += count;
    }
    let mailbox = Arc::new(FocusMailbox::default());
    let reporter = FocusReporter {
        mailbox: Arc::clone(&mailbox),
    };
    reporter.report(true);
    let output = venus.try_clone().unwrap();
    thread::spawn(move || report_focus(&mailbox, output))
        .join()
        .unwrap();

    // Eon reads what was written, then the end of reports.
    eon.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut returned = Vec::new();
    eon.read_to_end(&mut returned).unwrap();
    assert_eq!(returned, vec![0; filled]);
    // Presentation control still reads Eon's commands.
    eon.write_all(b"present\n").unwrap();
    let mut command = [0; 8];
    (&venus).read_exact(&mut command).unwrap();
    assert_eq!(&command, b"present\n");
    drop(reporter);
}

#[test]
fn focus_reporting_follows_admission() {
    // What Venus writes on its stdout, given how many frames it should hold.
    let admit = |admission, workspace, focused, frames: usize| {
        let (mut venus, mut eon) = UnixStream::pair().unwrap();
        eon.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let reporter = admit_presentation(&mut venus, admission, workspace, focused).unwrap();
        let mut written = vec![0; 8 * frames];
        eon.read_exact(&mut written).unwrap();
        let reports = reporter.is_some();
        drop((reporter, venus));
        eon.read_to_end(&mut written).unwrap();
        (written, reports)
    };
    // A focus that arrived before admission is reported after readiness.
    assert_eq!(
        admit(true, true, true, 2),
        (b"ready-v1focus:1\n".to_vec(), true)
    );
    assert_eq!(admit(false, true, false, 1), (b"focus:0\n".to_vec(), true));
    // eonbase reports readiness alone, or nothing.
    assert_eq!(admit(true, false, true, 1), (b"ready-v1".to_vec(), false));
    assert_eq!(admit(false, false, true, 0), (Vec::new(), false));

    // Failed readiness starts no reports.
    let (mut venus, eon) = UnixStream::pair().unwrap();
    drop(eon);
    assert!(admit_presentation(&mut venus, true, true, true).is_err());
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
fn spinner_turns_at_most_ten_times_a_second_and_only_while_shown() {
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

    // Polled every millisecond for two seconds, the badge steps every 100 ms: ten frames, one
    // braille cycle, a second.
    let turns = (0..=2000u64)
        .filter(|&ms| update_spinner(true, &mut deadline, now + Duration::from_millis(ms)))
        .collect::<Vec<_>>();
    assert_eq!(turns.len(), 20);
    assert!(turns.windows(2).all(|pair| pair[1] - pair[0] == 100));
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
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn quit_prompt_refreshes_and_a_stop_in_flight_holds_enter() {
    use eon_workspace_protocol::v41::{self as workspace, BusyTerminals};
    use winit::keyboard::ModifiersState;

    run_quit_probe("quit", |app, event_loop, busy, actions, _| {
        let described = |app: &Application| app.window.as_ref().unwrap().accessibility.dialog();
        // Alt+Shift+Q opens the quit prompt while p1 is busy.
        app.input
            .set_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
        assert!(press_shortcut(app, event_loop, KeyCode::KeyQ));
        assert!(release_shortcut(app, event_loop, KeyCode::KeyQ));
        app.input.set_modifiers(ModifiersState::empty());
        assert!(quit_prompt_open(app));
        let asked = described(app);
        assert_eq!(asked, app.dialog_scene());

        // p1's command ends: only `busy` changes, and the prompt is drawn and described again.
        let generation = app.presentation.generation;
        let mut idle = busy.clone();
        idle.busy = Some(BusyTerminals {
            terminals: vec![],
            omitted: 0,
        });
        app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
            idle,
        )));
        assert!(
            app.presentation.generation > generation,
            "the prompt is redrawn"
        );
        assert_ne!(described(app), asked);
        assert_eq!(
            described(app),
            app.dialog_scene(),
            "AccessKit reads the new prompt"
        );

        // Enter sends one Stop for this generation.
        assert!(press_shortcut(app, event_loop, KeyCode::Enter));
        assert!(release_shortcut(app, event_loop, KeyCode::Enter));
        assert!(app.close_prompt.is_none());
        let QuitProgress::Stopping(ticket) = app.quit else {
            panic!("Enter queues the Stop");
        };
        assert_eq!(actions.recv_timeout(Duration::from_secs(5)), Ok(stop_g7()));
        // While it is in flight, another Enter is taken by the shortcut chain, so it reaches
        // no terminal, and sends no second Stop.
        assert!(press_shortcut(app, event_loop, KeyCode::Enter));
        assert!(release_shortcut(app, event_loop, KeyCode::Enter));
        assert_eq!(app.quit, QuitProgress::Stopping(ticket));
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());
        // Alt+Shift+D still detaches.
        app.input
            .set_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
        assert!(press_shortcut(app, event_loop, KeyCode::KeyD));
        assert!(event_loop.exiting());
    });
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn a_close_request_quits_as_alt_shift_q_does() {
    use eon_workspace_protocol::v41::{self as workspace, BusyTerminals};

    run_quit_probe("close", |app, event_loop, busy, actions, terminal| {
        let id = app.window.as_ref().unwrap().window.id();
        let close = |app: &mut Application| {
            app.window_event(event_loop, id, WindowEvent::CloseRequested);
        };
        let show = |app: &mut Application, snapshot: &workspace::Snapshot| {
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                snapshot.clone(),
            )));
        };

        let escape = |app: &mut Application| {
            assert!(press_shortcut(app, event_loop, KeyCode::Escape));
            assert!(release_shortcut(app, event_loop, KeyCode::Escape));
        };
        let mut idle = busy.clone();
        idle.busy = Some(BusyTerminals {
            terminals: vec![],
            omitted: 0,
        });

        // While p1 is busy, a close request opens the quit prompt and the window stays.
        close(app);
        assert!(quit_prompt_shown(app));
        assert!(!event_loop.exiting());
        // A second close request leaves the prompt open.
        close(app);
        assert!(quit_prompt_shown(app));
        // p1's command ends while the prompt is open: the prompt still owns the answer, so
        // another close and the snapshots after it send no Stop without Enter.
        show(app, &idle);
        close(app);
        show(app, &idle);
        assert!(quit_prompt_shown(app));
        assert_eq!(app.quit, QuitProgress::Idle);
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());
        assert!(!event_loop.exiting());
        // Esc cancels, stopping nothing.
        escape(app);
        assert!(app.close_prompt.is_none());
        assert_eq!(app.dialog_scene(), None);
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());

        // A close request over the attention inbox or Shortcuts shows the quit prompt in its
        // place, and Esc answers the prompt. The dialog changes once, so the attached terminal
        // never regains focus in between.
        show(app, &busy);
        app.handle_transport(TransportEvent::Server(ServerMessage::Attached));
        app.window_event(event_loop, id, WindowEvent::Focused(true));
        terminal_focus(terminal);
        app.set_attention_inbox(true);
        assert_eq!(terminal_focus(terminal), [session::FocusEvent::Lost]);
        close(app);
        assert!(!app.attention_inbox.open);
        assert!(quit_prompt_shown(app));
        assert!(terminal_focus(terminal).is_empty(), "no focus-in between");
        escape(app);
        assert_eq!(app.dialog_scene(), None);
        assert_eq!(terminal_focus(terminal), [session::FocusEvent::Gained]);
        app.set_shortcut_viewer(true);
        close(app);
        assert!(!app.shortcut_viewer.open);
        assert!(quit_prompt_shown(app));
        escape(app);
        assert_eq!(app.dialog_scene(), None);
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());

        // While every terminal is idle, a close request quits without asking once the next
        // snapshot confirms it, and the window stays until the Stop is answered.
        show(app, &idle);
        close(app);
        assert_eq!(app.quit, QuitProgress::Pending);
        show(app, &idle);
        assert!(app.close_prompt.is_none());
        let QuitProgress::Stopping(ticket) = app.quit else {
            panic!("the close request queues the Stop");
        };
        assert_eq!(actions.recv_timeout(Duration::from_secs(5)), Ok(stop_g7()));
        assert!(!event_loop.exiting());

        // The Stop fails; without a running-windows summary (eonbase) a close request detaches.
        app.handle_workspace(WorkspaceEvent::Answered {
            ticket,
            accepted: false,
        });
        let mut eonbase = idle;
        eonbase.running = None;
        show(app, &eonbase);
        close(app);
        assert!(event_loop.exiting());
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());
    });
}

/// An empty Orbit frame at `revision`.
#[cfg(target_os = "linux")]
fn frame(revision: u64, screen: orbit_protocol::Screen) -> TransportEvent {
    use orbit_protocol::{Capabilities, Colors, Cursor, CursorShape, Dimensions, Frame, Rgb};

    TransportEvent::Server(ServerMessage::Frame(Box::new(Frame {
        revision,
        dimensions: Dimensions { cols: 0, rows: 0 },
        screen,
        scroll_position: orbit_protocol::ScrollPosition::default(),
        title: format!("output {revision}"),
        working_directory: String::new(),
        capabilities: Capabilities { hyperlinks: true },
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
        graphics: None,
    })))
}

/// A press outside the shortcut viewer, over the presented terminal, closes it and restores
/// terminal focus; that press and its release, from any button, reach no terminal and send no
/// workspace action. A press inside keeps the viewer open. The same presses with the viewer
/// closed do reach the terminal.
#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn a_press_outside_the_shortcut_viewer_closes_it_and_reaches_nothing() {
    use winit::{event::DeviceId, keyboard::ModifiersState};

    run_quit_probe("viewer-press", |app, event_loop, _, actions, terminal| {
        let id = app.window.as_ref().unwrap().window.id();
        let device_id = DeviceId::dummy();
        let move_to = |app: &mut Application, (x, y): (f32, f32)| {
            let position = PhysicalPosition::new(f64::from(x), f64::from(y));
            app.window_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id,
                    position,
                },
            );
        };
        let mouse = |app: &mut Application, button, state| {
            app.window_event(
                event_loop,
                id,
                WindowEvent::MouseInput {
                    device_id,
                    state,
                    button,
                },
            );
        };
        let click = |app: &mut Application, at, button| {
            move_to(app, at);
            mouse(app, button, ElementState::Pressed);
            mouse(app, button, ElementState::Released);
        };
        let sent = || {
            std::iter::from_fn(|| terminal.recv_timeout(Duration::from_millis(300)).ok())
                .collect::<Vec<_>>()
        };
        app.handle_transport(TransportEvent::Server(ServerMessage::Attached));
        app.handle_transport(frame(1, orbit_protocol::Screen::Primary));
        app.render();
        assert_eq!(app.presentation.presented_revision(), Some(1));
        app.window_event(event_loop, id, WindowEvent::Focused(true));
        sent();
        let screen = app.workspace_scene().unwrap().terminal;
        let outside = (screen.right() - 4.0, screen.top + screen.height / 2.0);
        assert!(screen.contains(outside.0, outside.1));
        let buttons = [MouseButton::Left, MouseButton::Middle, MouseButton::Right];
        for button in buttons {
            app.input.set_modifiers(ModifiersState::ALT);
            assert!(press_shortcut(app, event_loop, KeyCode::Slash));
            assert!(release_shortcut(app, event_loop, KeyCode::Slash));
            app.input.set_modifiers(ModifiersState::empty());
            assert!(app.shortcut_viewer.open);
            assert_eq!(sent(), [ClientMessage::Focus(session::FocusEvent::Lost)]);
            let bounds = app.dialog_scene().unwrap().bounds;
            assert!(!bounds.contains(outside.0, outside.1), "{bounds:?}");

            let inside = (
                bounds.left + bounds.width / 2.0,
                bounds.top + bounds.height / 2.0,
            );
            click(app, inside, button);
            assert!(
                app.shortcut_viewer.open,
                "{button:?} inside closed the viewer"
            );
            assert_eq!(sent(), [], "{button:?} inside reached the terminal");

            move_to(app, outside);
            mouse(app, button, ElementState::Pressed);
            assert!(
                !app.shortcut_viewer.open,
                "{button:?} outside kept the viewer"
            );
            mouse(app, button, ElementState::Released);
            assert_eq!(
                sent(),
                [ClientMessage::Focus(session::FocusEvent::Gained)],
                "{button:?} reached the terminal"
            );
            assert!(actions.recv_timeout(Duration::from_millis(300)).is_err());
        }
        // The fixture sees terminal input: with the viewer closed, each press reaches it, as a
        // press of that button or, for the left one, the start of a selection.
        for (button, expected) in buttons.into_iter().zip([
            session::MouseButton::Left,
            session::MouseButton::Middle,
            session::MouseButton::Right,
        ]) {
            click(app, outside, button);
            let messages = sent();
            assert!(
                messages.iter().any(|message| match message {
                    ClientMessage::Mouse(event) => {
                        event.action == session::MouseAction::Press
                            && event.button == Some(expected)
                    }
                    ClientMessage::Selection(SelectionAction::Begin { .. }) => {
                        button == MouseButton::Left
                    }
                    _ => false,
                }),
                "{button:?} with the viewer closed sent {messages:?}"
            );
        }
        event_loop.exit();
    });
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn a_close_request_detaches_while_the_workspace_is_unavailable() {
    run_quit_probe("unavailable", |app, event_loop, _, actions, _| {
        let id = app.window.as_ref().unwrap().window.id();
        app.handle_workspace(WorkspaceEvent::Unavailable {
            notice: "Eon is not answering.".into(),
            detail: None,
        });
        app.window_event(event_loop, id, WindowEvent::CloseRequested);
        assert!(event_loop.exiting());
        assert!(app.close_prompt.is_none());
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());
    });
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn a_close_request_detaches_after_a_failed_stop() {
    use eon_workspace_protocol::v41::{self as workspace, BusyTerminals};

    run_quit_probe("failed-stop", |app, event_loop, busy, actions, _| {
        let id = app.window.as_ref().unwrap().window.id();
        let mut idle = busy;
        idle.busy = Some(BusyTerminals {
            terminals: vec![],
            omitted: 0,
        });
        let show = |app: &mut Application, response: workspace::Response| {
            app.handle_workspace(WorkspaceEvent::Response(response));
        };
        show(app, workspace::Response::Snapshot(idle.clone()));
        app.window_event(event_loop, id, WindowEvent::CloseRequested);
        show(app, workspace::Response::Snapshot(idle));
        let QuitProgress::Stopping(ticket) = app.quit else {
            panic!("the close request queues the Stop");
        };
        assert_eq!(actions.recv_timeout(Duration::from_secs(5)), Ok(stop_g7()));
        // The Stop fails, and Eon refuses every refresh until a Stop completes: another close
        // detaches instead of sending the failing Stop again.
        app.handle_workspace(WorkspaceEvent::Answered {
            ticket,
            accepted: false,
        });
        show(
            app,
            workspace::Response::Failure(workspace::Failure {
                code: "stop-incomplete".into(),
                detail: "window cleanup is incomplete; retry `eon window stop 00000007`".into(),
            }),
        );
        assert_eq!(
            app.workspace_model.notice(),
            Some("Quit did not finish; press Alt+Shift+Q to retry.")
        );
        assert!(!event_loop.exiting());
        app.window_event(event_loop, id, WindowEvent::CloseRequested);
        assert!(event_loop.exiting());
        assert!(actions.recv_timeout(Duration::from_millis(750)).is_err());
    });
}

/// The focus changes Venus reported to the attached terminal since the last call.
#[cfg(target_os = "linux")]
fn terminal_focus(terminal: &std::sync::mpsc::Receiver<ClientMessage>) -> Vec<session::FocusEvent> {
    std::iter::from_fn(|| terminal.recv_timeout(Duration::from_millis(300)).ok())
        .filter_map(|message| match message {
            ClientMessage::Focus(event) => Some(event),
            _ => None,
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn stop_g7() -> Outgoing {
    Outgoing::Stop {
        generation: GENERATION_G7.into(),
    }
}

/// The probe window's generation, which the Reserved Stop carries.
#[cfg(target_os = "linux")]
const GENERATION_G7: &str = "g1-00000000000000000000000000000007";

/// Whether the quit prompt is the dialog shown, and AccessKit describes it. The attention inbox
/// draws over every other dialog and has filter chips; the prompt draws over Shortcuts.
#[cfg(target_os = "linux")]
fn quit_prompt_shown(app: &Application) -> bool {
    let shown = app.dialog_scene();
    quit_prompt_open(app)
        && !app.attention_inbox.open
        && shown
            .as_ref()
            .is_some_and(|dialog| dialog.filters.is_empty())
        && app.window.as_ref().unwrap().accessibility.dialog() == shown
}

#[cfg(target_os = "linux")]
fn quit_prompt_open(app: &Application) -> bool {
    app.close_prompt
        .as_ref()
        .is_some_and(|prompt| matches!(prompt.target, PromptTarget::QuitWindow))
}

/// Drives the production shortcut chain, as winit's key events do.
#[cfg(target_os = "linux")]
fn press_shortcut(app: &mut Application, event_loop: &ActiveEventLoop, code: KeyCode) -> bool {
    app.handle_shortcut_key(
        event_loop,
        &ShortcutKey {
            physical_key: PhysicalKey::Code(code),
            state: ElementState::Pressed,
            repeat: false,
        },
    )
}

#[cfg(target_os = "linux")]
fn release_shortcut(app: &mut Application, event_loop: &ActiveEventLoop, code: KeyCode) -> bool {
    app.handle_shortcut_key(
        event_loop,
        &ShortcutKey {
            physical_key: PhysicalKey::Code(code),
            state: ElementState::Released,
            repeat: false,
        },
    )
}

/// Two quick popup toggles answered in one batch, hidden then shown again, replace the popup's
/// attachment with a new one on the same endpoint. Orbit frames drained for the old attachment
/// before that batch must not reach the new one, which is still connecting: one would mark it
/// lost and drop its transport.
#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn a_quick_popup_hide_and_show_keeps_the_popup_attached() {
    run_quit_probe("popup-toggle", |app, event_loop, busy, _, _| {
        use eon_workspace_protocol::v41 as workspace;
        use orbit_protocol::{
            Capabilities, Cell, CellStyle, CellWidth, Colors, Cursor, CursorShape, Dimensions,
            Frame, Rgb, Row, Screen, StyleColor, Underline,
        };
        use std::{
            io::{Read, Write},
            os::unix::net::{UnixListener, UnixStream},
            sync::mpsc,
        };

        fn read_hello(stream: &mut UnixStream) {
            let mut header = [0; session::HEADER_BYTES];
            stream.read_exact(&mut header).unwrap();
            let mut bytes = Vec::from(header);
            bytes.resize(session::client_message_len(&header).unwrap().unwrap(), 0);
            stream
                .read_exact(&mut bytes[session::HEADER_BYTES..])
                .unwrap();
            assert_eq!(
                session::decode_client_message(&bytes).unwrap(),
                ClientMessage::Hello
            );
        }

        let root = std::env::temp_dir().join(format!("venus-toggle-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut shown = busy.clone();
        // Only what EONW encodes for this window.
        (shown.running, shown.busy) = (None, None);
        super::show_test_popup(&mut shown);
        shown.tabs[0].popups[0].endpoint = root.join("popup.sock").into_os_string().into_vec();
        let mut hidden = shown.clone();
        hidden.tabs[0].selected_popup = None;

        // The popup's Orbit answers each attachment; the first also draws once when asked.
        let popup = UnixListener::bind(root.join("popup.sock")).unwrap();
        let (draw, drawing) = mpsc::channel::<()>();
        let (drew, drawn) = mpsc::channel::<()>();
        thread::spawn(move || {
            let mut held = Vec::new();
            for (index, stream) in popup.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                read_hello(&mut stream);
                stream
                    .write_all(&session::encode_server_message(&ServerMessage::Attached).unwrap())
                    .unwrap();
                if index == 0 {
                    drawing.recv().unwrap();
                    let style = CellStyle {
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
                    };
                    let frame = Frame {
                        revision: 1,
                        dimensions: Dimensions { cols: 1, rows: 1 },
                        screen: Screen::Alternate,
                        scroll_position: orbit_protocol::ScrollPosition::default(),
                        title: "yazi".into(),
                        working_directory: String::new(),
                        capabilities: Capabilities { hyperlinks: true },
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
                        rows: vec![Row {
                            wrapped: false,
                            wrap_continuation: false,
                            cells: vec![Cell {
                                width: CellWidth::Narrow,
                                text: "y".into(),
                                hyperlink: String::new(),
                                style,
                            }],
                        }],
                        graphics: None,
                    };
                    stream
                        .write_all(
                            &session::encode_server_message(&ServerMessage::Frame(Box::new(frame)))
                                .unwrap(),
                        )
                        .unwrap();
                    drew.send(()).unwrap();
                }
                held.push(stream);
            }
        });

        // Eon answers refreshes with the popup shown, the first action hiding it and the
        // second showing it again, and refuses subscriptions by closing them.
        let eon = UnixListener::bind(root.join("eon.sock")).unwrap();
        let (answered, answers) = mpsc::channel::<()>();
        {
            let (shown, hidden) = (shown.clone(), hidden.clone());
            thread::spawn(move || {
                let mut actions = 0;
                for stream in eon.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut header = [0; workspace::HEADER_BYTES];
                    if stream.read_exact(&mut header).is_err() {
                        continue;
                    }
                    let mut bytes = Vec::from(header);
                    bytes.resize(workspace::declared_message_len(&header).unwrap(), 0);
                    stream
                        .read_exact(&mut bytes[workspace::HEADER_BYTES..])
                        .unwrap();
                    let snapshot = match workspace::decode_request(&bytes).unwrap().action {
                        WorkspaceAction::Subscribe { .. } => continue,
                        WorkspaceAction::Workspace(CommonAction::Inspect) => &shown,
                        _ => {
                            actions += 1;
                            if actions == 1 { &hidden } else { &shown }
                        }
                    };
                    let _ = stream.write_all(
                        &workspace::encode_response(&workspace::Response::Snapshot(
                            snapshot.clone(),
                        ))
                        .unwrap(),
                    );
                    if actions == 2 {
                        let _ = answered.send(());
                    }
                }
            });
        }
        app.workspace_transport = Some(WorkspaceTransport::start(root.join("eon.sock"), || {}));

        // The popup shows and its attachment is admitted.
        app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
            shown.clone(),
        )));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.terminal.model.is_attached() {
            assert!(Instant::now() < deadline, "the popup attaches");
            thread::sleep(Duration::from_millis(20));
            app.user_event(event_loop, UserEvent::Transport);
        }

        // Two toggles: Eon answers both before the window drains them, and the popup draws.
        let focus = |pane: &str| WorkspaceAction::Workspace(CommonAction::FocusId(pane.into()));
        let transport = app.workspace_transport.as_ref().unwrap();
        transport.send(focus("p1")).unwrap();
        transport.send(focus("p1")).unwrap();
        answers.recv_timeout(Duration::from_secs(5)).unwrap();
        draw.send(()).unwrap();
        drawn.recv_timeout(Duration::from_secs(5)).unwrap();
        // The workspace worker and the Orbit reader queue what they read.
        thread::sleep(Duration::from_millis(300));

        app.user_event(event_loop, UserEvent::Transport);
        assert_eq!(
            app.terminal.active_endpoint,
            Some(root.join("popup.sock").into_os_string().into_vec())
        );
        assert!(
            !matches!(
                app.terminal.model.connection(),
                ConnectionState::Lost { .. }
            ),
            "{:?}",
            app.terminal.model.connection()
        );
        assert!(app.terminal.transport.is_some(), "the popup stays attached");
        event_loop.exit();
        std::fs::remove_dir_all(root).unwrap();
    });
}

/// What a quit probe checks once the window is presented with the busy snapshot: the app,
/// its event loop, that snapshot, every action but Inspect that Venus sent, and every message
/// Venus sent its Orbit attachment.
#[cfg(target_os = "linux")]
type QuitCheck = fn(
    &mut Application,
    &ActiveEventLoop,
    eon_workspace_protocol::v41::Snapshot,
    &std::sync::mpsc::Receiver<Outgoing>,
    &std::sync::mpsc::Receiver<ClientMessage>,
);

/// Opens a native window on a recorded workspace whose snapshot has running window g7, with
/// p1 busy, and runs `check` once the window is presented. The check must end the event loop.
#[cfg(target_os = "linux")]
fn run_quit_probe(name: &str, check: QuitCheck) {
    use eon_workspace_protocol::v41::{
        self as workspace, BusyTerminal, BusyTerminals, RunningWindows, TargetId, TerminalTarget,
        WindowEntry, WindowState,
    };
    use std::{os::unix::net::UnixListener, sync::mpsc};
    use winit::platform::wayland::EventLoopBuilderExtWayland;

    struct Probe {
        app: Application,
        busy: workspace::Snapshot,
        actions: mpsc::Receiver<Outgoing>,
        terminal: mpsc::Receiver<ClientMessage>,
        check: QuitCheck,
        checked: bool,
    }
    impl ApplicationHandler<UserEvent> for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.app.resumed(event_loop);
        }
        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
            if matches!(event, UserEvent::Exit) {
                event_loop.exit();
                return;
            }
            self.app.user_event(event_loop, event);
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.app.window_event(event_loop, id, event);
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.app.about_to_wait(event_loop);
            if !self.checked && !self.app.initial_scale_pending && self.app.window.is_some() {
                let app = &mut self.app;
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    self.busy.clone(),
                )));
                app.render();
                assert!(app.presentation.has_presented_geometry());
                (self.check)(
                    app,
                    event_loop,
                    self.busy.clone(),
                    &self.actions,
                    &self.terminal,
                );
                self.checked = true;
            }
        }
    }

    let root = std::env::temp_dir().join(format!("venus-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let workspace_socket = root.join("workspace.sock");
    let listener = UnixListener::bind(&workspace_socket).unwrap();
    let (sender, actions) = mpsc::channel();
    // Answers refreshes and records every other action; nothing drains the answers.
    thread::spawn(move || {
        let failure = workspace::Response::Failure(workspace::Failure {
            code: "test".into(),
            detail: "recorded".into(),
        });
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut header = [0; workspace::HEADER_BYTES];
            if stream.read_exact(&mut header).is_err() {
                continue;
            }
            let mut bytes = Vec::from(header);
            if eon_workspace_protocol::stop::is_reserved(&header) {
                use eon_workspace_protocol::stop;
                bytes.resize(stop::request_len(&header).unwrap(), 0);
                stream
                    .read_exact(&mut bytes[workspace::HEADER_BYTES..])
                    .unwrap();
                let generation = stop::decode_request(&bytes).unwrap();
                let refused = stop::Reply::Failure {
                    code: "test".into(),
                    detail: "recorded".into(),
                };
                let _ = stream.write_all(&stop::encode_reply(&refused).unwrap());
                if sender.send(Outgoing::Stop { generation }).is_err() {
                    return;
                }
                continue;
            }
            bytes.resize(workspace::declared_message_len(&header).unwrap(), 0);
            stream
                .read_exact(&mut bytes[workspace::HEADER_BYTES..])
                .unwrap();
            let action = workspace::decode_request(&bytes).unwrap().action;
            // Closing a subscription leaves Venus refreshing on its interval.
            if matches!(action, WorkspaceAction::Subscribe { .. }) {
                continue;
            }
            let _ = stream.write_all(&workspace::encode_response(&failure).unwrap());
            if action != WorkspaceAction::Workspace(CommonAction::Inspect)
                && sender.send(action.into()).is_err()
            {
                return;
            }
        }
    });
    // The busy snapshot's p1 endpoint, which Venus attaches to; its messages are recorded.
    let orbit = UnixListener::bind(root.join("p1.sock")).unwrap();
    let (terminal_sender, terminal) = mpsc::channel();
    thread::spawn(move || {
        for stream in orbit.incoming() {
            let Ok(mut stream) = stream else { return };
            let sender = terminal_sender.clone();
            thread::spawn(move || {
                loop {
                    let mut header = [0; session::HEADER_BYTES];
                    if stream.read_exact(&mut header).is_err() {
                        return;
                    }
                    let Ok(Some(len)) = session::client_message_len(&header) else {
                        return;
                    };
                    let mut bytes = Vec::from(header);
                    bytes.resize(len, 0);
                    if stream
                        .read_exact(&mut bytes[session::HEADER_BYTES..])
                        .is_err()
                    {
                        return;
                    }
                    if let Ok(message) = session::decode_client_message(&bytes)
                        && sender.send(message).is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    let busy = workspace::Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![workspace::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: workspace::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![workspace::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: root.join("p1.sock").into_os_string().into_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: vec![],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: Some(RunningWindows {
            this: WindowEntry {
                window: None,
                generation: GENERATION_G7.into(),
                name: None,
                started_at: None,
                live_terminals: Some(3),
                state: WindowState::Live,
            },
            others: None,
            omitted: 0,
        }),
        busy: Some(BusyTerminals {
            terminals: vec![BusyTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p1".into()),
                    terminal: "s1".into(),
                },
                name: "p1".into(),
            }],
            omitted: 0,
        }),
        sidebar: workspace::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut app = Application::new(
        launch::launch_arguments(
            ["--font-family", "DejaVu Sans Mono", "--font-size", "20"]
                .into_iter()
                .map(OsString::from)
                .chain([root.join("orbit.sock").into_os_string()]),
            None,
        )
        .unwrap(),
        event_loop.create_proxy(),
    );
    app.workspace_transport = Some(WorkspaceTransport::start(workspace_socket, || {}));
    let mut probe = Probe {
        app,
        busy,
        actions,
        terminal,
        check,
        checked: false,
    };
    let proxy = event_loop.create_proxy();
    let (stop, stopped) = mpsc::channel::<()>();
    let watchdog = thread::spawn(move || {
        if stopped.recv_timeout(Duration::from_secs(20)) == Err(mpsc::RecvTimeoutError::Timeout) {
            let _ = proxy.send_event(UserEvent::Exit);
        }
    });
    event_loop.run_app(&mut probe).unwrap();
    let _ = stop.send(());
    watchdog.join().unwrap();
    assert!(probe.checked);
    drop(probe);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
fn pane_actions_wait_for_selection_and_cancel_on_local_input() {
    use eon_workspace_protocol::v41 as workspace;
    use std::{fs, os::unix::net::UnixListener, sync::mpsc};
    use winit::{
        event::DeviceId, keyboard::ModifiersState, platform::wayland::EventLoopBuilderExtWayland,
    };

    struct Probe {
        app: Application,
        snapshot: Snapshot,
        actions: mpsc::Receiver<WorkspaceAction>,
        checked: bool,
    }
    impl ApplicationHandler<UserEvent> for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.app.create_window(event_loop).unwrap();
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.app.window_event(event_loop, id, event);
            if self.checked || !self.app.presentation.has_presented_geometry() {
                return;
            }
            self.checked = true;
            let app = &mut self.app;
            let device_id = DeviceId::dummy();
            for focus in [WorkspaceFocus::Terminal, WorkspaceFocus::Sidebar] {
                app.set_workspace_focus(focus);
                for focused in [false, true] {
                    app.window_event(event_loop, id, WindowEvent::Focused(focused));
                    let scene = app.workspace_scene().unwrap();
                    for pane in &scene.panes {
                        let (target, _) = scene
                            .pane_action(&pane.id, PaneHeaderAction::Close)
                            .unwrap();
                        assert_eq!(
                            scene.hit_test(
                                target.left + target.width / 2.0,
                                target.top + target.height / 2.0,
                            ),
                            Some(if pane.selected {
                                WorkspaceHit::PaneAction(&pane.id, PaneHeaderAction::Close)
                            } else {
                                WorkspaceHit::Pane(&pane.id)
                            })
                        );
                    }
                }
            }
            app.set_workspace_focus(WorkspaceFocus::Terminal);
            app.render();
            let (button, _) = app
                .workspace_scene()
                .unwrap()
                .pane_action("p2", PaneHeaderAction::Close)
                .unwrap();
            app.window_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id,
                    position: PhysicalPosition::new(
                        f64::from(button.left + button.width / 2.0),
                        f64::from(button.top + button.height / 2.0),
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
            assert!(app.header_press.is_none());
            assert_eq!(
                self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                WorkspaceAction::Workspace(CommonAction::FocusId("p2".into()))
            );
            let mut selected = self.snapshot.clone();
            selected.tabs[0].selected_pane = Some("p2".into());
            selected.tabs[0].columns[0].expanded = "p2".into();
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                selected,
            )));
            app.render();
            let click = |app: &mut Application, state| {
                app.window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state,
                        button: MouseButton::Left,
                    },
                );
            };
            click(app, ElementState::Released);
            assert!(
                self.actions.try_recv().is_err(),
                "the first release cannot close"
            );
            let (button, _) = app
                .workspace_scene()
                .unwrap()
                .pane_action("p2", PaneHeaderAction::Close)
                .unwrap();
            app.cursor = PhysicalPosition::new(
                f64::from(button.left + button.width / 2.0),
                f64::from(button.top + button.height / 2.0),
            );
            click(app, ElementState::Pressed);
            click(app, ElementState::Released);
            assert_eq!(
                self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                WorkspaceAction::ClosePane { pane: "p2".into() }
            );
            // The real transport queues FocusId; replies are delivered explicitly in wire order.
            for cancel in 0..5 {
                app.set_shortcut_viewer(false);
                app.input.set_modifiers(ModifiersState::empty());
                app.zoom = None;
                app.set_workspace_focus(WorkspaceFocus::Terminal);
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    self.snapshot.clone(),
                )));
                app.render();
                // AccessKit can still activate Zoom on a pane whose mouse buttons are hidden.
                app.pane_button("p2", PaneHeaderAction::Zoom, true);
                assert_eq!(
                    self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                    WorkspaceAction::Workspace(CommonAction::FocusId("p2".into()))
                );
                let ticket = app
                    .pending_pane
                    .as_ref()
                    .expect("semantic activation queues delayed zoom")
                    .ticket;
                assert!(!app.zoomed());
                match cancel {
                    1 => app.set_workspace_focus(WorkspaceFocus::Tabs),
                    2 => {
                        app.input.set_modifiers(ModifiersState::ALT);
                        assert!(press_shortcut(app, event_loop, KeyCode::Slash));
                        assert!(app.shortcut_viewer.open);
                        release_shortcut(app, event_loop, KeyCode::Slash);
                    }
                    3 => {
                        assert!(
                            !press_shortcut(app, event_loop, KeyCode::KeyA),
                            "ordinary text goes past the shortcut chain to the terminal"
                        );
                        release_shortcut(app, event_loop, KeyCode::KeyA);
                    }
                    4 => {
                        app.window_event(event_loop, id, WindowEvent::Ime(Ime::Commit("x".into())))
                    }
                    _ => {}
                }
                if cancel != 0 {
                    assert!(app.pending_pane.is_none(), "local interaction {cancel}");
                }
                let mut selected = self.snapshot.clone();
                selected.tabs[0].selected_pane = Some("p2".into());
                selected.tabs[0].columns[0].expanded = "p2".into();
                app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                    selected,
                )));
                assert!(
                    !app.zoomed(),
                    "a snapshot alone does not accept our request"
                );
                app.handle_workspace(WorkspaceEvent::Answered {
                    ticket,
                    accepted: true,
                });
                assert_eq!(
                    app.zoomed(),
                    cancel == 0,
                    "completion after local interaction {cancel}"
                );
                if cancel == 2 {
                    assert!(app.shortcut_viewer.open);
                }
            }
            app.set_shortcut_viewer(false);
            app.input.set_modifiers(ModifiersState::empty());
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                self.snapshot.clone(),
            )));
            app.set_workspace_focus(WorkspaceFocus::PaneAction(PaneHeaderAction::Close));
            app.render();
            assert!(press_shortcut(app, event_loop, KeyCode::Enter));
            release_shortcut(app, event_loop, KeyCode::Enter);
            assert_eq!(
                self.actions.recv_timeout(Duration::from_secs(2)).unwrap(),
                WorkspaceAction::ClosePane { pane: "p1".into() }
            );
            assert_eq!(
                app.workspace_focus,
                WorkspaceFocus::Panes,
                "Close must not leave Enter aimed at the next pane's Close"
            );
            let mut closed = self.snapshot.clone();
            closed.tabs[0].columns[0].panes.remove(0);
            closed.tabs[0].columns[0].expanded = "p2".into();
            closed.tabs[0].selected_pane = Some("p2".into());
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                closed,
            )));
            assert!(press_shortcut(app, event_loop, KeyCode::Enter));
            assert_eq!(app.workspace_focus, WorkspaceFocus::Panes);

            let mut stacked = self.snapshot.clone();
            for index in 3..=30 {
                let mut pane = stacked.tabs[0].columns[0].panes[0].clone();
                pane.id = format!("p{index}");
                pane.session = format!("s{index}");
                pane.label = format!("p{index}");
                pane.endpoint.extend(index.to_string().bytes());
                stacked.tabs[0].columns[0].panes.push(pane);
            }
            app.handle_workspace(WorkspaceEvent::Response(workspace::Response::Snapshot(
                stacked,
            )));
            app.set_workspace_focus(WorkspaceFocus::PaneAction(PaneHeaderAction::Close));
            app.render();
            let metrics = app.window.as_ref().unwrap().renderer.metrics();
            app.scroll_workspace(MouseScrollDelta::LineDelta(0.0, -1.0), false, metrics);
            assert!(app.pane_scroll > 0.0);
            assert!(
                app.workspace_scene()
                    .unwrap()
                    .pane_action("p1", PaneHeaderAction::Close)
                    .is_none()
            );
            assert_eq!(
                app.workspace_focus,
                WorkspaceFocus::Panes,
                "scrolling a focused button away must return focus to its title"
            );
            event_loop.exit();
        }
    }
    let root = std::env::temp_dir().join(format!("venus-pane-actions-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let socket = root.join("workspace.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut snapshot = super::chrome::quit_snapshot();
    snapshot.running = None;
    snapshot.workspaces = vec![workspace::WorkspaceInfo {
        id: "s1".into(),
        persistent_id: Some("s1".into()),
        registration_error: None,
        name: None,
        tab_count: 1,
        attention: None,
        recency: 0,
    }];
    snapshot.tabs = vec![Tab {
        id: "t1".into(),
        directory: b"/tmp".to_vec(),
        pending: false,
        selected_pane: Some("p1".into()),
        selected_popup: None,
        columns: vec![workspace::Column {
            expanded: "p1".into(),
            panes: (1..=2)
                .map(|n| Pane {
                    id: format!("p{n}"),
                    label: format!("p{n}"),
                    pinned_name: None,
                    session: format!("s{n}"),
                    endpoint: root.join(format!("p{n}.sock")).into_os_string().into_vec(),
                    live: true,
                    attention: None,
                })
                .collect(),
        }],
        popups: vec![],
        attention_rollup: None,
    }];
    let response =
        workspace::encode_response(&workspace::Response::Snapshot(snapshot.clone())).unwrap();
    let (sender, actions) = mpsc::channel();
    let server = thread::spawn(move || {
        for _ in 0..8 {
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let mut header = [0; workspace::HEADER_BYTES];
                stream.read_exact(&mut header).unwrap();
                let mut bytes = Vec::from(header);
                bytes.resize(workspace::declared_message_len(&header).unwrap(), 0);
                stream
                    .read_exact(&mut bytes[workspace::HEADER_BYTES..])
                    .unwrap();
                let action = workspace::decode_request(&bytes).unwrap().action;
                if matches!(action, WorkspaceAction::Subscribe { .. }) {
                    continue;
                }
                stream.write_all(&response).unwrap();
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
    let arguments =
        launch::launch_arguments([root.join("unused-orbit.sock").into_os_string()], None).unwrap();
    let mut app = Application::new(arguments, event_loop.create_proxy());
    app.initial_scale_pending = false;
    app.workspace_model
        .apply(workspace::Response::Snapshot(snapshot.clone()));
    app.workspace_transport = Some(WorkspaceTransport::start(socket, || {}));
    let mut probe = Probe {
        app,
        snapshot,
        actions,
        checked: false,
    };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.checked);
    drop(probe);
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
