use super::*;
use crate::DrawCursor;
use orbit_protocol::CursorShape;

#[test]
fn tail_cursor_outlines_one_color_and_resets_when_hidden_or_blinking_off() {
    let style = plain_style();
    let mut scene = Scene {
        revision: 1,
        columns: 4,
        rows: 1,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: SceneColor::default(),
        cursor: Some(DrawCursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            column: 0,
            row: 0,
            at_wide_tail: false,
            color: SceneColor { r: 1, g: 2, b: 3 },
            explicit_color: false,
        }),
        content: vec![DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: [
                CellWidth::Narrow,
                CellWidth::Wide,
                CellWidth::SpacerTail,
                CellWidth::Narrow,
            ]
            .into_iter()
            .map(|width| DrawCell {
                width,
                text: String::new(),
                hyperlink: String::new(),
                style,
            })
            .collect(),
        }],
    };
    let viewport = SceneRect {
        left: 0.0,
        top: 0.0,
        width: 100.0,
        height: 100.0,
    };
    let metrics = CellMetrics::for_scale(1.0);
    let trail = SceneColor {
        r: 0x12,
        g: 0xab,
        b: 0xcf,
    };
    let outline = cursor_outline(trail);
    let mut animation = CursorAnimation::default();
    let mut first = RectangleBatch::new(100, 100);
    assert!(!build_tail_cursor(
        &mut first,
        &mut animation,
        &scene,
        true,
        metrics,
        viewport,
        trail,
        outline,
        1.0,
        0.0,
    ));
    assert_eq!(
        first.bytes.len(),
        VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize * 5
    );

    scene.cursor.as_mut().unwrap().column = 2;
    scene.cursor.as_mut().unwrap().at_wide_tail = true;
    let mut moving = RectangleBatch::new(100, 100);
    assert!(build_tail_cursor(
        &mut moving,
        &mut animation,
        &scene,
        true,
        metrics,
        viewport,
        trail,
        outline,
        1.0,
        0.0,
    ));
    assert_eq!(
        moving.bytes.len(),
        VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize * 7
    );
    let quad = VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize;
    let vertex_color = |vertex: &[u8]| {
        [8, 12, 16].map(|offset| f32::from_ne_bytes(vertex[offset..offset + 4].try_into().unwrap()))
    };
    for (bytes, color) in moving
        .bytes
        .chunks_exact(quad)
        .zip([outline, trail, trail, outline, outline, outline, outline])
    {
        let expected = [color.r, color.g, color.b].map(|value| f32::from(value) / 255.0);
        assert!(
            bytes
                .chunks_exact(VERTEX_SIZE as usize)
                .all(|vertex| vertex_color(vertex) == expected)
        );
    }
    scene.cursor.as_mut().unwrap().explicit_color = true;
    let mut explicit = RectangleBatch::new(100, 100);
    assert!(build_tail_cursor(
        &mut explicit,
        &mut animation,
        &scene,
        true,
        metrics,
        viewport,
        trail,
        outline,
        1.0,
        0.0,
    ));
    let expected = [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0];
    assert_eq!(explicit.bytes.len(), 3 * quad);
    assert!(
        explicit.bytes[explicit.bytes.len() - quad..]
            .chunks_exact(VERTEX_SIZE as usize)
            .all(|vertex| vertex_color(vertex) == expected)
    );
    scene.cursor.as_mut().unwrap().explicit_color = false;
    for (shape, colors) in [
        (CursorShape::Bar, &[outline, trail][..]),
        (CursorShape::Underline, &[outline, trail][..]),
        (
            CursorShape::Block,
            &[trail, outline, outline, outline, outline][..],
        ),
        (
            CursorShape::BlockHollow,
            &[
                outline, outline, outline, outline, trail, trail, trail, trail,
            ][..],
        ),
    ] {
        scene.cursor.as_mut().unwrap().shape = shape;
        let mut body = RectangleBatch::new(100, 100);
        assert!(!build_tail_cursor(
            &mut body,
            &mut CursorAnimation::default(),
            &scene,
            true,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.0,
        ));
        assert_eq!(body.bytes.len(), colors.len() * quad, "{shape:?}");
        for (bytes, color) in body.bytes.chunks_exact(quad).zip(colors) {
            let expected = [color.r, color.g, color.b].map(|value| f32::from(value) / 255.0);
            assert!(
                bytes
                    .chunks_exact(VERTEX_SIZE as usize)
                    .all(|vertex| vertex_color(vertex) == expected)
            );
        }
    }
    scene.cursor.as_mut().unwrap().shape = CursorShape::Block;
    let mut tick = RectangleBatch::new(100, 100);
    assert!(build_tail_cursor(
        &mut tick,
        &mut animation,
        &scene,
        true,
        metrics,
        viewport,
        trail,
        outline,
        1.0,
        0.016,
    ));

    scene.cursor.as_mut().unwrap().blinking = true;
    let mut hidden = RectangleBatch::new(100, 100);
    assert!(!build_tail_cursor(
        &mut hidden,
        &mut animation,
        &scene,
        false,
        metrics,
        viewport,
        trail,
        outline,
        1.0,
        0.01,
    ));
    assert!(hidden.bytes.is_empty());
    assert!(!animation.is_active());
}

