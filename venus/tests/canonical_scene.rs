use eon_workspace_protocol::v21::{Pane, Snapshot, Tab};
use orbit_protocol::{
    Capabilities, Cell, CellStyle, CellWidth, Colors, Cursor, CursorShape, CursorViewport,
    Dimensions, Frame, Graphics, ImageLayer, Placement, PlacementState, Rgb, Row, Screen,
    SourceRect, StyleColor, Underline,
    session::{
        self, ClipboardLocation, Failure, FailureCode, PixelFormat, PreviewOutcome,
        RESOURCE_CHUNK_BYTES, ResourceAnnounce, ResourceChunk, ScrollOutcome, ServerMessage,
        VerticalDirection, VerticalPreview, WheelOutcome,
    },
};
use winit::dpi::PhysicalSize;
use yazelix_venus::{
    CellMetrics, ClipboardEffect, ConnectionState, EonBar, ImageResource, LocalNoticeSource,
    ModelEffect, ModelError, ScenePlacement, ScenePreview, SceneRect, SessionModel, WorkspaceHit,
    WorkspaceScene,
};

#[test]
fn shared_popup_projection_keeps_hidden_work_and_one_input_surface() {
    use eon_workspace_protocol::v21 as wire;
    use yazelix_venus::WorkspaceModel;
    let mut snapshot = wire::Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
        }],
        active_tab: "t1".into(),
        geometry: wire::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![wire::PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: wire::PopupKey::Bound(wire::Shortcut {
                modifiers: wire::ALT | wire::SHIFT,
                key: "KeyL".into(),
            }),
        }],
        tabs: vec![wire::Tab {
            id: "t1".into(),
            directory: b"/work".to_vec(),
            pending: false,
            selected_pane: None,
            selected_popup: Some("u1".into()),
            columns: Vec::new(),
            popups: vec![wire::Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "s1".into(),
                endpoint: b"/run/agent.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v21::Sidebar::Open,
    };
    let mut model = WorkspaceModel::default();
    let apply = |model: &mut WorkspaceModel, snapshot: &wire::Snapshot| {
        let bytes = wire::encode_response(&wire::Response::Snapshot(snapshot.clone())).unwrap();
        model.apply(wire::decode_response(&bytes).unwrap());
    };
    apply(&mut model, &snapshot);
    assert_eq!(
        model.active_attachment(),
        Some((&b"/run/agent.sock"[..], true))
    );
    let project = |snapshot: &wire::Snapshot, size| {
        WorkspaceScene::from_snapshot(
            snapshot,
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, label| (label.into(), label.len() as f32 * 10.0),
        )
    };
    let scene = project(&snapshot, PhysicalSize::new(960, 600));
    assert!(scene.panes.is_empty());
    // The popup card keeps an 8-pixel gap; its terminal keeps the popup margins inside it.
    assert_eq!(scene.terminal.left, 16.0);
    assert!(scene.terminal.top > scene.tab_viewport.bottom() + 4.0);
    assert_eq!(scene.terminal.bottom(), 588.0);
    assert_eq!(scene.hit_test(0.0, 50.0), None);
    assert_eq!(
        scene.hit_test(40.0, scene.terminal.top + 1.0),
        Some(WorkspaceHit::Terminal)
    );
    assert_eq!(
        scene.hit_test(scene.tabs[0].rect.left + 1.0, scene.tabs[0].rect.top + 1.0),
        Some(WorkspaceHit::Tab("t1"))
    );
    snapshot.tabs[0].selected_popup = None;
    apply(&mut model, &snapshot);
    assert_eq!(model.active_attachment(), None);
    let hidden = project(&snapshot, PhysicalSize::new(960, 600));
    assert!(hidden.visible_terminal().is_none());
    assert_eq!(hidden.hit_test(40.0, 60.0), None);
    assert_eq!(model.snapshot().unwrap().tabs[0].popups.len(), 1);
    snapshot.tabs[0].selected_popup = Some("u1".into());
    // The card gaps yield before the popup loses its one padded cell.
    let narrow = project(&snapshot, PhysicalSize::new(34, 74));
    assert_eq!(narrow.terminal.width, 34.0);
    assert_eq!(narrow.terminal.height, 42.0);
    assert!(narrow.terminal.top >= narrow.tab_viewport.bottom());
    assert!(narrow.terminal.bottom() <= 74.0);
}

#[test]
fn scrollback_label_follows_only_current_authoritative_frames()
-> Result<(), Box<dyn std::error::Error>> {
    let mut model = SessionModel::new();
    assert_eq!(model.scrollback_label(), None);
    apply_wire(&mut model, ServerMessage::Attached)?;
    for (revision, distance, history, expected) in [
        (1, 0, 240, None),
        (2, 1, 240, Some("↓ 1 row")),
        (3, 240, 240, Some("↓ 240 rows")),
        (4, 243, 243, Some("↓ 243 rows")),
        (5, 100, 100, Some("↓ 100 rows")),
        (6, u64::MAX, u64::MAX, Some("↓ 18446744073709551615 rows")),
        (7, 0, 0, None),
    ] {
        let mut next = frame(revision, Screen::Primary);
        next.scroll_position = orbit_protocol::ScrollPosition {
            rows_from_live: distance,
            history_rows: history,
        };
        apply_wire(&mut model, ServerMessage::Frame(Box::new(next)))?;
        assert_eq!(model.scrollback_label().as_deref(), expected);
    }
    let mut held = frame(8, Screen::Primary);
    held.scroll_position = orbit_protocol::ScrollPosition {
        rows_from_live: 12,
        history_rows: 240,
    };
    apply_wire(&mut model, ServerMessage::Frame(Box::new(held.clone())))?;
    apply_wire(
        &mut model,
        ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: held.revision,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: held.dimensions.cols,
                edge_reached: false,
                rows: held.rows.clone(),
            },
        }),
    )?;
    assert_eq!(model.scrollback_label().as_deref(), Some("↓ 12 rows"));
    model.clear_viewport_preview();
    assert!(model.scroll_preview().is_none());
    apply_wire(
        &mut model,
        ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { requested_rows: -3 }),
    )?;
    assert_eq!(model.scrollback_label(), None);
    model.clear_viewport_preview();
    assert_eq!(
        model.scrollback_label(),
        None,
        "idle cleanup must keep routing evidence"
    );
    apply_wire(
        &mut model,
        ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "rejected input".into(),
        }),
    )?;
    assert_eq!(
        model.scrollback_label(),
        None,
        "input rejection is not a new frame"
    );
    held.revision = 9;
    apply_wire(&mut model, ServerMessage::Frame(Box::new(held.clone())))?;
    assert_eq!(model.scrollback_label().as_deref(), Some("↓ 12 rows"));
    model.prepare_reconnect();
    assert!(model.scene().is_some(), "recovery retains old pixels");
    assert_eq!(model.scrollback_label(), None);
    apply_wire(&mut model, ServerMessage::Attached)?;
    assert_eq!(model.scrollback_label(), None);
    apply_wire(&mut model, ServerMessage::Frame(Box::new(held)))?;
    assert_eq!(model.scrollback_label().as_deref(), Some("↓ 12 rows"));
    apply_wire(
        &mut model,
        ServerMessage::Frame(Box::new(frame(10, Screen::Alternate))),
    )?;
    assert_eq!(model.scrollback_label(), None);
    assert_eq!(SessionModel::new().scrollback_label(), None);
    Ok(())
}

