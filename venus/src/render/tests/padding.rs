use super::*;

#[test]
fn padding_color_follows_scrolled_rows_and_orbit_preview_edges() {
    let metrics = CellMetrics::for_scale(1.0);
    let viewport = SceneRect {
        left: 10.0,
        top: 10.0,
        width: 100.0,
        height: 100.0,
    };
    let scene = padding_scene();
    let grid = scene_grid(&scene, viewport, metrics);
    for (direction, offset) in [
        (VerticalDirection::Up, metrics.height / 2.0),
        (VerticalDirection::Down, -metrics.height / 2.0),
    ] {
        let mut row = scene.content[0].clone();
        for cell in &mut row.cells {
            cell.style.background = SceneColor {
                r: 200,
                g: 100,
                b: 50,
            };
        }
        let preview = ScenePreview::Viewport {
            frame_revision: 1,
            direction,
            edge_reached: false,
            rows: vec![row.clone()],
        };
        let mut batch = RectangleBatch::new(200, 200);
        let top_colors = build_padding_rectangles(
            &mut batch,
            &scene,
            metrics,
            viewport,
            viewport,
            PaddingColor::Extend,
            (Some(&preview), offset),
        );
        assert!(
            top_colors.contains(
                &padding_pixel(&batch, grid.left + metrics.width + 1.0, viewport.top + 1.0)
                    .unwrap()
            ),
            "the header band's contrast check sees the scrolled top padding"
        );
        let (side_y, padding_y) = match direction {
            VerticalDirection::Up => (grid.top + 1.0, viewport.top + 1.0),
            VerticalDirection::Down => (grid.bottom() - 1.0, viewport.bottom() - 1.0),
        };
        assert_eq!(
            padding_pixel(&batch, viewport.left + 1.0, side_y),
            Some(row.cells[0].style.background)
        );
        assert_eq!(
            padding_pixel(&batch, grid.left + metrics.width + 1.0, padding_y),
            Some(row.cells[1].style.background)
        );
        let mut settled = RectangleBatch::new(200, 200);
        build_padding_rectangles(
            &mut settled,
            &scene,
            metrics,
            viewport,
            viewport,
            PaddingColor::Extend,
            (Some(&preview), 0.0),
        );
        let canonical = match direction {
            VerticalDirection::Up => 0,
            VerticalDirection::Down => 1,
        };
        assert_eq!(
            padding_pixel(&settled, grid.left + metrics.width + 1.0, padding_y),
            Some(scene.content[canonical].cells[1].style.background),
            "a settled preview stays outside the grid"
        );
    }
}

#[test]
fn padding_color_whole_scroll_uses_visible_row_color_and_exceptions() {
    let metrics = CellMetrics::for_scale(1.0);
    let viewport = SceneRect {
        left: 10.0,
        top: 10.0,
        width: 100.0,
        height: 100.0,
    };
    for (departed, visible) in [
        ("", ""),
        ("default", ""),
        ("\u{e0b0}", ""),
        ("", "default"),
        ("", "\u{e0b0}"),
    ] {
        let mut scene = padding_scene();
        for (index, reason) in [(0, departed), (1, visible)] {
            let cell = &mut scene.content[index].cells[0];
            if reason == "default" {
                cell.style = plain_style();
            } else {
                cell.text = reason.into();
            }
        }
        let preview = ScenePreview::Viewport {
            frame_revision: 1,
            direction: VerticalDirection::Down,
            edge_reached: false,
            rows: vec![scene.content[1].clone()],
        };
        let grid = scene_grid(&scene, viewport, metrics);
        let mut batch = RectangleBatch::new(200, 200);
        build_padding_rectangles(
            &mut batch,
            &scene,
            metrics,
            viewport,
            viewport,
            PaddingColor::Extend,
            (Some(&preview), -metrics.height),
        );
        assert_eq!(
            padding_pixel(&batch, grid.left + metrics.width + 1.0, viewport.top + 1.0),
            visible
                .is_empty()
                .then_some(scene.content[1].cells[1].style.background),
            "departed={departed:?}, visible={visible:?}"
        );
    }
}