#[test]
fn trail_quad_is_two_clipped_finite_triangles_in_stable_order() {
    let mut rectangles = RectangleBatch::new(100, 100);
    rectangles.clip = Some(SceneRect {
        left: 10.0,
        top: 20.0,
        width: 50.0,
        height: 40.0,
    });
    let points = [
        CursorPoint { x: -5.0, y: 10.0 },
        CursorPoint { x: 70.0, y: 15.0 },
        CursorPoint { x: 65.0, y: 70.0 },
        CursorPoint { x: 5.0, y: 65.0 },
    ];
    rectangles.push_quad(points, SceneColor { r: 1, g: 2, b: 3 }, 1.0);

    assert_eq!(
        rectangles.bytes.len(),
        VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
    );
    let positions = rectangles
        .bytes
        .chunks_exact(VERTEX_SIZE as usize)
        .map(|vertex| {
            (
                f32::from_ne_bytes(vertex[..4].try_into().unwrap()),
                f32::from_ne_bytes(vertex[4..8].try_into().unwrap()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        positions,
        [
            points[0], points[1], points[2], points[0], points[2], points[3]
        ]
        .map(|point| {
            let x = point.x.clamp(10.0, 60.0) / 100.0 * 2.0 - 1.0;
            let y = 1.0 - point.y.clamp(20.0, 60.0) / 100.0 * 2.0;
            (x, y)
        })
    );
    assert!(
        positions
            .iter()
            .all(|(x, y)| x.is_finite() && y.is_finite())
    );

    rectangles.push_quad(
        [CursorPoint {
            x: f32::NAN,
            y: 0.0,
        }; 4],
        SceneColor::default(),
        1.0,
    );
    assert_eq!(
        rectangles.bytes.len(),
        VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
    );
}

#[test]
fn rounded_outline_keeps_corners_and_interior_clear_when_clipped() {
    let bounds = SceneRect {
        left: 10.0,
        top: 20.0,
        width: 80.0,
        height: 60.0,
    };
    for radius in [0.0, 12.0, 100.0] {
        for clip in [
            bounds,
            SceneRect {
                top: 30.0,
                height: 40.0,
                ..bounds
            },
        ] {
            let mut batch = RectangleBatch::new(100, 100);
            batch.clip = Some(clip);
            batch.push_rounded_outline(bounds, radius, SceneColor::default());
            let quads: Vec<_> = batch
                .bytes
                .chunks_exact(VERTEX_SIZE as usize * 6)
                .map(|quad| {
                    let value = |vertex: usize, component: usize| {
                        let offset = vertex * VERTEX_SIZE as usize + component * 4;
                        f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                    };
                    let rect = SceneRect {
                        left: (value(0, 0) + 1.0) * 50.0,
                        top: (1.0 - value(0, 1)) * 50.0,
                        width: (value(2, 0) - value(0, 0)) * 50.0,
                        height: (value(0, 1) - value(2, 1)) * 50.0,
                    };
                    let alpha = value(0, 5);
                    assert!((0.0..=1.0).contains(&alpha));
                    assert!(rect.left >= clip.left - 0.001 && rect.right() <= clip.right() + 0.001);
                    assert!(rect.top >= clip.top - 0.001 && rect.bottom() <= clip.bottom() + 0.001);
                    (rect, alpha)
                })
                .collect();
            let painted = |x, y| {
                quads
                    .iter()
                    .any(|(rect, alpha)| *alpha > 0.0 && rect.contains(x, y))
            };
            assert!(!painted(50.0, 50.0), "outline must not fill the terminal");
            assert!(painted(10.5, 50.0), "side stroke is missing");
            if clip == bounds {
                assert!(painted(50.0, 20.5), "top stroke is missing");
                if radius > 1.0 {
                    assert!(!painted(10.5, 20.5), "corner is square");
                }
            } else {
                assert!(
                    !painted(50.0, 30.5),
                    "clipping must not create a new top border"
                );
            }
        }
    }
}

#[test]
fn selected_pane_outline_uses_scaled_widths_without_overlap_or_gaps() {
    use eon_workspace_protocol::v24::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
    let focus = SceneColor {
        r: 178,
        g: 7,
        b: 251,
    };
    let neutral = SceneColor {
        r: 86,
        g: 106,
        b: 126,
    };
    for ((border, focus_border), scale, [border_width, focus_width]) in [
        ((1, 1), 1.0, [1.0_f32, 1.0]),
        ((1, 1), 1.3, [1.0_f32, 1.0]),
        ((1, 1), 2.0, [2.0_f32, 2.0]),
        ((1, 2), 1.0, [1.0_f32, 2.0]),
        ((1, 2), 1.3, [1.0_f32, 3.0]),
        ((1, 2), 2.0, [2.0_f32, 4.0]),
        ((1, 3), 1.0, [1.0_f32, 3.0]),
        ((1, 3), 1.3, [1.0_f32, 4.0]),
        ((1, 3), 2.0, [2.0_f32, 6.0]),
        ((2, 1), 1.0, [2.0_f32, 1.0]),
        ((2, 1), 1.3, [3.0_f32, 1.0]),
        ((2, 1), 2.0, [4.0_f32, 2.0]),
        ((2, 2), 1.0, [2.0_f32, 2.0]),
        ((2, 2), 1.3, [3.0_f32, 3.0]),
        ((2, 2), 2.0, [4.0_f32, 4.0]),
        ((2, 3), 1.0, [2.0_f32, 3.0]),
        ((2, 3), 1.3, [3.0_f32, 4.0]),
        ((2, 3), 2.0, [4.0_f32, 6.0]),
        ((3, 1), 1.0, [3.0_f32, 1.0]),
        ((3, 1), 1.3, [4.0_f32, 1.0]),
        ((3, 1), 2.0, [6.0_f32, 2.0]),
        ((3, 2), 1.0, [3.0_f32, 2.0]),
        ((3, 2), 1.3, [4.0_f32, 3.0]),
        ((3, 2), 2.0, [6.0_f32, 4.0]),
        ((3, 3), 1.0, [3.0_f32, 3.0]),
        ((3, 3), 1.3, [4.0_f32, 4.0]),
        ((3, 3), 2.0, [6.0_f32, 6.0]),
    ] {
        let metrics = CellMetrics::for_scale(scale);
        for column_count in [2, 1] {
            for pane_count in [4, 1] {
                for selected in 0..pane_count {
                    for selected_column in 0..column_count {
                        let pane_id = |column, pane| format!("p{}", column * pane_count + pane + 1);
                        let expanded = |column| {
                            if column == 0 {
                                selected
                            } else {
                                pane_count - 1 - selected
                            }
                        };
                        let snapshot = Snapshot {
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
                                selected_pane: Some(pane_id(
                                    selected_column,
                                    expanded(selected_column),
                                )),
                                selected_popup: None,
                                columns: (0..column_count)
                                    .map(|column| Column {
                                        panes: (0..pane_count)
                                            .map(|pane| Pane {
                                                label: (pane_id(column, pane)).to_string(),
                                                pinned_name: None,
                                                id: pane_id(column, pane),
                                                session: format!("s{column}-{pane}"),
                                                endpoint: format!("/tmp/{column}-{pane}.sock")
                                                    .into_bytes(),
                                                live: true,
                                                attention: None,
                                            })
                                            .collect(),
                                        expanded: pane_id(column, expanded(column)),
                                    })
                                    .collect(),
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
                        };
                        let workspace = WorkspaceScene::from_snapshot(
                            &snapshot,
                            PhysicalSize::new(1600, 900),
                            metrics,
                            0.0,
                            0.0,
                            |_, text| (text.into(), text.len() as f32 * metrics.width),
                        );
                        for frames in [true, false] {
                            for (panes, current_column) in workspace.column_panes() {
                                let selected = panes.iter().position(|pane| pane.selected).unwrap();
                                let terminal = if current_column {
                                    workspace.terminal
                                } else {
                                    workspace
                                        .other_columns
                                        .iter()
                                        .find(|column| column.panes[0].id == panes[0].id)
                                        .unwrap()
                                        .terminal
                                };
                                let card = SceneRect {
                                    left: panes[0].rect.left,
                                    width: panes[0].rect.width,
                                    ..workspace.pane_viewport
                                };
                                let fill = workspace.card_fill(
                                    card,
                                    metrics,
                                    focus_width.max(border_width),
                                );
                                let bottom = (card.bottom()
                                    - (chrome_title_metrics(metrics).text_height / 2.0)
                                        .min(metrics.padding - focus_width.max(border_width)))
                                .floor();
                                assert_eq!(fill.bounds.bottom(), bottom);
                                if let Some(sidebar) = workspace.sidebar() {
                                    assert_eq!(
                                        workspace
                                            .card_rect(
                                                workspace.sidebar_area().unwrap(),
                                                metrics,
                                                focus_width.max(border_width)
                                            )
                                            .bottom(),
                                        bottom
                                    );
                                    assert_eq!(sidebar.bottom(), card.bottom());
                                }
                                if panes.last().unwrap().selected {
                                    assert_eq!(terminal.bottom(), card.bottom());
                                } else {
                                    let last = panes.last().unwrap();
                                    assert_eq!(
                                        crate::scene::title_rule_y(last.rect.top, last.rect.height),
                                        crate::scene::title_rule_y(
                                            card.bottom() - last.rect.height,
                                            last.rect.height
                                        )
                                    );
                                }
                                let rules = workspace
                                    .pane_outlines(
                                        panes,
                                        terminal,
                                        fill,
                                        border_width,
                                        metrics,
                                        focus_width.max(border_width),
                                    )
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, outline)| {
                                        let pane = &panes[index.min(panes.len() - 1)];
                                        let label = SceneRect {
                                            left: pane.rect.left + metrics.padding / 2.0,
                                            width: 80.0 * metrics.scale,
                                            ..pane.rect
                                        };
                                        let scrollback = SceneRect {
                                            left: card.right() - 60.0 * metrics.scale,
                                            width: 40.0 * metrics.scale,
                                            ..pane.rect
                                        };
                                        let gaps = if outline.title_y.is_some() {
                                            [label, scrollback]
                                        } else {
                                            [SceneRect::default(); 2]
                                        };
                                        (outline, gaps)
                                    })
                                    .collect::<Vec<_>>();
                                if frames && selected + 1 < panes.len() {
                                    let next = &panes[selected + 1];
                                    let next_line =
                                        crate::scene::title_rule_y(next.rect.top, next.rect.height);
                                    let pitch = next_line - rules[selected].0.bounds.bottom();
                                    if border <= 2 && focus_border <= 2 {
                                        assert!(
                                            (pitch - next.rect.height).abs() <= 1.0,
                                            "expanded bottom pitch: scale={scale}, widths={border}/{focus_border}, selected={selected}, pitch={pitch}, row={}",
                                            next.rect.height
                                        );
                                    }
                                    for rows in panes[selected + 1..].windows(2) {
                                        let pitch = crate::scene::title_rule_y(
                                            rows[1].rect.top,
                                            rows[1].rect.height,
                                        ) - crate::scene::title_rule_y(
                                            rows[0].rect.top,
                                            rows[0].rect.height,
                                        );
                                        assert!((pitch - next.rect.height).abs() <= 1.0);
                                    }
                                    if column_count == 1 && border == 1 && focus_border == 1 {
                                        println!(
                                            "scale={scale}, expanded={selected}: bottom={}, next={next_line}, pitch={pitch}, row={}",
                                            rules[selected].0.bounds.bottom(),
                                            next.rect.height
                                        );
                                    }
                                }
                                let mut batch = RectangleBatch::new(1600, 900);
                                batch.push_pane_outlines(
                                    fill,
                                    workspace.pane_viewport,
                                    &rules,
                                    frames.then_some(if current_column { focus } else { neutral }),
                                    physical_outline_width(border, metrics.scale),
                                    if current_column {
                                        physical_outline_width(focus_border, metrics.scale)
                                    } else {
                                        physical_outline_width(border, metrics.scale)
                                    },
                                );
                                let decode = |batch: &RectangleBatch| {
                                    let quads = batch
                                        .bytes
                                        .chunks_exact(VERTEX_SIZE as usize * 6)
                                        .map(|quad| {
                                            let value = |vertex: usize, component: usize| {
                                                let offset =
                                                    vertex * VERTEX_SIZE as usize + component * 4;
                                                f32::from_ne_bytes(
                                                    quad[offset..offset + 4].try_into().unwrap(),
                                                )
                                            };
                                            (
                                                SceneRect {
                                                    left: (value(0, 0) + 1.0) * 800.0,
                                                    top: (1.0 - value(0, 1)) * 450.0,
                                                    width: (value(2, 0) - value(0, 0)) * 800.0,
                                                    height: (value(0, 1) - value(2, 1)) * 450.0,
                                                },
                                                SceneColor {
                                                    r: (value(0, 2) * 255.0).round() as u8,
                                                    g: (value(0, 3) * 255.0).round() as u8,
                                                    b: (value(0, 4) * 255.0).round() as u8,
                                                },
                                                value(0, 5),
                                            )
                                        })
                                        .collect::<Vec<_>>();
                                    assert!(
                                        quads
                                            .iter()
                                            .all(|(_, _, alpha)| (0.0..=1.0).contains(alpha))
                                    );
                                    for (edge, _, coverage) in quads
                                        .iter()
                                        .filter(|(_, color, alpha)| *color == focus && *alpha > 0.0)
                                    {
                                        for (under, _, alpha) in
                                            quads.iter().filter(|(_, color, alpha)| {
                                                *color == neutral && *alpha > 0.0
                                            })
                                        {
                                            if let Some(overlap) = edge.intersection(*under) {
                                                assert!(
                                                    overlap.width < 0.001 || overlap.height < 0.001,
                                                    "neutral alpha {alpha} under focus coverage {coverage} at {overlap:?}: scale={scale}, panes={pane_count}, selected={selected}"
                                                );
                                            }
                                        }
                                    }
                                    quads
                                };
                                let color_at = |quads: &[(SceneRect, SceneColor, f32)], x, y| {
                                    let mut rgba = [0.0; 4];
                                    for (_, color, alpha) in
                                        quads.iter().filter(|(rect, _, _)| rect.contains(x, y))
                                    {
                                        for (target, source) in
                                            rgba[..3].iter_mut().zip([color.r, color.g, color.b])
                                        {
                                            *target =
                                                f32::from(source) * alpha + *target * (1.0 - alpha);
                                        }
                                        rgba[3] = alpha + rgba[3] * (1.0 - alpha);
                                    }
                                    (rgba[3] > 0.0).then(|| SceneColor {
                                        r: (rgba[0] / rgba[3]).round() as u8,
                                        g: (rgba[1] / rgba[3]).round() as u8,
                                        b: (rgba[2] / rgba[3]).round() as u8,
                                    })
                                };
                                let quads = decode(&batch);
                                let outer = fill
                                    .bands()
                                    .map(|band| SceneRect {
                                        left: band.left.floor().max(fill.bounds.left),
                                        width: band.right().ceil().min(fill.bounds.right())
                                            - band.left.floor().max(fill.bounds.left),
                                        ..band
                                    })
                                    .collect::<Vec<_>>();
                                for (index, (rect, _, _)) in quads.iter().enumerate() {
                                    assert!(
                                        uncovered(*rect, &outer)
                                            .iter()
                                            .all(|outside| outside.width < 0.001
                                                || outside.height < 0.001),
                                        "outline escaped the column outer edge: {rect:?}, scale={scale}, selected={selected}"
                                    );
                                    for (other, _, _) in &quads[..index] {
                                        if let Some(overlap) = rect.intersection(*other) {
                                            assert!(
                                                overlap.width < 0.001 || overlap.height < 0.001,
                                                "stack strokes must paint once: {overlap:?}, scale={scale}, selected={selected}"
                                            );
                                        }
                                    }
                                }
                                for y in ((fill.bounds.top + workspace.card_radius()).ceil() as u32
                                    ..(fill.bounds.bottom() - workspace.card_radius()).floor()
                                        as u32)
                                    .map(|y| y as f32 + 0.5)
                                {
                                    for x in [card.left + 0.5, card.right() - 0.5] {
                                        assert_eq!(
                                            color_at(&quads, x, y).is_some(),
                                            frames,
                                            "column sides must be continuous: scale={scale}, selected={selected}, x={x}, y={y}"
                                        );
                                    }
                                }
                                let last = panes.last().unwrap();
                                let bottom_width = if last.selected && current_column {
                                    focus_width
                                } else {
                                    border_width
                                };
                                let bottom_color = if last.selected && current_column {
                                    focus
                                } else {
                                    neutral
                                };
                                for y in ((bottom - bottom_width - 2.0) as u32
                                    ..card.bottom().ceil() as u32)
                                    .map(|y| y as f32 + 0.5)
                                {
                                    assert_eq!(
                                        color_at(&quads, card.left + card.width / 2.0, y),
                                        (frames && y >= bottom - bottom_width && y < bottom)
                                            .then_some(bottom_color),
                                        "the bottom closes with exactly one stroke: scale={scale}, selected={selected}, y={y}"
                                    );
                                }
                                for (index, pane) in panes.iter().enumerate() {
                                    let frame = if pane.selected {
                                        rules[index].0.bounds
                                    } else {
                                        pane.frame(terminal)
                                    };
                                    let rule_y = rules[index].0.title_y.unwrap();
                                    let width = if pane.selected && current_column {
                                        focus_width
                                    } else {
                                        border_width
                                    };
                                    let color = if pane.selected && current_column {
                                        focus
                                    } else {
                                        neutral
                                    };
                                    for offset in (0..=border_width.max(focus_width) as u32)
                                        .map(|n| n as f32 + 0.5)
                                    {
                                        let expected = (frames && offset < width).then_some(color);
                                        assert_eq!(
                                            color_at(
                                                &quads,
                                                frame.left + frame.width / 2.0,
                                                rule_y + offset
                                            ),
                                            expected,
                                            "top: scale={scale}, panes={pane_count}, index={index}, selected={selected}, column={current_column}, offset={offset}"
                                        );
                                        if pane.selected {
                                            for (x, y) in [
                                                (
                                                    frame.left + offset,
                                                    frame.top + frame.height / 2.0,
                                                ),
                                                (
                                                    frame.right() - offset,
                                                    frame.top + frame.height / 2.0,
                                                ),
                                                (
                                                    frame.left + frame.width / 2.0,
                                                    frame.bottom() - offset,
                                                ),
                                            ] {
                                                assert_eq!(
                                                    color_at(&quads, x, y),
                                                    expected,
                                                    "expanded side/bottom: scale={scale}, selected={selected}, column={current_column}, offset={offset}"
                                                );
                                            }
                                            assert_ne!(
                                                color_at(
                                                    &quads,
                                                    frame.right() - 0.5,
                                                    frame.bottom() - 0.5
                                                ),
                                                Some(focus),
                                                "expanded bottom corner must stay rounded"
                                            );
                                        } else {
                                            if index > selected {
                                                assert_eq!(
                                                    color_at(
                                                        &quads,
                                                        frame.left + offset,
                                                        frame.top - workspace.card_radius() - 0.5
                                                    ),
                                                    (frames && offset < width).then_some(neutral),
                                                    "lower collapsed sides must extend up toward the expanded pane"
                                                );
                                            }
                                            if index < selected {
                                                assert_eq!(
                                                    color_at(
                                                        &quads,
                                                        frame.left + offset,
                                                        frame.top + workspace.card_radius() + 0.5
                                                    ),
                                                    (frames && offset < width).then_some(neutral),
                                                    "collapsed sides must continue toward the expanded pane"
                                                );
                                            }
                                        }
                                    }
                                    if frames && !pane.selected {
                                        let radius = workspace.card_radius();
                                        let arc = SceneRect {
                                            left: frame.left + width + 0.01,
                                            top: if index < selected {
                                                frame.top + width
                                            } else {
                                                rule_y + width - radius
                                            },
                                            width: radius - width - 0.01,
                                            height: radius - width,
                                        };
                                        assert!(
                                            quads.iter().any(|(rect, color, _)| *color == neutral
                                                && rect.intersection(arc).is_some()),
                                            "collapsed corners must turn toward the expanded pane: scale={scale}, selected={selected}, index={index}"
                                        );
                                    }
                                    for gap in rules[index].1 {
                                        assert_eq!(
                                            color_at(
                                                &quads,
                                                gap.left + gap.width / 2.0,
                                                rule_y + 0.5
                                            ),
                                            None,
                                            "frame crosses title/scrollback gap"
                                        );
                                    }
                                }
                                if frames && current_column {
                                    for delta in [-workspace.card_radius(), -1.0, 1.0] {
                                        let scroll =
                                            rules[selected].0.bounds.top - card.top - delta;
                                        let mut scrolled_panes = panes.to_vec();
                                        for pane in &mut scrolled_panes {
                                            pane.rect.top -= scroll;
                                        }
                                        let scrolled_terminal = SceneRect {
                                            top: terminal.top - scroll,
                                            ..terminal
                                        };
                                        let scrolled = workspace
                                            .pane_outlines(
                                                &scrolled_panes,
                                                scrolled_terminal,
                                                fill,
                                                border_width,
                                                metrics,
                                                focus_width.max(border_width),
                                            )
                                            .into_iter()
                                            .zip(rules.iter().map(|(_, gaps)| *gaps))
                                            .collect::<Vec<_>>();
                                        let mut batch = RectangleBatch::new(1600, 900);
                                        batch.push_pane_outlines(
                                            fill,
                                            workspace.pane_viewport,
                                            &scrolled,
                                            Some(focus),
                                            border_width,
                                            focus_width,
                                        );
                                        let quads = decode(&batch);
                                        assert!(
                                            quads
                                                .iter()
                                                .all(|(rect, _, _)| rect.top >= card.top - 0.001
                                                    && rect.bottom() <= card.bottom() + 0.001),
                                            "scrolled frames escaped the allocation"
                                        );
                                        let frame = scrolled[selected].0.bounds;
                                        assert_eq!(
                                            color_at(
                                                &quads,
                                                frame.left + frame.width / 2.0,
                                                card.top + delta.max(0.0) + 0.5
                                            ),
                                            (delta >= 0.0 || delta + focus_width > 0.5)
                                                .then_some(focus),
                                            "scrolling must clip the original top, not round/pin it at the viewport"
                                        );
                                        assert_eq!(
                                            color_at(
                                                &quads,
                                                frame.left + 0.5,
                                                frame.top.max(card.top)
                                                    + workspace.card_radius()
                                                    + 0.5
                                            ),
                                            Some(focus),
                                            "clipped selected sides remain visible"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn retained_resize_keeps_strokes_clear_of_displayed_rows() {
    use eon_workspace_protocol::v24::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
    let metrics = CellMetrics::for_scale(1.0);
    for (pane_count, expanded) in [(4, 1), (4, 4), (3, 2), (1, 1)] {
        let snapshot = Snapshot {
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
                selected_pane: Some(format!("p{expanded}")),
                selected_popup: None,
                columns: vec![Column {
                    panes: (1..=pane_count)
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
                    expanded: format!("p{expanded}"),
                }],
                popups: vec![],
                attention_rollup: None,
            }],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![],
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
        };
        let workspace = |height, zoom| {
            WorkspaceScene::from_snapshot_with_zoom(
                &snapshot,
                crate::EonBar::default(),
                PhysicalSize::new(1280, height),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
                None,
                zoom,
                crate::SidebarView::default(),
            )
        };
        for (before_zoom, after_zoom) in [(false, false), (false, true), (true, false)] {
            let old = workspace(740, before_zoom);
            let current = workspace(720, after_zoom);
            assert_eq!(current.pane_viewport.top, 46.0);
            assert_eq!(current.panes[0].rect.top, current.pane_viewport.top);
            let shared_top = current.card_rect(current.chrome, metrics, 1.0).top;
            assert_eq!(shared_top, 56.0);
            assert_eq!(current.sidebar().unwrap().top, shared_top);
            let size = metrics.surface_size(old.terminal.screen_size()).unwrap();
            let retained = Scene {
                revision: 1,
                columns: size.cols,
                rows: size.rows,
                screen: Screen::Primary,
                title: String::new(),
                working_directory: String::new(),
                graphics: SceneGraphics::default(),
                background: palette::TERMINAL_FALLBACK,
                foreground: palette::CARD_OUTLINE,
                cursor: None,
                content: vec![],
            };
            let displayed = scene_grid(&retained, current.terminal, metrics)
                .intersection(current.terminal)
                .unwrap()
                .intersection(current.pane_viewport)
                .unwrap();
            if pane_count == 4 && expanded == 1 && !before_zoom && !after_zoom {
                assert_eq!(retained.rows, 32);
                assert_eq!(displayed.bottom() - metrics.height, 634.0);
                assert_eq!(displayed.bottom(), 652.0);
            }
            let paint = workspace_paint(&current, Some(&retained), &[], metrics);
            let cards = card_backdrops(&paint, Some(&retained), &[], metrics, 1.0);
            let fill = paint.card_fill(paint.chrome, metrics, 1.0);
            assert!(
                cards
                    .iter()
                    .all(|(card, _)| card.bounds.bottom() == fill.bounds.bottom())
            );
            let rules = paint
                .pane_outlines(&paint.panes, paint.terminal, fill, 1.0, metrics, 1.0)
                .into_iter()
                .map(|rule| (rule, [SceneRect::default(); 2]))
                .collect::<Vec<_>>();
            let (clip, _) =
                terminal_paint_clip(&paint, paint.chrome, paint.terminal, metrics, true, 1.0)
                    .unwrap();
            assert_eq!(displayed.intersection(clip), Some(displayed));
            assert_eq!(paint.terminal, current.terminal);
            assert_eq!(paint.panes, current.panes);
            let mut resized = retained.clone();
            resized.rows = metrics
                .surface_size(current.terminal.screen_size())
                .unwrap()
                .rows;
            let settled = workspace_paint(&current, Some(&resized), &[], metrics);
            assert!(matches!(settled, Cow::Borrowed(_)));
            if pane_count == 4 && expanded == 1 && !before_zoom && !after_zoom {
                assert_eq!(rules[0].0.bounds.bottom(), 653.0);
                let settled_fill = settled.card_fill(settled.chrome, metrics, 1.0);
                assert_eq!(
                    settled.pane_outlines(
                        &settled.panes,
                        settled.terminal,
                        settled_fill,
                        1.0,
                        metrics,
                        1.0
                    )[0]
                    .bounds
                    .bottom(),
                    642.0
                );
            }
            let selected = paint.panes.iter().position(|pane| pane.selected).unwrap();
            println!(
                "retained resize: zoom={before_zoom}->{after_zoom}, panes={pane_count}, expanded={expanded}, rows={}, grid bottom={}, shared line={}, frame bottom={}",
                retained.rows,
                displayed.bottom(),
                fill.bounds.bottom(),
                rules[selected].0.bounds.bottom()
            );
            let mut batch = RectangleBatch::new(1280, 720);
            batch.push_pane_outlines(
                fill,
                paint.pane_viewport,
                &rules,
                Some(palette::CARD_OUTLINE),
                1.0,
                1.0,
            );
            for rect in painted_rects(&batch) {
                assert!(
                    displayed
                        .intersection(rect)
                        .is_none_or(|overlap| overlap.width < 0.001 || overlap.height < 0.001),
                    "retained rows under stroke: panes={pane_count}, expanded={expanded}, displayed={displayed:?}, stroke={rect:?}"
                );
            }
        }
        if pane_count != 1 {
            continue;
        }
        // A popup's retained rows lower its card in both frame modes until the resize lands.
        let mut snapshot = snapshot.clone();
        snapshot.entries = vec![eon_workspace_protocol::v24::PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: eon_workspace_protocol::v24::PopupKey::Bound(
                eon_workspace_protocol::v24::Shortcut {
                    modifiers: eon_workspace_protocol::v24::ALT,
                    key: "KeyA".into(),
                },
            ),
        }];
        snapshot.tabs[0].popups = vec![eon_workspace_protocol::v24::Popup {
            id: "u1".into(),
            entry: "agent".into(),
            session: "s-popup".into(),
            endpoint: b"/tmp/popup.sock".to_vec(),
            attention: None,
        }];
        snapshot.tabs[0].selected_popup = Some("u1".into());
        let popup = |height| {
            WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new(1280, height),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
            )
        };
        let old = popup(740);
        let current = popup(720);
        let size = metrics.surface_size(old.terminal.screen_size()).unwrap();
        let retained = Scene {
            revision: 1,
            columns: size.cols,
            rows: size.rows,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: palette::TERMINAL_FALLBACK,
            foreground: palette::CARD_OUTLINE,
            cursor: None,
            content: vec![],
        };
        let displayed = scene_grid(&retained, current.terminal, metrics)
            .intersection(current.terminal)
            .unwrap()
            .intersection(current.pane_viewport)
            .unwrap();
        assert!(
            displayed.bottom()
                > current
                    .card_fill(current.chrome, metrics, 1.0)
                    .bounds
                    .bottom()
        );
        let paint = workspace_paint(&current, Some(&retained), &[], metrics);
        let fill = paint.card_fill(paint.chrome, metrics, 1.0);
        assert!(fill.bounds.bottom() >= displayed.bottom());
        assert_eq!(
            paint
                .card_rect(paint.sidebar_area().unwrap(), metrics, 1.0)
                .bottom(),
            fill.bounds.bottom()
        );
        for pane_frames in [true, false] {
            let (clip, card) = terminal_paint_clip(
                &paint,
                paint.chrome,
                paint.terminal,
                metrics,
                pane_frames,
                1.0,
            )
            .unwrap();
            assert!(card.is_some());
            assert_eq!(
                displayed.intersection(clip),
                Some(displayed),
                "popup rows clipped with pane_frames={pane_frames}"
            );
        }
        let mut resized = retained.clone();
        resized.rows = metrics
            .surface_size(current.terminal.screen_size())
            .unwrap()
            .rows;
        assert!(matches!(
            workspace_paint(&current, Some(&resized), &[], metrics),
            Cow::Borrowed(_)
        ));
    }
}

#[test]
fn shared_bottom_preserves_grid_and_bounds_short_scene_paint() {
    use eon_workspace_protocol::v24::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
    let mut fonts = glyphon::FontSystem::new();
    let mut swash = SwashCache::new();
    for (scale, font_size, line_height, focus_border, hints_visible) in [
        (1.4, 16.0, 1.125, 2, true),
        (1.0, 96.0, 1.125, 2, false),
        (1.0, 16.0, 1.125, 1, false),
        (1.0, 16.0, 1.125, 2, false),
        (1.3, 16.0, 1.125, 2, false),
        (2.0, 16.0, 1.125, 2, false),
        (1.0, 16.0, 3.0, 2, false),
        (1.0, 16.0, 1.125, 3, false),
    ] {
        let mut metrics = CellMetrics::for_scale(scale);
        metrics.font_size = font_size * scale as f32;
        metrics.width *= font_size / 16.0;
        metrics.height = (metrics.font_size * line_height).round();
        let width = (3000.0 * font_size / 16.0) as u32;
        let neutral = physical_outline_width(1, metrics.scale);
        let stroke = physical_outline_width(focus_border, metrics.scale);
        let heights: &[u32] = &[1, 50, 74, 105, 112, 140, 150, 160, 220, 888, 900];
        for &height in heights {
            for pane_scroll in [0.0, f32::MAX] {
                for column_count in [1, 2] {
                    for (pane_count, expanded) in [(1, 1), (4, 1), (4, 2), (4, 4)] {
                        let snapshot = Snapshot {
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
                                selected_pane: Some(format!("p{expanded}")),
                                selected_popup: None,
                                columns: (0..column_count)
                                    .map(|column| {
                                        let prefix = if column == 0 { "p" } else { "q" };
                                        Column {
                                            panes: (1..=if column == 0 { pane_count } else { 1 })
                                                .map(|n| Pane {
                                                    label: format!("{prefix}{n}"),
                                                    pinned_name: None,
                                                    id: format!("{prefix}{n}"),
                                                    session: format!("{prefix}-s{n}"),
                                                    endpoint: format!("/tmp/{prefix}{n}.sock")
                                                        .into_bytes(),
                                                    live: true,
                                                    attention: None,
                                                })
                                                .collect(),
                                            expanded: format!(
                                                "{prefix}{}",
                                                if column == 0 { expanded } else { 1 }
                                            ),
                                        }
                                    })
                                    .collect(),
                                popups: vec![],
                                attention_rollup: None,
                            }],
                            geometry: PopupGeometry {
                                side_margin: 8.0,
                                vertical_margin: 4.0,
                            },
                            entries: vec![],
                            codex_quota: None,
                            claude_quota: None,
                            sidebar: Sidebar::Open,
                        };
                        let workspace_at = |height| {
                            let size = PhysicalSize::new(width, height);
                            WorkspaceScene::from_snapshot_with_bar(
                                &snapshot,
                                crate::EonBar::default(),
                                size,
                                metrics,
                                0.0,
                                pane_scroll,
                                |_, text| (text.into(), text.len() as f32 * metrics.width),
                                hints_visible.then(|| {
                                    crate::ShortcutHintsScene::new(
                                        Vec::new(),
                                        size,
                                        metrics,
                                        WorkspaceFocus::Terminal,
                                        |_| 0.0,
                                    )
                                }),
                            )
                        };
                        let workspace = workspace_at(height);
                        let fill = workspace.card_fill(workspace.chrome, metrics, stroke);
                        let terminals = std::iter::once(workspace.terminal)
                            .chain(workspace.other_columns.iter().map(|column| column.terminal));
                        let grid_bottoms = terminals
                            .filter(|terminal| {
                                terminal.intersection(workspace.pane_viewport).is_some()
                            })
                            .filter_map(|terminal| {
                                let size = metrics.surface_size(terminal.screen_size())?;
                                Some(metrics.grid_rect(terminal, size.cols, size.rows).bottom())
                            })
                            .collect::<Vec<_>>();
                        let expected = (workspace.chrome.bottom()
                            - (chrome_title_metrics(metrics).text_height / 2.0)
                                .min(metrics.padding - stroke))
                        .floor();
                        let expected = grid_bottoms
                            .iter()
                            .fold(expected, |line, grid| line.max(grid.ceil() + stroke))
                            .min(workspace.chrome.bottom());
                        assert!(
                            fill.bounds.top >= workspace.chrome.top
                                && fill.bounds.bottom() <= workspace.chrome.bottom(),
                            "card outside allocation: scale={scale}, height={height}, font={font_size}, card={:?}, allocation={:?}",
                            fill.bounds,
                            workspace.chrome
                        );
                        if fill.bounds.height > 0.0 {
                            assert_eq!(fill.bounds.bottom(), expected);
                        }
                        let columns = vec![None; workspace.other_columns.len()];
                        let cards = card_backdrops(&workspace, None, &columns, metrics, stroke);
                        assert_eq!(
                            cards.len(),
                            column_count + usize::from(workspace.sidebar().is_some())
                        );
                        for (card, _) in &cards {
                            assert_eq!(
                                card.bounds.bottom(),
                                fill.bounds.bottom(),
                                "columns and sidebar must share one line"
                            );
                        }
                        let mut raw_grids = Vec::new();
                        for (terminal, allocated) in
                            std::iter::once((workspace.terminal, workspace.chrome)).chain(
                                workspace
                                    .other_columns
                                    .iter()
                                    .map(|column| (column.terminal, column.chrome)),
                            )
                        {
                            let Some(size) = metrics.surface_size(terminal.screen_size()) else {
                                continue;
                            };
                            let grid = metrics.grid_rect(terminal, size.cols, size.rows);
                            raw_grids.push(grid);
                            let Some((clip, _)) = terminal_paint_clip(
                                &workspace, allocated, terminal, metrics, true, stroke,
                            ) else {
                                continue;
                            };
                            if let Some(visible) =
                                grid.intersection(workspace.pane_viewport).and_then(|grid| {
                                    grid.intersection(
                                        workspace.card_fill(allocated, metrics, stroke).bounds,
                                    )
                                })
                            {
                                assert_eq!(
                                    visible.intersection(clip),
                                    Some(visible),
                                    "terminal clip removes visible grid pixels: scale={scale}, height={height}, expanded={expanded}, grid={grid:?}, clip={clip:?}"
                                );
                            }
                        }
                        if height >= 800 {
                            let size = metrics
                                .surface_size(workspace.terminal.screen_size())
                                .unwrap();
                            let grid_bottom = metrics
                                .grid_rect(workspace.terminal, size.cols, size.rows)
                                .bottom();
                            for grid in &grid_bottoms {
                                assert!(
                                    *grid <= expected - stroke,
                                    "grid covered: scale={scale}, line_height={line_height}, stroke={stroke}, grid={grid}, line={expected}"
                                );
                            }
                            if column_count == 1
                                && (expanded == 4 || hints_visible && pane_count == 1)
                            {
                                assert_eq!(workspace.terminal.bottom(), workspace.chrome.bottom());
                                println!(
                                    "scale={scale}, font_size={font_size}, line_height={line_height}, stroke={stroke}: allocated={}, line={expected}, grid={grid_bottom}, clearance={}",
                                    workspace.chrome.bottom(),
                                    expected - grid_bottom
                                );
                            }
                        } else if height == 112
                            && scale == 1.0
                            && font_size == 16.0
                            && line_height == 1.125
                        {
                            assert_eq!(workspace.terminal.height, 42.0);
                            assert_eq!(((workspace.terminal.height - 24.0) / 18.0).floor(), 1.0);
                        }
                        let rules = workspace
                            .pane_outlines(
                                &workspace.panes,
                                workspace.terminal,
                                fill,
                                neutral,
                                metrics,
                                stroke,
                            )
                            .into_iter()
                            .map(|rule| (rule, [SceneRect::default(); 2]))
                            .collect::<Vec<_>>();
                        if expanded < pane_count
                            && column_count == 1
                            && pane_scroll == 0.0
                            && height == 900
                        {
                            let next = &workspace.panes[expanded];
                            let next_line =
                                crate::scene::title_rule_y(next.rect.top, next.rect.height);
                            let edge = rules[expanded - 1].0.bounds.bottom();
                            println!(
                                "raised edge: scale={scale}, font={font_size}, line_height={line_height}, stroke={stroke}, expanded={expanded}, terminal bottom={}, edge={edge}, next={next_line}, pitch={}, row={}",
                                workspace.terminal.bottom(),
                                next_line - edge,
                                next.rect.height
                            );
                        }
                        if height == 112
                            && scale == 1.0
                            && font_size == 16.0
                            && line_height == 1.125
                            && expanded == 2
                            && column_count == 1
                            && pane_scroll == 0.0
                        {
                            assert_eq!(workspace.pane_viewport.bottom(), 104.0);
                            assert_eq!(raw_grids[0].top, 94.0);
                            assert_eq!(raw_grids[0].bottom(), 112.0);
                            assert!(rules[1].0.bounds.bottom() >= 112.0 + stroke);
                            println!(
                                "short scrolled viewport: stroke={stroke}, raw grid=94..112, shared line={expected}, frame bottom={}",
                                rules[1].0.bounds.bottom()
                            );
                        }
                        if expanded < pane_count
                            && let Some(pane) = workspace.panes.last()
                            && let Some(clip) = pane.rect.intersection(workspace.pane_viewport)
                        {
                            let title = chrome_title_metrics(metrics);
                            let edge = rules.last().unwrap().0.bounds.bottom();
                            let row = chrome::bottom_title_row(pane.rect, edge, title.text_height);
                            if fill.bounds.height > 0.0
                                && (pane.rect.bottom() - workspace.pane_viewport.bottom()).abs()
                                    < 0.001
                            {
                                assert!(
                                    (edge - expected).abs() < 0.001,
                                    "last edge must meet the shared line: edge={edge}, line={expected}"
                                );
                            }
                            let top = row.top + (row.height - title.text_height) / 2.0;
                            assert!(
                                top + title.text_height <= pane.rect.bottom() + 0.001,
                                "label box outside band: font={font_size}, height={height}, scroll={pane_scroll}, top={top}, text_height={}, band={:?}",
                                title.text_height,
                                pane.rect
                            );
                            if pane.rect.height >= title.text_height
                                && clip.height == pane.rect.height
                            {
                                let mut buffer = Buffer::new(
                                    &mut fonts,
                                    Metrics::new(title.font_size, title.line_height),
                                );
                                buffer.set_size(Some(row.width), Some(title.text_height));
                                buffer.set_wrap(Wrap::None);
                                buffer.set_text(
                                    pane.label(),
                                    &Attrs::new().family(Family::SansSerif),
                                    shaping(pane.label()),
                                    None,
                                );
                                buffer.shape_until_scroll(&mut fonts, false);
                                let line = buffer.layout_runs().next().unwrap();
                                let mut glyph_count = 0;
                                for glyph in line.glyphs {
                                    let physical = glyph.physical((row.left, top), 1.0);
                                    let Some(ink) = swash.get_image(&mut fonts, physical.cache_key)
                                    else {
                                        continue;
                                    };
                                    let ink_top =
                                        line.line_y.round() as i32 + physical.y - ink.placement.top;
                                    assert!(
                                        ink_top >= clip.top.floor() as i32
                                            && ink_top + ink.placement.height as i32
                                                <= clip.bottom().ceil() as i32,
                                        "last collapsed label glyph outside band: font_size={font_size}, glyph={}, top={ink_top}, height={}, clip={clip:?}",
                                        &line.text[glyph.start..glyph.end],
                                        ink.placement.height
                                    );
                                    glyph_count += 1;
                                }
                                assert!(
                                    glyph_count > 0,
                                    "the regression must exercise rasterized glyphs"
                                );
                            }
                            assert_eq!(
                                row.top,
                                (edge - row.height / 2.0).min(pane.rect.top).min(
                                    pane.rect.bottom() - (row.height + title.text_height) / 2.0
                                )
                            );
                            println!(
                                "label: scale={scale}, font_size={font_size}, line_height={line_height}, stroke={stroke}: box={top}..{}, band={}..{}",
                                top + title.text_height,
                                pane.rect.top,
                                pane.rect.bottom()
                            );
                        }
                        let mut batch = RectangleBatch::new(width, height);
                        batch.push_pane_outlines(
                            fill,
                            workspace.pane_viewport,
                            &rules,
                            Some(palette::CARD_OUTLINE),
                            neutral,
                            stroke,
                        );
                        for column in &workspace.other_columns {
                            let peer_fill = workspace.card_fill(column.chrome, metrics, stroke);
                            let rules = workspace
                                .pane_outlines(
                                    &column.panes,
                                    column.terminal,
                                    peer_fill,
                                    neutral,
                                    metrics,
                                    stroke,
                                )
                                .into_iter()
                                .map(|rule| (rule, [SceneRect::default(); 2]))
                                .collect::<Vec<_>>();
                            batch.push_pane_outlines(
                                peer_fill,
                                workspace.pane_viewport,
                                &rules,
                                Some(palette::CARD_OUTLINE),
                                neutral,
                                neutral,
                            );
                        }
                        if fill.bounds.height == 0.0 {
                            assert!(batch.bytes.is_empty(), "empty cards must not paint strokes");
                        }
                        let radius = rounded_radius(fill.bounds, fill.radius);
                        let bottom_curve = fill.bounds.bottom() - radius;
                        for rect in painted_rects(&batch) {
                            assert!(
                                rect.bottom() <= expected + 0.001,
                                "stroke escaped card: height={height}, expanded={expanded}, rect={rect:?}, line={expected}"
                            );
                            for grid in &raw_grids {
                                assert!(
                                    grid.intersection(rect).is_none_or(|overlap| {
                                        overlap.width < 0.001 || overlap.height < 0.001
                                    }),
                                    "stroke covers raw grid: scale={scale}, height={height}, expanded={expanded}, scroll={pane_scroll}, grid={grid:?}, stroke={rect:?}"
                                );
                            }
                            for y in (rect.top.max(bottom_curve).floor() as u32
                                ..rect.bottom().ceil() as u32)
                                .map(|y| y as f32 + 0.5)
                            {
                                for x in [rect.left + 0.5, rect.right() - 0.5] {
                                    if rect.contains(x, y) {
                                        assert!(
                                            cards.iter().any(|(card, _)| {
                                                card.bands().any(|band| band.contains(x, y))
                                            }),
                                            "stroke outside bottom curve at {x},{y}"
                                        );
                                    }
                                }
                            }
                        }
                        let old =
                            workspace_at(height.saturating_add(2 * metrics.height.ceil() as u32));
                        let scenes = std::iter::once(old.terminal)
                            .chain(old.other_columns.iter().map(|column| column.terminal))
                            .map(|terminal| {
                                metrics
                                    .surface_size(terminal.screen_size())
                                    .map(|size| Scene {
                                        revision: 1,
                                        columns: size.cols,
                                        rows: size.rows,
                                        screen: Screen::Primary,
                                        title: String::new(),
                                        working_directory: String::new(),
                                        graphics: SceneGraphics::default(),
                                        background: palette::TERMINAL_FALLBACK,
                                        foreground: palette::CARD_OUTLINE,
                                        cursor: None,
                                        content: vec![],
                                    })
                            })
                            .collect::<Vec<_>>();
                        let peers = scenes[1..].iter().map(Option::as_ref).collect::<Vec<_>>();
                        let paint =
                            workspace_paint(&workspace, scenes[0].as_ref(), &peers, metrics);
                        assert_eq!(paint.terminal, workspace.terminal);
                        assert_eq!(paint.panes, workspace.panes);
                        let cards =
                            card_backdrops(&paint, scenes[0].as_ref(), &peers, metrics, stroke);
                        let bottom = cards[0].0.bounds.bottom();
                        assert!(cards.iter().all(|(card, _)| card.bounds.bottom() == bottom));
                        let mut displayed = Vec::new();
                        let mut batch = RectangleBatch::new(width, height);
                        for ((panes, terminal, allocated, current), scene) in std::iter::once((
                            paint.panes.as_slice(),
                            paint.terminal,
                            paint.chrome,
                            true,
                        ))
                        .chain(paint.other_columns.iter().map(|column| {
                            (
                                column.panes.as_slice(),
                                column.terminal,
                                column.chrome,
                                false,
                            )
                        }))
                        .zip(&scenes)
                        {
                            if let Some(grid) = scene
                                .as_ref()
                                .and_then(|scene| {
                                    scene_grid(scene, terminal, metrics).intersection(terminal)
                                })
                                .and_then(|grid| grid.intersection(paint.pane_viewport))
                            {
                                displayed.push(grid);
                                let card = paint.card_fill(allocated, metrics, stroke);
                                if grid.bottom() > card.bounds.top {
                                    let (clip, _) = terminal_paint_clip(
                                        &paint, allocated, terminal, metrics, true, stroke,
                                    )
                                    .expect("visible retained rows must not be removed");
                                    assert!(
                                        clip.bottom() + 0.001 >= grid.bottom(),
                                        "card clip trims retained rows: grid={grid:?}, clip={clip:?}"
                                    );
                                }
                            }
                            let fill = paint.card_fill(allocated, metrics, stroke);
                            assert!(
                                fill.bounds.top >= allocated.top
                                    && fill.bounds.bottom() <= allocated.bottom()
                            );
                            let rules = paint
                                .pane_outlines(panes, terminal, fill, neutral, metrics, stroke)
                                .into_iter()
                                .map(|rule| (rule, [SceneRect::default(); 2]))
                                .collect::<Vec<_>>();
                            if let Some(pane) = panes.last().filter(|pane| !pane.selected) {
                                let text_height = chrome_title_metrics(metrics).text_height;
                                let row = chrome::bottom_title_row(
                                    pane.rect,
                                    rules.last().unwrap().0.bounds.bottom(),
                                    text_height,
                                );
                                assert!(
                                    row.top + (row.height + text_height) / 2.0
                                        <= pane.rect.bottom() + 0.001
                                );
                            }
                            batch.push_pane_outlines(
                                fill,
                                paint.pane_viewport,
                                &rules,
                                Some(palette::CARD_OUTLINE),
                                neutral,
                                if current { stroke } else { neutral },
                            );
                        }
                        for rect in painted_rects(&batch) {
                            assert!(rect.bottom() <= bottom + 0.001);
                            for grid in displayed.iter().chain(&raw_grids) {
                                assert!(
                                    grid.intersection(rect)
                                        .is_none_or(|overlap| overlap.width < 0.001
                                            || overlap.height < 0.001),
                                    "retained sweep: scale={scale}, height={height}, expanded={expanded}, scroll={pane_scroll}, grid={grid:?}, stroke={rect:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn title_rules_leave_label_and_scrollback_gaps_inside_the_fixed_card() {
    let card = SceneRect {
        left: 10.0,
        top: 30.0,
        width: 180.0,
        height: 160.0,
    };
    let label = SceneRect {
        left: 16.0,
        top: 21.0,
        width: 68.0,
        height: 18.0,
    };
    let scrollback = SceneRect {
        left: 150.0,
        width: 24.0,
        ..label
    };
    for top in [20.0, 70.0, 21.25, -10.0] {
        let mut batch = RectangleBatch::new(200, 200);
        let viewport = SceneRect {
            top: 20.0,
            height: 180.0,
            ..card
        };
        batch.clip = Some(viewport);
        let top = crate::scene::title_rule_y(top, 20.0);
        let frame = SceneRect {
            top,
            height: card.bottom() - top,
            ..card
        };
        batch.push_pane_outlines(
            CardFill {
                bounds: viewport,
                radius: 8.0,
            },
            viewport,
            &[(
                PaneOutline {
                    bounds: frame,
                    clip: frame,
                    radius: rounded_radius(frame, 8.0),
                    title_y: Some(top),
                    expanded: true,
                },
                [label, scrollback],
            )],
            Some(SceneColor::default()),
            1.0,
            1.0,
        );
        assert_eq!(batch.clip, Some(viewport));
        let quads = batch
            .bytes
            .chunks_exact(VERTEX_SIZE as usize * 6)
            .map(|quad| {
                let value = |vertex: usize, component: usize| {
                    let offset = vertex * VERTEX_SIZE as usize + component * 4;
                    f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                };
                SceneRect {
                    left: (value(0, 0) + 1.0) * 100.0,
                    top: (1.0 - value(0, 1)) * 100.0,
                    width: (value(2, 0) - value(0, 0)) * 100.0,
                    height: (value(0, 1) - value(2, 1)) * 100.0,
                }
            })
            .collect::<Vec<_>>();
        let y = top;
        assert!(
            quads.iter().all(|rect| rect.top >= viewport.top - 0.001
                && rect.bottom() <= viewport.bottom() + 0.001)
        );
        if y >= viewport.top {
            assert!(
                !quads.iter().any(|rect| rect.contains(10.5, y + 0.5)),
                "every pane's top must have its own rounded corner"
            );
            assert!(quads.iter().any(|rect| rect.contains(100.0, y + 0.5)));
            assert!(!quads.iter().any(|rect| rect.contains(160.0, y + 0.5)));
            assert!(quads.iter().any(|rect| rect.contains(180.0, y + 0.5)));
            assert!(!quads.iter().any(|rect| rect.contains(50.0, y + 0.5)));
            if y == card.top {
                assert!(
                    !quads.iter().any(|rect| rect.contains(10.5, y + 0.5)),
                    "rounded top corner must stay clear"
                );
            }
        } else {
            assert!(
                !quads
                    .iter()
                    .any(|rect| rect.contains(100.0, viewport.top + 0.5)),
                "clipping must not invent a top edge"
            );
            assert!(
                quads
                    .iter()
                    .any(|rect| rect.contains(card.left + 0.5, viewport.top + 20.0))
            );
        }
    }
}

#[test]
fn rectangle_encoding_is_row_major_and_bounded() {
    let mut rectangles = RectangleBatch::new(100, 100);
    rectangles.push(0.0, 0.0, 10.0, 20.0, SceneColor { r: 1, g: 2, b: 3 }, 1.0);
    assert_eq!(
        rectangles.bytes.len(),
        VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
    );
    assert_eq!(&rectangles.bytes[..4], &(-1.0_f32).to_ne_bytes());
    assert_eq!(&rectangles.bytes[4..8], &1.0_f32.to_ne_bytes());
}

#[test]
fn column_backgrounds_cover_the_gaps_between_cards() {
    let column = |left: f32| SceneRect {
        left,
        top: 40.0,
        width: 289.0,
        height: 500.0,
    };
    let terminals = [column(8.0), column(305.0), column(602.0)];
    let spans = background_spans(&terminals, 901.0);
    // The strips tile the window's width, so no gap stays unpainted.
    assert_eq!(spans.first().map(|span| span.0), Some(0.0));
    assert_eq!(spans.last().map(|span| span.1), Some(901.0));
    for (pair, terminal) in spans.windows(2).zip(&terminals) {
        assert_eq!(pair[0].1, pair[1].0);
        assert!(pair[0].0 <= terminal.left && terminal.right() <= pair[0].1);
    }
    assert_eq!(background_spans(&terminals[..1], 901.0), [(0.0, 901.0)]);
}

#[test]
fn frame_fills_everything_outside_the_cards() {
    let frame = SceneRect {
        left: 0.0,
        top: 32.0,
        width: 901.0,
        height: 568.0,
    };
    let card = |left: f32, top: f32| SceneRect {
        left,
        top,
        width: 289.0,
        height: 500.0,
    };
    // A popup card can sit at another height than the columns beside it.
    let cards = [card(8.0, 40.0), card(305.0, 48.0), card(602.0, 40.0)];
    let free = uncovered(frame, &cards);
    assert!(
        free.iter()
            .all(|rect| cards.iter().all(|card| rect.intersection(*card).is_none()))
    );
    let area = |rect: &SceneRect| rect.width * rect.height;
    let covered = free.iter().chain(&cards).map(area).sum::<f32>();
    assert_eq!(covered, area(&frame));
    assert_eq!(uncovered(frame, &[]), [frame]);
}

#[test]
fn terminal_paint_stays_inside_rounded_fill_during_cursor_handoff() {
    use eon_workspace_protocol::v24::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};

    let pane = |column, index| Pane {
        label: format!("p{column}-{index}"),
        pinned_name: None,
        id: format!("p{column}-{index}"),
        session: format!("s{column}-{index}"),
        endpoint: format!("/tmp/{column}-{index}.sock").into_bytes(),
        live: true,
        attention: None,
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
            selected_pane: Some("p0-40".into()),
            selected_popup: None,
            columns: vec![
                Column {
                    panes: (0..80).map(|index| pane(0, index)).collect(),
                    expanded: "p0-40".into(),
                },
                Column {
                    panes: vec![pane(1, 0)],
                    expanded: "p1-0".into(),
                },
            ],
            popups: vec![],
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        codex_quota: None,
        claude_quota: None,
        sidebar: Sidebar::Closed,
    };
    let mut scene = Scene {
        revision: 1,
        columns: 80,
        rows: 60,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: palette::CARD_OUTLINE,
        cursor: Some(DrawCursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            column: 0,
            row: 0,
            at_wide_tail: false,
            color: palette::CARD_OUTLINE,
            explicit_color: false,
        }),
        content: vec![DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![DrawCell {
                width: CellWidth::Narrow,
                text: "█".into(),
                hyperlink: String::new(),
                style: DrawStyle {
                    background_is_default: false,
                    selected: true,
                    ..plain_style()
                },
            }],
        }],
    };
    let covers = |batch: &RectangleBatch, x: f32, y: f32| {
        batch
            .bytes
            .chunks_exact(3 * VERTEX_SIZE as usize)
            .any(|triangle| {
                let points = [0, 1, 2].map(|index| {
                    let vertex = &triangle[index * VERTEX_SIZE as usize..];
                    CursorPoint {
                        x: (f32::from_ne_bytes(vertex[..4].try_into().unwrap()) + 1.0) * 1000.0,
                        y: (1.0 - f32::from_ne_bytes(vertex[4..8].try_into().unwrap())) * 450.0,
                    }
                });
                let cross = |a: CursorPoint, b: CursorPoint| {
                    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
                };
                let signs = [
                    cross(points[0], points[1]),
                    cross(points[1], points[2]),
                    cross(points[2], points[0]),
                ];
                let area = (points[1].x - points[0].x) * (points[2].y - points[0].y)
                    - (points[1].y - points[0].y) * (points[2].x - points[0].x);
                area.abs() > 0.0001
                    && (signs.iter().all(|v| *v >= 0.0) || signs.iter().all(|v| *v <= 0.0))
            })
    };
    for scale in [1.0, 1.3] {
        let metrics = CellMetrics::for_scale(scale);
        let workspace = |snapshot: &Snapshot| {
            WorkspaceScene::from_snapshot(
                snapshot,
                PhysicalSize::new(2000, 900),
                metrics,
                0.0,
                0.0,
                |_, label| (label.into(), label.len() as f32 * metrics.width),
            )
        };
        snapshot.tabs[0].selected_pane = Some("p0-40".into());
        let source = workspace(&snapshot);
        snapshot.tabs[0].selected_pane = Some("p1-0".into());
        let destination = workspace(&snapshot);
        let card = destination.card_rect(
            destination.chrome,
            metrics,
            physical_outline_width(2, metrics.scale),
        );
        let assert_corners_clear = |batch: &RectangleBatch| {
            let radius = destination.card_radius().floor();
            for row in 0..radius as u32 {
                let dy = radius - row as f32 - 0.5;
                for offset in 0..radius as u32 {
                    let dx = radius - offset as f32 - 0.5;
                    if dx * dx + dy * dy <= radius * radius {
                        continue;
                    }
                    for x in [
                        card.left + offset as f32 + 0.5,
                        card.right() - offset as f32 - 0.5,
                    ] {
                        for y in [
                            card.top + row as f32 + 0.5,
                            card.bottom() - row as f32 - 0.5,
                        ] {
                            assert!(
                                !covers(batch, x, y),
                                "paint outside fill at ({x},{y}), scale={scale}"
                            );
                        }
                    }
                }
            }
        };
        let mut animation = CursorAnimation::default();
        scene.cursor.as_mut().unwrap().row = 0;
        let mut initial = RectangleBatch::new(2000, 900);
        build_tail_cursor(
            &mut initial,
            &mut animation,
            &scene,
            true,
            metrics,
            source.terminal,
            palette::CARD_OUTLINE,
            palette::TERMINAL_FALLBACK,
            1.0,
            0.0,
        );
        scene.cursor.as_mut().unwrap().row = 43;
        for frame in 0..24 {
            let mut moving = RectangleBatch::new(2000, 900);
            let (clip, card_clip) = terminal_paint_clip(
                &destination,
                destination.chrome,
                destination.terminal,
                metrics,
                true,
                physical_outline_width(2, metrics.scale),
            )
            .unwrap();
            moving.clip = Some(clip);
            moving.card_clip = card_clip;
            let active = build_tail_cursor(
                &mut moving,
                &mut animation,
                &scene,
                true,
                metrics,
                destination.terminal,
                palette::CARD_OUTLINE,
                palette::TERMINAL_FALLBACK,
                1.0,
                1.0 / 240.0,
            );
            if frame == 0 {
                assert!(active, "the regression must exercise a moving cursor");
                if scale == 1.0 {
                    assert!(
                        !covers(&moving, 1004.5, 888.5),
                        "cursor handoff paints outside the rounded fill"
                    );
                }
            }
            assert_corners_clear(&moving);
        }
        for left in [card.left, card.right() - metrics.width] {
            for top in [card.top, card.bottom() - metrics.height] {
                let mut cells = RectangleBatch::new(2000, 900);
                cells.clip = Some(card);
                cells.card_clip = Some(destination.card_fill(
                    destination.chrome,
                    metrics,
                    physical_outline_width(2, metrics.scale),
                ));
                build_scene_rectangles(
                    &mut cells,
                    &scene,
                    &[],
                    None,
                    true,
                    metrics,
                    SceneRect {
                        left: left - metrics.padding,
                        top: top - metrics.padding,
                        ..card
                    },
                    false,
                );
                assert_corners_clear(&cells);
                assert!(
                    covers(
                        &cells,
                        left + metrics.width / 2.0,
                        top + metrics.height / 2.0
                    ),
                    "interior cell paint must survive clipping"
                );
            }
        }
    }
}

#[test]
fn card_backdrop_keeps_cards_like_the_agent_sidebar_at_terminal_opacity() {
    use eon_workspace_protocol::v24::{
        ALT, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Sidebar, Snapshot,
        Tab,
    };
    let terminal_scene = Scene {
        revision: 1,
        columns: 1,
        rows: 1,
        screen: orbit_protocol::Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: palette::CARD_OUTLINE,
        cursor: None,
        content: vec![DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![crate::DrawCell {
                width: orbit_protocol::CellWidth::Narrow,
                text: " ".into(),
                hyperlink: String::new(),
                style: DrawStyle {
                    foreground: palette::CARD_OUTLINE,
                    background: palette::CARD_OUTLINE,
                    underline_color: palette::CARD_OUTLINE,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                    selected: false,
                    background_is_default: false,
                    protected: false,
                    underline: orbit_protocol::Underline::None,
                },
            }],
        }],
    };
    // Exercise the actual backdrop collection: inner card curves must never cut its fill.
    for scale in [1.0, 1.3] {
        let metrics = CellMetrics::for_scale(scale);
        for column_count in [1, 2] {
            for pane_count in [1, 3, 4, 5, 80] {
                for selected in [0, pane_count / 2, pane_count - 1] {
                    let pane_id = |column, pane| format!("p{column}-{pane}");
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
                            selected_pane: Some(pane_id(0, selected)),
                            selected_popup: None,
                            columns: (0..column_count)
                                .map(|column| Column {
                                    panes: (0..pane_count)
                                        .map(|pane| Pane {
                                            label: (pane_id(column, pane)).to_string(),
                                            pinned_name: None,
                                            id: pane_id(column, pane),
                                            session: format!("s{column}-{pane}"),
                                            endpoint: format!("/tmp/{column}-{pane}.sock")
                                                .into_bytes(),
                                            live: true,
                                            attention: None,
                                        })
                                        .collect(),
                                    expanded: pane_id(column, selected),
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
                        sidebar: Sidebar::Open,
                    };
                    for popup in [false, true] {
                        snapshot.tabs[0].selected_popup = popup.then(|| "u1".into());
                        for scroll in [0.0, 36.0 * metrics.scale, 4000.0] {
                            let workspace = WorkspaceScene::from_snapshot(
                                &snapshot,
                                PhysicalSize::new(2000, 900),
                                metrics,
                                0.0,
                                scroll,
                                |_, text| (text.into(), text.len() as f32 * metrics.width),
                            );
                            let fill_top = if popup {
                                crate::scene::popup_card_rect(workspace.chrome, metrics).top
                            } else {
                                workspace
                                    .card_rect(
                                        workspace.chrome,
                                        metrics,
                                        physical_outline_width(2, metrics.scale),
                                    )
                                    .top
                            };
                            for (terminal, allocated) in
                                std::iter::once((workspace.terminal, workspace.chrome)).chain(
                                    workspace
                                        .other_columns
                                        .iter()
                                        .map(|column| (column.terminal, column.chrome)),
                                )
                            {
                                let clip_with = |pane_frames| {
                                    terminal_paint_clip(
                                        &workspace,
                                        allocated,
                                        terminal,
                                        metrics,
                                        pane_frames,
                                        physical_outline_width(2, metrics.scale),
                                    )
                                    .map(|(clip, card)| (clip, card.map(|card| card.bounds)))
                                };
                                if popup {
                                    // The popup card bounds its paint in both frame modes.
                                    assert_eq!(clip_with(false), clip_with(true));
                                } else {
                                    assert_eq!(
                                        clip_with(false),
                                        terminal
                                            .intersection(workspace.pane_viewport)
                                            .map(|clip| (clip, None))
                                    );
                                }
                                let Some((clip, card_clip)) = terminal_paint_clip(
                                    &workspace,
                                    allocated,
                                    terminal,
                                    metrics,
                                    true,
                                    physical_outline_width(2, metrics.scale),
                                ) else {
                                    continue;
                                };
                                let mut cells = RectangleBatch::new(2000, 900);
                                cells.clip = Some(clip);
                                cells.card_clip = card_clip;
                                build_scene_rectangles(
                                    &mut cells,
                                    &terminal_scene,
                                    &[],
                                    None,
                                    true,
                                    metrics,
                                    terminal,
                                    true,
                                );
                                for vertex in cells.bytes.chunks_exact(VERTEX_SIZE as usize) {
                                    let y = (1.0
                                        - f32::from_ne_bytes(vertex[4..8].try_into().unwrap()))
                                        * 450.0;
                                    assert!(
                                        y >= fill_top - 0.001,
                                        "terminal paint above fixed fill: y={y}, fill={fill_top}, scale={scale}, panes={pane_count}, scroll={scroll}"
                                    );
                                }
                            }
                            let columns = vec![None; workspace.other_columns.len()];
                            let cards = card_backdrops(
                                &workspace,
                                None,
                                &columns,
                                metrics,
                                physical_outline_width(2, metrics.scale),
                            );
                            let frame = SceneRect {
                                left: 0.0,
                                top: workspace.tab_viewport.bottom(),
                                width: 2000.0,
                                height: 900.0 - workspace.tab_viewport.bottom(),
                            };
                            let mut rectangles = RectangleBatch::new(2000, 900);
                            push_card_backdrop(&mut rectangles, frame, &cards, 0.6);
                            let floats = rectangles
                                .bytes
                                .chunks_exact(4)
                                .map(|v| f32::from_ne_bytes(v.try_into().unwrap()))
                                .collect::<Vec<_>>();
                            let quads = floats
                                .chunks_exact(36)
                                .map(|quad| {
                                    let x = |i: usize| (quad[i * 6] + 1.0) * 1000.0;
                                    let y = |i: usize| (1.0 - quad[i * 6 + 1]) * 450.0;
                                    (
                                        SceneRect {
                                            left: x(0),
                                            top: y(0),
                                            width: x(2) - x(0),
                                            height: y(2) - y(0),
                                        },
                                        quad[5],
                                    )
                                })
                                .collect::<Vec<_>>();
                            let covering = |x, y| {
                                quads
                                    .iter()
                                    .filter(|(rect, _)| rect.contains(x, y))
                                    .map(|(_, alpha)| *alpha)
                                    .collect::<Vec<_>>()
                            };
                            let mut expected = vec![if popup {
                                workspace
                                    .card_fill(
                                        workspace.chrome,
                                        metrics,
                                        physical_outline_width(2, metrics.scale),
                                    )
                                    .bounds
                            } else {
                                workspace.card_rect(
                                    workspace.chrome,
                                    metrics,
                                    physical_outline_width(2, metrics.scale),
                                )
                            }];
                            expected.extend(workspace.other_columns.iter().map(|column| {
                                workspace.card_rect(
                                    column.chrome,
                                    metrics,
                                    physical_outline_width(2, metrics.scale),
                                )
                            }));
                            expected.extend(workspace.sidebar_area().map(|area| {
                                workspace.card_rect(
                                    area,
                                    metrics,
                                    physical_outline_width(2, metrics.scale),
                                )
                            }));
                            assert_eq!(cards.len(), expected.len());
                            for card in expected {
                                let center = card.left + card.width / 2.0;
                                assert_eq!(
                                    covering(center, card.top - 0.5),
                                    [1.0],
                                    "fill above top: scale={scale}, popup={popup}, scroll={scroll}"
                                );
                                for x in [card.left + 0.5, card.right() - 0.5] {
                                    for y in [card.top + 0.5, card.bottom() - 0.5] {
                                        assert_eq!(
                                            covering(x, y),
                                            [1.0],
                                            "fill outside rounded corner"
                                        );
                                    }
                                }
                                let radius = workspace.card_radius().floor();
                                for row in 0..radius as u32 {
                                    let dy = radius - row as f32 - 0.5;
                                    for offset in [0.5, radius / 2.0 + 0.5, radius - 0.5] {
                                        let inside =
                                            (radius - offset).powi(2) + dy * dy < radius * radius;
                                        for x in [card.left + offset, card.right() - offset] {
                                            for y in [
                                                card.top + row as f32 + 0.5,
                                                card.bottom() - row as f32 - 0.5,
                                            ] {
                                                assert_eq!(
                                                    covering(x, y),
                                                    [if inside { 0.6 } else { 1.0 }],
                                                    "outer curve coverage"
                                                );
                                            }
                                        }
                                    }
                                }
                                // All rows, including titles, inner corners and all four collapsed panes.
                                for row in
                                    (card.top.ceil() as u32 + 1)..(card.bottom().floor() as u32 - 1)
                                {
                                    let y = row as f32 + 0.5;
                                    assert_eq!(
                                        covering(center, y),
                                        [0.6],
                                        "band or double alpha at row {row}"
                                    );
                                    if y > card.top + workspace.card_radius()
                                        && y < card.bottom() - workspace.card_radius()
                                    {
                                        for x in [card.left + 0.5, card.right() - 0.5] {
                                            assert_eq!(
                                                covering(x, y),
                                                [0.6],
                                                "notch beside an inner pane curve"
                                            );
                                        }
                                    }
                                }
                                assert_eq!(covering(center, card.bottom() + 0.5), [1.0]);
                            }
                        }
                    }
                }
            }
        }
    }
}