#[test]
fn eon_workspace_becomes_one_bounded_native_accordion() {
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 2,
        }],
        active_tab: "t1".into(),
        tabs: vec![
            Tab {
                pending: false,
                selected_popup: None,
                popups: Vec::new(),
                id: "t1".into(),
                directory: b"/tmp/eon".to_vec(),
                selected_pane: Some("pane-2".into()),
                columns: vec![eon_workspace_protocol::v21::Column {
                    panes: vec![
                        Pane {
                            label: "pane-1".into(),
                            pinned_name: None,
                            id: "pane-1".into(),
                            session: "session-1".into(),
                            endpoint: b"/run/eon/orbit.sock".to_vec(),
                            live: true,
                            attention: None,
                        },
                        Pane {
                            label: "pane-2".into(),
                            pinned_name: None,
                            id: "pane-2".into(),
                            session: "session-2".into(),
                            endpoint: b"/run/eon/session-2.sock".to_vec(),
                            live: true,
                            attention: None,
                        },
                    ],
                    expanded: "pane-2".into(),
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
                columns: vec![eon_workspace_protocol::v21::Column {
                    panes: vec![Pane {
                        label: "pane-3".into(),
                        pinned_name: None,
                        id: "pane-3".into(),
                        session: "session-3".into(),
                        endpoint: b"/run/eon/session-3.sock".to_vec(),
                        live: false,
                        attention: None,
                    }],
                    expanded: "pane-3".into(),
                }],
                attention_rollup: None,
            },
        ],
        geometry: eon_workspace_protocol::v21::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v21::Sidebar::Open,
    };
    let size = PhysicalSize::new(800, 600);
    let metrics = CellMetrics::for_scale(1.0);
    let initial = WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
        (text.to_owned(), text.chars().count() as f32 * 10.0)
    });
    let scene = WorkspaceScene::from_snapshot(
        &snapshot,
        size,
        metrics,
        initial.active_tab_scroll(),
        initial.selected_pane_scroll(),
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );

    assert_eq!(
        scene
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["t1", "t2"]
    );
    assert!(scene.tabs[1].rect.width > scene.tabs[0].rect.width);
    assert!(scene.tabs[0].rect.right() < scene.tabs[1].rect.left);
    assert_eq!(
        scene.hit_test(
            scene.tabs[0].rect.right() + 1.0,
            scene.tabs[0].rect.top + 2.0
        ),
        None,
    );
    assert_eq!(
        scene
            .panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["pane-1", "pane-2"]
    );
    assert_eq!(scene.panes.iter().filter(|pane| pane.selected).count(), 1);
    assert!(
        scene
            .panes
            .iter()
            .all(|pane| pane.rect.height == metrics.height.max(metrics.font_size * 1.25))
    );
    assert_eq!(scene.pane_scroll_limit(), 0.0);
    // The card keeps one 8-pixel gap above the window's bottom edge.
    assert_eq!(size.height as f32 - scene.pane_viewport.bottom(), 8.0);
    // On a short window the gaps yield before the terminal loses its one padded row.
    let short = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(800, 105),
        metrics,
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert_eq!(
        short.terminal.height,
        metrics.padding * 2.0 + metrics.height
    );
    assert_eq!(
        scene.hit_test(10.0, size.height as f32 - 1.0),
        None,
        "the bottom margin must not be a terminal or pane target"
    );
    assert!(scene.panes.iter().all(|pane| {
        pane.rect.top >= scene.pane_viewport.top
            && pane.rect.bottom() <= scene.pane_viewport.bottom()
    }));
    assert_eq!(scene.panes[0].rect.bottom(), scene.panes[1].rect.top);
    assert!(scene.terminal.height > metrics.height);
    assert_eq!(scene.terminal.top, scene.panes[1].rect.bottom());
    assert_eq!(
        initial.hit_test(
            initial.panes[0].rect.left + 1.0,
            initial.panes[0].rect.top + 1.0
        ),
        Some(WorkspaceHit::Pane("pane-1"))
    );
    assert_eq!(
        scene.hit_test(scene.terminal.left + 1.0, scene.terminal.top + 1.0),
        Some(WorkspaceHit::Terminal)
    );
    assert!(scene.tab_scroll_limit().is_finite());
    assert!(scene.pane_scroll_limit().is_finite());

    let mut fitting_snapshot = snapshot.clone();
    fitting_snapshot.tabs[0].columns[0].panes.push(Pane {
        label: "pane-4".into(),
        pinned_name: None,
        id: "pane-4".into(),
        session: "session-4".into(),
        endpoint: b"/run/eon/session-4.sock".to_vec(),
        live: true,
        attention: None,
    });
    let fitting =
        WorkspaceScene::from_snapshot(&fitting_snapshot, size, metrics, 0.0, 0.0, |_, text| {
            (text.to_owned(), text.chars().count() as f32 * 10.0)
        });
    assert_eq!(fitting.pane_scroll_limit(), 0.0);
    assert!(fitting.panes.iter().all(|pane| {
        pane.rect.top >= fitting.pane_viewport.top
            && pane.rect.bottom() <= fitting.pane_viewport.bottom()
    }));
    assert_eq!(fitting.terminal.top, fitting.panes[1].rect.bottom());
    assert_eq!(fitting.panes[2].rect.top, fitting.terminal.bottom());
    assert_eq!(
        scene.terminal.height - fitting.terminal.height,
        scene.panes[0].rect.height
    );

    let mut overflow_snapshot = fitting_snapshot;
    for number in 5..=32 {
        overflow_snapshot.tabs[0].columns[0].panes.push(Pane {
            label: format!("pane-{number}"),
            pinned_name: None,
            id: format!("pane-{number}"),
            session: format!("session-{number}"),
            endpoint: format!("/run/eon/session-{number}.sock").into_bytes(),
            live: true,
            attention: None,
        });
    }
    overflow_snapshot.tabs[0].selected_pane = Some("pane-32".into());
    overflow_snapshot.tabs[0].columns[0].expanded = "pane-32".into();
    let unscrolled =
        WorkspaceScene::from_snapshot(&overflow_snapshot, size, metrics, 0.0, 0.0, |_, text| {
            (text.to_owned(), text.chars().count() as f32 * 10.0)
        });
    assert!(unscrolled.pane_scroll_limit() > 0.0);
    assert!(
        unscrolled
            .panes
            .iter()
            .all(|pane| pane.rect.height == scene.panes[0].rect.height)
    );
    let revealed = WorkspaceScene::from_snapshot(
        &overflow_snapshot,
        size,
        metrics,
        0.0,
        unscrolled.selected_pane_scroll(),
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    let selected = revealed
        .panes
        .iter()
        .find(|pane| pane.selected)
        .expect("selected pane");
    assert!(selected.rect.top >= revealed.pane_viewport.top);
    assert_eq!(revealed.terminal.top, selected.rect.bottom());
    assert!(revealed.terminal.bottom() <= revealed.pane_viewport.bottom());
    assert!(matches!(
        revealed.hit_test(
            revealed.tabs[0].rect.left + 1.0,
            revealed.tabs[0].rect.top + 1.0
        ),
        Some(WorkspaceHit::Tab(_))
    ));

    let mut second_tab = snapshot.clone();
    second_tab.active_tab = "t2".into();
    let narrow = WorkspaceScene::from_snapshot(
        &second_tab,
        PhysicalSize::new(100, 600),
        metrics,
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    let revealed = WorkspaceScene::from_snapshot(
        &second_tab,
        PhysicalSize::new(100, 600),
        metrics,
        narrow.active_tab_scroll(),
        narrow.selected_pane_scroll(),
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert_eq!(
        revealed.hit_test(
            revealed.tabs[1].rect.left + 1.0,
            revealed.tabs[1].rect.top + 1.0
        ),
        Some(WorkspaceHit::Tab("t2"))
    );

    let tiny = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(1, 1),
        metrics,
        f32::NAN,
        f32::INFINITY,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert_eq!((tiny.tab_scroll(), tiny.pane_scroll()), (0.0, 0.0));
    assert!(tiny.terminal.bottom() <= 1.0);
}

#[test]
fn canonical_orbit_frame_becomes_one_deterministic_scene() {
    let mut model = SessionModel::new();
    apply_wire(&mut model, ServerMessage::Attached).unwrap();
    apply_wire(
        &mut model,
        ServerMessage::Frame(Box::new(frame(7, Screen::Alternate))),
    )
    .unwrap();

    let scene = model.scene().unwrap();
    assert!(!scene.has_selected_content());
    assert_eq!(scene.content[0].cells[0].text, "e\u{301}");
    assert_eq!(scene.content[0].cells[1].text, "界");
    assert_eq!(scene.content[0].cells[1].width, CellWidth::Wide);
    assert_eq!(scene.content[0].cells[1].hyperlink, "https://yazelix.dev");
    assert_eq!(scene.accessible_text(), "e\u{301}界");
    assert!(!scene.cursor.unwrap().explicit_color);
    let mut explicit = frame(8, Screen::Alternate);
    explicit.colors.cursor = Some(Rgb { r: 1, g: 2, b: 3 });
    let explicit_cursor = yazelix_venus::Scene::from_frame(&explicit, 0)
        .cursor
        .unwrap();
    assert!(explicit_cursor.explicit_color);
    assert_eq!(
        explicit_cursor.color,
        yazelix_venus::Color { r: 1, g: 2, b: 3 }
    );
    assert_eq!(
        scene.snapshot(),
        concat!(
            "revision=7 screen=Alternate size=4x1 title=\"Eon ✦\" cwd=\"/tmp/venus\"\n",
            "run 0,0+1 \"e\\u{301}\" fg=f0f1f5 bg=10131a\n",
            "run 1,0+2 \"界\" fg=22cc88 bg=10131a\n",
            "cursor 1,0 Block visible=true tail=false\n"
        )
    );
}

#[test]
fn blinking_state_drives_native_wakeups_without_changing_frame_data() {
    let mut blinking = frame(8, Screen::Primary);
    blinking.rows[0].cells[0].style.blink = true;
    blinking.cursor.blinking = true;

    let scene = yazelix_venus::Scene::from_frame(&blinking, 0);
    assert!(scene.has_blinking_content());
    assert!(scene.content[0].cells[0].style.blink);
    assert!(scene.cursor.unwrap().blinking);
}

#[test]
fn concealed_cells_stay_out_of_drawing_and_accessibility() {
    let mut concealed = frame(9, Screen::Primary);
    concealed.rows[0].cells[0].text = "secret".into();
    concealed.rows[0].cells[0].style.invisible = true;

    let scene = yazelix_venus::Scene::from_frame(&concealed, 0);
    assert!(
        scene
            .glyph_runs()
            .iter()
            .all(|run| !run.text.contains("secret"))
    );
    assert_eq!(scene.accessible_text(), " 界");
}

#[test]
fn orbit_selected_style_is_the_only_visual_selection_source() {
    let mut selected = frame(10, Screen::Primary);
    selected.rows[0].cells[0].style.selected = true;

    let scene = yazelix_venus::Scene::from_frame(&selected, 0);
    assert!(scene.has_selected_content());
    assert!(scene.content[0].cells[0].style.selected);
    assert_eq!(scene.content[0].cells[0].style.foreground, scene.background);
    assert_eq!(scene.content[0].cells[0].style.background, scene.foreground);
    assert_eq!(scene.accessible_text(), "e\u{301}界");
}

#[test]
fn ordered_frames_reject_stale_revisions_without_replacing_state() {
    let mut model = attached_model();
    model
        .apply(ServerMessage::Frame(Box::new(frame(9, Screen::Primary))))
        .unwrap();
    let error = model
        .apply(ServerMessage::Frame(Box::new(frame(9, Screen::Alternate))))
        .unwrap_err();
    assert!(matches!(error, ModelError::Frame(_)));
    assert_eq!(model.scene().unwrap().revision, 9);
    assert_eq!(model.scene().unwrap().screen, Screen::Primary);
}

#[test]
fn canonical_scroll_outcomes_reuse_the_existing_result_and_frame_owners() {
    let mut model = attached_model();
    model
        .apply(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "stale wheel".into(),
        }))
        .unwrap();
    model
        .apply(ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted))
        .unwrap();
    assert!(model.notice().is_none());

    model
        .apply(ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: -1,
            frame: Box::new(frame(1, Screen::Primary)),
        }))
        .unwrap();
    assert_eq!(model.scene().unwrap().revision, 1);

    model
        .apply(ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 1,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::TerminalRouted,
        }))
        .unwrap();
    assert!(matches!(
        model.scroll_preview(),
        Some(ScenePreview::TerminalOwned {
            frame_revision: 1,
            direction: VerticalDirection::Up,
        })
    ));
    assert_eq!(model.scene().unwrap().revision, 1);

    model
        .apply(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "stale scroll".into(),
        }))
        .unwrap();
    assert!(matches!(
        model.scroll_preview(),
        Some(ScenePreview::TerminalOwned { .. })
    ));
    assert_eq!(model.scene().unwrap().revision, 1);
}

