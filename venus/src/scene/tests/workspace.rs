use super::*;

#[test]
fn compact_pane_headers_share_title_height_and_full_row_targets() {
    use eon_workspace_protocol::v45::{Column, PopupGeometry};

    let snapshot = Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
                        tag: None,
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
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
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
                    top: 52.0,
                    width: 1296.0,
                    height: 840.0
                }
            );
            // The Stack band, one title row and four rise pixels, sits above the headers.
            assert_eq!(
                scene.header_viewport,
                SceneRect {
                    top: 76.0,
                    height: 816.0,
                    ..scene.pane_viewport
                }
            );
            assert_eq!(
                scene.terminal,
                SceneRect {
                    left: 296.0,
                    top: 130.0,
                    width: 1296.0,
                    height: 722.0
                }
            );
        }
        if scale <= 1.3 {
            use eon_workspace_protocol::v45::{CodexQuota, CodexQuotaState};
            let mut snapshot = snapshot.clone();
            snapshot.codex_quota = Some(CodexQuota {
                state: CodexQuotaState::Unknown,
                observed_at: 0,
                windows: Vec::new(),
            });
            let cpu = CpuUse {
                percent: 12,
                label: "\u{f4bc} 12%".into(),
                tooltip: "CPU".into(),
                description: "CPU".into(),
                load: crate::Load::Normal,
            };
            let ram = RamUse {
                label: "\u{efc5} 43%".into(),
                tooltip: "RAM".into(),
                description: "RAM".into(),
                load: crate::Load::Normal,
                percent: 43,
            };
            let clock = LocalClock {
                label: "15:43".into(),
                tooltip: "Clock".into(),
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
            ]) {
                let painted = widgets.bar_pill_rect(rect, metrics);
                assert_eq!((painted.top, painted.height), (pill.top, pill.height));
                assert_eq!(rect.top, 0.0);
                assert_eq!(rect.height, widgets.header.height);
            }
            for rect in widgets.controls.iter().map(|control| control.rect) {
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
    for (count, before, after) in [(1, 31, 30), (3, 28, 28), (4, 26, 27)] {
        let mut snapshot = snapshot.clone();
        snapshot.tabs[0].columns[0].panes.truncate(count);
        snapshot.tabs[0].selected_pane = Some("p1".into());
        snapshot.tabs[0].columns[0].expanded = "p1".into();
        let hints = ShortcutHintsScene::new(Vec::new(), size, metrics, None, None, |_| 0.0);
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
        let stack = scene.header_viewport.top - scene.pane_viewport.top;
        let rows = |header: f32| {
            ((scene.chrome.height - stack - count as f32 * header - metrics.padding * 2.0)
                / metrics.height)
                .floor() as u16
        };
        let actual_rows =
            ((scene.terminal.height - metrics.padding * 2.0) / metrics.height).floor() as u16;
        println!(
            "1280x720, {count} panes: {} -> {actual_rows} rows, allocated height {}",
            rows((metrics.height * 1.5).round()),
            scene.chrome.height
        );
        assert_eq!(rows((metrics.height * 1.5).round()), before);
        assert_eq!(actual_rows, after);
        // The first header uses ten reserve pixels; the remainder follows its terminal.
        let cap = if count > 1 { 14.0 } else { 10.0 };
        assert_eq!(scene.terminal.top, scene.panes[0].rect.bottom());
        assert_eq!(
            scene.terminal.height,
            scene.header_viewport.height - count as f32 * scene.panes[0].rect.height - cap
        );
        if count > 1 {
            assert_eq!(
                scene.panes[1].rect.top,
                scene.terminal.bottom() + cap - 10.0
            );
            assert_eq!(
                scene.panes[count - 1].rect.bottom(),
                scene.header_viewport.bottom()
            );
        } else {
            assert_eq!(scene.terminal.bottom(), scene.header_viewport.bottom());
        }
        let painted = scene.card_rect(
            scene.chrome,
            metrics,
            crate::render::physical_outline_width(2, metrics.scale),
        );
        // The card's top line carries the Stack tab; the first header sits one band lower.
        assert_eq!(
            painted.top,
            (scene.pane_viewport.top + scene.panes[0].rect.height / 2.0).floor()
        );
        assert_eq!(scene.panes[0].rect.top, scene.header_viewport.top + 10.0);
        assert_eq!(
            painted.bottom(),
            (scene.chrome.bottom() - chrome_title_metrics(metrics).text_height / 2.0).floor()
        );
        assert_eq!(painted.top - scene.chrome.top, 10.0);
    }
}

