use super::*;

#[test]
fn bounded_preview_rows_cover_multi_row_fractional_offsets() {
    let metrics = CellMetrics {
        scale: 1.0,
        width: 10.0,
        height: 20.0,
        font_size: 16.0,
        title_cap_top_em: None,
        padding: 5.0,
    };
    let scene = Scene {
        revision: 7,
        columns: 1,
        rows: 2,
        screen: Screen::Primary,
        title: String::new(),
        working_directory: String::new(),
        graphics: SceneGraphics::default(),
        background: palette::TERMINAL_FALLBACK,
        foreground: SceneColor::default(),
        cursor: None,
        content: Vec::new(),
    };
    let row = DrawRow {
        wrapped: false,
        wrap_continuation: false,
        cells: Vec::new(),
    };
    let viewport = SceneRect {
        left: 0.0,
        top: 0.0,
        width: 20.0,
        height: 50.0,
    };
    let grid = scene_grid(&scene, viewport, metrics);
    assert!(row_intersects_clip(
        grid.top - metrics.height / 2.0,
        metrics.height,
        grid
    ));
    assert!(!row_intersects_clip(grid.bottom(), metrics.height, grid));

    for (direction, offset) in [
        (VerticalDirection::Up, 3.0 * metrics.height - 0.25),
        (VerticalDirection::Down, -3.0 * metrics.height + 0.25),
    ] {
        let preview = ScenePreview::Viewport {
            frame_revision: scene.revision,
            direction,
            edge_reached: false,
            rows: vec![row.clone(); 3],
        };
        let (actual_direction, rows) = preview_rows(Some(&preview)).unwrap();
        assert_eq!(rows.len(), 3);
        let origin = shifted(viewport, offset);
        let first = preview_row_top(&scene, actual_direction, 0, metrics, origin);
        let top = preview_row_top(&scene, actual_direction, 2, metrics, origin);
        assert_eq!((top - first).abs(), 2.0 * metrics.height);
        assert!(
            (SceneRect {
                left: grid.left,
                top,
                width: grid.width,
                height: metrics.height,
            })
            .intersection(grid)
            .is_some()
        );
    }
}

#[test]
fn retained_frame_text_preserves_padding_after_resize() {
    let metrics = CellMetrics::for_scale(1.0);
    let (workspace, clip) = retained_frame_text_clip(PhysicalSize::new(1014, 900));
    let fill = workspace.card_fill(
        workspace.chrome,
        metrics,
        physical_outline_width(2, metrics.scale),
    );
    assert_eq!(
        fill.bounds,
        SceneRect {
            left: 8.0,
            top: 56.0,
            width: 998.0,
            height: 826.0,
        }
    );
    let mut fonts = clipping_test_fonts();
    let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
    let mut text = Vec::new();
    for column in [96, 98] {
        let cell = SceneRect {
            left: workspace.terminal.left + metrics.padding + column as f32 * metrics.width,
            top: workspace.terminal.top + metrics.padding + 44.0 * metrics.height,
            width: metrics.width,
            height: metrics.height,
        };
        let attrs = cell_font.attrs();
        let box_attrs = attrs
            .clone()
            .metrics(Metrics::new(cell_font.box_size, metrics.height))
            .letter_spacing(cell_font.box_letter_spacing);
        let mut buffer = Buffer::new(
            &mut fonts.font_system,
            Metrics::new(cell_font.size, metrics.height),
        );
        buffer.set_size(Some(metrics.width), Some(metrics.height));
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        set_cell_layer(
            &mut buffer,
            "│",
            &cell_text_segments("│"),
            &attrs,
            &attrs,
            &box_attrs,
            true,
        );
        buffer.shape_until_scroll(&mut fonts.font_system, false);
        if column == 98 {
            assert_eq!((cell.left, cell.top), (1000.0, 870.0));
            let run = buffer.layout_runs().next().unwrap();
            let physical = run.glyphs[0].physical((cell.left, cell.top), 1.0);
            let image = SwashCache::new()
                .get_image_uncached(&mut fonts.font_system, physical.cache_key)
                .unwrap();
            let ink_left = physical.x + image.placement.left;
            let ink_top = run.line_y.round() as i32 + physical.y - image.placement.top;
            let x = 1004 - ink_left;
            let y = 881 - ink_top;
            assert!(x >= 0 && x < image.placement.width as i32);
            assert!(y >= 0 && y < image.placement.height as i32);
            assert_ne!(
                image.data[y as usize * image.placement.width as usize + x as usize],
                0
            );
            assert!(!fill.bands().any(|band| band.contains(1004.5, 881.5)));
        }
        if let Some(bounds) = cell.intersection(clip) {
            text.push(PlacedText {
                buffer,
                custom_glyph: None,
                left: cell.left,
                top: cell.top,
                right: text_clip_bounds(bounds).right,
                bottom: text_clip_bounds(bounds).bottom,
                bound_left: text_clip_bounds(bounds).left,
                bound_top: text_clip_bounds(bounds).top,
                color: Color::rgb(255, 255, 255),
            });
        }
    }
    let areas = text_areas(&text, None).collect::<Vec<_>>();
    assert!(
        areas
            .iter()
            .all(|area| area.bounds.right <= 1004 || area.bounds.bottom <= 881),
        "retained-frame text admits ink at (1004,881) outside the rounded fill"
    );
    assert_eq!(areas.len(), 1, "the inset interior glyph must survive");
    assert!(areas.iter().all(|area| {
        area.bounds.left as f32 >= workspace.terminal.left + metrics.padding
            && area.bounds.right as f32 <= workspace.terminal.right() - metrics.padding
    }));
}