#[test]
fn a_new_attachment_replaces_state_without_a_compatibility_window() {
    let mut first = attached_model();
    first
        .apply(ServerMessage::Frame(Box::new(frame(42, Screen::Primary))))
        .unwrap();

    let mut reopened = attached_model();
    reopened
        .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Alternate))))
        .unwrap();
    assert_eq!(reopened.scene().unwrap().revision, 1);
    assert_eq!(reopened.scene().unwrap().screen, Screen::Alternate);
}

#[test]
fn reconnect_preserves_the_last_scene_and_accepts_a_fresh_revision() {
    let mut model = attached_model();
    model
        .apply(ServerMessage::Frame(Box::new(frame(42, Screen::Primary))))
        .unwrap();
    model.mark_lost("Orbit closed the local session");

    model.prepare_reconnect();
    assert_eq!(model.connection(), &ConnectionState::Connecting);
    assert_eq!(model.scene().unwrap().revision, 42);
    assert!(model.awaiting_current_frame());

    model.mark_lost("The selected Eon pane is offline");
    assert_eq!(
        model.connection(),
        &ConnectionState::Lost {
            detail: "The selected Eon pane is offline".into()
        }
    );
    assert_eq!(model.scene().unwrap().revision, 42);
    model.prepare_reconnect();

    model.apply(ServerMessage::Attached).unwrap();
    assert!(model.awaiting_current_frame());
    model
        .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Alternate))))
        .unwrap();
    assert!(!model.awaiting_current_frame());
    assert_eq!(model.scene().unwrap().revision, 1);
    assert_eq!(model.scene().unwrap().screen, Screen::Alternate);
}