#[test]
fn band_space_keeps_one_terminal_height_whichever_pane_is_expanded() {
    // Fractional scales sum fractional rows, so positions compare within a hundredth of a pixel.
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    for (scale, cap) in [(1.0, 14.0), (1.125, 16.0), (4.0 / 3.0, 18.0), (2.0, 28.0)] {
        let metrics = CellMetrics::for_scale(scale);
        let scenes: Vec<_> = (0..4)
            .map(|expanded| {
                let mut snapshot = super::bar::one_pane_snapshot();
                let tab = &mut snapshot.tabs[0];
                let pane = tab.columns[0].panes[0].clone();
                tab.columns[0].panes = (1..=4)
                    .map(|n| eon_workspace_protocol::v45::Pane {
                        label: format!("p{n}"),
                        id: format!("p{n}"),
                        ..pane.clone()
                    })
                    .collect();
                tab.selected_pane = Some(format!("p{}", expanded + 1));
                tab.columns[0].expanded = format!("p{}", expanded + 1);
                let (_, overhead) = WorkspaceScene::initial_overhead(&snapshot, metrics);
                let height = (overhead + metrics.padding * 2.0 + 28.0 * metrics.height).ceil();
                let size = PhysicalSize::new(1280, height as u32);
                let hints = ShortcutHintsScene::new(Vec::new(), size, metrics, None, None, |_| 0.0);
                WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    EonBar::default(),
                    size,
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                    Some(hints),
                )
            })
            .collect();
        for (expanded, scene) in scenes.iter().enumerate() {
            let panes = &scene.panes;
            assert_eq!(panes.len(), 4);
            assert_eq!(scene.terminal.height, scenes[0].terminal.height);
            assert_eq!(
                metrics
                    .surface_size(scene.terminal.screen_size())
                    .unwrap()
                    .rows,
                28
            );
            assert!(near(scene.terminal.top, panes[expanded].rect.bottom()));
            // The last title row ends the column, or the terminal when the last pane is expanded.
            assert!(
                near(
                    panes[3].rect.bottom().max(scene.terminal.bottom()),
                    scene.header_viewport.bottom()
                ),
                "the column must fill its viewport"
            );
            for (index, pair) in panes.windows(2).enumerate() {
                // A first pane splits the reserve around its header and terminal;
                // later expanded panes use it all before their header.
                let terminal = if index == expanded {
                    scene.terminal.height
                } else {
                    0.0
                };
                let space = if index + 1 == expanded {
                    cap
                } else if expanded == 0 && index == 0 {
                    cap - (panes[0].rect.top - scene.header_viewport.top)
                } else {
                    0.0
                };
                assert!(
                    near(pair[1].rect.top, pair[0].rect.bottom() + terminal + space),
                    "scale {scale}, pane {expanded} expanded, row {index}: {} after {}",
                    pair[1].rect.top,
                    pair[0].rect.bottom()
                );
                if space > 0.0 {
                    assert_eq!(
                        scene.hit_test(pair[0].rect.left + 1.0, pair[1].rect.top - space / 2.0),
                        None
                    );
                }
            }
        }
    }
}