#[test]
fn retained_frame_text_does_not_paint_in_an_empty_padded_interior() {
    let metrics = CellMetrics::for_scale(1.0);
    for width in [24, 23, 1] {
        let (workspace, clip) = retained_frame_text_clip(PhysicalSize::new(width, 300));
        assert!(workspace.terminal.width <= metrics.padding * 2.0);
        let cell = SceneRect {
            left: workspace.terminal.left + metrics.padding,
            top: workspace.terminal.top + metrics.padding,
            width: metrics.width,
            height: metrics.height,
        };
        assert!(
            cell.intersection(clip).is_none(),
            "no retained text may paint when the padded interior is empty: width {width}"
        );
    }
}

#[test]
fn fractional_text_bounds_admit_only_pixel_centers_inside_the_clip() {
    for edge in [-1.75, -1.5, -1.25, 0.0, 0.25, 0.5, 0.75, 1.0] {
        for width in [0.25, 1.0, 2.5] {
            let clip = SceneRect {
                left: edge,
                top: edge,
                width,
                height: width,
            };
            let bounds = text_clip_bounds(clip);
            for y in -4..6 {
                for x in -4..6 {
                    assert_eq!(
                        x >= bounds.left
                            && x < bounds.right
                            && y >= bounds.top
                            && y < bounds.bottom,
                        clip.contains(x as f32 + 0.5, y as f32 + 0.5),
                        "pixel ({x},{y}), clip {clip:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn fractional_card_text_has_no_exterior_ink_with_hints_and_header_scroll() {
    let mut fonts = clipping_test_fonts();
    let mut swash = SwashCache::new();
    let mut exterior_ink = Vec::new();
    for (scale, bottom_scroll) in [(1.25, 247.5), (1.3, 300.0)] {
        let metrics = CellMetrics::for_scale(scale);
        let workspace = fractional_card_workspace(scale, bottom_scroll);
        let fill = workspace
            .card_fill(
                workspace.chrome,
                metrics,
                physical_outline_width(2, metrics.scale),
            )
            .bounds;
        if scale == 1.25 {
            assert_eq!(fill.bottom(), 857.5);
        }
        let top_scroll = bottom_scroll + workspace.terminal.top + metrics.padding
            - (fill.top - metrics.height / 2.0);
        for (scroll, at_bottom) in [(bottom_scroll, true), (top_scroll, false)] {
            let workspace = fractional_card_workspace(scale, scroll);
            let (clip, _) = terminal_paint_clip(
                &workspace,
                workspace.chrome,
                workspace.terminal,
                metrics,
                true,
                physical_outline_width(2, metrics.scale),
            )
            .unwrap();
            let clip = terminal_text_clip(workspace.terminal, clip, metrics, true);
            let cell = SceneRect {
                left: workspace.terminal.left + metrics.padding,
                top: workspace.terminal.top + metrics.padding,
                width: metrics.width,
                height: metrics.height,
            };
            let bounds = text_clip_bounds(cell.intersection(clip).unwrap());
            let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
            let attrs = cell_font.attrs();
            let box_attrs = attrs
                .clone()
                .metrics(Metrics::new(cell_font.box_size, metrics.height))
                .letter_spacing(cell_font.box_letter_spacing);
            let mut buffer = Buffer::new(
                &mut fonts.font_system,
                Metrics::new(cell_font.size, metrics.height),
            );
            buffer.set_size(Some(metrics.width), Some(metrics.height));
            buffer.set_wrap(Wrap::None);
            buffer.set_monospace_width(Some(metrics.width));
            set_cell_layer(
                &mut buffer,
                "│",
                &cell_text_segments("│"),
                &attrs,
                &attrs,
                &box_attrs,
                true,
            );
            buffer.shape_until_scroll(&mut fonts.font_system, false);
            let text = [PlacedText {
                buffer,
                custom_glyph: None,
                left: cell.left,
                top: cell.top,
                right: bounds.right,
                bottom: bounds.bottom,
                bound_left: bounds.left,
                bound_top: bounds.top,
                color: Color::rgb(255, 255, 255),
            }];
            let mut unclipped_exterior_ink = 0;
            let mut interior_ink = 0;
            // Rasterize the same Swash glyphs and half-open bounds consumed by Glyphon.
            for area in text_areas(&text, None) {
                for run in area.buffer.layout_runs() {
                    for glyph in run.glyphs {
                        let physical = glyph.physical((area.left, area.top), area.scale);
                        let image = swash
                            .get_image_uncached(&mut fonts.font_system, physical.cache_key)
                            .unwrap();
                        assert_eq!(image.content, glyphon::SwashContent::Mask);
                        let left = physical.x + image.placement.left;
                        let top = run.line_y.round() as i32 + physical.y - image.placement.top;
                        for (index, alpha) in image.data.iter().enumerate() {
                            if *alpha == 0 {
                                continue;
                            }
                            let x = left + (index as u32 % image.placement.width) as i32;
                            let y = top + (index as u32 / image.placement.width) as i32;
                            let center_y = y as f32 + 0.5;
                            if (at_bottom && center_y >= fill.bottom())
                                || (!at_bottom && center_y < fill.top)
                            {
                                unclipped_exterior_ink += 1;
                            }
                            if x >= area.bounds.left
                                && x < area.bounds.right
                                && y >= area.bounds.top
                                && y < area.bounds.bottom
                            {
                                if fill.contains(x as f32 + 0.5, center_y) {
                                    interior_ink += 1;
                                } else {
                                    exterior_ink.push((scale, scroll, x, y));
                                }
                            }
                        }
                    }
                }
            }
            assert!(
                unclipped_exterior_ink > 0,
                "fixture must cross the fill edge"
            );
            assert!(interior_ink > 0, "clipped interior ink must survive");
        }
    }
    assert!(exterior_ink.is_empty(), "exterior ink: {exterior_ink:?}");
}

#[test]
fn fractional_card_text_survives_glyphon_preparation_with_hints_and_header_scroll() {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let cache = Cache::new(&device);
    let mut viewport = Viewport::new(&device, &cache);
    viewport.update(
        &queue,
        Resolution {
            width: 2000,
            height: 900,
        },
    );
    let mut atlas = TextAtlas::new(&device, &queue, &cache, wgpu::TextureFormat::Rgba8Unorm);
    let mut renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
    let mut fonts = clipping_test_fonts();
    let mut swash = SwashCache::new();
    // Shift scroll with the new title margin to keep the original y=844 glyph-edge repro.
    for (scale, scroll) in [(1.25, 275.0), (1.25, 266.5), (1.3, 320.0)] {
        let metrics = CellMetrics::for_scale(scale);
        let workspace = fractional_card_workspace(scale, scroll);
        let (clip, card) = terminal_paint_clip(
            &workspace,
            workspace.chrome,
            workspace.terminal,
            metrics,
            true,
            physical_outline_width(2, metrics.scale),
        )
        .unwrap();
        let clip = terminal_text_clip(workspace.terminal, clip, metrics, true);
        let card = card.unwrap();
        if scale == 1.25 {
            assert_eq!(
                card.bounds.bottom(),
                if scroll == 275.0 { 848.0 } else { 857.0 }
            );
        }
        let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
        let left = workspace.terminal.left + metrics.padding;
        let top = workspace.terminal.top + metrics.padding;
        if scale == 1.25 && scroll == 275.0 {
            assert_eq!(top, 822.0);
            assert_eq!(top - cell_font.top_offset, 820.0);
            assert_eq!(metrics.height, 23.0);
        }
        let mut buffer = Buffer::new(
            &mut fonts.font_system,
            Metrics::new(cell_font.size, metrics.height),
        );
        buffer.set_size(Some(metrics.width), Some(metrics.height));
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        buffer.set_text("g\u{323}", &cell_font.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts.font_system, false);
        assert!(buffer.layout_runs().any(|run| !run.glyphs.is_empty()));
        if scale == 1.25 && scroll == 275.0 {
            let run = buffer.layout_runs().next().unwrap();
            assert!(
                run.glyphs.iter().any(|glyph| {
                    let physical = glyph.physical((left, top - cell_font.top_offset), 1.0);
                    let image = swash
                        .get_image_uncached(&mut fonts.font_system, physical.cache_key)
                        .unwrap();
                    let ink_top = run.line_y.round() as i32 + physical.y - image.placement.top;
                    let row = 844 - ink_top;
                    row >= 0
                        && row < image.placement.height as i32
                        && image.data[(row as u32 * image.placement.width) as usize
                            ..((row as u32 + 1) * image.placement.width) as usize]
                            .iter()
                            .any(|alpha| *alpha != 0)
                }),
                "the regression must contain ink in the bottom row at y=844"
            );
        }
        let rect = SceneRect {
            left,
            top,
            width: metrics.width,
            height: metrics.height,
        }
        .intersection(clip)
        .unwrap();
        let text = [PlacedText {
            buffer,
            custom_glyph: None,
            left,
            top: top - cell_font.top_offset,
            right: text_clip_bounds(rect).right,
            bottom: text_clip_bounds(rect).bottom,
            bound_left: text_clip_bounds(rect).left,
            bound_top: text_clip_bounds(rect).top,
            color: Color::rgb(255, 255, 255),
        }];
        // The depth callback is reached once per glyph admitted by Glyphon's preparation.
        let mut prepare = |areas: Vec<TextArea<'_>>| {
            let mut glyphs = 0;
            renderer
                .prepare_with_depth(
                    &device,
                    &queue,
                    &mut fonts.font_system,
                    &mut atlas,
                    &viewport,
                    areas,
                    &mut swash,
                    |_| {
                        glyphs += 1;
                        0.0
                    },
                )
                .unwrap();
            glyphs
        };
        let reference = TextArea {
            buffer: &text[0].buffer,
            left: text[0].left,
            top: text[0].top,
            scale: 1.0,
            bounds: TextBounds {
                left: text[0].bound_left,
                top: text[0].bound_top,
                right: text[0].right,
                bottom: text[0].bottom,
            },
            default_color: text[0].color,
            custom_glyphs: &[],
        };
        let expected = prepare(vec![reference]);
        assert!(expected > 0);
        if scale == 1.25 && scroll == 275.0 {
            assert_eq!(expected, 2, "both g and its combining dot must be prepared");
        }
        let areas: Vec<_> = text_areas(&text, None).collect();
        let actual = prepare(areas);
        assert_eq!(
            actual, expected,
            "Glyphon preparation at scale {scale}, scroll {scroll}"
        );
        assert_eq!(text_areas(&text, None).count(), 1);
    }
}