#[test]
fn attachment_and_server_failures_are_explicit_and_bounded() {
    let mut busy = SessionModel::new();
    busy.apply(ServerMessage::Busy).unwrap();
    busy.mark_lost("late socket close");
    assert_eq!(busy.connection(), &ConnectionState::Busy);
    assert!(busy.is_terminal());

    let mut incompatible = SessionModel::new();
    incompatible.mark_incompatible(3);
    assert_eq!(
        incompatible.connection(),
        &ConnectionState::Incompatible { version: 3 }
    );
    assert!(incompatible.is_terminal());

    let mut attached = attached_model();
    assert!(!attached.is_terminal());
    for (code, expected) in [
        (FailureCode::InvalidInput, "Orbit rejected input: detail"),
        (FailureCode::Protocol, "Orbit protocol failure: detail"),
        (FailureCode::Terminal, "Orbit terminal failure: detail"),
    ] {
        attached
            .apply(ServerMessage::Failure(Failure {
                code,
                detail: "detail".into(),
            }))
            .unwrap();
        assert_eq!(attached.notice(), Some(expected));
    }
    attached
        .apply(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "x".repeat(2_000),
        }))
        .unwrap();
    let notice = attached.notice().unwrap().to_owned();
    assert!(notice.starts_with("Orbit rejected input: "));
    assert_eq!(notice.chars().count(), 1_025);
    attached
        .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Primary))))
        .unwrap();
    assert_eq!(attached.notice(), Some(notice.as_str()));
    attached.apply(ServerMessage::Accepted).unwrap();
    assert!(attached.notice().is_none());
    attached.mark_lost("Orbit closed the local session");
    attached.mark_lost("late socket error");
    attached.set_venus_notice(LocalNoticeSource::Input, "late input error");
    assert!(matches!(
        attached.connection(),
        ConnectionState::Lost { detail } if detail == "Orbit closed the local session"
    ));
    assert!(attached.notice().is_none());
}

#[test]
fn notices_recover_independently_by_source() {
    let mut model = attached_model();
    model.set_venus_notice(LocalNoticeSource::Input, "Venus could not encode input");
    model.apply(ServerMessage::Accepted).unwrap();
    assert_eq!(model.notice(), Some("Venus could not encode input"));
    assert!(!model.clear_venus_notice(LocalNoticeSource::Resize));
    model.set_venus_notice(LocalNoticeSource::Resize, "Window is too large");
    model.set_venus_notice(LocalNoticeSource::Queue, "Venus input queue is full");
    assert_eq!(model.notice(), Some("Venus input queue is full"));
    assert!(model.clear_venus_notice(LocalNoticeSource::Queue));
    assert_eq!(model.notice(), Some("Window is too large"));
    assert!(model.clear_venus_notice(LocalNoticeSource::Resize));
    assert_eq!(model.notice(), Some("Venus could not encode input"));
    assert!(model.clear_venus_notice(LocalNoticeSource::Input));
    model
        .apply(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "bad key".into(),
        }))
        .unwrap();
    assert!(!model.clear_venus_notice(LocalNoticeSource::Input));
    assert_eq!(model.notice(), Some("Orbit rejected input: bad key"));
    model.set_venus_notice(LocalNoticeSource::Queue, "Venus input queue is full");
    assert_eq!(model.notice(), Some("Venus input queue is full"));
    model.apply(ServerMessage::Accepted).unwrap();
    assert_eq!(model.notice(), Some("Venus input queue is full"));
    assert!(model.clear_venus_notice(LocalNoticeSource::Queue));
    assert!(model.notice().is_none());
}

#[test]
fn clipboard_text_is_an_attached_one_shot_effect_not_presentation_state() {
    let mut model = SessionModel::new();
    assert_eq!(
        model
            .apply(ServerMessage::CopiedText {
                location: ClipboardLocation::Selection,
                text: "not attached".into(),
            })
            .unwrap_err(),
        ModelError::UnexpectedMessage
    );
    assert_eq!(
        model
            .apply(ServerMessage::ClipboardWrite {
                location: ClipboardLocation::Primary,
                text: "not attached".into(),
            })
            .unwrap_err(),
        ModelError::UnexpectedMessage
    );

    let mut model = attached_model();
    model
        .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Primary))))
        .unwrap();
    assert_eq!(
        model
            .apply(ServerMessage::CopiedText {
                location: ClipboardLocation::Selection,
                text: "e\u{301}界\nsecond".into(),
            })
            .unwrap(),
        Some(ModelEffect::Clipboard(ClipboardEffect::SelectionCopy {
            location: ClipboardLocation::Selection,
            text: "e\u{301}界\nsecond".into(),
        }))
    );
    model
        .apply(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidInput,
            detail: "bad key".into(),
        }))
        .unwrap();
    assert_eq!(
        model
            .apply(ServerMessage::ClipboardWrite {
                location: ClipboardLocation::Selection,
                text: "terminal".into(),
            })
            .unwrap(),
        Some(ModelEffect::Clipboard(ClipboardEffect::TerminalWrite {
            location: ClipboardLocation::Selection,
            text: "terminal".into(),
        }))
    );
    assert_eq!(model.notice(), Some("Orbit rejected input: bad key"));
    assert_eq!(model.apply(ServerMessage::Accepted).unwrap(), None);
    assert_eq!(
        model
            .apply(ServerMessage::SelectionFinished { frame_revision: 9 })
            .unwrap(),
        None
    );
    assert_eq!(model.scene().unwrap().revision, 1);
}

