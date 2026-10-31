use super::*;

#[test]
fn compact_pane_headers_share_title_height_and_full_row_targets() {
    use eon_workspace_protocol::v26::{Column, PopupGeometry};

    let snapshot = Snapshot {
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
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp/eon".to_vec(),
            pending: false,
            selected_pane: Some("p2".into()),
            selected_popup: None,
            columns: vec![Column {
                panes: (1..=4)
                    .map(|n| Pane {
                        label: format!("p{n}"),
                        pinned_name: None,
                        id: format!("p{n}"),
                        session: format!("s{n}"),
                        endpoint: format!("/tmp/p{n}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: "p2".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: Sidebar::Open,
        attention_queue: Vec::new(),
    };
    for (scale, line_height) in [
        (1.0, 1.125),
        (1.25, 1.125),
        (1.3, 1.125),
        (1.5, 1.125),
        (2.0, 1.125),
        (1.0, 2.0),
        (1.3, 2.0),
    ] {
        let mut metrics = CellMetrics::for_scale(scale);
        metrics.height = (metrics.font_size * line_height).round();
        let scene = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1600, 900),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        );
        if scale == 1.0 && line_height == 1.125 {
            assert_eq!(scene.header.height, 40.0);
            assert_eq!(scene.tabs[0].rect.top, 8.0);
            assert_eq!(scene.tabs[0].rect.height, 24.0);
            assert_eq!(
                scene.pane_viewport,
                SceneRect {
                    left: 296.0,
                    top: 46.0,
                    width: 1296.0,
                    height: 846.0
                }
            );
            assert_eq!(
                scene.terminal,
                SceneRect {
                    left: 296.0,
                    top: 86.0,
                    width: 1296.0,
                    height: 766.0
                }
            );
        }
        if scale <= 1.3 {
            use eon_workspace_protocol::v26::{CodexQuota, CodexQuotaState};
            let mut snapshot = snapshot.clone();
            snapshot.codex_quota = Some(CodexQuota {
                state: CodexQuotaState::Unknown,
                observed_at: 0,
                windows: Vec::new(),
            });
            let cpu = CpuUse {
                label: "CPU 12%".into(),
                description: "CPU".into(),
            };
            let ram = RamUse {
                label: "RAM 43%".into(),
                description: "RAM".into(),
            };
            let clock = LocalClock {
                label: "15:43".into(),
                description: "Clock".into(),
            };
            let widgets = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar {
                    version: Some("0.1.1"),
                    cpu: Some(&cpu),
                    ram: Some(&ram),
                    clock: Some(&clock),
                    ..EonBar::default()
                },
                PhysicalSize::new(1600, 900),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
                None,
            );
            assert_eq!(widgets.header, scene.header);
            assert_eq!(widgets.pane_viewport, scene.pane_viewport);
            assert_eq!(widgets.terminal, scene.terminal);
            assert_eq!(widgets.tabs[0].rect, scene.tabs[0].rect);
            let pill = widgets.bar_pill_rect(widgets.tabs[0].rect, metrics);
            let centre = pill.top + pill.height / 2.0;
            for rect in widgets.quotas.iter().map(|quota| quota.rect).chain([
                widgets.cpu.as_ref().unwrap().rect,
                widgets.ram.as_ref().unwrap().rect,
                widgets.clock.as_ref().unwrap().rect,
            ]) {
                let painted = widgets.bar_pill_rect(rect, metrics);
                assert_eq!((painted.top, painted.height), (pill.top, pill.height));
                assert_eq!(rect.top, 0.0);
                assert_eq!(rect.height, widgets.header.height);
            }
            for rect in widgets
                .controls
                .iter()
                .map(|control| control.rect)
                .chain([widgets.version.as_ref().unwrap().rect])
            {
                let content = widgets.bar_item_rect(rect, metrics);
                assert!((content.top + content.height / 2.0 - centre).abs() < 0.001);
                assert_eq!(content.height, rect.height);
            }
            let logo = widgets.quotas[0].logo;
            assert!((logo.top + logo.height / 2.0 - centre).abs() < 0.001);
        }
        for pane in &scene.panes {
            assert_eq!(pane.rect.height, chrome_title_metrics(metrics).text_height);
            assert!(pane.rect.height >= metrics.height);
            for x in [pane.rect.left + 0.5, pane.rect.right() - 0.5] {
                for y in [pane.rect.top + 0.5, pane.rect.bottom() - 0.5] {
                    assert_eq!(scene.hit_test(x, y), Some(WorkspaceHit::Pane(&pane.id)));
                }
            }
        }
        for pane in &scene.panes {
            let frame = pane.frame(scene.terminal);
            assert_eq!(frame.top, title_rule_y(pane.rect.top, pane.rect.height));
            if pane.selected {
                assert_eq!(frame.bottom(), scene.terminal.bottom());
            } else {
                assert_eq!(frame.height, pane.rect.height);
            }
        }
        assert_eq!(scene.panes[1].rect.bottom(), scene.terminal.top);
        assert_eq!(scene.terminal.bottom(), scene.panes[2].rect.top);
        assert_eq!(
            scene
                .card_rect(
                    scene.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale)
                )
                .top,
            (scene.pane_viewport.top + chrome_title_metrics(metrics).text_height / 2.0).floor(),
            "painted pane top at scale {scale}"
        );
    }
    // Resizing can collapse every header to the same Y; direction still follows pane order.
    for size in [PhysicalSize::new(1280, 1), PhysicalSize::new(1, 720)] {
        let metrics = CellMetrics::for_scale(1.3);
        let scene = WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
            (text.into(), text.len() as f32 * metrics.width)
        });
        let outlines = scene.pane_outlines(
            &scene.panes,
            scene.terminal,
            scene.card_fill(
                scene.chrome,
                metrics,
                crate::render::physical_outline_width(2, metrics.scale),
            ),
            3.0,
            metrics,
            3.0,
        );
        assert!(
            outlines
                .iter()
                .all(|outline| outline.bounds.top.is_finite() && outline.bounds.height.is_finite())
        );
    }
    let metrics = CellMetrics::for_scale(1.0);
    let size = PhysicalSize::new(1280, 720);
    for (count, before, after) in [(1, 32, 33), (3, 29, 30), (4, 28, 29)] {
        let mut snapshot = snapshot.clone();
        snapshot.tabs[0].columns[0].panes.truncate(count);
        snapshot.tabs[0].selected_pane = Some("p1".into());
        snapshot.tabs[0].columns[0].expanded = "p1".into();
        let hints =
            ShortcutHintsScene::new(Vec::new(), size, metrics, WorkspaceFocus::Terminal, |_| 0.0);
        let scene = WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar::default(),
            size,
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
            Some(hints),
        );
        let rows = |header: f32| {
            ((scene.chrome.height - count as f32 * header - metrics.padding * 2.0) / metrics.height)
                .floor() as u16
        };
        assert_eq!(rows((metrics.height * 1.5).round()), before);
        let actual_rows =
            ((scene.terminal.height - metrics.padding * 2.0) / metrics.height).floor() as u16;
        assert_eq!(actual_rows, after);
        assert_eq!(rows(scene.panes[0].rect.height), after);
        println!(
            "1280x720, {count} panes: {before} -> {after} rows, allocated height {}",
            scene.chrome.height
        );
        let painted = scene.card_rect(
            scene.chrome,
            metrics,
            crate::render::physical_outline_width(2, metrics.scale),
        );
        assert_eq!(
            painted.top,
            (scene.panes[0].rect.top + scene.panes[0].rect.height / 2.0).floor()
        );
        assert_eq!(
            painted.bottom(),
            (scene.chrome.bottom() - chrome_title_metrics(metrics).text_height / 2.0).floor()
        );
        assert_eq!(painted.top - scene.chrome.top, 10.0);
    }
}

