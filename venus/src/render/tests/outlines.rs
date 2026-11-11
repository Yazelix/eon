use super::*;
use crate::DrawCursor;
use orbit_protocol::CursorShape;

#[test]
fn popup_tab_joins_the_left_border_and_returns_into_the_top_edge() {
    let card = CardFill {
        bounds: SceneRect {
            left: 10.0,
            top: 40.0,
            width: 300.0,
            height: 200.0,
        },
        radius: 8.0,
    };
    let tab = FrameTitleTab {
        top: 26.0,
        start: 100.0,
        left_radius: 14.0,
        row: SceneRect::default(),
        return_run: 21.0,
        action: None,
        title: FrameTitle {
            label: "Agent".into(),
            width: 82.0,
        },
    };
    for width in [1.0, 2.0, 3.0] {
        let mut batch = RectangleBatch::new(400, 300);
        batch.push_title_tab_outline(card, &tab, width, palette::CARD_OUTLINE);
        let ink = painted_rects(&batch).collect::<Vec<_>>();
        for y in [39.5, 40.5, 41.5, 47.5] {
            assert!(ink.iter().any(|r| r.contains(10.5, y)), "left seam at {y}");
        }
        assert!(
            !ink.iter().any(|r| r.contains(17.0, 40.5)),
            "old corner remains"
        );
        assert!(
            ink.iter().any(|r| r.contains(123.0, 40.5)),
            "right return is missing"
        );
        assert!(
            !ink.iter().any(|r| r.contains(50.0, 40.5)),
            "stroke crosses title"
        );
        let fill = title_tab_bands(card, &tab, 0.0);
        assert!(
            fill.iter().any(|r| r.contains(10.5, 40.5)),
            "old corner cuts fill"
        );
    }
}

#[test]
fn title_return_arcs_keep_normal_stroke_width_at_their_tangent_join() {
    let card = CardFill {
        bounds: SceneRect {
            left: 10.0,
            top: 40.0,
            width: 300.0,
            height: 200.0,
        },
        radius: 8.0,
    };
    let tab = FrameTitleTab {
        top: 26.0,
        start: 100.0,
        left_radius: 14.0,
        row: SceneRect::default(),
        return_run: 21.0,
        action: None,
        title: FrameTitle {
            label: "Agent".into(),
            width: 82.0,
        },
    };
    // Reference circles for the chosen 21-by-14 return, independent of band tessellation.
    let radius = 11.375;
    let start = tab.start;
    for inset in [0.0, 1.0, 2.0, 3.0] {
        let join = 33.0 + inset * (5.0 / 13.0);
        let mut checked = 0;
        for band in title_tab_bands(card, &tab, inset) {
            let y = band.top + 0.5;
            if y < tab.top + inset || y >= card.bounds.top + inset {
                continue;
            }
            let (x, cy, offset_radius) = if y < join {
                (start, tab.top + radius, radius - inset)
            } else {
                (
                    start + tab.return_run,
                    card.bounds.top - radius,
                    radius + inset,
                )
            };
            assert!(
                ((band.right() - x).hypot(y - cy) - offset_radius).abs() < 0.001,
                "chosen return is not a normal stroke offset at {y}, inset {inset}"
            );
            assert!(band.left.is_finite() && band.width > 0.0);
            checked += 1;
        }
        assert!(checked >= 10);
    }
    let mut batch = RectangleBatch::new(400, 300);
    batch.push_title_tab_outline(card, &tab, 3.0, palette::CARD_OUTLINE);
    let ink = painted_rects(&batch).collect::<Vec<_>>();
    assert!(
        ink.iter().any(|rect| rect.contains(10.5, 40.5)),
        "left seam broke"
    );
    assert!(
        !ink.iter().any(|rect| rect.contains(50.0, 40.5)),
        "stroke crossed the title"
    );
}

fn popup_title_snapshot(
    label: &str,
    vertical_margin: f32,
) -> eon_workspace_protocol::v39::Snapshot {
    use eon_workspace_protocol::v39::{
        Popup, PopupEntry, PopupGeometry, PopupKey, Sidebar, Snapshot, Tab,
    };
    Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: None,
            selected_popup: Some("u1".into()),
            columns: vec![],
            popups: vec![Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "popup".into(),
                endpoint: b"/tmp/popup.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        }],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin,
        },
        entries: vec![PopupEntry {
            id: "agent".into(),
            label: label.into(),
            shortcut: PopupKey::Unbound,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: vec![],
    }
}

fn popup_title_workspace(
    metrics: CellMetrics,
    size: PhysicalSize<u32>,
    label: &str,
    vertical_margin: f32,
) -> WorkspaceScene {
    let snapshot = popup_title_snapshot(label, vertical_margin);
    WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
        (text.into(), text.len() as f32 * metrics.width)
    })
}

#[test]
fn scrolled_terminals_paint_below_the_stack_band() {
    use eon_workspace_protocol::v39::{Column, Pane};
    let metrics = CellMetrics::for_scale(1.0);
    let stroke = physical_outline_width(1, metrics.scale);
    let mut snapshot = popup_title_snapshot("Popup", 4.0);
    let tab = &mut snapshot.tabs[0];
    tab.selected_popup = None;
    tab.popups.clear();
    tab.selected_pane = Some("p0".into());
    tab.columns = vec![Column {
        panes: (0..30)
            .map(|pane| Pane {
                id: format!("p{pane}"),
                label: format!("p{pane}  Codex"),
                session: format!("s{pane}"),
                endpoint: format!("/tmp/{pane}.sock").into_bytes(),
                pinned_name: None,
                live: true,
                attention: None,
            })
            .collect(),
        expanded: "p0".into(),
    }];
    let project = |scroll| {
        WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1280, 400),
            metrics,
            0.0,
            scroll,
            |_, label| (label.into(), label.len() as f32 * metrics.width),
        )
    };
    let header = project(0.0).panes[0].rect.height;
    // A few pixels past the expanded pane's header, its terminal reaches into the band.
    let workspace = project(header + 5.0);
    assert!(workspace.terminal.top < workspace.header_viewport.top);
    for frames in [true, false] {
        let (clip, _) = terminal_paint_clip(
            &workspace,
            workspace.chrome,
            workspace.terminal,
            metrics,
            frames,
            stroke,
        )
        .unwrap();
        assert!(clip.top >= workspace.header_viewport.top, "frames {frames}");
    }
}

#[test]
fn every_column_card_carries_a_stack_tab_one_card_gap_below_the_bar() {
    use eon_workspace_protocol::v39::{Column, Pane};
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let stroke = physical_outline_width(1, metrics.scale);
        for (columns, panes) in [(1, 1), (1, 3), (2, 1), (2, 3), (4, 2)] {
            for expanded in 0..panes {
                let mut snapshot = popup_title_snapshot("Popup", 4.0);
                let tab = &mut snapshot.tabs[0];
                tab.selected_popup = None;
                tab.popups.clear();
                let id = |column: usize, pane: usize| format!("p{}", column * panes + pane);
                tab.selected_pane = Some(id(columns - 1, expanded));
                tab.columns = (0..columns)
                    .map(|column| Column {
                        panes: (0..panes)
                            .map(|pane| Pane {
                                id: id(column, pane),
                                label: format!("{}  Codex", id(column, pane)),
                                session: format!("s{column}-{pane}"),
                                endpoint: format!("/tmp/{column}-{pane}.sock").into_bytes(),
                                pinned_name: None,
                                live: true,
                                attention: None,
                            })
                            .collect(),
                        expanded: id(column, expanded),
                    })
                    .collect();
                // Four columns fit their numbered labels from 1280 pixels.
                let widths = if columns == 4 {
                    &[(1280, false), (1280, true)][..]
                } else {
                    &[(400, false), (1280, false), (1280, true)]
                };
                for &(width, zoom) in widths {
                    let workspace = WorkspaceScene::from_snapshot_with_zoom(
                        &snapshot,
                        crate::EonBar::default(),
                        PhysicalSize::new(width, 800),
                        metrics,
                        0.0,
                        0.0,
                        |_, label| (label.into(), label.len() as f32 * metrics.width),
                        None,
                        zoom,
                        crate::SidebarView::default(),
                    );
                    let tabs = FrameTitleTab::workspace(
                        &mut fonts.font_system,
                        metrics,
                        &workspace,
                        true,
                        (stroke, stroke),
                    );
                    let shown = if workspace.zoomed_pane().is_some() {
                        1
                    } else {
                        columns
                    };
                    assert_eq!(tabs.len(), shown);
                    let pills = workspace.bar_pill_rect(SceneRect::default(), metrics);
                    for (tab, (chrome, label, panes)) in tabs.iter().zip(workspace.column_cards()) {
                        let index = if chrome == workspace.chrome {
                            workspace.selected_column
                        } else {
                            workspace
                                .other_columns
                                .iter()
                                .find(|column| column.chrome == chrome)
                                .unwrap()
                                .index
                        };
                        let expected = if columns == 1 {
                            "Stack".to_string()
                        } else {
                            format!("Stack {}", index + 1)
                        };
                        assert_eq!(label, Some(expected.as_str()));
                        let tab = tab.as_ref().expect("the Stack tab fits");
                        if width == 1280 || scale == 1.0 {
                            assert_eq!(tab.title.label, expected);
                        }
                        assert_eq!(tab.row.top, workspace.pane_viewport.top);
                        // The cap clears the bar's pills by one card gap.
                        let gap = tab.top - pills.bottom();
                        assert!(
                            (gap - crate::scene::tokens::card_gap(metrics)).abs() <= 1.0,
                            "scale={scale}, gap={gap}"
                        );
                        // A collapsed first row starts below the Stack band; an expanded
                        // first row also uses its top-clearance reserve.
                        let first = &panes[0];
                        if !first.selected {
                            assert_eq!(first.rect.top, workspace.header_viewport.top);
                        }
                        assert!(tab.row.bottom() < first.rect.top);
                    }
                    if !zoom {
                        let lower = workspace
                            .panes
                            .iter()
                            .skip(1)
                            .map(|pane| pane.rect.top - workspace.header_viewport.top)
                            .collect::<Vec<_>>();
                        assert!(
                            lower
                                .iter()
                                .all(|top| *top >= workspace.panes[0].rect.height),
                            "lower rows stay below the first"
                        );
                    }
                }
            }
        }
    }
}