#[test]
fn revision_bound_scroll_previews_follow_atomic_frames() {
    let mut model = attached_model();
    let mut initial = frame(1, Screen::Primary);
    let row = initial.rows[0].clone();
    let mut older = row.clone();
    older.cells[0].text = "older".into();
    initial.dimensions.rows = 2;
    initial.rows.push(row.clone());
    model
        .apply(ServerMessage::Frame(Box::new(initial)))
        .unwrap();
    model
        .apply(ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 1,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::Viewport {
                cols: 4,
                edge_reached: false,
                rows: vec![row.clone(), older],
            },
        }))
        .unwrap();
    assert!(matches!(
        model.scroll_preview(),
        Some(ScenePreview::Viewport {
            frame_revision: 1,
            direction: VerticalDirection::Up,
            rows,
            ..
        }) if rows.len() == 2
            && rows[0].cells[0].text == "e\u{301}"
            && rows[1].cells[0].text == "older"
    ));

    let mut next_frame = frame(2, Screen::Primary);
    next_frame.dimensions.rows = 2;
    next_frame.rows.push(row.clone());
    for (cols, rows) in [(3, vec![row.clone()]), (4, vec![row.clone(); 3])] {
        assert_eq!(
            model
                .apply(ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                    requested_rows: -1,
                    applied_rows: -1,
                    frame: Box::new(next_frame.clone()),
                    next: PreviewOutcome::Viewport {
                        cols,
                        edge_reached: false,
                        rows,
                    },
                }))
                .unwrap_err(),
            ModelError::UnexpectedMessage
        );
        assert_eq!(model.scene().unwrap().revision, 1);
        assert!(matches!(
            model.scroll_preview(),
            Some(ScenePreview::Viewport {
                frame_revision: 1,
                ..
            })
        ));
    }

    model
        .apply(ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows: -1,
            applied_rows: -1,
            frame: Box::new(next_frame),
            next: PreviewOutcome::Viewport {
                cols: 4,
                edge_reached: false,
                rows: vec![row],
            },
        }))
        .unwrap();
    assert_eq!(model.scene().unwrap().revision, 2);
    assert!(matches!(
        model.scroll_preview(),
        Some(ScenePreview::Viewport {
            frame_revision: 2,
            direction: VerticalDirection::Up,
            rows,
            ..
        }) if rows.len() == 1
    ));

    model
        .apply(ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 1,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::TerminalRouted,
        }))
        .unwrap();
    assert!(matches!(
        model.scroll_preview(),
        Some(ScenePreview::Viewport {
            frame_revision: 2,
            direction: VerticalDirection::Up,
            ..
        })
    ));
    assert_eq!(model.scene().unwrap().revision, 2);
}

#[test]
fn continuous_frames_keep_only_compatible_scroll_previews() {
    let mut output = frame(2, Screen::Primary);
    output.rows[0].cells[0].text = "x".into();
    let mut resized = frame(2, Screen::Primary);
    resized.dimensions.rows += 1;
    resized.rows.push(resized.rows[0].clone());
    let mut recolored = frame(2, Screen::Primary);
    recolored.colors.palette[2].r += 1;

    for (next, retained) in [
        (output, true),
        (frame(2, Screen::Alternate), false),
        (resized, false),
        (recolored, false),
    ] {
        let mut model = attached_model();
        let initial = frame(1, Screen::Primary);
        let row = initial.rows[0].clone();
        model
            .apply(ServerMessage::Frame(Box::new(initial)))
            .unwrap();
        model
            .apply(ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: 1,
                direction: VerticalDirection::Up,
                outcome: PreviewOutcome::Viewport {
                    cols: 4,
                    edge_reached: false,
                    rows: vec![row],
                },
            }))
            .unwrap();
        apply_wire(&mut model, ServerMessage::Frame(Box::new(next))).unwrap();
        let preview = model.scroll_preview().map(|preview| match preview {
            ScenePreview::Viewport {
                frame_revision,
                direction,
                ..
            } => (*frame_revision, *direction),
            ScenePreview::TerminalOwned { .. } => panic!("unexpected terminal-owned preview"),
        });
        assert_eq!(preview, retained.then_some((1, VerticalDirection::Up)));
    }

    let mut model = attached_model();
    model
        .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Primary))))
        .unwrap();
    model
        .apply(ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision: 1,
            direction: VerticalDirection::Up,
            outcome: PreviewOutcome::TerminalRouted,
        }))
        .unwrap();
    apply_wire(
        &mut model,
        ServerMessage::Frame(Box::new(frame(2, Screen::Primary))),
    )
    .unwrap();
    assert!(model.scroll_preview().is_none());
}

#[test]
fn kept_previews_go_stale_at_live_and_at_the_oldest_retained_row() {
    let at = |revision, rows_from_live, history_rows| {
        let mut frame = frame(revision, Screen::Primary);
        frame.scroll_position = orbit_protocol::ScrollPosition {
            rows_from_live,
            history_rows,
        };
        frame
    };
    let kept_over = |answered: Frame, next: Frame| {
        let mut model = attached_model();
        let revision = answered.revision;
        model
            .apply(ServerMessage::Frame(Box::new(answered)))
            .unwrap();
        model
            .apply(ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: revision,
                direction: VerticalDirection::Up,
                outcome: PreviewOutcome::Viewport {
                    cols: 4,
                    edge_reached: false,
                    rows: frame(1, Screen::Primary).rows,
                },
            }))
            .unwrap();
        apply_wire(&mut model, ServerMessage::Frame(Box::new(next))).unwrap();
        assert!(matches!(
            model.scroll_preview(),
            Some(ScenePreview::Viewport { frame_revision, .. }) if *frame_revision == revision
        ));
        model.preview_may_be_stale()
    };

    // Output below a pinned viewport, or pruning above it, leaves its rows in place.
    assert!(!kept_over(at(1, 10, 100), at(2, 12, 102)));
    assert!(!kept_over(at(1, 10, 100), at(2, 12, 60)));
    // Pruning the rows it shows moves it to the oldest retained row, where later pruning
    // replaces them; at live, output moves the history above it.
    assert!(kept_over(at(1, 10, 100), at(2, 60, 60)));
    assert!(kept_over(at(1, 60, 60), at(2, 62, 62)));
    assert!(kept_over(at(1, 0, 100), at(2, 0, 102)));
}

#[test]
fn invalid_session_and_frame_bytes_never_reach_draw_state() {
    let mut encoded =
        session::encode_server_message(&ServerMessage::Frame(Box::new(frame(1, Screen::Primary))))
            .unwrap();
    encoded[0] ^= 1;
    assert!(session::decode_server_message(&encoded).is_err());
    encoded[0] ^= 1;
    encoded[4..6].copy_from_slice(&10_u16.to_le_bytes());
    assert!(matches!(
        session::decode_server_message(&encoded),
        Err(session::Error::UnsupportedVersion { version: 10 })
    ));

    let mut model = SessionModel::new();
    assert_eq!(
        model
            .apply(ServerMessage::Frame(Box::new(frame(1, Screen::Primary))))
            .unwrap_err(),
        ModelError::UnexpectedMessage
    );
    assert!(model.scene().is_none());

    assert_eq!(
        model.apply(ServerMessage::Accepted).unwrap_err(),
        ModelError::UnexpectedMessage
    );
    let mut attached = attached_model();
    assert_eq!(
        attached.apply(ServerMessage::Busy).unwrap_err(),
        ModelError::UnexpectedMessage
    );
    assert!(attached.is_attached());
}