#[test]
fn popup_outer_chrome_reuses_the_pane_stack_rectangle() {
    use eon_workspace_protocol::v26::{
        ALT, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Tab,
    };

    let mut snapshot = Snapshot {
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
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT,
                key: "KeyA".into(),
            }),
        }],
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/work".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v26::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "session-1".into(),
                    endpoint: b"/run/pane.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: vec![Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "session-2".into(),
                endpoint: b"/run/popup.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v26::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    // Even the smallest windows keep terminal padding; only the gaps and empty grid yield.
    for scale in [1.0, 1.25, 1.3, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for count in 1..=4 {
            let mut snapshot = snapshot.clone();
            snapshot.tabs[0].columns = (1..=count)
                .map(|index| {
                    let mut column = snapshot.tabs[0].columns[0].clone();
                    column.panes[0].id = format!("p{index}");
                    column.expanded = format!("p{index}");
                    column
                })
                .collect();
            for size in [
                PhysicalSize::new(1, 1),
                PhysicalSize::new(120, 600),
                PhysicalSize::new(800, 105),
                PhysicalSize::new(320, 180),
                PhysicalSize::new(2000, 900),
            ] {
                for selected_popup in [None, Some("u1".into())] {
                    snapshot.tabs[0].selected_popup = selected_popup;
                    for scroll in [0.0, 247.5, f32::MAX] {
                        let hints = ShortcutHintsScene::new(
                            Vec::new(),
                            size,
                            metrics,
                            WorkspaceFocus::Terminal,
                            |_| 0.0,
                        );
                        let scene = WorkspaceScene::from_snapshot_with_bar(
                            &snapshot,
                            EonBar::default(),
                            size,
                            metrics,
                            0.0,
                            scroll,
                            |_, text| (text.into(), text.len() as f32 * metrics.width),
                            Some(hints),
                        );
                        for (allocated, terminal) in std::iter::once((scene.chrome, scene.terminal))
                            .chain(
                                scene
                                    .other_columns
                                    .iter()
                                    .map(|column| (column.chrome, column.terminal)),
                            )
                        {
                            let fill = scene.card_fill(
                                allocated,
                                metrics,
                                crate::render::physical_outline_width(2, metrics.scale),
                            );
                            assert!(
                                metrics.padding >= fill.radius,
                                "padding below radius at scale {scale}, size {size:?}, columns {count}"
                            );
                            assert!(
                                terminal.left + metrics.padding >= fill.bounds.left + fill.radius
                            );
                            assert!(
                                terminal.right() - metrics.padding
                                    <= fill.bounds.right() - fill.radius
                            );
                        }
                    }
                }
            }
        }
    }
    let project = |snapshot: &Snapshot| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(960, 600),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    let stack = project(&snapshot);
    snapshot.tabs[0].selected_popup = Some("u1".into());
    let popup = project(&snapshot);

    assert_eq!(popup.chrome, stack.chrome);
    let painted = stack.card_rect(stack.chrome, CellMetrics::for_scale(1.0), 2.0);
    assert_eq!(
        painted.top,
        stack.panes[0].rect.top + stack.panes[0].rect.height / 2.0
    );
    // The popup ends on the edge the column card and the sidebar card share.
    let popup_card = popup
        .card_fill(popup.chrome, CellMetrics::for_scale(1.0), 2.0)
        .bounds;
    assert_eq!(popup_card.bottom(), painted.bottom());
    assert_eq!(
        popup.card_rect(popup.chrome, CellMetrics::for_scale(1.0), 2.0),
        painted
    );
    for scale in [1.0, 1.3] {
        let metrics = CellMetrics::for_scale(scale);
        for count in 1..=4 {
            let mut snapshot = snapshot.clone();
            snapshot.tabs[0].columns = (1..=count)
                .map(|index| {
                    let mut column = snapshot.tabs[0].columns[0].clone();
                    column.panes[0].id = format!("p{index}");
                    column.expanded = format!("p{index}");
                    column
                })
                .collect();
            let project = |snapshot: &Snapshot| {
                WorkspaceScene::from_snapshot(
                    snapshot,
                    PhysicalSize::new(2200, 900),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                )
            };
            snapshot.tabs[0].selected_popup = None;
            let panes = project(&snapshot);
            snapshot.tabs[0].selected_popup = Some("u1".into());
            let popup = project(&snapshot);
            let painted = panes.card_rect(
                panes.chrome,
                metrics,
                crate::render::physical_outline_width(2, metrics.scale),
            );
            assert_eq!(popup.pane_viewport, panes.pane_viewport);
            assert_eq!(
                popup
                    .card_rect(
                        popup.chrome,
                        metrics,
                        crate::render::physical_outline_width(2, metrics.scale)
                    )
                    .top,
                painted.top
            );
            assert_eq!(popup.sidebar().unwrap().top, painted.top);
            let stroke = crate::render::physical_outline_width(2, metrics.scale);
            let popup_bottom = popup
                .card_fill(popup.chrome, metrics, stroke)
                .bounds
                .bottom();
            assert_eq!(
                popup_bottom,
                painted.bottom(),
                "popup bottom at scale {scale}, {count} columns"
            );
            assert_eq!(
                popup_bottom,
                popup
                    .card_rect(popup.sidebar_area().unwrap(), metrics, stroke)
                    .bottom(),
                "popup and sidebar bottoms at scale {scale}, {count} columns"
            );
            for column in &panes.other_columns {
                assert_eq!(
                    panes
                        .card_rect(
                            column.chrome,
                            metrics,
                            crate::render::physical_outline_width(2, metrics.scale)
                        )
                        .top,
                    painted.top
                );
            }
            assert!(
                (popup.terminal.top
                    - painted.top
                    - snapshot.geometry.vertical_margin * metrics.scale
                    - popup_title_gutter(metrics))
                .abs()
                    < 0.001,
                "popup retains its label gutter at scale {scale}, {count} columns"
            );
        }
    }
    // Short windows must not let the popup's top border cross its terminal cells.
    for scale in [1.0, 1.3] {
        let base = CellMetrics::for_scale(scale);
        let metrics = CellMetrics {
            font_size: 32.0 * base.scale,
            height: 36.0 * base.scale,
            ..base
        };
        for count in [1, 2, 3, 4, 0] {
            let mut snapshot = snapshot.clone();
            let column = snapshot.tabs[0].columns[0].clone();
            snapshot.tabs[0].columns = (0..count)
                .map(|index| {
                    let id = format!("p{}", index + 1);
                    let mut column = column.clone();
                    column.panes[0].id.clone_from(&id);
                    column.panes[0].attention = Some(eon_workspace_protocol::v26::Attention {
                        state: AttentionState::Unavailable,
                        record: None,
                    });
                    column.expanded = id;
                    column
                })
                .collect();
            snapshot.tabs[0].selected_pane = (count > 0).then(|| "p1".into());
            snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
            for height in [130, 1, 50, 74, 105, 140, 150, 160, 220, 900] {
                let project = |snapshot: &Snapshot| {
                    WorkspaceScene::from_snapshot_with_zoom(
                        snapshot,
                        EonBar::default(),
                        PhysicalSize::new(5000, height),
                        metrics,
                        0.0,
                        0.0,
                        |_, text| (text.into(), text.len() as f32 * metrics.width),
                        None,
                        false,
                        SidebarView {
                            focused: Some("p1"),
                            ..SidebarView::default()
                        },
                    )
                };
                snapshot.tabs[0].selected_popup = None;
                let panes = project(&snapshot);
                let pane_card = panes.card_rect(
                    panes.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale),
                );
                for (headers, _) in panes.column_panes() {
                    let first = &headers[0];
                    assert_eq!(
                        title_rule_y(first.rect.top, first.rect.height),
                        pane_card.top
                    );
                }
                if count > 0 {
                    assert!(panes.terminal.top >= pane_card.top);
                }
                let sidebar = panes.sidebar().unwrap();
                assert_eq!(sidebar.top, pane_card.top);
                for row in panes.sidebar_rows() {
                    assert!(row.rect.top >= sidebar.top);
                    if let Some(visible) = row.rect.intersection(panes.sidebar_list) {
                        assert!(visible.top >= sidebar.top);
                        assert!(visible.bottom() <= sidebar.bottom());
                    }
                }
                snapshot.tabs[0].selected_popup = Some("u1".into());
                let popup = project(&snapshot);
                let popup_card = popup_card_rect(popup.chrome, metrics);
                assert!(
                    popup.terminal.top >= popup_card.top,
                    "popup terminal {:?} crosses card {:?}: scale {scale}, {count} columns, height {height}",
                    popup.terminal,
                    popup_card
                );
                assert!(
                    popup.terminal.height + 0.001
                        >= minimum_terminal(metrics, metrics.height).min(popup.chrome.height)
                );
                assert!(popup.terminal.bottom() <= popup.chrome.bottom() + 0.001);
                if popup.chrome.height
                    >= minimum_terminal(metrics, metrics.height) + pane_card.top - popup.chrome.top
                {
                    assert_eq!(popup_card.top, pane_card.top);
                }
                let stroke = crate::render::physical_outline_width(2, metrics.scale);
                let popup_bottom = popup
                    .card_fill(popup.chrome, metrics, stroke)
                    .bounds
                    .bottom();
                assert_eq!(
                    popup_bottom,
                    pane_card.bottom(),
                    "popup bottom: scale {scale}, {count} columns, height {height}"
                );
                assert_eq!(
                    popup_bottom,
                    popup
                        .card_rect(popup.sidebar_area().unwrap(), metrics, stroke)
                        .bottom(),
                    "popup and sidebar bottoms: scale {scale}, {count} columns, height {height}"
                );
            }
        }
    }
    let metrics = CellMetrics::for_scale(1.0);
    let size = PhysicalSize::new(1280, 720);
    for count in [1, 3, 4] {
        let mut snapshot = snapshot.clone();
        let pane = snapshot.tabs[0].columns[0].panes[0].clone();
        snapshot.tabs[0].columns[0].panes = (1..=count)
            .map(|index| Pane {
                id: format!("p{index}"),
                ..pane.clone()
            })
            .collect();
        let hints =
            ShortcutHintsScene::new(Vec::new(), size, metrics, WorkspaceFocus::Terminal, |_| 0.0);
        let popup = WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar::default(),
            size,
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
            Some(hints),
        );
        let old_height = popup.chrome.height
            - 2.0 * snapshot.geometry.vertical_margin * metrics.scale
            - popup_title_gutter(metrics);
        let moved_edge = popup
            .card_rect(
                popup.chrome,
                metrics,
                crate::render::physical_outline_width(2, metrics.scale),
            )
            .top
            - popup.chrome.top;
        assert_eq!(popup.terminal.height, old_height - moved_edge);
        assert_eq!(
            popup.terminal.bottom(),
            popup.chrome.bottom() - snapshot.geometry.vertical_margin * metrics.scale
        );
        let rows = |height: f32| ((height - 2.0 * metrics.padding) / metrics.height).floor() as u16;
        assert_eq!((rows(old_height), rows(popup.terminal.height)), (33, 32));
        println!(
            "1280x720, popup over {count} panes: {} -> {} rows, terminal height {old_height} -> {}",
            rows(old_height),
            rows(popup.terminal.height),
            popup.terminal.height
        );
    }
    let size = PhysicalSize::new(960, 600);
    let metrics = CellMetrics::for_scale(1.0);
    let gap = card_gap(metrics);
    // The card keeps one gap from the window's sides.
    assert_eq!(
        (stack.chrome.left, stack.chrome.right()),
        (gap, 960.0 - gap)
    );
    assert_ne!(popup.terminal, popup.chrome);
    // Tab hit rectangles keep their original inset; the card area adds the title margin.
    let small = CellMetrics {
        width: 5.0,
        height: 9.0,
        font_size: 8.0,
        ..metrics
    };
    for (metrics, title_margin) in [(metrics, 6.0), (small, 3.0)] {
        let scene = WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
            (text.into(), text.len() as f32 * 10.0)
        });
        let gap = card_gap(metrics);
        let pill = scene.tabs[0].rect;
        assert_eq!((pill.left, pill.top), (gap, gap), "{metrics:?}");
        assert_eq!(
            scene.chrome.top - pill.bottom(),
            gap + title_margin,
            "{metrics:?}"
        );
    }
    // Toggling the popup keeps the shared card area, even where gaps yield on narrow or
    // short windows with several columns.
    let mut columns = snapshot.clone();
    for index in 2..=3 {
        columns.tabs[0]
            .columns
            .push(eon_workspace_protocol::v26::Column {
                panes: vec![Pane {
                    label: format!("p{index}"),
                    pinned_name: None,
                    id: format!("p{index}"),
                    session: format!("session-{}", index + 1),
                    endpoint: format!("/run/pane-{index}.sock").into_bytes(),
                    live: true,
                    attention: None,
                }],
                expanded: format!("p{index}"),
            });
    }
    for size in [PhysicalSize::new(120, 600), PhysicalSize::new(800, 105)] {
        let areas = [None, Some("u1".into())].map(|selected_popup| {
            columns.tabs[0].selected_popup = selected_popup;
            WorkspaceScene::from_snapshot(&columns, size, metrics, 0.0, 0.0, |_, text| {
                (text.into(), text.len() as f32 * 10.0)
            })
            .pane_viewport
        });
        assert_eq!(areas[0], areas[1], "{size:?}");
    }
    for selected_popup in [None, Some("u1".into())] {
        snapshot.tabs[0].selected_popup = selected_popup;
        let baseline = project(&snapshot);
        let hint = ShortcutHint {
            modifiers: "Alt".into(),
            key: "/".into(),
            label: "all keys".into(),
            accessible_label: "Alt+/ — Show shortcuts".into(),
            priority: 0,
        };
        for focus in [
            WorkspaceFocus::Terminal,
            WorkspaceFocus::Header(WorkspaceHeaderControl::Shortcuts),
        ] {
            let footer =
                ShortcutHintsScene::new(vec![hint.clone()], size, metrics, focus, |text| {
                    text.len() as f32 * 10.0
                });
            let footer_rect = footer.rect;
            let with_hints = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
                Some(footer),
            );
            // The footer's band replaces the bottom gap as the cards' bottom margin.
            assert_eq!(
                baseline.terminal.height - with_hints.terminal.height,
                footer_rect.height - gap
            );
            assert_eq!(baseline.pane_viewport.bottom(), size.height as f32 - gap);
            assert_eq!(with_hints.pane_viewport.bottom(), footer_rect.top);
            assert_eq!(footer_rect.bottom(), size.height as f32);
            assert_eq!(with_hints.chrome.left, gap);
            assert_eq!(with_hints.hit_test(20.0, footer_rect.top + 5.0), None);
            assert_eq!(
                with_hints.has_control(WorkspaceHeaderControl::Shortcuts),
                focus != WorkspaceFocus::Terminal
            );
            assert!(with_hints.has_control(WorkspaceHeaderControl::CloseTab));
        }
    }
}