fn roomy_popup_title_workspace(
    metrics: CellMetrics,
    size: PhysicalSize<u32>,
    label: &str,
) -> WorkspaceScene {
    let mut workspace = popup_title_workspace(
        metrics,
        size,
        label,
        chrome_title_metrics(metrics).text_height / metrics.scale,
    );
    // Keep a full padding band above the title even when large fonts consume the normal bar gap.
    workspace.header.height = workspace
        .header
        .height
        .min(workspace.chrome.top - metrics.padding);
    workspace.tab_viewport.height = workspace.header.height;
    workspace
}

#[test]
fn popup_tabs_fit_labels_and_partition_translucent_backgrounds() {
    for size in [6.0, 16.0, 96.0] {
        let mut fonts = FontSetup::new(&FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            size,
            ..FontSettings::default()
        })
        .unwrap();
        for scale in [1.0, 1.3, 2.0] {
            let metrics = fonts.metrics(scale);
            for width in [80, 400, 2400] {
                for label in [
                    "Claude",
                    "Beadseer",
                    "A very long popup name ".repeat(30).as_str(),
                    "שלום",
                ] {
                    let workspace =
                        roomy_popup_title_workspace(metrics, PhysicalSize::new(width, 1800), label);
                    for border in [1, 2, 3] {
                        let stroke = physical_outline_width(border, metrics.scale);
                        let Some(tab) = FrameTitleTab::popup(
                            &mut fonts.font_system,
                            metrics,
                            &workspace,
                            stroke,
                        ) else {
                            assert!(
                                width < 400 || (width == 400 && size == 96.0),
                                "unexpected omission: {label}, {width}, {size}, {scale}"
                            );
                            continue;
                        };
                        let card = workspace.card_fill(workspace.chrome, metrics, stroke);
                        assert!(
                            tab.start + tab.return_run + stroke
                                <= card.bounds.right() - card.radius
                        );
                        assert_eq!(workspace.popup_label(), Some(label));
                        let mut batch = RectangleBatch::new(width, 1800);
                        push_title_tab_backdrop(
                            &mut batch,
                            SceneRect {
                                left: 0.0,
                                top: workspace.header.bottom(),
                                width: width as f32,
                                height: 1800.0 - workspace.header.bottom(),
                            },
                            &[(Some(card), palette::TERMINAL_FALLBACK)],
                            0.8,
                            &[Some(tab.clone())],
                        );
                        let text_x = card.bounds.left + metrics.padding;
                        for y in [
                            (workspace.chrome.top + stroke).ceil() + 0.5,
                            card.bounds.top + 0.5,
                        ] {
                            // The title's two halves each receive exactly one terminal fill.
                            let pixels = painted_rects(&batch)
                                .zip(batch.bytes.chunks_exact(VERTEX_SIZE as usize * 6))
                                .filter(|(rect, _)| rect.contains(text_x + 0.25, y))
                                .collect::<Vec<_>>();
                            assert_eq!(
                                pixels.len(),
                                1,
                                "{label}, {width}, {size}, {scale}, {border}, {y}"
                            );
                            let alpha = f32::from_ne_bytes(pixels[0].1[20..24].try_into().unwrap());
                            assert_eq!(alpha, 0.8);
                        }
                        let outer = title_tab_bands(card, &tab, 0.0);
                        for (index, band) in outer.iter().enumerate() {
                            assert!(
                                outer[index + 1..]
                                    .iter()
                                    .all(|other| band.intersection(*other).is_none()),
                                "fill overlaps at {scale}"
                            );
                        }
                        for y in [
                            ((tab.top + card.bounds.top) / 2.0).floor() + 0.5,
                            card.bounds.top - 0.5,
                        ] {
                            let band = outer
                                .iter()
                                .find(|band| band.top <= y && band.bottom() > y)
                                .unwrap();
                            for (x, alpha) in
                                [(band.right() - 0.25, 0.8), (band.right() + 0.25, 1.0)]
                            {
                                let pixels = painted_rects(&batch)
                                    .zip(batch.bytes.chunks_exact(VERTEX_SIZE as usize * 6))
                                    .filter(|(rect, _)| rect.contains(x, y))
                                    .collect::<Vec<_>>();
                                assert_eq!(
                                    pixels.len(),
                                    1,
                                    "right curve/return at {x},{y}: {size}, {scale}"
                                );
                                assert_eq!(
                                    f32::from_ne_bytes(pixels[0].1[20..24].try_into().unwrap()),
                                    alpha
                                );
                            }
                        }
                        let cut = card.bounds.top + rounded_radius(card.bounds, card.radius);
                        let mut outline = RectangleBatch::new(width, 1800);
                        outline.push_title_tab_outline(card, &tab, stroke, palette::CARD_OUTLINE);
                        outline.clip = Some(SceneRect {
                            top: cut,
                            height: workspace.chrome.bottom() - cut,
                            ..workspace.chrome
                        });
                        outline.push_card_outline(
                            card.bounds,
                            card.radius,
                            &[],
                            stroke,
                            palette::CARD_OUTLINE,
                        );
                        let ink = painted_rects(&outline).collect::<Vec<_>>();
                        for x in [card.bounds.left + 0.5, card.bounds.right() - 0.5] {
                            for y in [cut - 0.5, cut + 0.5] {
                                assert!(
                                    ink.iter().any(|r| r.contains(x, y)),
                                    "cut seam at {x},{y}: {size}, {scale}, {border}"
                                );
                            }
                        }
                        let text = SceneRect {
                            left: text_x,
                            top: workspace.chrome.top,
                            width: tab.title.ink_width(metrics, stroke),
                            height: chrome_title_metrics(metrics).text_height,
                        };
                        assert!(
                            ink.iter()
                                .all(|r| r.intersection(text).is_none_or(|overlap| {
                                    let pixels = text_clip_bounds(overlap);
                                    pixels.left >= pixels.right || pixels.top >= pixels.bottom
                                })),
                            "stroke crosses text box: {size}, {scale}, {border}"
                        );
                    }
                }
            }
        }
    }
    let metrics = CellMetrics::for_scale(1.0);
    let (workspace, _) = retained_frame_text_clip(PhysicalSize::new(1280, 800));
    assert!(
        FrameTitleTab::popup(
            &mut clipping_test_fonts().font_system,
            metrics,
            &workspace,
            1.0
        )
        .is_none()
    );
}