#[test]
fn resource_streams_assemble_complete_pixels_and_frames_carry_placements() {
    let mut model = attached_model();
    let mut first = frame(1, Screen::Primary);
    first.graphics = Some(Graphics {
        cell_width: 10,
        cell_height: 20,
        placements_omitted: true,
        placements: vec![
            placement(5, -1, 0, PlacementState::Pending),
            placement(9, 2, 1, PlacementState::Unavailable),
        ],
    });
    apply_wire(&mut model, ServerMessage::Frame(Box::new(first))).unwrap();
    let graphics = &model.scene().unwrap().graphics;
    let attachment = graphics.attachment;
    assert!(graphics.placements_omitted);
    assert_eq!(
        graphics.placements,
        [
            ScenePlacement {
                resource: 5,
                state: PlacementState::Pending,
                source: SourceRect {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 40,
                },
                target: SceneRect {
                    left: -0.5,
                    top: 0.25,
                    width: 3.0,
                    height: 2.0,
                },
            },
            ScenePlacement {
                resource: 9,
                state: PlacementState::Unavailable,
                source: SourceRect {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 40,
                },
                target: SceneRect {
                    left: 2.5,
                    top: 1.25,
                    width: 3.0,
                    height: 2.0,
                },
            },
        ]
    );

    // 300 by 300 RGBA is one full chunk and a short last one.
    assert_eq!(
        resource_wire(&mut model, announce(5, PixelFormat::Rgba, 300, 300)),
        Ok(None)
    );
    let first_chunk = vec![1; RESOURCE_CHUNK_BYTES as usize];
    let last_chunk = vec![2; 300 * 300 * 4 - RESOURCE_CHUNK_BYTES as usize];
    assert_eq!(
        resource_wire(&mut model, chunk(5, 0, first_chunk.clone())),
        Ok(None)
    );
    assert_eq!(
        resource_wire(&mut model, chunk(5, 1, last_chunk.clone())),
        Ok(Some(ModelEffect::Resource(ImageResource {
            attachment,
            resource: 5,
            format: PixelFormat::Rgba,
            width: 300,
            height: 300,
            pixels: [first_chunk, last_chunk].concat(),
        })))
    );
    assert_eq!(
        resource_wire(&mut model, announce(7, PixelFormat::Rgb, 1, 1)),
        Ok(None)
    );
    assert_eq!(
        resource_wire(&mut model, chunk(7, 0, vec![3, 4, 5])),
        Ok(Some(ModelEffect::Resource(ImageResource {
            attachment,
            resource: 7,
            format: PixelFormat::Rgb,
            width: 1,
            height: 1,
            pixels: vec![3, 4, 5],
        })))
    );
    assert_eq!(
        resource_wire(&mut model, ServerMessage::ResourceRetire { resource: 5 }),
        Ok(None)
    );

    // A new attachment starts identities over and forgets the old resources; the scene kept
    // through the reconnect still names the old attachment.
    model.prepare_reconnect();
    apply_wire(&mut model, ServerMessage::Attached).unwrap();
    assert_eq!(model.scene().unwrap().graphics.attachment, attachment);
    assert_eq!(
        resource_wire(&mut model, ServerMessage::ResourceRetire { resource: 7 }),
        Err(ModelError::Resource(
            "resource message before the first frame"
        ))
    );
    apply_wire(
        &mut model,
        ServerMessage::Frame(Box::new(frame(1, Screen::Primary))),
    )
    .unwrap();
    assert_ne!(model.scene().unwrap().graphics.attachment, attachment);
    assert!(model.scene().unwrap().graphics.placements.is_empty());
    assert_eq!(
        resource_wire(&mut model, ServerMessage::ResourceRetire { resource: 7 }),
        Err(ModelError::Resource(
            "retirement names no complete, unretired resource"
        ))
    );
    assert_eq!(
        resource_wire(&mut model, announce(1, PixelFormat::Rgb, 1, 1)),
        Ok(None)
    );
}

#[test]
fn invalid_resource_streams_fail_deterministically() {
    let complete = |identity| {
        [
            announce(identity, PixelFormat::Rgb, 1, 1),
            chunk(identity, 0, vec![0; 3]),
        ]
    };
    let full = || vec![0; RESOURCE_CHUNK_BYTES as usize];
    let cases: Vec<(Vec<ServerMessage>, &str)> = vec![
        (
            [
                complete(5).as_slice(),
                &[announce(5, PixelFormat::Rgb, 1, 1)],
            ]
            .concat(),
            "identity is not greater than every earlier one",
        ),
        (
            [
                complete(5).as_slice(),
                &[announce(4, PixelFormat::Rgb, 1, 1)],
            ]
            .concat(),
            "identity is not greater than every earlier one",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                announce(6, PixelFormat::Rgb, 1, 1),
            ],
            "another resource is still incomplete",
        ),
        (
            vec![chunk(5, 0, vec![0; 3])],
            "chunk of no incomplete resource",
        ),
        (
            [complete(5).as_slice(), &[chunk(5, 1, vec![0; 3])]].concat(),
            "chunk of no incomplete resource",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                chunk(5, 1, vec![0; 97_856]),
            ],
            "chunk is out of order",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                chunk(6, 0, full()),
            ],
            "chunk is out of order",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                chunk(5, 0, full()),
                chunk(5, 0, full()),
            ],
            "chunk is out of order",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                chunk(5, 0, vec![0; 97_856]),
            ],
            "chunk has the wrong size",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                chunk(5, 0, full()),
                chunk(5, 1, vec![0; 97_855]),
            ],
            "chunk has the wrong size",
        ),
        (
            vec![announce(5, PixelFormat::Rgb, 1, 1), chunk(5, 0, vec![0; 4])],
            "chunk has the wrong size",
        ),
        (
            vec![ServerMessage::ResourceRetire { resource: 5 }],
            "retirement names no complete, unretired resource",
        ),
        (
            vec![
                announce(5, PixelFormat::Rgba, 300, 300),
                ServerMessage::ResourceRetire { resource: 5 },
            ],
            "retirement names no complete, unretired resource",
        ),
        (
            [
                complete(5).as_slice(),
                &[
                    ServerMessage::ResourceRetire { resource: 5 },
                    ServerMessage::ResourceRetire { resource: 5 },
                ],
            ]
            .concat(),
            "retirement names no complete, unretired resource",
        ),
    ];
    for (messages, detail) in cases {
        let mut model = streaming_model();
        let (failing, accepted) = messages.split_last().unwrap();
        for message in accepted {
            resource_wire(&mut model, message.clone()).unwrap();
        }
        assert_eq!(
            resource_wire(&mut model, failing.clone()),
            Err(ModelError::Resource(detail)),
            "{messages:?}"
        );
        // The failure dropped any assembly, so only order state remains.
        assert_eq!(
            resource_wire(&mut model, announce(100, PixelFormat::Rgb, 1, 1)),
            Ok(None),
            "{messages:?}"
        );
    }

    // Before attachment the message is out of order; before the first frame it is a resource
    // failure.
    assert_eq!(
        SessionModel::new().apply(announce(5, PixelFormat::Rgb, 1, 1)),
        Err(ModelError::UnexpectedMessage)
    );
    assert_eq!(
        attached_model().apply(announce(5, PixelFormat::Rgb, 1, 1)),
        Err(ModelError::Resource(
            "resource message before the first frame"
        ))
    );

    // Metadata the codec would refuse is refused again before any reservation.
    let oversized = ResourceAnnounce {
        resource: 5,
        generation: 1,
        format: PixelFormat::Rgba,
        width: 4096,
        height: 4096,
        byte_length: 4096 * 4096 * 4,
        chunk_count: 256,
    };
    assert_eq!(
        streaming_model().apply(ServerMessage::ResourceAnnounce(oversized)),
        Err(ModelError::Resource("metadata is outside the limits"))
    );

    // At most 1,024 resources are announced and not retired.
    let mut model = streaming_model();
    for identity in 0..1024 {
        for message in complete(identity) {
            model.apply(message).unwrap();
        }
    }
    assert_eq!(
        model.apply(announce(2000, PixelFormat::Rgb, 1, 1)),
        Err(ModelError::Resource("too many resources are not retired"))
    );
    model
        .apply(ServerMessage::ResourceRetire { resource: 3 })
        .unwrap();
    assert_eq!(
        model.apply(announce(2000, PixelFormat::Rgb, 1, 1)),
        Ok(None)
    );
}

