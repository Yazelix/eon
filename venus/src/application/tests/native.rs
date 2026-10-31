use super::*;
use crate::launch;
#[cfg(target_os = "linux")]
use eon_workspace_protocol::v26::{Pane, Tab};

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
                .apply(eon_workspace_protocol::v26::Response::Snapshot(Snapshot {
                    workspace: "w1".into(),
                    active_workspace: "w1".into(),
                    workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                        id: "w1".into(),
                        name: None,
                        tab_count: 1,
                        attention: None,
                        recency: 0,
                    }],
                    active_tab: "t1".into(),
                    geometry: eon_workspace_protocol::v26::PopupGeometry {
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
                        columns: vec![eon_workspace_protocol::v26::Column {
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
                    sidebar: eon_workspace_protocol::v26::Sidebar::Open,
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
    use eon_workspace_protocol::v26 as workspace;
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
                eon_workspace_protocol::v26::Response::Snapshot(empty),
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
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 2,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v26::PopupGeometry {
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
                columns: vec![eon_workspace_protocol::v26::Column {
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
        sidebar: eon_workspace_protocol::v26::Sidebar::Open,
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