#[test]
fn popup_tabs_clear_the_bar_and_text_or_keep_the_border_gap_title() {
    let mut fonts = clipping_test_fonts();
    let default_metrics = CellMetrics::for_scale(1.0);
    let default_workspace =
        popup_title_workspace(default_metrics, PhysicalSize::new(1280, 800), "Claude", 4.0);
    assert!(
        FrameTitleTab::popup(
            &mut fonts.font_system,
            default_metrics,
            &default_workspace,
            1.0
        )
        .is_some(),
        "the approved default contour must remain visible"
    );
    for scale in [1.3, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let stroke = physical_outline_width(1, metrics.scale);
        let mut workspace =
            popup_title_workspace(metrics, PhysicalSize::new(2400, 1800), "Claude", 4.0);
        let bar_bottom = workspace.chrome.top - stroke - 3.25;
        workspace.header.height = bar_bottom;
        workspace.tab_viewport.height = bar_bottom;
        let tab = FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke)
            .expect("fractional bar leaves room for the cap");
        assert!(tab.top >= bar_bottom.ceil() + 1.0);
        assert!(tab.top + stroke + 1.0 <= workspace.chrome.top);
        let card = workspace.card_fill(workspace.chrome, metrics, stroke);
        let mut batch = RectangleBatch::new(2400, 1800);
        push_title_tab_backdrop(
            &mut batch,
            SceneRect {
                left: 0.0,
                top: bar_bottom,
                width: 2400.0,
                height: 1800.0 - bar_bottom,
            },
            &[(Some(card), palette::TERMINAL_FALLBACK)],
            0.8,
            &[Some(tab.clone())],
        );
        for (rect, quad) in
            painted_rects(&batch).zip(batch.bytes.chunks_exact(VERTEX_SIZE as usize * 6))
        {
            if f32::from_ne_bytes(quad[20..24].try_into().unwrap()) == 0.8 {
                assert!(
                    rect.top >= bar_bottom,
                    "translucent fill over fractional bar: {rect:?}"
                );
            }
        }
        workspace.header.height = workspace.chrome.top;
        workspace.tab_viewport.height = workspace.chrome.top;
        assert!(
            FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke).is_none()
        );
        assert!(
            workspace.popup_title_row(metrics).is_some(),
            "zero-gap popup must keep its existing title"
        );
    }
    let metrics = CellMetrics {
        padding: 12.0,
        ..CellMetrics::for_scale(2.0)
    };
    let stroke = physical_outline_width(3, metrics.scale);
    assert!(metrics.padding / 3.0 < stroke);
    let workspace = roomy_popup_title_workspace(metrics, PhysicalSize::new(2400, 1800), "Claude");
    assert!(
        FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke).is_none(),
        "narrow padding cannot clear the thick stroke"
    );
    assert!(
        workspace.popup_title_row(metrics).is_some(),
        "thick-stroke fallback must retain the existing title"
    );
}

#[test]
fn popup_title_ink_can_overhang_its_advance_without_touching_the_stroke() {
    let mut fonts = FontSetup::new(&FontSettings {
        size: 96.0,
        ..FontSettings::default()
    })
    .unwrap();
    fonts
        .font_system
        .db_mut()
        .set_sans_serif_family("DejaVu Sans");
    let mut swash = SwashCache::new();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let workspace = roomy_popup_title_workspace(metrics, PhysicalSize::new(2400, 1800), "f");
        for border in [1, 2, 3] {
            let stroke = physical_outline_width(border, metrics.scale);
            let tab =
                FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke).unwrap();
            let title = chrome_title_metrics(metrics);
            let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
            let buffer = shape_styled_text(
                &mut fonts.font_system,
                metrics,
                cell_font,
                &tab.title.label,
                tab.title.width,
                title.text_height,
                DrawStyleKind::Status(glyphon::Wrap::None),
            );
            let left = workspace.chrome.left + metrics.padding;
            let clip = text_clip_bounds(SceneRect {
                left,
                top: workspace.chrome.top,
                width: tab.title.ink_width(metrics, stroke),
                height: title.text_height,
            });
            let card = workspace.card_fill(workspace.chrome, metrics, stroke);
            let mut outline = RectangleBatch::new(2400, 1800);
            outline.push_title_tab_outline(card, &tab, stroke, palette::CARD_OUTLINE);
            let stroke_pixels = painted_rects(&outline).collect::<Vec<_>>();
            let mut overhang = 0;
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    let physical = glyph.physical((left, workspace.chrome.top), 1.0);
                    let ink = swash
                        .get_image_uncached(&mut fonts.font_system, physical.cache_key)
                        .unwrap();
                    assert_eq!(ink.content, glyphon::SwashContent::Mask);
                    for (index, alpha) in ink.data.iter().enumerate() {
                        if *alpha == 0 {
                            continue;
                        }
                        let x = physical.x
                            + ink.placement.left
                            + (index as u32 % ink.placement.width) as i32;
                        let y = run.line_y.round() as i32 + physical.y - ink.placement.top
                            + (index as u32 / ink.placement.width) as i32;
                        if x as f32 + 0.5 >= left + tab.title.width {
                            overhang += 1;
                        }
                        assert!(
                            x < clip.right,
                            "glyph ink clipped: {scale}, {border}, x={x}, clip={clip:?}"
                        );
                        assert!(
                            !stroke_pixels
                                .iter()
                                .any(|r| r.contains(x as f32 + 0.5, y as f32 + 0.5)),
                            "glyph ink touches the curve: {scale}, {border}, {x},{y}"
                        );
                    }
                }
            }
            assert!(
                overhang > 0,
                "the regression must rasterize ink beyond the 'f' advance"
            );
        }
    }
}

#[test]
fn popup_fallback_keeps_rtl_ink_inside_its_measured_border_gap() {
    let mut fonts = clipping_test_fonts();
    fonts
        .font_system
        .db_mut()
        .set_sans_serif_family("DejaVu Sans");
    let mut swash = SwashCache::new();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let mut workspace =
            roomy_popup_title_workspace(metrics, PhysicalSize::new(2400, 1800), "שלום");
        workspace.header.height = workspace.chrome.top;
        workspace.tab_viewport.height = workspace.chrome.top;
        let stroke = physical_outline_width(3, metrics.scale);
        assert!(
            FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke).is_none()
        );
        let row = workspace.popup_title_row(metrics).unwrap();
        let available = row.width - metrics.padding * 2.0;
        let title = FrameTitle::fit(
            &mut fonts.font_system,
            metrics,
            workspace.popup_label().unwrap(),
            available,
        );
        assert_eq!(title.label, "שלום");
        assert!(
            title.width < available / 2.0,
            "layout must use the measured advance, not the full row"
        );
        let gap = title.border_gap(row, metrics);
        let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
        let buffer = shape_styled_text(
            &mut fonts.font_system,
            metrics,
            cell_font,
            &title.label,
            title.width,
            row.height,
            DrawStyleKind::Status(glyphon::Wrap::None),
        );
        let card = workspace.card_fill(workspace.chrome, metrics, stroke);
        let mut outline = RectangleBatch::new(2400, 1800);
        outline.clip = Some(workspace.chrome);
        outline.push_card_outline(
            card.bounds,
            card.radius,
            &[gap],
            stroke,
            palette::CARD_OUTLINE,
        );
        let stroke_pixels = painted_rects(&outline).collect::<Vec<_>>();
        let mut pixels = 0;
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((row.left + metrics.padding, row.top), 1.0);
                let ink = swash
                    .get_image_uncached(&mut fonts.font_system, physical.cache_key)
                    .unwrap();
                assert_eq!(ink.content, glyphon::SwashContent::Mask);
                for (index, alpha) in ink.data.iter().enumerate() {
                    if *alpha == 0 {
                        continue;
                    }
                    let x = physical.x
                        + ink.placement.left
                        + (index as u32 % ink.placement.width) as i32;
                    let y = run.line_y.round() as i32 + physical.y - ink.placement.top
                        + (index as u32 / ink.placement.width) as i32;
                    let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                    assert!(
                        gap.contains(x, y),
                        "RTL ink shifted outside its gap: {scale}, {x},{y}, {gap:?}"
                    );
                    assert!(
                        !stroke_pixels.iter().any(|r| r.contains(x, y)),
                        "fallback RTL ink crosses the frame"
                    );
                    pixels += 1;
                }
            }
        }
        assert!(pixels > 0, "fallback test must rasterize RTL glyphs");
    }
}