fn streaming_model() -> SessionModel {
    let mut model = attached_model();
    apply_wire(
        &mut model,
        ServerMessage::Frame(Box::new(frame(1, Screen::Primary))),
    )
    .unwrap();
    model
}

/// Apply a resource message after a round trip through the codec, which it must pass.
fn resource_wire(
    model: &mut SessionModel,
    message: ServerMessage,
) -> Result<Option<ModelEffect>, ModelError> {
    let bytes = session::encode_server_message(&message).unwrap();
    model.apply(session::decode_server_message(&bytes).unwrap())
}

fn announce(resource: u64, format: PixelFormat, width: u32, height: u32) -> ServerMessage {
    ServerMessage::ResourceAnnounce(
        ResourceAnnounce::new(resource, 1, format, width, height).unwrap(),
    )
}

fn chunk(resource: u64, index: u32, bytes: Vec<u8>) -> ServerMessage {
    ServerMessage::ResourceChunk(ResourceChunk {
        resource,
        index,
        bytes,
    })
}

fn placement(resource: u64, col: i32, row: i32, state: PlacementState) -> Placement {
    Placement {
        resource,
        layer: ImageLayer::BelowText,
        z: 0,
        col,
        row,
        grid_cols: 3,
        grid_rows: 2,
        offset_x: 5,
        offset_y: 5,
        width: 30,
        height: 40,
        source: SourceRect {
            x: 1,
            y: 2,
            width: 30,
            height: 40,
        },
        state,
    }
}

fn attached_model() -> SessionModel {
    let mut model = SessionModel::new();
    model.apply(ServerMessage::Attached).unwrap();
    model
}

fn apply_wire(
    model: &mut SessionModel,
    message: ServerMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = session::encode_server_message(&message)?;
    model.apply(session::decode_server_message(&bytes)?)?;
    Ok(())
}

fn frame(revision: u64, screen: Screen) -> Frame {
    let background = Rgb {
        r: 0x10,
        g: 0x13,
        b: 0x1a,
    };
    let foreground = Rgb {
        r: 0xf0,
        g: 0xf1,
        b: 0xf5,
    };
    let mut palette = [Rgb::BLACK; 256];
    palette[2] = Rgb {
        r: 0x22,
        g: 0xcc,
        b: 0x88,
    };
    Frame {
        revision,
        dimensions: Dimensions { cols: 4, rows: 1 },
        screen,
        scroll_position: orbit_protocol::ScrollPosition::default(),
        title: "Eon ✦".into(),
        working_directory: "/tmp/venus".into(),
        capabilities: Capabilities { hyperlinks: true },
        colors: Colors {
            background,
            foreground,
            cursor: None,
            palette,
        },
        cursor: Cursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            viewport: Some(CursorViewport {
                x: 1,
                y: 0,
                at_wide_tail: false,
            }),
        },
        rows: vec![Row {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![
                cell("e\u{301}", CellWidth::Narrow, StyleColor::None, ""),
                cell(
                    "界",
                    CellWidth::Wide,
                    StyleColor::Palette(2),
                    "https://yazelix.dev",
                ),
                cell("", CellWidth::SpacerTail, StyleColor::None, ""),
                cell("", CellWidth::Narrow, StyleColor::None, ""),
            ],
        }],
        graphics: None,
    }
}

fn cell(text: &str, width: CellWidth, foreground: StyleColor, hyperlink: &str) -> Cell {
    Cell {
        width,
        style: CellStyle {
            foreground,
            background: StyleColor::None,
            underline_color: StyleColor::None,
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
            underline: Underline::None,
        },
        text: text.into(),
        hyperlink: hyperlink.into(),
    }
}