#[test]
fn shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space() {
    let groups = vec![ShortcutGroup::new(
        "Projects and tools",
        (0..32)
            .map(|index| ShortcutRow::new("Alt+Z", format!("Entry {index}")))
            .collect(),
    )];
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for (width, height) in [(960, 600), (320, 240), (320, 180)] {
            let size = PhysicalSize::new(
                (f64::from(width) * scale) as u32,
                (f64::from(height) * scale) as u32,
            );
            let start =
                DialogScene::new(DialogKind::Shortcuts, groups.clone(), size, metrics, -100.0);
            let end = DialogScene::new(
                DialogKind::Shortcuts,
                groups.clone(),
                size,
                metrics,
                f32::MAX,
            );

            assert_eq!(start.scroll, 0.0);
            assert!(end.max_scroll > 0.0);
            assert_eq!(end.scroll, end.max_scroll);
            assert!(start.bounds.width <= size.width as f32);
            assert!(start.bounds.height <= size.height as f32);
            assert!(start.title.bottom() <= start.subtitle.top);
            assert!(
                start.subtitle.bottom() + metrics.height * 0.25 <= start.groups[0].heading.top,
                "header text overlaps content at {size:?}, scale {scale}"
            );
            assert!(start.groups[0].heading.bottom() <= start.groups[0].rows[0].rect.top);
            if height >= 240 {
                assert!(start.groups[0].rows[0].rect.bottom() <= start.content.bottom());
            }
            assert_eq!(start.title, end.title);
            assert_eq!(start.subtitle, end.subtitle);
            assert_eq!(start.content, end.content);
            assert!(start.content.bottom() < start.bounds.bottom());
            assert_eq!(end.groups[0].rows.len(), 32);
        }
    }

    let metrics = CellMetrics::for_scale(1.0);
    let tiny = DialogScene::new(
        DialogKind::Shortcuts,
        Vec::new(),
        PhysicalSize::new(1, 1),
        metrics,
        0.0,
    );
    assert!(tiny.bounds.right() <= 1.0 && tiny.bounds.bottom() <= 1.0);
    assert!(
        tiny.content.right() <= tiny.bounds.right()
            && tiny.content.bottom() <= tiny.bounds.bottom()
    );
    assert!(tiny.content.height >= 0.0);

    // The close prompt fits its rows: header labels, then popup entry labels.
    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: id.as_bytes().to_vec(),
        live: true,
        attention: None,
    };
    let tab = Tab {
        id: "t1".into(),
        directory: b"/tmp".to_vec(),
        pending: false,
        selected_pane: Some("p1".into()),
        selected_popup: None,
        columns: vec![eon_workspace_protocol::v26::Column {
            panes: vec![pane("p1"), pane("p2")],
            expanded: "p1".into(),
        }],
        popups: vec![eon_workspace_protocol::v26::Popup {
            id: "u1".into(),
            entry: "agent".into(),
            session: "popup-session".into(),
            endpoint: b"/tmp/popup.sock".to_vec(),
            attention: None,
        }],
        attention_rollup: None,
    };
    let entries = [PopupEntry {
        id: "agent".into(),
        label: "Agent".into(),
        shortcut: eon_workspace_protocol::v26::PopupKey::Bound(
            eon_workspace_protocol::v26::Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyL".into(),
            },
        ),
    }];
    let size = PhysicalSize::new(960, 600);
    let prompt =
        |detach| DialogScene::close_last_tab(&tab, &entries, detach, None, size, metrics, 0.0);
    let asked = prompt(Some("Alt+Shift+D".into()));
    let panes = tab.panes().count();
    assert_eq!(
        asked.kind,
        DialogKind::CloseLastTab {
            terminals: panes + 1,
            detach: Some("Alt+Shift+D".into()),
            workspace: None,
        }
    );
    let rows = &asked.groups[0].rows;
    assert_eq!(rows.len(), panes + 1);
    assert!(rows[..panes].iter().all(|row| row.shortcut == "Pane"));
    assert_eq!(rows[0].action, tab.panes().next().unwrap().label);
    assert_eq!(
        (rows[panes].shortcut.as_str(), rows[panes].action.as_str()),
        ("Popup", "Agent")
    );
    assert_eq!(asked.groups[0].heading.height, 0.0);
    assert_eq!(asked.max_scroll, 0.0);
    assert!(rows[panes].rect.bottom() <= asked.content.bottom());
    assert!(asked.bounds.height < size.height as f32 / 2.0);
    assert_eq!(asked.kind.footer().len(), 2);
    // Without the detach chord the prompt drops its hint line and shrinks.
    let taken = prompt(None);
    assert_eq!(taken.kind.footer(), ["Enter Quit · Esc Cancel"]);
    assert!(taken.bounds.height < asked.bounds.height);
    // While other workspaces remain, the close removes the shown one instead.
    let workspace = DialogScene::close_last_tab(
        &tab,
        &entries,
        Some("Alt+Shift+D".into()),
        Some("notes".into()),
        size,
        metrics,
        0.0,
    );
    assert_eq!(workspace.kind.title(), "Close the last tab?");
    assert_eq!(
        workspace.kind.subtitle(),
        "This closes workspace notes and stops 3 terminals."
    );
    assert_eq!(
        workspace.kind.footer(),
        [
            "Enter Close · Esc Cancel",
            "Alt+Shift+D keeps it running instead"
        ]
    );
    // A list taller than the window scrolls within the prompt.
    let crowded = DialogScene::close_last_tab(
        &tab,
        &entries,
        None,
        None,
        PhysicalSize::new(320, 180),
        metrics,
        f32::MAX,
    );
    assert!(crowded.max_scroll > 0.0);
    assert_eq!(crowded.scroll, crowded.max_scroll);
    assert!(crowded.bounds.bottom() <= 180.0);
}