#[test]
fn padding_color_stays_in_pane_and_popup_shapes_in_both_frame_modes() {
    use eon_workspace_protocol::v24::{
        ALT, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Sidebar, Snapshot,
        Tab,
    };
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v24::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p0-0".into()),
            selected_popup: None,
            columns: (0..2)
                .map(|col| Column {
                    panes: (0..2)
                        .map(|row| Pane {
                            label: format!("p{col}-{row}"),
                            pinned_name: None,
                            id: format!("p{col}-{row}"),
                            session: format!("s{col}-{row}"),
                            endpoint: format!("/tmp/{col}-{row}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        })
                        .collect(),
                    expanded: format!("p{col}-0"),
                })
                .collect(),
            popups: vec![Popup {
                id: "u1".into(),
                entry: "files".into(),
                session: "files".into(),
                endpoint: b"/tmp/files.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT,
                key: "KeyF".into(),
            }),
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: Sidebar::Closed,
    };
    for scale in [1.0, 1.3] {
        let metrics = CellMetrics::for_scale(scale);
        let stroke = physical_outline_width(3, metrics.scale);
        for popup in [false, true] {
            snapshot.tabs[0].selected_popup = popup.then(|| "u1".into());
            for frames in [false, true] {
                let scene = padding_scene();
                let peer = Scene {
                    background: SceneColor {
                        r: 70,
                        g: 80,
                        b: 90,
                    },
                    ..padding_scene()
                };
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(400, 300),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                );
                let columns = if popup { vec![] } else { vec![Some(&peer)] };
                let workspace = workspace_paint(&workspace, Some(&scene), &columns, metrics);
                let mut batch = RectangleBatch::new(400, 300);
                build_workspace_padding(
                    &mut batch,
                    &workspace,
                    Some(&scene),
                    &columns,
                    metrics,
                    PaddingColor::Extend,
                    frames,
                    stroke,
                    (None, 0.0),
                );
                let grid = scene_grid(&scene, workspace.terminal, metrics);
                let title_top = if popup {
                    workspace
                        .card_fill(workspace.chrome, metrics, stroke)
                        .bounds
                        .top
                } else {
                    workspace
                        .panes
                        .iter()
                        .find(|pane| pane.selected)
                        .unwrap()
                        .frame(workspace.terminal)
                        .top
                };
                let band_y = (title_top + workspace.terminal.top) / 2.0;
                let band_x = grid.left + metrics.width + 1.0;
                assert!(band_y > title_top + stroke && band_y < workspace.terminal.top);
                assert_eq!(
                    padding_pixel(&batch, band_x, band_y),
                    (frames && !popup).then_some(scene.content[0].cells[1].style.background),
                    "header band: popup={popup}, frames={frames}, scale={scale}"
                );
                assert_eq!(
                    padding_pixel(&batch, grid.left - 1.0, grid.top + 1.0),
                    Some(scene.content[0].cells[0].style.background)
                );
                assert_eq!(
                    padding_pixel(&batch, grid.left + metrics.width + 1.0, grid.bottom() + 1.0),
                    Some(scene.content[1].cells[1].style.background),
                    "displayed grid determines the bottom edge"
                );
                if frames || popup {
                    let outer = workspace.card_fill(workspace.chrome, metrics, stroke);
                    let pane = workspace
                        .pane_outlines(
                            &workspace.panes,
                            workspace.terminal,
                            outer,
                            stroke,
                            metrics,
                            stroke,
                        )
                        .into_iter()
                        .find(|outline| outline.expanded)
                        .map_or(outer, |outline| CardFill {
                            bounds: outline.bounds,
                            radius: outline.radius,
                        });
                    for vertex in batch.bytes.chunks_exact(VERTEX_SIZE as usize) {
                        let x = (f32::from_ne_bytes(vertex[..4].try_into().unwrap()) + 1.0) * 200.0;
                        let y =
                            (1.0 - f32::from_ne_bytes(vertex[4..8].try_into().unwrap())) * 150.0;
                        // Peer columns have their own card; inspect selected-card vertices here.
                        if !popup && x > workspace.chrome.right() + 0.001 {
                            continue;
                        }
                        for shape in [outer, pane] {
                            assert!(
                                shape.bands().any(|band| x >= band.left - 0.001
                                    && x <= band.right() + 0.001
                                    && y >= band.top - 0.001
                                    && y <= band.bottom() + 0.001),
                                "paint outside rounded shape: {x},{y}, popup={popup}, frames={frames}"
                            );
                        }
                    }
                    assert_eq!(
                        padding_pixel(&batch, outer.bounds.left + 0.1, outer.bounds.bottom() - 0.1),
                        None
                    );
                } else {
                    assert!(
                        padding_pixel(&batch, 0.5, grid.top + 1.0).is_some(),
                        "frameless padding reaches the window edge"
                    );
                    assert!(
                        padding_pixel(&batch, 399.5, grid.top + 1.0).is_some(),
                        "peer padding reaches the other window edge"
                    );
                }
                let mut disabled = RectangleBatch::new(400, 300);
                build_workspace_padding(
                    &mut disabled,
                    &workspace,
                    Some(&scene),
                    &columns,
                    metrics,
                    PaddingColor::Background,
                    frames,
                    stroke,
                    (None, 0.0),
                );
                assert!(disabled.bytes.is_empty());
                if frames && !popup {
                    for pane in workspace.panes.iter().filter(|pane| !pane.selected) {
                        assert_eq!(
                            padding_pixel(&batch, band_x, pane.rect.top + pane.rect.height * 0.75),
                            None,
                            "collapsed headers have no terminal padding"
                        );
                    }
                    for (screen, mode, reason, extends) in [
                        (Screen::Primary, PaddingColor::Extend, "default", false),
                        (Screen::Primary, PaddingColor::Extend, "\u{e0b0}", false),
                        (Screen::Primary, PaddingColor::ExtendAlways, "default", true),
                        (Screen::Alternate, PaddingColor::Extend, "default", true),
                    ] {
                        let mut edge = scene.clone();
                        edge.screen = screen;
                        if reason == "default" {
                            edge.content[0].cells[0].style = plain_style();
                        } else {
                            edge.content[0].cells[0].text = reason.into();
                        }
                        let mut changed = RectangleBatch::new(400, 300);
                        let bands = build_workspace_padding(
                            &mut changed,
                            &workspace,
                            Some(&edge),
                            &columns,
                            metrics,
                            mode,
                            true,
                            stroke,
                            (None, 0.0),
                        );
                        let expected = extends.then_some(edge.content[0].cells[1].style.background);
                        assert_eq!(
                            padding_pixel(&changed, band_x, band_y),
                            expected,
                            "{screen:?}, {mode:?}, {reason}"
                        );
                        assert_eq!(
                            padding_pixel(&changed, band_x, workspace.terminal.top + 1.0),
                            expected
                        );
                        assert_eq!(
                            bands[0].contains(&edge.content[0].cells[1].style.background),
                            extends,
                            "the label's contrast check sees the painted band"
                        );
                    }
                    let fill = workspace.card_fill(workspace.chrome, metrics, stroke);
                    let label_gap = SceneRect {
                        left: grid.left,
                        top: title_top,
                        width: metrics.width * 3.0,
                        height: stroke,
                    };
                    let rules = workspace
                        .pane_outlines(
                            &workspace.panes,
                            workspace.terminal,
                            fill,
                            stroke,
                            metrics,
                            stroke,
                        )
                        .into_iter()
                        .map(|outline| (outline, [label_gap, SceneRect::default()]))
                        .collect::<Vec<_>>();
                    let mut chrome = RectangleBatch::new(400, 300);
                    chrome.push_pane_outlines(
                        fill,
                        workspace.pane_viewport,
                        &rules,
                        Some(palette::CARD_OUTLINE),
                        stroke,
                        stroke,
                    );
                    let padding_end = batch.bytes.len();
                    batch.push_pane_outlines(
                        fill,
                        workspace.pane_viewport,
                        &rules,
                        Some(palette::CARD_OUTLINE),
                        stroke,
                        stroke,
                    );
                    assert_eq!(
                        &batch.bytes[padding_end..],
                        chrome.bytes.as_slice(),
                        "padding preserves title cutouts and selected outlines"
                    );
                    assert_eq!(
                        padding_pixel(&chrome, band_x, title_top + 0.5),
                        None,
                        "label cutout stays clear of the title stroke"
                    );
                    assert_eq!(
                        padding_pixel(
                            &chrome,
                            workspace.terminal.right() - metrics.padding,
                            title_top + 0.5
                        ),
                        Some(palette::CARD_OUTLINE)
                    );
                    let zoomed = WorkspaceScene::from_snapshot_with_zoom(
                        &snapshot,
                        crate::EonBar::default(),
                        PhysicalSize::new(400, 300),
                        metrics,
                        0.0,
                        0.0,
                        |_, text| (text.into(), text.len() as f32 * metrics.width),
                        None,
                        true,
                        crate::SidebarView::default(),
                    );
                    let mut zoomed_batch = RectangleBatch::new(400, 300);
                    build_workspace_padding(
                        &mut zoomed_batch,
                        &zoomed,
                        Some(&scene),
                        &[],
                        metrics,
                        PaddingColor::Extend,
                        true,
                        stroke,
                        (None, 0.0),
                    );
                    let zoom_title = zoomed.panes[0].frame(zoomed.terminal).top;
                    assert_eq!(
                        padding_pixel(
                            &zoomed_batch,
                            zoomed.terminal.left + metrics.padding + metrics.width + 1.0,
                            (zoom_title + zoomed.terminal.top) / 2.0
                        ),
                        None,
                        "zoom header paint stays unchanged"
                    );
                }
            }
        }
    }
}

#[test]
fn padding_color_extends_each_edge_and_corner() {
    let metrics = CellMetrics::for_scale(1.0);
    let scene = padding_scene();
    let viewport = SceneRect {
        left: 10.0,
        top: 10.0,
        width: 100.0,
        height: 100.0,
    };
    let grid = scene_grid(&scene, viewport, metrics);
    let mut batch = RectangleBatch::new(200, 200);
    build_padding_rectangles(
        &mut batch,
        &scene,
        metrics,
        viewport,
        viewport,
        PaddingColor::Extend,
        (None, 0.0),
    );
    for (row, y) in [
        (0, viewport.top + 1.0),
        (0, grid.top + 1.0),
        (1, grid.bottom() - 1.0),
        (1, viewport.bottom() - 1.0),
    ] {
        for (col, x) in [
            (0, viewport.left + 1.0),
            (0, grid.left + 1.0),
            (1, grid.left + metrics.width + 1.0),
            (2, grid.right() - 1.0),
            (2, viewport.right() - 1.0),
        ] {
            if grid.contains(x, y) {
                continue;
            }
            assert_eq!(
                padding_pixel(&batch, x, y),
                Some(scene.content[row].cells[col].style.background),
                "padding at {x},{y}"
            );
        }
    }
    assert_eq!(padding_pixel(&batch, grid.left + 1.0, grid.top + 1.0), None);
    assert_eq!(padding_pixel(&batch, viewport.left - 1.0, grid.top), None);
    assert_eq!(
        padding_pixel(&batch, viewport.right() + 1.0, grid.top),
        None
    );
}

#[test]
fn padding_color_declines_only_primary_vertical_edges() {
    let metrics = CellMetrics::for_scale(1.0);
    let viewport = SceneRect {
        left: 10.0,
        top: 10.0,
        width: 100.0,
        height: 100.0,
    };
    for reason in [
        "default", "matching", "\u{e0b0}", "\u{e0c8}", "\u{e0ca}", "\u{e0cc}", "\u{e0d2}",
        "\u{e0d4}", "\u{e0c9}", "\u{e0cb}", "\u{e0d3}", "█",
    ] {
        for screen in [Screen::Primary, Screen::Alternate] {
            for mode in [
                PaddingColor::Extend,
                PaddingColor::ExtendAlways,
                PaddingColor::Background,
            ] {
                let mut scene = padding_scene();
                scene.screen = screen;
                let cell = &mut scene.content[0].cells[1];
                match reason {
                    "default" => {
                        cell.style.background = scene.background;
                        cell.style.background_is_default = true;
                    }
                    "matching" => cell.style.background = scene.background,
                    text => cell.text = text.into(),
                }
                let mut batch = RectangleBatch::new(200, 200);
                build_padding_rectangles(
                    &mut batch,
                    &scene,
                    metrics,
                    viewport,
                    viewport,
                    mode,
                    (None, 0.0),
                );
                let declined = mode == PaddingColor::Extend
                    && screen == Screen::Primary
                    && !matches!(reason, "\u{e0c9}" | "\u{e0cb}" | "\u{e0d3}" | "█");
                let painted = mode != PaddingColor::Background;
                assert_eq!(
                    padding_pixel(&batch, 11.0, 11.0).is_some(),
                    painted && !declined,
                    "{reason}, {screen:?}, {mode:?}"
                );
                assert_eq!(
                    padding_pixel(&batch, 11.0, 109.0).is_some(),
                    painted,
                    "bottom is independent"
                );
                assert_eq!(
                    padding_pixel(&batch, 11.0, 30.0).is_some(),
                    painted,
                    "sides always extend"
                );
            }
        }
    }
    let mut scene = padding_scene();
    for cell in scene.content.iter_mut().flat_map(|row| &mut row.cells) {
        cell.style = plain_style();
    }
    for mode in [
        PaddingColor::Extend,
        PaddingColor::ExtendAlways,
        PaddingColor::Background,
    ] {
        let mut batch = RectangleBatch::new(200, 200);
        build_padding_rectangles(
            &mut batch,
            &scene,
            metrics,
            viewport,
            viewport,
            mode,
            (None, 0.0),
        );
        assert!(
            batch.bytes.is_empty(),
            "default background retains the backdrop's opacity"
        );
    }
}