#[test]
fn columns_sit_side_by_side_and_zoom_only_the_selected_pane() {
    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: format!("/run/eon/{id}.sock").into_bytes(),
        live: true,
        attention: None,
    };
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
        }],
        active_tab: "t1".into(),
        tabs: vec![Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: "t1".into(),
            directory: b"/tmp/eon".to_vec(),
            selected_pane: Some("p3".into()),
            columns: vec![
                eon_workspace_protocol::v21::Column {
                    panes: vec![pane("p1"), pane("p2")],
                    expanded: "p2".into(),
                },
                eon_workspace_protocol::v21::Column {
                    panes: vec![pane("p3")],
                    expanded: "p3".into(),
                },
                eon_workspace_protocol::v21::Column {
                    panes: vec![pane("p4")],
                    expanded: "p4".into(),
                },
            ],
            attention_rollup: None,
        }],
        geometry: eon_workspace_protocol::v21::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v21::Sidebar::Open,
    };
    let size = PhysicalSize::new(901, 600);
    let scene = WorkspaceScene::from_snapshot(
        &snapshot,
        size,
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );

    // The selected column stays primary; the others keep their positions.
    assert_eq!(scene.selected_column, 1);
    assert_eq!(
        scene
            .panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["p3"]
    );
    assert_eq!(
        scene
            .column_panes()
            .map(|(panes, selected)| (
                panes
                    .iter()
                    .map(|pane| pane.id.as_str())
                    .collect::<Vec<_>>(),
                selected
            ))
            .collect::<Vec<_>>(),
        [
            (vec!["p1", "p2"], false),
            (vec!["p3"], true),
            (vec!["p4"], false)
        ]
    );

    // Column cards keep one 8-pixel gap from the window's sides and between each other.
    let mut terminals = scene
        .other_columns
        .iter()
        .map(|column| (column.index, column.terminal))
        .chain([(scene.selected_column, scene.terminal)])
        .collect::<Vec<_>>();
    terminals.sort_by_key(|(index, _)| *index);
    assert_eq!(terminals[0].1.left, 8.0);
    for pair in terminals.windows(2) {
        assert_eq!(pair[1].1.left - pair[0].1.right(), 8.0);
    }
    assert_eq!(terminals[2].1.right(), size.width as f32 - 8.0);
    // The last column takes the remainder, and headers share their column's span.
    let widths: Vec<f32> = terminals.iter().map(|entry| entry.1.width).collect();
    assert_eq!(widths, [289.0, 289.0, 291.0]);
    for column in &scene.other_columns {
        for pane in &column.panes {
            assert_eq!(pane.rect.left, column.terminal.left);
            assert_eq!(pane.rect.width, column.terminal.width);
            assert_eq!(pane.rect.height, scene.panes[0].rect.height);
        }
    }
    for pane in &scene.panes {
        assert_eq!(pane.rect.left, scene.terminal.left);
        assert_eq!(pane.rect.width, scene.terminal.width);
    }

    // Each column shows its own expanded pane; its terminal area names that pane.
    let first = &scene.other_columns[0];
    assert!(
        first
            .panes
            .iter()
            .any(|pane| pane.id == "p2" && pane.selected)
    );
    let inside = first.terminal;
    assert!(matches!(
        scene.hit_test(inside.left + 5.0, inside.top + 5.0),
        Some(WorkspaceHit::ColumnTerminal("p2"))
    ));
    assert!(matches!(
        scene.hit_test(scene.terminal.left + 5.0, scene.terminal.top + 5.0),
        Some(WorkspaceHit::Terminal)
    ));

    // Gaps yield before any column loses its one padded terminal cell.
    let narrow = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(102, 600),
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert!(
        std::iter::once(narrow.terminal)
            .chain(narrow.other_columns.iter().map(|column| column.terminal))
            .all(|terminal| terminal.width >= 34.0)
    );

    // Zoom hides the selected pane's neighbours as well as the other columns.
    let mut stacked = snapshot.clone();
    stacked.tabs[0].columns[1].panes = vec![pane("p5"), pane("p3"), pane("p6")];
    let zoomed = |snapshot: &Snapshot| {
        WorkspaceScene::from_snapshot_with_zoom(
            snapshot,
            EonBar::default(),
            size,
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            None,
            true,
            yazelix_venus::SidebarView::default(),
        )
    };
    let zoom = zoomed(&stacked);
    assert_eq!(
        zoom.panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        ["p3"]
    );
    assert_eq!(zoom.zoomed_pane().map(|pane| pane.id.as_str()), Some("p3"));
    assert_eq!(zoom.selected_column, 1);
    assert!(zoom.other_columns.is_empty());
    assert_eq!(zoom.pane_viewport, scene.pane_viewport);
    assert_eq!(zoom.pane_viewport.top, 46.0);
    assert_eq!(zoom.panes[0].rect.top, zoom.pane_viewport.top);
    for column in &scene.other_columns {
        assert_eq!(column.panes[0].rect.top, zoom.pane_viewport.top);
    }
    assert_eq!(zoom.terminal.left, 8.0);
    assert_eq!(zoom.terminal.right(), size.width as f32 - 8.0);
    assert_eq!(
        (zoom.terminal.top, zoom.terminal.height),
        (scene.terminal.top, scene.terminal.height)
    );
    assert_eq!(
        zoom.column_panes()
            .map(|(panes, selected)| (panes.len(), panes[0].id.as_str(), selected))
            .collect::<Vec<_>>(),
        [(1, "p3", true)]
    );
    assert_eq!(zoom.panes[0].rect.width, zoom.terminal.width);
    assert!(matches!(
        zoom.hit_test(inside.left + 5.0, inside.top + 5.0),
        Some(WorkspaceHit::Terminal)
    ));
    assert!(zoom.tabs[0].rect.width > scene.tabs[0].rect.width);
    let unzoomed = WorkspaceScene::from_snapshot(
        &stacked,
        size,
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert!(zoom.terminal.height > unzoomed.terminal.height);
    let metrics = CellMetrics::for_scale(1.0);
    assert_ne!(
        metrics.surface_size(zoom.terminal.screen_size()),
        metrics.surface_size(unzoomed.terminal.screen_size())
    );
    let restored = WorkspaceScene::from_snapshot(
        &stacked,
        size,
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
    );
    assert_eq!(restored, unzoomed);
    for pane in unzoomed.panes.iter().filter(|pane| !pane.selected) {
        assert!(
            !matches!(zoom.hit_test(pane.rect.left + 5.0, pane.rect.top + 5.0), Some(WorkspaceHit::Pane(id)) if id == pane.id)
        );
    }

    let small = |scroll| {
        WorkspaceScene::from_snapshot(
            &stacked,
            PhysicalSize::new(1600, 112),
            CellMetrics::for_scale(1.0),
            0.0,
            scroll,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
        )
    };
    let clipped = small(0.0);
    assert!(clipped.visible_terminal().unwrap().height < clipped.terminal.height);
    let restored = small(clipped.selected_pane_scroll());
    assert_eq!(restored.visible_terminal(), Some(restored.terminal));
    println!(
        "unzoom reveals the selected pane at scroll {}",
        clipped.selected_pane_scroll()
    );

    for (scale, font_size, height) in [
        (1.0, 16.0, 1),
        (1.0, 96.0, 112),
        (1.3, 16.0, 105),
        (2.0, 96.0, 150),
        (2.0, 16.0, 720),
    ] {
        let mut metrics = CellMetrics::for_scale(scale);
        metrics.font_size = font_size * scale as f32;
        metrics.width *= font_size / 16.0;
        metrics.height = (metrics.font_size * 1.125).round();
        let tiny = WorkspaceScene::from_snapshot_with_zoom(
            &stacked,
            EonBar::default(),
            PhysicalSize::new(1600, height),
            metrics,
            0.0,
            f32::MAX,
            |_, text| (text.to_owned(), text.chars().count() as f32 * metrics.width),
            None,
            true,
            yazelix_venus::SidebarView::default(),
        );
        assert_eq!(tiny.panes.len(), 1);
        assert!(tiny.other_columns.is_empty());
        assert!(tiny.terminal.top >= tiny.pane_viewport.top);
        assert!(tiny.terminal.bottom() <= tiny.pane_viewport.bottom() + 0.001);
        println!(
            "zoom fallback: scale={scale}, font={font_size}, height={height}, terminal={:?}",
            tiny.terminal
        );
    }

    // A single column's stack still zooms; only a tab with one pane refuses.
    let mut single = stacked.clone();
    single.tabs[0].columns.drain(..1);
    single.tabs[0].columns.pop();
    assert_eq!(zoomed(&single).panes.len(), 1);
    single.tabs[0].columns[0]
        .panes
        .retain(|pane| pane.id == "p3");
    assert!(zoomed(&single).zoomed_pane().is_none());
    let mut popup = snapshot.clone();
    popup.entries.push(eon_workspace_protocol::v21::PopupEntry {
        id: "git".into(),
        label: "Git".into(),
        shortcut: eon_workspace_protocol::v21::PopupKey::Bound(
            eon_workspace_protocol::v21::Shortcut {
                modifiers: eon_workspace_protocol::v21::ALT,
                key: "KeyG".into(),
            },
        ),
    });
    popup.tabs[0]
        .popups
        .push(eon_workspace_protocol::v21::Popup {
            id: "u1".into(),
            entry: "git".into(),
            session: "session-git".into(),
            endpoint: b"/run/eon/git.sock".to_vec(),
            attention: None,
        });
    popup.tabs[0].selected_popup = Some("u1".into());
    assert!(zoomed(&popup).zoomed_pane().is_none());
}