#[test]
fn zero_cursor_trail_duration_draws_only_the_unchanged_cursor() {
    let mut scene = padding_scene();
    scene.cursor = Some(DrawCursor {
        visible: true,
        blinking: false,
        password_input: false,
        shape: CursorShape::Block,
        column: 0,
        row: 0,
        at_wide_tail: false,
        color: SceneColor { r: 1, g: 2, b: 3 },
        explicit_color: false,
    });
    let metrics = CellMetrics::for_scale(1.0);
    let viewport = SceneRect {
        left: 0.0,
        top: 0.0,
        width: 100.0,
        height: 100.0,
    };
    let trail = SceneColor {
        r: 20,
        g: 80,
        b: 240,
    };
    let outline = cursor_outline(trail);
    for explicit in [false, true] {
        scene.cursor.as_mut().unwrap().explicit_color = explicit;
        let mut animation = CursorAnimation::default();
        for (column, row) in [(0, 0), (2, 1), (1, 0)] {
            let cursor = scene.cursor.as_mut().unwrap();
            cursor.column = column;
            cursor.row = row;
            let mut off = RectangleBatch::new(100, 100);
            assert!(!build_tail_cursor(
                &mut off,
                &mut animation,
                &scene,
                true,
                metrics,
                viewport,
                trail,
                outline,
                0.0,
                0.016,
            ));
            assert!(!animation.is_active());
            let mut stationary = RectangleBatch::new(100, 100);
            assert!(!build_tail_cursor(
                &mut stationary,
                &mut CursorAnimation::default(),
                &scene,
                true,
                metrics,
                viewport,
                trail,
                outline,
                1.5,
                0.0,
            ));
            assert!(!off.bytes.is_empty());
            assert_eq!(off.bytes, stationary.bytes);
        }
    }
}

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
    use eon_workspace_protocol::v39::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
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
                            workspace: "s1".into(),
                            active_workspace: "s1".into(),
                            workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
                            codex_quota_failure: None,
                            claude_quota_failure: None,
                            running: None,
                            busy: None,
                            sidebar: Sidebar::Open,
                            attention_queue: Vec::new(),
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
                                // The Stack band stays fixed while its segment joins the
                                // first pane's allocated title row.
                                let band = chrome_title_metrics(metrics).text_height
                                    + metrics.padding / 3.0;
                                assert!(
                                    (workspace.header_viewport.top
                                        - workspace.pane_viewport.top
                                        - band)
                                        .abs()
                                        < 0.001
                                );
                                assert_eq!(
                                    rules[0].0.title_y,
                                    Some(crate::scene::title_rule_y(
                                        panes[0].rect.top,
                                        panes[0].rect.height
                                    ))
                                );
                                let stack = workspace.stack_outline(&rules[0].0, fill);
                                assert_eq!(stack.title_y, Some(fill.bounds.top));
                                let mut rules = rules;
                                rules.push((stack, [SceneRect::default(); 2]));
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
                                    &[],
                                    None,
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
                                for offset in (0..border_width as u32).map(|n| n as f32 + 0.5) {
                                    for (x, y) in [
                                        (card.left + offset, rules[0].0.bounds.top - 0.5),
                                        (card.left + card.width / 2.0, fill.bounds.top + offset),
                                    ] {
                                        assert_eq!(
                                            color_at(&quads, x, y),
                                            frames.then_some(neutral),
                                            "the Stack segment stays neutral: scale={scale}, selected={selected}, x={x}, y={y}"
                                        );
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
                                    let top = workspace.header_viewport.top;
                                    for delta in [-workspace.card_radius(), -1.0, 1.0] {
                                        let scroll = rules[selected].0.bounds.top - top - delta;
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
                                            &[],
                                            None,
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
                                                top + delta.max(0.0) + 0.5
                                            ),
                                            (delta >= 0.0 || delta + focus_width > 0.5)
                                                .then_some(focus),
                                            "scrolling must clip the original top, not round/pin it at the viewport"
                                        );
                                        assert_eq!(
                                            color_at(
                                                &quads,
                                                frame.left + 0.5,
                                                frame.top.max(top) + workspace.card_radius() + 0.5
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
    use eon_workspace_protocol::v39::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
    let metrics = CellMetrics::for_scale(1.0);
    for (pane_count, expanded) in [(4, 1), (4, 4), (4, 2), (1, 1)] {
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: Sidebar::Open,
            attention_queue: Vec::new(),
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
            assert_eq!(current.pane_viewport.top, 52.0);
            assert_eq!(current.header_viewport.top, 76.0);
            assert_eq!(
                current.panes[0].rect.top,
                current.header_viewport.top + if current.panes[0].selected { 10.0 } else { 0.0 }
            );
            let shared_top = current.card_rect(current.chrome, metrics, 1.0).top;
            assert_eq!(shared_top, 62.0);
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
            if pane_count == 4 && expanded == 2 && !before_zoom && !after_zoom {
                assert_eq!(retained.rows, 29);
                assert_eq!(displayed.bottom() - metrics.height, 646.0);
                assert_eq!(displayed.bottom(), 664.0);
            }
            let paint = workspace_paint(&current, Some(&retained), &[], metrics);
            let cards = card_backdrops(&paint, Some(&retained), &[], metrics, 1.0);
            let fill = paint.card_fill(paint.chrome, metrics, 1.0);
            assert!(
                cards
                    .iter()
                    .all(|(card, _)| card.unwrap().bounds.bottom() == fill.bounds.bottom())
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
            if pane_count == 4 && expanded == 2 && !before_zoom && !after_zoom {
                assert_eq!(rules[1].0.bounds.bottom(), 665.0);
                let settled_fill = settled.card_fill(settled.chrome, metrics, 1.0);
                assert_eq!(
                    settled.pane_outlines(
                        &settled.panes,
                        settled.terminal,
                        settled_fill,
                        1.0,
                        metrics,
                        1.0
                    )[1]
                    .bounds
                    .bottom(),
                    662.0
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
                &[],
                None,
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
        snapshot.entries = vec![eon_workspace_protocol::v39::PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: eon_workspace_protocol::v39::PopupKey::Bound(
                eon_workspace_protocol::v39::Shortcut {
                    modifiers: eon_workspace_protocol::v39::ALT,
                    key: "KeyA".into(),
                },
            ),
        }];
        snapshot.tabs[0].popups = vec![eon_workspace_protocol::v39::Popup {
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
    use eon_workspace_protocol::v39::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
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
                            workspace: "s1".into(),
                            active_workspace: "s1".into(),
                            workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
                            codex_quota_failure: None,
                            claude_quota_failure: None,
                            running: None,
                            busy: None,
                            sidebar: Sidebar::Open,
                            attention_queue: Vec::new(),
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
                                        None,
                                        None,
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
                                card.unwrap().bounds.bottom(),
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
                            if let Some(visible) = grid
                                .intersection(workspace.header_viewport)
                                .and_then(|grid| {
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
                        // A stack that overflows (the 96 px font) keeps its terminal below the
                        // fold until it is scrolled to its end.
                        if height >= 800 && workspace.pane_scroll() == workspace.pane_scroll_limit()
                        {
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
                            &[],
                            None,
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
                                &[],
                                None,
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
                                                card.is_some_and(|card| {
                                                    card.bands().any(|band| band.contains(x, y))
                                                })
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
                        let bottom = cards[0].0.unwrap().bounds.bottom();
                        assert!(
                            cards
                                .iter()
                                .all(|(card, _)| card.unwrap().bounds.bottom() == bottom)
                        );
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
                                &[],
                                None,
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
        width: 380.0,
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
        for banded in [false, true] {
            let mut batch = RectangleBatch::new(400, 200);
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
            let fill = CardFill {
                bounds: viewport,
                radius: 8.0,
            };
            let tab = banded.then(|| PaneTitleTab {
                card: CardFill {
                    bounds: frame,
                    radius: 8.0,
                },
                tab: FrameTitleTab {
                    top: top - 14.0,
                    start: 85.0,
                    left_radius: 14.0,
                    row: label,
                    return_run: 21.0,
                    action: None,
                    title: FrameTitle {
                        label: "label".into(),
                        width: label.width,
                    },
                },
            });
            batch.push_pane_outlines(
                fill,
                viewport,
                &[(
                    PaneOutline {
                        bounds: frame,
                        clip: frame,
                        radius: rounded_radius(frame, 8.0),
                        title_y: Some(top),
                        expanded: true,
                    },
                    super::super::chrome::pane_rule_gaps(
                        label,
                        scrollback,
                        Some([
                            SceneRect {
                                left: 330.0,
                                width: 20.0,
                                ..label
                            },
                            SceneRect {
                                left: 350.0,
                                width: 20.0,
                                ..label
                            },
                        ]),
                        banded,
                    ),
                )],
                &[tab],
                None,
                Some(SceneColor::default()),
                1.0,
                1.0,
            );
            assert_eq!(batch.clip, Some(viewport));
            let quads = painted_rects(&batch).collect::<Vec<_>>();
            assert!(quads.iter().all(|rect| rect.top >= viewport.top - 0.001
                && rect.bottom() <= viewport.bottom() + 0.001));
            if top >= viewport.top {
                assert!(
                    !quads.iter().any(|rect| rect.contains(160.0, top + 0.5)),
                    "header stroke crosses scrollback, banded={banded}"
                );
                assert_eq!(
                    quads.iter().any(|rect| rect.contains(10.5, top + 0.5)),
                    banded
                );
                assert_eq!(
                    quads.iter().any(|rect| rect.contains(120.0, top + 0.5)),
                    !banded,
                    "the raised band must not retain the lowered middle rule"
                );
                assert!(!quads.iter().any(|rect| rect.contains(180.0, top + 0.5)));
                if banded && top - 14.0 >= viewport.top {
                    assert!(quads.iter().any(|rect| rect.contains(180.0, top - 13.5)));
                }
                assert!(!quads.iter().any(|rect| rect.contains(50.0, top + 0.5)));
            } else {
                assert!(
                    !quads
                        .iter()
                        .any(|rect| rect.contains(120.0, viewport.top + 0.5)),
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
    use eon_workspace_protocol::v39::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};

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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Closed,
        attention_queue: Vec::new(),
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
    use eon_workspace_protocol::v39::{
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
                        workspace: "s1".into(),
                        active_workspace: "s1".into(),
                        workspaces: vec![eon_workspace_protocol::v39::WorkspaceInfo {
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
                        codex_quota_failure: None,
                        claude_quota_failure: None,
                        running: None,
                        busy: None,
                        sidebar: Sidebar::Open,
                        attention_queue: Vec::new(),
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
                                            .intersection(workspace.header_viewport)
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
                            // Sidebar badges read the sidebar card's background from the end.
                            if let Some(area) = workspace.sidebar_area() {
                                assert_eq!(
                                    cards.last().unwrap().0.unwrap().bounds,
                                    workspace.card_rect(
                                        area,
                                        metrics,
                                        physical_outline_width(2, metrics.scale),
                                    )
                                );
                            }
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

#[test]
fn heading_action_notches_mirror_the_title_notch_without_double_blending() {
    use crate::HeadingAction;
    use eon_workspace_protocol::v39::Sidebar;
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let mut snapshot = popup_title_snapshot(
            "Files",
            chrome_title_metrics(metrics).text_height / metrics.scale,
        );
        snapshot.sidebar = Sidebar::Open;
        // Wide enough for the sidebar to show at every scale.
        let mut workspace = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(3200, 900),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        );
        // As `roomy_popup_title_workspace`: a full padding band above the titles.
        workspace.header.height = workspace
            .header
            .height
            .min(workspace.chrome.top - metrics.padding);
        workspace.tab_viewport.height = workspace.header.height;
        for border in [1, 3] {
            let stroke = physical_outline_width(border, metrics.scale);
            let area = workspace.sidebar_area().unwrap();
            for (action, tab, card) in [
                (
                    HeadingAction::HidePopup,
                    FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, stroke),
                    workspace.card_fill(workspace.chrome, metrics, stroke),
                ),
                (
                    HeadingAction::HideSidebar,
                    FrameTitleTab::sidebar(
                        &mut fonts.font_system,
                        metrics,
                        &workspace,
                        stroke,
                        stroke,
                    ),
                    CardFill {
                        bounds: workspace.card_rect(area, metrics, stroke),
                        radius: workspace.card_radius(),
                    },
                ),
            ] {
                let context = format!("{action:?} at scale {scale}, border {border}");
                let tab = tab.unwrap_or_else(|| panic!("no tab: {context}"));
                let target = workspace.heading_action(action).unwrap();
                let start = tab
                    .action
                    .unwrap_or_else(|| panic!("no action notch: {context}"));
                let mirror = |x: f32| card.bounds.left + card.bounds.right() - x;
                // The title's return reaches the top edge before the action notch's leaves it.
                let title_end = tab.start + tab.return_run;
                let action_begin = mirror(start + tab.return_run);
                assert!(title_end + stroke < action_begin, "{context}");
                for inset in [0.0, stroke] {
                    let bands = title_tab_bands(card, &tab, inset);
                    for (index, band) in bands.iter().enumerate() {
                        assert!(
                            bands[index + 1..]
                                .iter()
                                .all(|other| band.intersection(*other).is_none()),
                            "dual-notch fill must not blend a pixel twice: {context}"
                        );
                    }
                    // Inside the stroke too, the notch holds the button's pill at its top row.
                    let y = target.top + 0.5;
                    for x in [
                        target.left + metrics.padding / 2.0,
                        target.left + target.width / 2.0,
                        target.right() - metrics.padding / 2.0,
                    ] {
                        assert!(
                            bands.iter().any(|band| band.contains(x, y)),
                            "button outside its notch at {x}, inset {inset}: {context}"
                        );
                    }
                }
                // Above the card's top edge both notches fill and the edge between them is bare.
                let bands = title_tab_bands(card, &tab, 0.0);
                let above = card.bounds.top - 0.5;
                let filled = |x: f32| bands.iter().any(|band| band.contains(x, above));
                assert!(filled(card.bounds.left + metrics.padding), "{context}");
                assert!(filled(target.left + target.width / 2.0), "{context}");
                assert!(!filled((title_end + action_begin) / 2.0), "{context}");
                // The stroke follows the action notch's cap as it follows the title's.
                let outline = title_tab_outline_rects(card, &tab, stroke);
                for x in [
                    card.bounds.left + metrics.padding * 2.0,
                    target.left + target.width / 2.0,
                ] {
                    assert!(
                        outline
                            .iter()
                            .any(|(rect, _)| rect.contains(x, tab.top + 0.5)),
                        "no cap stroke at {x}: {context}"
                    );
                }
            }
        }
    }
}

#[test]
fn compact_heading_button_gap_keeps_the_titles_margin_and_clears_its_gap() {
    use crate::HeadingAction;
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let label = "A popup label long enough to be fitted against its Hide button gjpqy";
        let snapshot = popup_title_snapshot(
            label,
            chrome_title_metrics(metrics).text_height / metrics.scale,
        );
        for width in [420, 760, 1600] {
            let workspace = WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new(width, 900),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
            );
            let row = workspace.popup_title_row(metrics).unwrap();
            let hide = workspace.heading_action(HeadingAction::HidePopup).unwrap();
            let gap = chrome::heading_button_gap(hide, metrics);
            // The edge breaks half a padding before and after the button, as around a title.
            assert_eq!(gap.left, hide.left - metrics.padding / 2.0);
            assert_eq!(gap.right(), hide.right() + metrics.padding / 2.0);
            assert!(gap.right() <= row.right());
            // The compact title, fitted as the popup fits it, ends before the button's gap.
            let title = FrameTitle::fit(
                &mut fonts.font_system,
                metrics,
                label,
                (hide.left - row.left - metrics.padding * 2.0).max(0.0),
            );
            assert!(
                title.border_gap(row, metrics).right() <= gap.left,
                "title gap runs into the button's at scale {scale}, width {width}"
            );
        }
    }
}

#[test]
fn agents_title_uses_the_shared_tab_without_changing_sidebar_bounds() {
    use eon_workspace_protocol::v39::Sidebar;
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        for width in [1100, 1600] {
            let mut snapshot = popup_title_snapshot("Popup", 4.0);
            snapshot.sidebar = Sidebar::Open;
            let workspace = WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new(width, 900),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
            );
            let before = workspace.clone();
            let stroke = physical_outline_width(1, metrics.scale);
            let Some(area) = workspace.sidebar_area() else {
                continue;
            };
            let tab =
                FrameTitleTab::sidebar(&mut fonts.font_system, metrics, &workspace, stroke, stroke)
                    .expect("visible sidebar has title clearance");
            assert_eq!(tab.title.label, "Agents");
            assert!(tab.top > workspace.header.bottom());
            let card = CardFill {
                bounds: workspace.card_rect(area, metrics, stroke),
                radius: workspace.card_radius(),
            };
            assert_eq!(tab.return_run, (card.bounds.top - tab.top) * 1.5);
            assert!(tab.start + tab.return_run + stroke <= card.bounds.right() - card.radius);
            assert_eq!(workspace.sidebar_area(), before.sidebar_area());
            assert_eq!(workspace.sidebar_list, before.sidebar_list);
            assert_eq!(workspace.terminal, before.terminal);
        }
    }
}

#[test]
fn all_native_dialog_titles_fit_inside_existing_dismissal_bounds() {
    use crate::{DialogKind, DialogScene};
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        for width in [400, 1280] {
            for kind in [
                DialogKind::Shortcuts,
                DialogKind::AttentionInbox,
                DialogKind::WindowMenu,
                DialogKind::QuitOtherWindow {
                    name: "Other".into(),
                },
                DialogKind::CloseLastTab {
                    terminals: 3,
                    detach: Some("Alt+Shift+D".into()),
                    workspace: None,
                },
                DialogKind::QuitWindow {
                    terminals: Some(3),
                    running: 1,
                    detach: None,
                },
            ] {
                let viewer = DialogScene::new(
                    kind,
                    vec![],
                    PhysicalSize::new(width, 800),
                    metrics,
                    0.0,
                    &crate::CellText(metrics),
                );
                let before = viewer.clone();
                for border in [1, 2, 3] {
                    let stroke = physical_outline_width(border, metrics.scale);
                    let (card, tab) =
                        FrameTitleTab::dialog(&mut fonts.font_system, metrics, &viewer, stroke)
                            .expect("normal and narrow dialogs have existing title clearance");
                    assert!(tab.top >= viewer.bounds.top);
                    assert_eq!(tab.return_run, (card.bounds.top - tab.top) * 1.5);
                    assert_eq!(card.radius, crate::scene::tokens::card_gap(metrics));
                    assert!(
                        tab.start + tab.return_run + stroke <= card.bounds.right() - card.radius
                    );
                    for band in title_tab_bands(card, &tab, 0.0) {
                        assert!(band.left >= viewer.bounds.left - 0.001);
                        assert!(band.right() <= viewer.bounds.right() + 0.001);
                        assert!(band.top >= viewer.bounds.top.floor());
                        assert!(band.bottom() <= viewer.bounds.bottom().ceil());
                    }
                    assert_eq!(
                        viewer, before,
                        "title rendering must leave action/content geometry alone"
                    );
                }
            }
        }
    }
}

#[test]
fn short_dialogs_fall_back_before_their_title_contour_leaves_dismissal_bounds() {
    use crate::{DialogKind, DialogScene};
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    let metrics = fonts.metrics(1.0);
    for height in [1, 16, 32, 64, 200] {
        let viewer = DialogScene::new(
            DialogKind::Shortcuts,
            vec![],
            PhysicalSize::new(400, height),
            metrics,
            0.0,
            &crate::CellText(metrics),
        );
        if let Some((card, tab)) =
            FrameTitleTab::dialog(&mut fonts.font_system, metrics, &viewer, 1.0)
        {
            for band in title_tab_bands(card, &tab, 0.0) {
                assert!(
                    band.top >= viewer.bounds.top.floor()
                        && band.bottom() <= viewer.bounds.bottom().ceil(),
                    "title contour outside dismissal bounds at height {height}: {band:?}, {:?}",
                    viewer.bounds
                );
            }
        }
    }
}

#[test]
fn chosen_title_return_starts_beside_the_fitted_label_and_spans_one_and_a_half_rises() {
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    let metrics = fonts.metrics(1.0);
    for width in [400, 1280] {
        let workspace = roomy_popup_title_workspace(
            metrics,
            PhysicalSize::new(width, 800),
            "A deliberately long popup title that needs fitting in the narrow window",
        );
        let tab = FrameTitleTab::popup(&mut fonts.font_system, metrics, &workspace, 1.0).unwrap();
        let card = workspace.card_fill(workspace.chrome, metrics, 1.0);
        let text_end = card.bounds.left + metrics.padding + tab.title.width;
        assert_eq!(tab.return_run, (card.bounds.top - tab.top) * 1.5);
        assert!(
            (tab.start - text_end).abs() <= metrics.padding / 2.0,
            "curve must start beside visible text, not after its extra run: {}",
            tab.start - text_end
        );
    }
}

#[test]
fn empty_columns_keep_title_and_backdrop_slots_aligned() {
    use eon_workspace_protocol::v39::{Column, Pane};
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    let metrics = fonts.metrics(1.0);
    let mut snapshot = popup_title_snapshot("Popup", 4.0);
    let tab = &mut snapshot.tabs[0];
    tab.selected_popup = None;
    tab.selected_pane = Some("p0".into());
    tab.columns = (0..3)
        .map(|index| Column {
            expanded: format!("p{index}"),
            panes: vec![Pane {
                id: format!("p{index}"),
                label: format!("Pane {index}"),
                session: format!("s{index}"),
                endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                pinned_name: None,
                live: true,
                attention: None,
            }],
        })
        .collect();
    for empty in 0..3 {
        let mut workspace = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1600, 800),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        );
        if empty == 0 {
            workspace.panes.clear();
        } else {
            workspace.other_columns[empty - 1].panes.clear();
        }
        let titles = FrameTitleTab::workspace(
            &mut fonts.font_system,
            metrics,
            &workspace,
            true,
            (1.0, 1.0),
        );
        let cards = card_backdrops(&workspace, None, &[None, None], metrics, 1.0);
        assert_eq!(titles.len(), 3);
        assert_eq!(cards.len(), 3);
        for (index, (title, (card, _))) in titles.iter().zip(&cards).enumerate() {
            if index == empty {
                assert!(title.is_none());
                assert!(card.is_none());
            } else {
                let title = title.as_ref().unwrap();
                let card = card.unwrap();
                assert_eq!(title.title.label, format!("Stack {}", index + 1));
                assert_eq!(title.row.left, card.bounds.left);
            }
        }
        let mut batch = RectangleBatch::new(1600, 800);
        push_title_tab_backdrop(&mut batch, workspace.pane_viewport, &cards, 0.8, &titles);
        for (title, (card, _)) in titles.iter().zip(&cards) {
            if let (Some(title), Some(card)) = (title, card) {
                assert!(painted_rects(&batch).any(|rect| {
                    rect.contains(card.bounds.left + metrics.padding, title.top + 0.5)
                }));
            }
        }
    }
}

#[test]
fn clipped_sidebar_title_falls_back_when_its_card_edge_does_not_align() {
    use eon_workspace_protocol::v39::Sidebar;
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    let metrics = fonts.metrics(1.0);
    let mut snapshot = popup_title_snapshot("Popup", 4.0);
    snapshot.sidebar = Sidebar::Open;
    let mut workspace = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(1600, 800),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
    );
    workspace.pane_viewport.height =
        crate::scene::tokens::chrome_title_metrics(metrics).text_height / 2.0;
    let area = workspace.sidebar_area().unwrap();
    assert_ne!(
        workspace.card_rect(area, metrics, 1.0).top,
        crate::scene::title_rule_y(
            area.top,
            crate::scene::tokens::chrome_title_metrics(metrics).text_height
        )
    );
    assert!(
        FrameTitleTab::sidebar(&mut fonts.font_system, metrics, &workspace, 1.0, 1.0).is_none()
    );
}

#[test]
fn fallback_dialog_frame_uses_configured_stroke_at_fractional_scale() {
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    let metrics = fonts.metrics(1.3);
    let viewer = crate::DialogScene::new(
        crate::DialogKind::Shortcuts,
        vec![],
        PhysicalSize::new(64, 300),
        metrics,
        0.0,
        &crate::CellText(metrics),
    );
    let stroke = physical_outline_width(3, metrics.scale);
    let tab = FrameTitleTab::dialog(&mut fonts.font_system, metrics, &viewer, stroke);
    assert!(tab.is_none(), "narrow dialog must exercise the fallback");
    assert_eq!(stroke, 4.0);
    let mut batch = RectangleBatch::new(400, 300);
    batch.push_dialog_frame(
        viewer.bounds,
        metrics.padding.min(viewer.bounds.width / 8.0),
        tab.as_ref(),
        stroke,
    );
    let ink = painted_rects(&batch)
        .zip(batch.bytes.chunks_exact(VERTEX_SIZE as usize * 6))
        .filter_map(|(rect, quad)| {
            let channel = |offset| f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap());
            let color = palette::CARD_OUTLINE;
            (channel(8) == f32::from(color.r) / 255.0
                && channel(12) == f32::from(color.g) / 255.0
                && channel(16) == f32::from(color.b) / 255.0)
                .then_some(rect)
        })
        .collect::<Vec<_>>();
    let y = viewer.bounds.top + viewer.bounds.height / 2.0;
    assert!(
        ink.iter()
            .any(|r| r.contains(viewer.bounds.left + stroke - 0.5, y))
    );
    assert!(
        !ink.iter()
            .any(|r| r.contains(viewer.bounds.left + stroke + 0.5, y))
    );
}

#[test]
fn dialog_title_edge_keeps_a_clear_row_below_bar_chips() {
    let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
    for scale in [1.0, 1.3, 2.0] {
        let metrics = fonts.metrics(scale);
        let size = PhysicalSize::new(1280, 800);
        let workspace = popup_title_workspace(metrics, size, "Popup", 4.0);
        let viewer = crate::DialogScene::new(
            crate::DialogKind::Shortcuts,
            vec![],
            size,
            metrics,
            0.0,
            &crate::CellText(metrics),
        );
        let (card, _) = FrameTitleTab::dialog(
            &mut fonts.font_system,
            metrics,
            &viewer,
            physical_outline_width(1, metrics.scale),
        )
        .unwrap();
        assert!(
            card.bounds.top >= workspace.header.bottom().ceil() + 1.0,
            "dialog edge {} touches bar bottom {} at scale {scale}",
            card.bounds.top,
            workspace.header.bottom()
        );
    }
}

#[test]
fn pane_headers_clear_adjacent_descenders_and_preserve_joined_focus_strokes() {
    use eon_workspace_protocol::v39::{Column, Pane};
    let mut fonts = clipping_test_fonts();
    let mut swash = SwashCache::new();
    // Fractional title rows make the adjacent rules and cap snap at different pixel phases.
    for (scale, font_size) in [
        (1.0, 16.0),
        (1.125, 16.0),
        (1.175, 16.0),
        (1.3, 16.0),
        (4.0 / 3.0, 16.0),
        (2.0, 16.0),
        (1.25, 17.0),
        (1.5, 17.0),
        (1.75, 16.3),
        (1.75, 16.8),
    ] {
        let mut metrics = CellMetrics::for_scale(scale);
        metrics.font_size = font_size * scale as f32;
        metrics.height = (metrics.font_size * 1.125).round();
        for selected in 0..4 {
            for width in [760, 1280] {
                let mut snapshot = popup_title_snapshot("Popup", 4.0);
                let tab = &mut snapshot.tabs[0];
                tab.selected_popup = None;
                tab.popups.clear();
                tab.selected_pane = Some(format!("p{selected}"));
                tab.columns = vec![Column {
                    expanded: format!("p{selected}"),
                    panes: (0..4)
                        .map(|index| Pane {
                            id: format!("p{index}"),
                            label: format!("p{index}  gjpqy {}", "long/path/".repeat(8)),
                            pinned_name: None,
                            session: format!("s{index}"),
                            endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        })
                        .collect(),
                }];
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(width, 800),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                );
                let border = physical_outline_width(1, metrics.scale);
                let focus = physical_outline_width(2, metrics.scale);
                let fill = workspace.card_fill(workspace.chrome, metrics, focus);
                let outlines = workspace.pane_outlines(
                    &workspace.panes,
                    workspace.terminal,
                    fill,
                    border,
                    metrics,
                    focus,
                );
                let right = workspace.panes[selected].rect.right() - metrics.padding * 3.0;
                let tabs = PaneTitleTab::workspace(
                    &mut fonts.font_system,
                    &mut swash,
                    metrics,
                    &workspace,
                    (border, focus),
                    Some(right),
                )
                .remove(0);
                assert_eq!(tabs.len(), workspace.panes.len());
                let exact_pitch = scale == 1.0 || scale == 2.0;
                let tolerance = if exact_pitch { 0.0 } else { 1.0 };
                if selected == 0 {
                    let cap = tabs[selected].as_ref().unwrap().tab.top;
                    let pitch = chrome_title_metrics(metrics).text_height.floor();
                    let gap = cap - fill.bounds.top;
                    assert!(
                        (gap - pitch).abs() <= tolerance,
                        "first band gap {gap} differs from the collapsed pitch {pitch} \
                         (scale {scale}, font {font_size})"
                    );
                } else if let Some(above) = selected.checked_sub(1) {
                    let cap = tabs[selected].as_ref().unwrap().tab.top;
                    assert!(
                        cap >= workspace.panes[above].rect.bottom(),
                        "the band's cap must stay out of the collapsed title row above it \
                         (scale {scale}, font {font_size}, pane {selected})"
                    );
                    if let Some(earlier) = above.checked_sub(1) {
                        let previous_rule = outlines[above].title_y.unwrap();
                        let pitch = previous_rule - outlines[earlier].title_y.unwrap();
                        let gap = cap - previous_rule;
                        assert!(
                            (gap - pitch).abs() <= tolerance,
                            "upper gap {gap} differs from the collapsed pitch {pitch} \
                             (scale {scale}, font {font_size}, pane {selected})"
                        );
                    }
                }
                if let Some(next) = outlines.get(selected + 1)
                    && (selected == 0 || exact_pitch)
                {
                    let pitch = workspace.panes[selected + 1].rect.height;
                    // The closing rule occupies the inside edge of the Stack's bottom stroke.
                    let expected = if selected + 2 == outlines.len() {
                        pitch - border
                    } else {
                        pitch
                    };
                    let gap = next.title_y.unwrap() - outlines[selected].bounds.bottom();
                    assert!(
                        (gap - expected).abs() <= tolerance,
                        "lower gap {gap} differs from the expected pitch {expected} \
                         (scale {scale}, font {font_size}, pane {selected})"
                    );
                }
                for (pane, notch) in workspace.panes.iter().zip(&tabs) {
                    if !pane.selected {
                        assert!(notch.is_none(), "a collapsed title must stay compact");
                    } else {
                        let fitted = &notch
                            .as_ref()
                            .expect("visible header must have a raised band")
                            .tab;
                        let limit = if pane.selected {
                            right
                        } else {
                            pane.rect.right()
                        };
                        assert!(
                            fitted.start + fitted.return_run <= limit,
                            "title fitting enters the reserved right slot"
                        );
                        if let Some(actions) = pane.actions {
                            assert!(
                                fitted.start + fitted.return_run < actions[0].left,
                                "title fitting must clear the actual action slot"
                            );
                            let header = notch.as_ref().unwrap();
                            let middle = SceneRect {
                                left: fitted.start + fitted.return_run,
                                top: fitted.top,
                                width: 1.0,
                                height: 1.0,
                            };
                            assert!(
                                header.bands(0.0).iter().any(|band| {
                                    band.contains(middle.left + 0.5, middle.top + 0.5)
                                }),
                                "the pane header must fill the valley between title and actions"
                            );
                            let bands = header.bands(0.0);
                            for (i, band) in bands.iter().enumerate() {
                                assert!(
                                    bands[i + 1..]
                                        .iter()
                                        .all(|other| band.intersection(*other).is_none()),
                                    "header band must not blend a pixel twice"
                                );
                            }
                        }
                    }
                }
                let mut ink = Vec::new();
                let rules = outlines
                    .into_iter()
                    .zip(&workspace.panes)
                    .enumerate()
                    .map(|(index, (outline, pane))| {
                        let row = if index == 3 && !pane.selected {
                            super::super::chrome::bottom_title_row(
                                pane.rect,
                                outline.bounds.bottom(),
                                chrome_title_metrics(metrics).text_height,
                            )
                        } else {
                            pane.rect
                        };
                        let title = tabs[index]
                            .as_ref()
                            .map(|tab| tab.tab.title.clone())
                            .unwrap_or_else(|| {
                                FrameTitle::fit(
                                    &mut fonts.font_system,
                                    metrics,
                                    pane.label(),
                                    pane.header_content(metrics).width - 2.0 * metrics.padding,
                                )
                            });
                        let (buffer, _) = super::super::chrome::status_glyph_ink(
                            &mut fonts.font_system,
                            &mut swash,
                            metrics,
                            &title.label,
                        );
                        for run in buffer.layout_runs() {
                            for glyph in run.glyphs {
                                let physical =
                                    glyph.physical((row.left + metrics.padding, row.top), 1.0);
                                let Some(image) =
                                    swash.get_image(&mut fonts.font_system, physical.cache_key)
                                else {
                                    continue;
                                };
                                if image.placement.width > 0 && image.placement.height > 0 {
                                    ink.push(SceneRect {
                                        left: physical.x as f32 + image.placement.left as f32,
                                        top: run.line_y.round() + physical.y as f32
                                            - image.placement.top as f32,
                                        width: image.placement.width as f32,
                                        height: image.placement.height as f32,
                                    });
                                }
                            }
                        }
                        (
                            outline,
                            super::super::chrome::pane_rule_gaps(
                                title.border_gap(row, metrics),
                                SceneRect::default(),
                                pane.visible_actions(),
                                tabs[index].is_some(),
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut batch = RectangleBatch::new(width, 800);
                batch.push_pane_outlines(
                    fill,
                    workspace.pane_viewport,
                    &rules,
                    &tabs,
                    Some(selected),
                    Some(palette::PRIMARY_TEXT),
                    border,
                    focus,
                );
                let strokes = painted_rects(&batch).collect::<Vec<_>>();
                for ((outline, _), pane) in rules.iter().zip(&workspace.panes) {
                    if pane.visible_actions().is_none() {
                        let target = pane.actions.unwrap()[0];
                        assert!(
                            strokes.iter().any(|rect| rect.contains(
                                target.left + target.width / 2.0,
                                outline.title_y.unwrap() + 0.5,
                            )),
                            "hidden button gap: scale={scale}, font={font_size}, selected={selected}, pane={}",
                            pane.id
                        );
                    }
                }
                for rect in &strokes {
                    // Decoding NDC vertices can move an exact raster boundary by a few ulps.
                    let collision = ink
                        .iter()
                        .filter_map(|ink| rect.intersection(*ink))
                        .find(|overlap| overlap.width > 0.01 && overlap.height > 0.01);
                    assert!(
                        collision.is_none(),
                        "header stroke crosses title ink: scale={scale}, selected={selected}, rect={rect:?}, overlap={collision:?}"
                    );
                }
                let notch = tabs[selected].as_ref().unwrap();
                let point = SceneRect {
                    left: notch.card.bounds.left + metrics.padding * 2.0,
                    top: notch.tab.top,
                    width: 1.0,
                    height: 1.0,
                };
                assert!(
                    strokes
                        .iter()
                        .any(|rect| rect.intersection(point).is_some()),
                    "raised header edge not drawn"
                );
                assert!(
                    strokes
                        .iter()
                        .all(|rect| rect.bottom() <= fill.bounds.bottom() + 0.001),
                    "pane stroke enters the footer"
                );
                let accent_color = [palette::ACCENT.r, palette::ACCENT.g, palette::ACCENT.b]
                    .map(|value| f32::from(value) / 255.0);
                let title_focus = painted_rects(&batch)
                    .zip(batch.bytes.chunks_exact(VERTEX_SIZE as usize * 6))
                    .filter(|(_, bytes)| {
                        let color: [f32; 3] = std::array::from_fn(|index| {
                            let start = (index + 2) * 4;
                            f32::from_ne_bytes(bytes[start..start + 4].try_into().unwrap())
                        });
                        color == accent_color
                    })
                    .map(|(rect, _)| rect)
                    .collect::<Vec<_>>();
                assert!(!title_focus.is_empty(), "title edge must retain F6 focus");
                assert!(
                    title_focus
                        .iter()
                        .all(|rect| rect.right() <= notch.tab.start + notch.tab.return_run + 0.001),
                    "title focus must stop before the empty middle and action group"
                );
            }
        }
    }
}

#[test]
fn first_pane_bands_keep_stack_clearance_in_selected_and_unselected_columns() {
    use eon_workspace_protocol::v39::Column;
    let mut fonts = clipping_test_fonts();
    let mut swash = SwashCache::new();
    for scale in [1.0, 1.125, 4.0 / 3.0, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for count in [1, 3] {
            for focused in [true, false] {
                let mut snapshot = popup_title_snapshot("Popup", 4.0);
                let tab = &mut snapshot.tabs[0];
                tab.selected_popup = None;
                tab.popups.clear();
                let pane = eon_workspace_protocol::v39::Pane {
                    id: "other".into(),
                    label: "other".into(),
                    pinned_name: None,
                    session: "other".into(),
                    endpoint: b"/tmp/other.sock".to_vec(),
                    live: true,
                    attention: None,
                };
                tab.columns = vec![Column {
                    expanded: "p0".into(),
                    panes: (0..count)
                        .map(|index| eon_workspace_protocol::v39::Pane {
                            id: format!("p{index}"),
                            label: format!("p{index} gjpqy"),
                            session: format!("s{index}"),
                            endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                            ..pane.clone()
                        })
                        .collect(),
                }];
                tab.selected_pane = Some(if focused { "p0" } else { "other" }.into());
                if !focused {
                    tab.columns.push(Column {
                        expanded: "other".into(),
                        panes: vec![pane],
                    });
                }
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(1600, 900),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                );
                let column = usize::from(!focused);
                let (area, _, panes) = workspace.column_cards().nth(column).unwrap();
                assert_eq!(panes[0].id, "p0");
                let terminal = if focused {
                    workspace.terminal
                } else {
                    workspace.other_columns[0].terminal
                };
                let border = physical_outline_width(1, scale as f32);
                let focus = physical_outline_width(2, scale as f32);
                let stroke = if focused { focus } else { border };
                let fill = workspace.card_fill(area, metrics, focus);
                let tabs = PaneTitleTab::workspace(
                    &mut fonts.font_system,
                    &mut swash,
                    metrics,
                    &workspace,
                    (border, focus),
                    None,
                )
                .remove(column);
                let cap = tabs[0].as_ref().unwrap().tab.top;
                let gap = cap - fill.bounds.top;
                let pitch = chrome_title_metrics(metrics).text_height.floor();
                let tolerance = if scale == 1.0 || scale == 2.0 {
                    0.0
                } else {
                    1.0
                };
                assert!(
                    (gap - pitch).abs() <= tolerance,
                    "first gap {gap}, pitch {pitch}, scale {scale}, panes {count}, focused {focused}"
                );
                let rules = workspace
                    .pane_outlines(panes, terminal, fill, border, metrics, stroke)
                    .into_iter()
                    .map(|outline| (outline, [SceneRect::default(); 2]))
                    .collect::<Vec<_>>();
                let mut batch = RectangleBatch::new(1600, 900);
                batch.push_pane_outlines(
                    fill,
                    workspace.pane_viewport,
                    &rules,
                    &tabs,
                    focused.then_some(0),
                    Some(palette::PRIMARY_TEXT),
                    border,
                    stroke,
                );
                assert!(
                    painted_rects(&batch).any(|rect| rect
                        .contains(panes[0].rect.right() - metrics.padding * 3.0, cap + 0.5,)),
                    "first band's stroke must be drawn at the measured cap"
                );
            }
        }
    }
}

#[test]
fn pane_title_cache_keys_geometry_typography_strokes_and_scrollback() {
    let metrics = CellMetrics::for_scale(1.0);
    let (workspace, _) = retained_frame_text_clip(PhysicalSize::new(1280, 800));
    let cache = PaneTitleCache {
        workspace: workspace.clone(),
        metrics,
        widths: (1.0, 2.0),
        scrollback: None,
        tabs: Vec::new(),
    };
    assert!(cache.matches(&workspace, metrics, (1.0, 2.0), &None));
    assert!(!cache.matches(&workspace, metrics, (2.0, 2.0), &None));
    assert!(!cache.matches(&workspace, CellMetrics::for_scale(1.3), (1.0, 2.0), &None));
    assert!(!cache.matches(&workspace, metrics, (1.0, 2.0), &Some("↓ 10 rows".into())));
    let mut changed = workspace.clone();
    changed.panes[0].id = "p2".into();
    assert!(!cache.matches(&changed, metrics, (1.0, 2.0), &None));
    changed = workspace.clone();
    changed.header_viewport.top += 1.0;
    assert!(!cache.matches(&changed, metrics, (1.0, 2.0), &None));
}