#[test]
fn popup_outer_chrome_reuses_the_pane_stack_rectangle() {
    use eon_workspace_protocol::v45::{
        ALT, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Tab,
    };

    let mut snapshot = Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
            columns: vec![eon_workspace_protocol::v45::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
                    id: "p1".into(),
                    session: "terminal-1".into(),
                    endpoint: b"/run/pane.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: vec![Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "terminal-2".into(),
                endpoint: b"/run/popup.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v45::Sidebar::Open,
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
                        let hints =
                            ShortcutHintsScene::new(Vec::new(), size, metrics, None, None, |_| 0.0);
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
    // The column card's top line carries its Stack tab; the first pane sits one Stack band lower.
    assert_eq!(
        painted.top,
        stack.pane_viewport.top + stack.panes[0].rect.height / 2.0
    );
    assert_eq!(stack.panes[0].rect.top, stack.header_viewport.top + 10.0);
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
                    column.panes[0].attention = Some(eon_workspace_protocol::v45::Attention {
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
                // Each column's first header sits below its Stack band.
                assert_eq!(
                    title_rule_y(
                        panes.pane_viewport.top,
                        chrome_title_metrics(metrics)
                            .text_height
                            .min(panes.chrome.height)
                    ),
                    pane_card.top
                );
                for (headers, _) in panes.column_panes() {
                    assert!(headers[0].rect.top >= panes.header_viewport.top);
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
        let hints = ShortcutHintsScene::new(Vec::new(), size, metrics, None, None, |_| 0.0);
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
    // Tab hit rectangles keep their original inset; the card area adds the bar-to-card margin.
    let small = CellMetrics {
        width: 5.0,
        height: 9.0,
        font_size: 8.0,
        ..metrics
    };
    for (metrics, title_margin) in [(metrics, 12.0), (small, 12.0)] {
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
            .push(eon_workspace_protocol::v45::Column {
                panes: vec![Pane {
                    label: format!("p{index}"),
                    pinned_name: None,
                    tag: None,
                    id: format!("p{index}"),
                    session: format!("terminal-{}", index + 1),
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
            modifiers: String::new(),
            modifier_names: String::new(),
            key: "Escape".into(),
            label: "terminal".into(),
            short: "terminal".into(),
            description: "Return focus to the terminal".into(),
            section: crate::HintSection::Actions,
            accessible_label: "Escape — Return focus to the terminal".into(),
            priority: 0,
        };
        // Shortcuts stays whether the bar fits every hint or drops one.
        for (label, complete) in [("terminal".to_owned(), true), ("界".repeat(400), false)] {
            let hint = ShortcutHint {
                label,
                ..hint.clone()
            };
            let footer = ShortcutHintsScene::new(vec![hint], size, metrics, None, None, |text| {
                text.len() as f32 * 10.0
            });
            assert_eq!(footer.complete, complete);
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
            assert!(with_hints.has_control(WorkspaceHeaderControl::Shortcuts));
            assert!(with_hints.has_control(WorkspaceHeaderControl::CloseTab));
        }
    }
}

#[test]
fn shortcut_viewer_sizes_to_its_rows_and_flows_into_columns_between_the_bars() {
    let close_hint = if cfg!(target_os = "macos") {
        "Esc or Option+/ to close"
    } else {
        "Esc or Alt+/ to close"
    };
    let metrics = CellMetrics::for_scale(1.0);
    let measure = crate::CellText(metrics);
    let group = |title: &str, rows: usize| {
        ShortcutGroup::new(
            title,
            (0..rows)
                .map(|index| ShortcutRow::new("Alt+Shift+Z", format!("Entry {index}")))
                .collect(),
        )
    };
    // A short list keeps one column, sized to its rows, and says nothing about scrolling.
    let short = DialogScene::new(
        DialogKind::Shortcuts,
        vec![group("Navigate", 3)],
        PhysicalSize::new(1280, 800),
        metrics,
        0.0,
        &measure,
    );
    assert!(short.bounds.width < 640.0 && short.bounds.height < 400.0);
    assert_eq!(short.max_scroll, 0.0);
    assert_eq!(short.footer(), [close_hint]);
    let label = short.groups[0].rows[0].label_width;
    assert!(label >= measure.width("Alt+Shift+Z") && label < measure.width("Alt+Shift+Z") + 2.0);
    // A list taller than one column flows into balanced columns instead of scrolling.
    let size = PhysicalSize::new(1280, 800);
    let long = DialogScene::new(
        DialogKind::Shortcuts,
        vec![group("Navigate", 30), group("Host", 30)],
        size,
        metrics,
        0.0,
        &measure,
    );
    assert_eq!(long.max_scroll, 0.0);
    assert_eq!(long.footer(), [close_hint]);
    let mut lefts = long
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .map(|row| row.rect.left)
        .collect::<Vec<_>>();
    lefts.dedup();
    assert!(lefts.len() >= 2, "{lefts:?}");
    for row in long.groups.iter().flat_map(|group| &group.rows) {
        assert!(row.rect.top >= long.content.top && row.rect.bottom() <= long.content.bottom());
        assert!(row.rect.right() <= long.content.right());
    }
    // A section split across columns repeats its heading above its continuation.
    assert!(!long.continued.is_empty());
    // The dialog stays clear of the Eon Bar and the hints bar.
    let edge = header_heights(metrics).0;
    assert!(long.bounds.top >= edge);
    assert!(long.bounds.bottom() <= size.height as f32 - edge);
    // The Window menu keeps one column; its scroll hint stays.
    let menu = DialogScene::new(
        DialogKind::WindowMenu,
        vec![group("This window", 3)],
        size,
        metrics,
        0.0,
        &measure,
    );
    assert!(menu.bounds.width < 640.0);
    assert_eq!(menu.footer(), DialogKind::WindowMenu.footer());
}

#[test]
fn shortcut_viewer_section_moving_whole_to_a_column_leaves_no_blank_space() {
    let metrics = CellMetrics::for_scale(1.0);
    let rows = |count| {
        (0..count)
            .map(|index| ShortcutRow::new("Alt+Z", format!("Entry {index}")))
            .collect()
    };
    // Two equal sections too tall for one column split at the second's first row.
    let scene = DialogScene::new(
        DialogKind::Shortcuts,
        vec![
            ShortcutGroup::new("Navigate", rows(15)),
            ShortcutGroup::new("Host", rows(15)),
        ],
        PhysicalSize::new(1280, 800),
        metrics,
        0.0,
        &CellText(metrics),
    );
    let (first, second) = (&scene.groups[0], &scene.groups[1]);
    assert!(second.rows[0].rect.left > first.rows[0].rect.left);
    assert_eq!(second.heading.left, second.rows[0].rect.left);
    assert!(scene.continued.is_empty());
    assert_eq!(scene.max_scroll, 0.0);
    // The content ends with its tallest column, within the pixel its height rounds up.
    let bottom = scene
        .groups
        .iter()
        .flat_map(|group| &group.rows)
        .map(|row| row.rect.bottom())
        .fold(0.0, f32::max);
    assert!(scene.content.bottom() - bottom < 1.0, "{scene:?}");
}

#[test]
fn dialog_rows_take_the_lines_their_descriptions_wrap_to() {
    // A word wider than its line breaks between glyphs: 21 glyphs of 10 px need three lines
    // of 106 px, not the two their total width suggests.
    let ten = |text: &str| text.chars().count() as f32 * 10.0;
    assert_eq!(crate::wrapped_lines(&"x".repeat(21), 106.0, ten), 3);
    assert_eq!(crate::wrapped_lines("aaaa bbbb cccc", 95.0, ten), 2);
    assert_eq!(crate::wrapped_lines("", 95.0, ten), 1);

    // Row heights follow the measured lines at the width each description draws in.
    struct Wraps(CellText);
    impl DialogText for Wraps {
        fn width(&self, text: &str) -> f32 {
            self.0.width(text)
        }
        fn lines(&self, text: &str, width: f32) -> usize {
            if text.starts_with("Long") {
                4
            } else {
                self.0.lines(text, width)
            }
        }
    }
    let metrics = CellMetrics::for_scale(1.0);
    let scene = DialogScene::new(
        DialogKind::Shortcuts,
        vec![ShortcutGroup::new(
            "Navigate",
            vec![
                ShortcutRow::new("Alt+A", "Long unbroken label"),
                ShortcutRow::new("Alt+B", "Short"),
            ],
        )],
        PhysicalSize::new(1280, 800),
        metrics,
        0.0,
        &Wraps(CellText(metrics)),
    );
    let [long, short] = &scene.groups[0].rows[..] else {
        panic!("two rows");
    };
    assert!(long.rect.height >= short.rect.height + metrics.height * 1.25 * 3.0 - 1.0);
    assert!(short.rect.top >= long.rect.bottom());
}

#[test]
fn shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space() {
    let groups = vec![ShortcutGroup::new(
        "Projects and tools",
        // More rows than three columns show in the largest window.
        (0..200)
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
            let start = DialogScene::new(
                DialogKind::Shortcuts,
                groups.clone(),
                size,
                metrics,
                -100.0,
                &crate::CellText(metrics),
            );
            let end = DialogScene::new(
                DialogKind::Shortcuts,
                groups.clone(),
                size,
                metrics,
                f32::MAX,
                &crate::CellText(metrics),
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
            assert_eq!(end.groups[0].rows.len(), 200);
        }
    }

    let metrics = CellMetrics::for_scale(1.0);
    let tiny = DialogScene::new(
        DialogKind::Shortcuts,
        Vec::new(),
        PhysicalSize::new(1, 1),
        metrics,
        0.0,
        &crate::CellText(metrics),
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
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
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
        columns: vec![eon_workspace_protocol::v45::Column {
            panes: vec![pane("p1"), pane("p2")],
            expanded: "p1".into(),
        }],
        popups: vec![eon_workspace_protocol::v45::Popup {
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
        shortcut: eon_workspace_protocol::v45::PopupKey::Bound(
            eon_workspace_protocol::v45::Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyL".into(),
            },
        ),
    }];
    let size = PhysicalSize::new(960, 600);
    let prompt = |detach| {
        DialogScene::close_last_tab(
            &tab,
            &entries,
            detach,
            None,
            size,
            metrics,
            0.0,
            &crate::CellText(metrics),
        )
    };
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
        &crate::CellText(metrics),
    );
    assert_eq!(workspace.kind.title(), "Close the last tab?");
    assert_eq!(
        workspace.kind.subtitle(),
        "This closes session notes and stops 3 terminals."
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
        &crate::CellText(metrics),
    );
    assert!(crowded.max_scroll > 0.0);
    assert_eq!(crowded.scroll, crowded.max_scroll);
    assert!(crowded.bounds.bottom() <= 180.0);
}

#[test]
fn quit_prompt_words_counts_and_lists_busy_terminals() {
    use eon_workspace_protocol::v45::{BusyTerminal, BusyTerminals, TargetId, TerminalTarget};

    let busy = |id: TargetId, name: &str| BusyTerminal {
        target: TerminalTarget {
            workspace: "s1".into(),
            tab: "t1".into(),
            id,
            terminal: "session".into(),
        },
        name: name.into(),
    };
    let metrics = CellMetrics::for_scale(1.0);
    let size = PhysicalSize::new(960, 600);
    let one = BusyTerminals {
        terminals: vec![busy(TargetId::Pane("p1".into()), "p1")],
        omitted: 0,
    };
    let prompt = |terminals, busy: Option<&BusyTerminals>, detach: Option<&str>| {
        DialogScene::quit_window(
            terminals,
            busy,
            detach.map(Into::into),
            size,
            metrics,
            0.0,
            &crate::CellText(metrics),
        )
    };
    let single = prompt(Some(1), Some(&one), Some("Alt+Shift+D"));
    assert_eq!(single.kind.title(), "Quit this window?");
    assert_eq!(
        single.kind.subtitle(),
        "This stops 1 terminal; 1 still running."
    );
    assert_eq!(
        single.kind.footer(),
        [
            "Enter Quit · Esc Cancel",
            "Alt+Shift+D keeps it running instead"
        ]
    );
    let rows = |scene: &DialogScene| {
        scene.groups[0]
            .rows
            .iter()
            .map(|row| (row.shortcut.clone(), row.action.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&single), [("Pane".into(), "p1".into())]);
    assert_eq!(single.max_scroll, 0.0);
    assert!(single.bounds.height < size.height as f32 / 2.0);

    // Named terminals in window order, then how many more are busy.
    let many = BusyTerminals {
        terminals: vec![
            busy(TargetId::Pane("p2".into()), "p2 cargo build"),
            busy(TargetId::Popup("u1".into()), "Agent"),
        ],
        omitted: 2,
    };
    let plural = prompt(Some(6), Some(&many), None);
    assert_eq!(
        plural.kind.subtitle(),
        "This stops 6 terminals; 4 still running."
    );
    assert_eq!(
        rows(&plural),
        [
            ("Pane".into(), "p2 cargo build".into()),
            ("Popup".into(), "Agent".into()),
            ("…".into(), "2 more".into()),
        ]
    );
    // A popup on the detach chord drops the hint line.
    assert_eq!(plural.kind.footer(), ["Enter Quit · Esc Cancel"]);

    let idle = BusyTerminals {
        terminals: vec![],
        omitted: 0,
    };
    assert_eq!(
        prompt(Some(2), Some(&idle), None).kind.subtitle(),
        "This stops 2 terminals."
    );
    assert_eq!(
        prompt(None, Some(&one), None).kind.subtitle(),
        "This stops every terminal of this window."
    );
}

/// A one-tab snapshot of columns holding `counts` panes each, numbered `p1` onward, with the
/// first pane of column `selected` selected and no sidebar.
fn stack_columns_snapshot(counts: &[usize], selected: usize) -> Snapshot {
    use eon_workspace_protocol::v45::{Column, PopupGeometry};

    let mut next = 1;
    let columns = counts
        .iter()
        .map(|count| {
            let first = next;
            next += count;
            Column {
                panes: (first..first + count)
                    .map(|n| Pane {
                        label: format!("p{n}"),
                        pinned_name: None,
                        tag: None,
                        id: format!("p{n}"),
                        session: format!("s{n}"),
                        endpoint: format!("/tmp/p{n}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: format!("p{first}"),
            }
        })
        .collect::<Vec<_>>();
    Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
            selected_pane: Some(columns[selected].expanded.clone()),
            selected_popup: None,
            columns,
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
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: Vec::new(),
    }
}

#[test]
fn stack_close_targets_sit_in_the_stack_band_and_hide_while_zoomed() {
    use HeadingAction::CloseStack;

    let project = |snapshot: &Snapshot, size: (u32, u32), metrics: CellMetrics, zoom: bool| {
        WorkspaceScene::from_snapshot_with_zoom(
            snapshot,
            EonBar::default(),
            PhysicalSize::new(size.0, size.1),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
            None,
            zoom,
            SidebarView::default(),
        )
    };
    for scale in [1.0, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let button = chrome_title_metrics(metrics).text_height;
        // With the first, the middle and the last column selected, target `i` belongs to the
        // column whose position is `i`, never to a card's place in the selected-first list.
        for selected in 0..3 {
            let snapshot = stack_columns_snapshot(&[2, 1, 3], selected);
            let scene = project(&snapshot, (1400, 900), metrics, false);
            let cards = scene
                .column_cards()
                .map(|(chrome, _, _, index)| (index, chrome))
                .collect::<Vec<_>>();
            assert_eq!(
                cards[0].0, selected,
                "the selected column's card comes first"
            );
            let mut indices = cards.iter().map(|(index, _)| *index).collect::<Vec<_>>();
            indices.sort_unstable();
            assert_eq!(indices, [0, 1, 2]);
            let mut lefts = cards.clone();
            lefts.sort_by(|a, b| a.1.left.total_cmp(&b.1.left));
            assert_eq!(
                lefts.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
                [0, 1, 2],
                "a card's index is its column's position from the left"
            );
            for (index, chrome) in cards {
                let target = scene
                    .heading_action(CloseStack(index))
                    .expect("every Stack shows its Close");
                // A square of the title height in the Stack band, half a padding inside the
                // right edge of its own column's card, above every pane header.
                assert_eq!((target.width, target.height), (button, button));
                assert_eq!(target.top, scene.pane_viewport.top);
                assert!(
                    (target.right() - (chrome.right() - metrics.padding / 2.0)).abs() < 0.001,
                    "selected={selected} index={index}"
                );
                assert!(target.left > chrome.left);
                assert!(target.bottom() <= scene.header_viewport.top);
                assert_eq!(
                    scene.hit_test(
                        target.left + target.width / 2.0,
                        target.top + target.height / 2.0
                    ),
                    Some(WorkspaceHit::HeadingAction(CloseStack(index)))
                );
                assert_eq!(
                    scene.heading_action_label(CloseStack(index)),
                    format!("Close Stack {}", index + 1)
                );
            }
            assert_eq!(scene.heading_action(CloseStack(3)), None);
            assert_eq!(
                scene.heading_actions().collect::<Vec<_>>(),
                [CloseStack(0), CloseStack(1), CloseStack(2)]
            );

            // While a pane is zoomed only it is laid out, and no Stack shows a Close.
            let zoomed = project(&snapshot, (1400, 900), metrics, true);
            assert!(zoomed.zoomed_pane().is_some());
            assert_eq!(zoomed.heading_actions().count(), 0);
            for index in 0..3 {
                assert_eq!(zoomed.heading_action(CloseStack(index)), None);
            }
        }

        // The only Stack is closed by its plain name.
        let lone = project(
            &stack_columns_snapshot(&[1], 0),
            (1400, 900),
            metrics,
            false,
        );
        assert!(lone.heading_action(CloseStack(0)).is_some());
        assert_eq!(lone.heading_action_label(CloseStack(0)), "Close Stack");
        assert_eq!(lone.heading_actions().collect::<Vec<_>>(), [CloseStack(0)]);

        // A column too narrow for a full button and its margins shows none.
        let narrow = project(
            &stack_columns_snapshot(&[1, 1, 1, 1], 0),
            (200, 900),
            metrics,
            false,
        );
        for (chrome, label, _, index) in narrow.column_cards() {
            assert!(label.is_some());
            assert!(chrome.width < button + metrics.padding * 4.0);
            assert_eq!(narrow.heading_action(CloseStack(index)), None);
        }
        assert_eq!(narrow.heading_actions().count(), 0);

        // A window too short for the Stack band has no Stack tab to hold one.
        let short = project(
            &stack_columns_snapshot(&[1, 1], 0),
            (1400, (112.0 * scale) as u32),
            metrics,
            false,
        );
        assert!(short.column_cards().all(|(_, label, _, _)| label.is_none()));
        assert_eq!(short.heading_actions().count(), 0);

        // A shown popup covers the columns: only its own Hide remains.
        let mut popup = stack_columns_snapshot(&[1, 1], 0);
        popup.tabs[0].selected_popup = Some("u1".into());
        popup.tabs[0].popups = vec![eon_workspace_protocol::v45::Popup {
            id: "u1".into(),
            entry: "files".into(),
            session: "popup".into(),
            endpoint: b"/tmp/popup.sock".to_vec(),
            attention: None,
        }];
        popup.entries = vec![eon_workspace_protocol::v45::PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: eon_workspace_protocol::v45::PopupKey::Unbound,
        }];
        let covered = project(&popup, (1400, 900), metrics, false);
        assert_eq!(
            covered.heading_actions().collect::<Vec<_>>(),
            [HeadingAction::HidePopup]
        );
    }
}

#[test]
fn close_stack_prompt_words_each_end_and_lists_every_terminal() {
    use eon_workspace_protocol::v45::{
        CloseEnd, CloseScope, CloseTerminal, TargetId, TerminalTarget, UntrackedRun,
    };

    let terminal = |id: TargetId, name: &str, busy| CloseTerminal {
        target: TerminalTarget {
            workspace: "s1".into(),
            tab: "t1".into(),
            id,
            terminal: "terminal-1".into(),
        },
        run: format!("run-{name}"),
        name: name.into(),
        busy,
    };
    let scope = |end, terminals: Vec<CloseTerminal>, untracked: Vec<UntrackedRun>| CloseScope {
        generation: "g7".into(),
        pane: "p1".into(),
        end,
        session: "work".into(),
        terminals,
        untracked,
    };
    let metrics = CellMetrics::for_scale(1.0);
    let prompt = |scope: &CloseScope, stack, detach: Option<&str>, changed| {
        DialogScene::close_stack(
            scope,
            stack,
            detach.map(Into::into),
            changed,
            PhysicalSize::new(960, 600),
            metrics,
            0.0,
            &crate::CellText(metrics),
        )
    };
    let rows = |dialog: &DialogScene| {
        dialog.groups[0]
            .rows
            .iter()
            .map(|row| (row.shortcut.clone(), row.action.clone()))
            .collect::<Vec<_>>()
    };
    let row = |kind: &str, text: &str| (kind.to_owned(), text.to_owned());

    // A column close: every terminal in the answer's order, the busy ones marked.
    let column = scope(
        CloseEnd::Column,
        vec![
            terminal(TargetId::Pane("p1".into()), "p1  shell", false),
            terminal(TargetId::Pane("p2".into()), "p2  sleep", true),
            terminal(TargetId::Popup("u1".into()), "Files", false),
        ],
        Vec::new(),
    );
    let dialog = prompt(&column, Some(2), None, false);
    assert_eq!(dialog.kind.title(), "Close Stack 2?");
    assert_eq!(
        dialog.kind.subtitle(),
        "This stops 3 terminals; 1 still running."
    );
    assert_eq!(dialog.kind.footer(), ["Enter Close · Esc Cancel"]);
    assert_eq!(
        rows(&dialog),
        [
            row("Pane", "p1  shell"),
            row("Pane", "p2  sleep · running"),
            row("Popup", "Files"),
        ]
    );
    // The only Stack of a tab has no number; one idle terminal is singular and unmarked.
    let one = scope(
        CloseEnd::Column,
        vec![terminal(TargetId::Pane("p1".into()), "p1", false)],
        Vec::new(),
    );
    let dialog = prompt(&one, None, None, false);
    assert_eq!(dialog.kind.title(), "Close Stack?");
    assert_eq!(dialog.kind.subtitle(), "This stops 1 terminal.");
    assert_eq!(rows(&dialog), [row("Pane", "p1")]);
    // An answer that replaces a consented one says so first.
    assert_eq!(
        prompt(&one, None, None, true).kind.subtitle(),
        "This Stack changed. This stops 1 terminal."
    );

    // A final close that removes the session, with the detach hint.
    let mut last = column.clone();
    last.end = CloseEnd::Session;
    let dialog = prompt(&last, None, Some("Alt+Shift+D"), true);
    assert_eq!(dialog.kind.title(), "Close the last tab?");
    assert_eq!(
        dialog.kind.subtitle(),
        "This Stack changed. This closes session work and stops 3 terminals; 1 still running."
    );
    assert_eq!(
        dialog.kind.footer(),
        [
            "Enter Close · Esc Cancel",
            "Alt+Shift+D keeps it running instead"
        ]
    );
    // A final close that quits the window lists the runs no tab tracks, by terminal ID, and
    // counts each as a terminal and as running.
    let window = scope(
        CloseEnd::Window,
        vec![terminal(TargetId::Pane("p1".into()), "p1", false)],
        vec![UntrackedRun {
            terminal: "terminal-9".into(),
            run: "run-z".into(),
        }],
    );
    let dialog = prompt(&window, None, Some("Alt+Shift+D"), false);
    assert_eq!(dialog.kind.title(), "Close the last tab?");
    assert_eq!(
        dialog.kind.subtitle(),
        "This quits the window and stops 2 terminals; 1 still running."
    );
    assert_eq!(
        dialog.kind.footer(),
        [
            "Enter Quit · Esc Cancel",
            "Alt+Shift+D keeps it running instead"
        ]
    );
    assert_eq!(
        rows(&dialog),
        [row("Pane", "p1"), row("Terminal", "terminal-9 · in no tab"),]
    );
    // No run ID is shown anywhere.
    assert!(rows(&dialog).iter().all(|(_, text)| !text.contains("run-")));
}

#[test]
fn stack_bands_label_columns_yield_whole_and_clip_scrolled_headers() {
    use eon_workspace_protocol::v45::{Column, PopupGeometry};

    let column = |first: usize, count: usize| Column {
        panes: (first..first + count)
            .map(|n| Pane {
                label: format!("p{n}"),
                pinned_name: None,
                tag: None,
                id: format!("p{n}"),
                session: format!("s{n}"),
                endpoint: format!("/tmp/p{n}.sock").into_bytes(),
                live: true,
                attention: None,
            })
            .collect(),
        expanded: format!("p{first}"),
    };
    let snapshot = |columns: Vec<Column>| Snapshot {
        focus: None,
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v45::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
            selected_pane: Some(columns[0].expanded.clone()),
            selected_popup: None,
            columns,
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
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: Vec::new(),
    };
    let project = |snapshot: &Snapshot, height: u32, metrics: CellMetrics, scroll: f32| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(1600, height),
            metrics,
            0.0,
            scroll,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        )
    };
    let labels = |scene: &WorkspaceScene| {
        scene
            .column_cards()
            .map(|(_, label, _, _)| label.map(str::to_owned))
            .collect::<Vec<_>>()
    };
    for scale in [1.0, 1.3, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let band = chrome_title_metrics(metrics).text_height + metrics.padding / 3.0;
        // One column reads "Stack"; several are numbered left to right, the selected one first.
        let lone = project(&snapshot(vec![column(1, 1)]), 900, metrics, 0.0);
        assert_eq!(labels(&lone), [Some("Stack".to_owned())]);
        assert!((lone.header_viewport.top - lone.pane_viewport.top - band).abs() < 0.001);
        assert_eq!(lone.header_viewport.bottom(), lone.pane_viewport.bottom());
        assert_eq!(lone.terminal.bottom(), lone.pane_viewport.bottom());
        let mut columns = snapshot(vec![column(1, 2), column(3, 1), column(4, 3)]);
        columns.tabs[0].selected_pane = Some("p3".into());
        let three = project(&columns, 900, metrics, 0.0);
        assert_eq!(
            labels(&three),
            [
                Some("Stack 2".to_owned()),
                Some("Stack 1".to_owned()),
                Some("Stack 3".to_owned())
            ]
        );
        assert!(
            three
                .column_panes()
                .all(|(panes, _)| panes[0].rect.top == lone.panes[0].rect.top)
        );
        // A popup covers the columns with its own card and no Stack band.
        let mut popup = snapshot(vec![column(1, 1)]);
        popup.tabs[0].selected_popup = Some("u1".into());
        popup.tabs[0].popups = vec![eon_workspace_protocol::v45::Popup {
            id: "u1".into(),
            entry: "files".into(),
            session: "popup".into(),
            endpoint: b"/tmp/popup.sock".to_vec(),
            attention: None,
        }];
        popup.entries = vec![eon_workspace_protocol::v45::PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: eon_workspace_protocol::v45::PopupKey::Unbound,
        }];
        let covered = project(&popup, 900, metrics, 0.0);
        assert_eq!(labels(&covered), [None]);
        assert_eq!(covered.header_viewport, covered.pane_viewport);
        assert_eq!(covered.pane_viewport, lone.pane_viewport);
    }
    // On a short window the band yields whole, before the first header's padded cell.
    let metrics = CellMetrics::for_scale(1.0);
    let short = project(&snapshot(vec![column(1, 1)]), 112, metrics, 0.0);
    assert_eq!(labels(&short), [None]);
    assert_eq!(short.header_viewport, short.pane_viewport);
    assert!(short.terminal.height >= minimum_terminal(metrics, metrics.height));
    assert!(short.terminal.bottom() <= short.pane_viewport.bottom() + 0.001);
    // Scrolled headers pass under the band: neither drawn there nor hit there.
    let tall = snapshot(vec![column(1, 30)]);
    let scrolled = project(&tall, 400, metrics, 200.0);
    assert!(scrolled.pane_scroll() > 0.0);
    assert_eq!(labels(&scrolled), [Some("Stack".to_owned())]);
    assert!(
        scrolled
            .panes
            .iter()
            .any(|pane| pane.rect.bottom() <= scrolled.header_viewport.top),
        "a header scrolled under the Stack band"
    );
    let band_point = (
        scrolled.chrome.left + 20.0,
        scrolled.pane_viewport.top + 2.0,
    );
    assert!(!matches!(
        scrolled.hit_test(band_point.0, band_point.1),
        Some(WorkspaceHit::Pane(_))
    ));
    let fill = scrolled.card_fill(scrolled.chrome, metrics, 1.0);
    let outlines =
        scrolled.pane_outlines(&scrolled.panes, scrolled.terminal, fill, 1.0, metrics, 1.0);
    assert!(
        outlines.iter().all(|outline| outline.clip.height == 0.0
            || outline.clip.top >= scrolled.header_viewport.top)
    );
    // The Stack segment reaches down to the header viewport while its first pane is scrolled away.
    let stack = scrolled.stack_outline(&outlines[0], fill);
    assert_eq!(stack.clip.top, fill.bounds.top);
    assert_eq!(stack.clip.bottom(), scrolled.header_viewport.top);
    // Scrolled a few pixels past the expanded pane's header, terminals pass under the band too:
    // nothing in it takes a click, and the visible terminal starts below it.
    let mut two = snapshot(vec![column(1, 30), column(31, 30)]);
    two.tabs[0].selected_pane = Some("p1".into());
    let header = project(&two, 400, metrics, 0.0).panes[0].rect.height;
    let scrolled = project(&two, 400, metrics, header + 5.0);
    // Other columns reveal their expanded pane, so only the selected terminal reaches the band.
    assert!(scrolled.terminal.top < scrolled.header_viewport.top);
    let visible = scrolled.visible_terminal().unwrap();
    assert!(visible.top >= scrolled.header_viewport.top);
    for card in std::iter::once(scrolled.chrome)
        .chain(scrolled.other_columns.iter().map(|column| column.chrome))
    {
        let mut y = scrolled.pane_viewport.top;
        while y < scrolled.header_viewport.top {
            for x in [
                card.left + 1.0,
                card.left + card.width / 2.0,
                card.right() - 1.0,
            ] {
                assert_eq!(
                    scrolled.hit_test(x, y),
                    None,
                    "({x}, {y}) in the Stack band"
                );
            }
            y += 1.0;
        }
    }
    let shifted = scrolled.with_displayed_grids(
        metrics,
        std::iter::once((scrolled.terminal, scrolled.terminal)),
    );
    assert_eq!(shifted.visible_terminal(), Some(visible));
}
