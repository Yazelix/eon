use super::*;

#[test]
fn shaped_preedit_follows_emitted_glyphs_without_charging_combining_marks() {
    let mut fonts = FontSetup::new(&FontSettings {
        family: Some("DejaVu Sans Mono".into()),
        size: 20.0,
        line_height: 1.5,
        ..Default::default()
    })
    .unwrap();
    let metrics = fonts.metrics(1.25);
    let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, true);
    let mut font_system = fonts.font_system;
    let mut text_width = |text: &str| {
        let mut buffer = Buffer::new(
            &mut font_system,
            Metrics::new(cell_font.size, metrics.height),
        );
        buffer.set_size(Some(100.0), Some(metrics.height));
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        buffer.set_text(text, &cell_font.attrs(), shaping(text), None);
        buffer.shape_until_scroll(&mut font_system, false);
        let glyphs = buffer
            .layout_runs()
            .next()
            .expect("the preedit should shape")
            .glyphs;
        let (left, right) = glyphs.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(left, right), glyph| (left.min(glyph.x), right.max(glyph.x + glyph.w)),
        );
        let (left_offset, width) = shaped_preedit_placement(&buffer);
        (width, right - left, left + left_offset)
    };

    let widths = ["e", "eee", "e\u{301}", "אבג"].map(|text| {
        let (width, emitted_width, left) = text_width(text);
        assert!(
            (width - emitted_width).abs() < 0.01,
            "{text:?} measured {width} but emitted {emitted_width}"
        );
        assert!(left.abs() < 0.01, "{text:?} starts at {left}");
        width
    });
    assert!((widths[0] - widths[2]).abs() < 0.01);
}

#[test]
fn ascii_uses_basic_shaping_without_weakening_unicode() {
    assert_eq!(shaping("plain ASCII"), Shaping::Basic);
    assert_eq!(shaping("e\u{301}"), Shaping::Advanced);
}

#[test]
fn wide_emoji_clusters_use_the_platform_colour_face() {
    use super::super::cells::NERD_FONT_FAMILY;

    let emoji_family = if cfg!(target_os = "macos") {
        "Apple Color Emoji"
    } else {
        "Noto Color Emoji"
    };
    let mut cache = glyphon::SwashCache::new();
    let mut observations = Vec::new();
    let mut all_match = true;
    for settings in [
        FontSettings {
            fallbacks: vec![NERD_FONT_FAMILY.into(), "Accelerando Terminal".into()],
            ..Default::default()
        },
        FontSettings::default(),
    ] {
        let mut fonts = FontSetup::new(&settings).unwrap();
        let configured = !settings.fallbacks.is_empty();
        assert_eq!(fonts.family.is_some(), configured);
        let metrics = fonts.metrics(1.0);
        let cell_font = fitted_cell_font(
            &mut fonts.font_system,
            metrics,
            fonts.family,
            fonts.configured,
        );
        for (text, wide, bold, emoji) in [
            ("🇯🇵", true, false, true),
            ("👨\u{200d}👩\u{200d}👧", true, false, true),
            ("❤\u{fe0f}", true, false, true),
            ("❤\u{fe0f}", true, true, true),
            ("中", true, false, false),
            ("❤\u{fe0e}", true, false, false),
            ("❤\u{fe0f}", false, false, false),
            ("A", false, false, false),
        ] {
            let style = DrawStyle {
                bold,
                ..plain_style()
            };
            let attrs = cell_text_attrs(cell_font, text, wide, style);
            let mut buffer = Buffer::new_empty(Metrics::new(cell_font.size, metrics.height));
            buffer.set_size(
                Some(metrics.width * if wide { 2.0 } else { 1.0 }),
                Some(metrics.height),
            );
            buffer.set_wrap(Wrap::None);
            buffer.set_monospace_width(Some(metrics.width));
            set_cell_layer(
                &mut buffer,
                text,
                &cell_text_segments(text),
                &attrs,
                &attrs,
                &attrs,
                false,
            );
            shape_cell_text(
                &mut buffer,
                &mut fonts.font_system,
                cell_font,
                text,
                style,
                &attrs,
            );
            let glyphs: Vec<_> = buffer
                .layout_runs()
                .flat_map(|run| run.glyphs)
                .map(|glyph| {
                    // harfrust hides unmapped ignorables as blank spaces; Venus's letter spacing can still add trailing advance.
                    let hidden = glyph.glyph_id != 0
                        && fonts
                            .font_system
                            .get_font(glyph.font_id, Weight::NORMAL)
                            .is_some_and(|font| {
                                glyph.glyph_id == font.as_swash().charmap().map(' ')
                            })
                        && cache
                            .get_image(
                                &mut fonts.font_system,
                                glyph.physical((0.0, 0.0), 1.0).cache_key,
                            )
                            .as_ref()
                            .is_none_or(|image| {
                                image.placement.width == 0 || image.placement.height == 0
                            });
                    (
                        glyph.glyph_id,
                        glyph.w,
                        hidden,
                        fonts
                            .font_system
                            .db()
                            .face(glyph.font_id)
                            .unwrap()
                            .families
                            .clone(),
                    )
                })
                .collect();
            let matches = if emoji {
                glyphs.iter().filter(|(_, _, hidden, _)| !hidden).count() == 1
                    && glyphs.iter().all(|(id, _, _, families)| {
                        *id != 0 && families.iter().any(|(family, _)| family == emoji_family)
                    })
            } else {
                attrs.family != Family::Name(emoji_family)
            };
            all_match &= matches;
            observations.push(format!(
                "configured={configured} {text:?} wide={wide} bold={bold}: {} glyphs (id, width, hidden, families) {glyphs:?}, matches={matches}",
                glyphs.len()
            ));
        }
    }
    assert!(
        all_match,
        "expected one visible nonzero glyph from {emoji_family} for each wide emoji and ordinary routing for controls:\n{}",
        observations.join("\n")
    );
}

#[test]
fn emoji_fallback_preserves_ordinary_cell_routing() {
    use super::super::cells::NERD_FONT_FAMILY;
    use glyphon::{FontSystem, fontdb};

    let emoji_family = if cfg!(target_os = "macos") {
        "Apple Color Emoji"
    } else {
        "Noto Color Emoji"
    };
    let mut original = FontSystem::new();
    let emoji_id = original
        .db()
        .query(&fontdb::Query {
            families: &[Family::Name(emoji_family)],
            ..Default::default()
        })
        .expect("the accepted font environment supplies the emoji face");
    assert_eq!(
        original
            .get_font(emoji_id, Weight::NORMAL)
            .unwrap()
            .as_swash()
            .charmap()
            .map('\u{2329}'),
        0,
        "the partial-coverage case must be absent from the emoji face"
    );
    let mut failures = Vec::new();
    for with_emoji in [false, true] {
        let mut db = fontdb::Database::new();
        for face in original.db().faces().filter(|face| {
            face.families.iter().any(|(family, _)| {
                ["DejaVu Sans Mono", "DejaVu Sans", NERD_FONT_FAMILY].contains(&family.as_str())
            }) || (with_emoji && face.id == emoji_id)
        }) {
            db.push_face_info(face.clone());
        }
        let mut fonts = FontSetup::resolve(
            &FontSettings {
                family: Some("DejaVu Sans Mono".into()),
                fallbacks: vec![NERD_FONT_FAMILY.into()],
                ..Default::default()
            },
            FontSystem::new_with_locale_and_db("en-US".into(), db),
        )
        .unwrap();
        let metrics = fonts.metrics(1.0);
        let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, true);
        let cases: &[(&str, bool)] = if with_emoji {
            &[("\u{2329}", false)]
        } else {
            &[("❤\u{fe0f}", false), ("🇯🇵", false), ("❤\u{fe0f}", true)]
        };
        for &(text, italic) in cases {
            let style = DrawStyle {
                bold: true,
                italic,
                ..plain_style()
            };
            let mut shape = |wide| {
                let attrs = cell_text_attrs(cell_font, text, wide, style);
                let mut buffer = Buffer::new_empty(Metrics::new(cell_font.size, metrics.height));
                buffer.set_size(Some(metrics.width * 2.0), Some(metrics.height));
                buffer.set_wrap(Wrap::None);
                buffer.set_monospace_width(Some(metrics.width));
                set_cell_layer(
                    &mut buffer,
                    text,
                    &cell_text_segments(text),
                    &attrs,
                    &attrs,
                    &attrs,
                    false,
                );
                shape_cell_text(
                    &mut buffer,
                    &mut fonts.font_system,
                    cell_font,
                    text,
                    style,
                    &attrs,
                );
                buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs)
                    .map(|glyph| {
                        (
                            glyph.glyph_id,
                            glyph.font_id,
                            fonts
                                .font_system
                                .db()
                                .face(glyph.font_id)
                                .unwrap()
                                .post_script_name
                                .clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let actual = shape(true);
            let ordinary = shape(false);
            if actual != ordinary {
                failures.push(format!(
                    "with_emoji={with_emoji} {text:?} bold=true italic={italic}: routed={actual:?}, ordinary={ordinary:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn row_text_context(fonts: &mut FontSetup, columns: u16) -> RowTextContext {
    let metrics = fonts.metrics(1.25);
    RowTextContext {
        metrics,
        cell_font: fitted_cell_font(
            &mut fonts.font_system,
            metrics,
            fonts.family,
            fonts.configured,
        ),
        family: fonts.family,
        columns,
        screen: Screen::Primary,
    }
}

#[test]
fn row_text_cache_reuses_equal_rows_after_placement_changes() {
    let mut fonts = clipping_test_fonts();
    let context = row_text_context(&mut fonts, 3);
    let mut row = padding_scene().content.remove(0);
    row.cells[0].text = "A".into();
    let slot = RowTextSlot {
        terminal: 0,
        preview: None,
        row: 0,
    };
    let origin = SceneRect {
        left: 0.25,
        top: 0.0,
        width: 200.0,
        height: 100.0,
    };
    let mut cache = RowTextCache::default();
    let mut text = Vec::new();
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        true,
        origin,
        10.25,
        origin,
        &mut text,
    );
    let first = Arc::clone(cache.rows[&slot].shaped.as_ref().unwrap());
    text.clear();
    // Blink, clip, position, cursor and hover are placement state, never shape state.
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        false,
        origin,
        15.75,
        origin,
        &mut text,
    );
    assert!(Arc::ptr_eq(
        &first,
        cache.rows[&slot].shaped.as_ref().unwrap()
    ));
    assert_eq!(text[0].top, 15.75 - context.cell_font.top_offset);
    cache.begin();
    cache.finish();
    assert!(cache.rows.is_empty(), "a missing scene cannot retain rows");
}

fn row_text_snapshot(text: &[PlacedText]) -> Vec<String> {
    text.iter()
        .map(|text| {
            let glyphs = text
                .buffer
                .layout_runs()
                .map(|run| {
                    format!(
                        "{:?}/{:?}/{:?}/{:?}",
                        run.line_w, run.line_y, run.line_height, run.glyphs
                    )
                })
                .collect::<Vec<_>>();
            format!(
                "{:?}/{:?}/{:?}/{:?}/{glyphs:?}",
                text.left.to_bits(),
                text.top.to_bits(),
                (text.bound_left, text.bound_top, text.right, text.bottom),
                text.color
            )
        })
        .collect()
}

// Original uncached cell-placement loop: one font database for both sides of the oracle.
fn uncached_row_text(
    row: &DrawRow,
    context: RowTextContext,
    fonts: &mut FontSystem,
    blink: bool,
    origin: SceneRect,
    top: f32,
    clip: SceneRect,
) -> Vec<PlacedText> {
    let mut runs = Vec::new();
    row.append_glyph_runs(0, &mut runs);
    runs.retain(|run| run.style.foreground_visible(blink));
    let mut text = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        if block_span(&run.text, run.columns == 1).is_some() {
            continue;
        }
        let left =
            origin.left + context.metrics.padding + f32::from(run.column) * context.metrics.width;
        let width = f32::from(run.columns) * context.metrics.width;
        let ink_width =
            f32::from(cell_ink_right(run, runs.get(index + 1), context.columns) - run.column)
                * context.metrics.width;
        let Some(bounds) = (SceneRect {
            left,
            top,
            width: ink_width.max(1.0),
            height: context.metrics.height.max(1.0),
        })
        .intersection(clip) else {
            continue;
        };
        let bounds = text_clip_bounds(bounds);
        for (buffer, box_drawing) in shape_cell_layers(
            fonts,
            context.metrics,
            context.cell_font,
            context.family,
            &run.text,
            width,
            context.metrics.height,
            run.style,
            run.columns == 2,
        )
        .into_iter()
        .zip([false, true])
        {
            let Some(buffer) = buffer else {
                continue;
            };
            text.push(PlacedText {
                buffer: buffer.into(),
                custom_glyph: None,
                left,
                top: top
                    - if box_drawing {
                        0.0
                    } else {
                        context.cell_font.top_offset
                    },
                bound_left: bounds.left,
                bound_top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
                color: Color::rgba(
                    run.style.foreground.r,
                    run.style.foreground.g,
                    run.style.foreground.b,
                    run.style.foreground_alpha(),
                ),
            });
        }
    }
    text
}

#[test]
fn row_text_cache_exactly_matches_uncached_placement_after_clip_and_blink_changes() {
    let mut fonts = clipping_test_fonts();
    let mut row = DrawRow {
        wrapped: false,
        wrap_continuation: false,
        cells: Vec::new(),
    };
    for (index, text) in [
        "A",
        "e\u{301}",
        "中",
        "",
        "👨\u{200d}👩\u{200d}👧",
        "",
        "⠿",
        "─",
        "█",
        "\u{f013}",
        " ",
        "j",
        "B",
    ]
    .into_iter()
    .enumerate()
    {
        row.cells.push(DrawCell {
            width: match index {
                2 | 4 => CellWidth::Wide,
                3 | 5 => CellWidth::SpacerTail,
                _ => CellWidth::Narrow,
            },
            text: text.into(),
            hyperlink: String::new(),
            style: DrawStyle {
                blink: index == 11,
                faint: index == 6,
                italic: index == 1,
                ..plain_style()
            },
        });
    }
    let context = row_text_context(&mut fonts, row.cells.len() as u16);
    let origin = SceneRect {
        left: 0.25,
        top: 0.0,
        width: 400.0,
        height: 200.0,
    };
    let slot = RowTextSlot {
        terminal: 0,
        preview: None,
        row: 0,
    };
    let mut cache = RowTextCache::default();
    for blink in [true, false, true] {
        for offset in [
            0.0,
            0.25,
            -0.25,
            context.metrics.height - 0.25,
            -context.metrics.height + 0.25,
        ] {
            for width in [25.5, 400.0] {
                let clip = SceneRect {
                    left: 0.5,
                    top: 20.5,
                    width,
                    height: 30.25,
                };
                let top = 20.25 + offset;
                let mut cached = Vec::new();
                cache.place(
                    slot,
                    &row,
                    context,
                    &mut fonts.font_system,
                    blink,
                    origin,
                    top,
                    clip,
                    &mut cached,
                );
                let uncached = uncached_row_text(
                    &row,
                    context,
                    &mut fonts.font_system,
                    blink,
                    origin,
                    top,
                    clip,
                );
                assert_eq!(
                    row_text_snapshot(&cached),
                    row_text_snapshot(&uncached),
                    "blink={blink}, offset={offset}, width={width}"
                );
                // A frame overlay may alter placements; it cannot poison the cached bounds.
                for text in &mut cached {
                    text.bottom = 0;
                }
            }
        }
    }
}

#[test]
fn row_text_cache_keeps_only_visible_slots_and_separates_terminals_and_previews() {
    let mut fonts = clipping_test_fonts();
    let context = row_text_context(&mut fonts, 3);
    let mut row = padding_scene().content.remove(0);
    row.cells[0].text = "A".into();
    let clip = SceneRect {
        left: 0.0,
        top: 0.0,
        width: 200.0,
        height: context.metrics.height * 2.5,
    };
    let mut cache = RowTextCache::default();
    for offset in 0..40 {
        cache.begin();
        for terminal in 0..2 {
            for index in 0..60 {
                let top = (index as f32 - offset as f32) * context.metrics.height - 0.25;
                let mut text = Vec::new();
                cache.place(
                    RowTextSlot {
                        terminal,
                        preview: None,
                        row: index,
                    },
                    &row,
                    context,
                    &mut fonts.font_system,
                    true,
                    clip,
                    top,
                    clip,
                    &mut text,
                );
            }
        }
        cache.finish();
        assert!(cache.rows.len() <= 2 * (2.5_f32.ceil() as usize + 2));
    }
    cache.begin();
    let mut text = Vec::new();
    for preview in [None, Some(true), Some(false)] {
        cache.place(
            RowTextSlot {
                terminal: 0,
                preview,
                row: 0,
            },
            &row,
            context,
            &mut fonts.font_system,
            true,
            clip,
            0.0,
            clip,
            &mut text,
        );
    }
    cache.finish();
    assert_eq!(cache.rows.len(), 3);
}

#[test]
fn fully_dirty_rows_bypass_retention_and_equal_rows_resume_reuse() {
    let mut fonts = clipping_test_fonts();
    let context = row_text_context(&mut fonts, 3);
    let mut row = padding_scene().content.remove(0);
    for cell in &mut row.cells {
        cell.text = "a".into();
    }
    let slot = RowTextSlot {
        terminal: 0,
        preview: None,
        row: 0,
    };
    let clip = SceneRect {
        left: 0.0,
        top: 0.0,
        width: 200.0,
        height: 100.0,
    };
    let mut cache = RowTextCache::default();
    let mut text = Vec::new();
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        0.0,
        clip,
        &mut text,
    );
    assert!(cache.rows[&slot].shaped.is_some());
    for value in ["b", "c", "d"] {
        for cell in &mut row.cells {
            cell.text = value.into();
            cell.style.selected = true;
            cell.hyperlink = format!("urn:{value}");
        }
        row.wrapped = !row.wrapped;
        row.wrap_continuation = !row.wrap_continuation;
        text.clear();
        cache.place(
            slot,
            &row,
            context,
            &mut fonts.font_system,
            true,
            clip,
            0.0,
            clip,
            &mut text,
        );
        assert!(
            cache.rows[&slot].shaped.is_none(),
            "fully dirty shapes must not be retained"
        );
        assert_eq!(cache.rows[&slot].content, row);
        let uncached =
            uncached_row_text(&row, context, &mut fonts.font_system, true, clip, 0.0, clip);
        assert_eq!(row_text_snapshot(&text), row_text_snapshot(&uncached));
    }
    text.clear();
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        0.0,
        clip,
        &mut text,
    );
    let stable = Arc::clone(cache.rows[&slot].shaped.as_ref().unwrap());
    text.clear();
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        0.0,
        clip,
        &mut text,
    );
    assert!(Arc::ptr_eq(
        &stable,
        cache.rows[&slot].shaped.as_ref().unwrap()
    ));
    // One changed cell keeps the ordinary cached-row path.
    row.cells[0].text = "e".into();
    text.clear();
    cache.place(
        slot,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        0.0,
        clip,
        &mut text,
    );
    assert!(cache.rows[&slot].shaped.is_some());
}

fn assert_row_cache_transition(change: impl FnOnce(&mut DrawRow, &mut RowTextContext)) {
    let mut fonts = clipping_test_fonts();
    let mut row = DrawRow {
        wrapped: false,
        wrap_continuation: false,
        cells: [
            ("e", CellWidth::Narrow),
            ("❤\u{fe0f}", CellWidth::Wide),
            ("", CellWidth::SpacerTail),
            ("─", CellWidth::Narrow),
            ("⠿", CellWidth::Narrow),
            ("R", CellWidth::Narrow),
        ]
        .into_iter()
        .map(|(text, width)| DrawCell {
            text: text.into(),
            width,
            hyperlink: String::new(),
            style: plain_style(),
        })
        .collect(),
    };
    let mut context = row_text_context(&mut fonts, 6);
    let original_row = row.clone();
    let original_context = context;
    let slots = [
        RowTextSlot {
            terminal: 0,
            preview: None,
            row: 0,
        },
        RowTextSlot {
            terminal: 0,
            preview: None,
            row: 1,
        },
        RowTextSlot {
            terminal: 1,
            preview: None,
            row: 0,
        },
    ];
    let clip = SceneRect {
        left: 0.25,
        top: 0.0,
        width: 400.0,
        height: 200.0,
    };
    let mut cache = RowTextCache::default();
    for slot in slots {
        let mut text = Vec::new();
        cache.place(
            slot,
            &row,
            context,
            &mut fonts.font_system,
            true,
            clip,
            10.25,
            clip,
            &mut text,
        );
    }
    let before = slots.map(|slot| Arc::clone(cache.rows[&slot].shaped.as_ref().unwrap()));
    change(&mut row, &mut context);
    assert!(
        row != original_row || context != original_context,
        "transition must change an input"
    );
    let target = slots[0];
    let mut text = Vec::new();
    cache.place(
        target,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        10.25,
        clip,
        &mut text,
    );
    let refreshed = Arc::clone(cache.rows[&target].shaped.as_ref().unwrap());
    assert!(
        !Arc::ptr_eq(&before[0], &refreshed),
        "changed input must invalidate the target row"
    );
    assert_eq!(cache.rows[&target].content, row);
    assert!(cache.rows[&target].context == context);
    let uncached = uncached_row_text(
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        10.25,
        clip,
    );
    assert_eq!(row_text_snapshot(&text), row_text_snapshot(&uncached));
    text.clear();
    cache.place(
        target,
        &row,
        context,
        &mut fonts.font_system,
        true,
        clip,
        10.25,
        clip,
        &mut text,
    );
    assert!(
        Arc::ptr_eq(&refreshed, cache.rows[&target].shaped.as_ref().unwrap()),
        "updated key must reuse the next equal row"
    );
    for index in 1..slots.len() {
        text.clear();
        cache.place(
            slots[index],
            &original_row,
            original_context,
            &mut fonts.font_system,
            true,
            clip,
            10.25,
            clip,
            &mut text,
        );
        assert!(
            Arc::ptr_eq(
                &before[index],
                cache.rows[&slots[index]].shaped.as_ref().unwrap()
            ),
            "other rows and terminals must retain their shapes"
        );
    }
}

macro_rules! row_cache_transition {
    ($name:ident, $change:expr) => {
        #[test]
        fn $name() {
            assert_row_cache_transition($change);
        }
    };
}

row_cache_transition!(row_cache_transition_text, |row, _| {
    row.cells[0].text = "g".into()
});
row_cache_transition!(row_cache_transition_combining, |row, _| {
    row.cells[0].text = "e\u{301}".into()
});
row_cache_transition!(row_cache_transition_wide_glyph, |row, _| {
    row.cells[1].text = "🇯🇵".into()
});
row_cache_transition!(row_cache_transition_widths, |row, _| {
    row.cells[1].width = CellWidth::Narrow;
    row.cells[2].width = CellWidth::Narrow;
});
row_cache_transition!(row_cache_transition_hyperlink, |row, _| {
    row.cells[0].hyperlink = "https://example.invalid".into()
});
row_cache_transition!(row_cache_transition_wrapped, |row, _| {
    row.wrapped = true
});
row_cache_transition!(row_cache_transition_wrap_continuation, |row, _| {
    row.wrap_continuation = true
});
row_cache_transition!(row_cache_transition_foreground, |row, _| {
    row.cells[0].style.foreground.r ^= 32
});
row_cache_transition!(row_cache_transition_background, |row, _| {
    row.cells[0].style.background.r ^= 32
});
row_cache_transition!(row_cache_transition_underline_color, |row, _| {
    row.cells[0].style.underline_color.r ^= 32
});
row_cache_transition!(row_cache_transition_bold, |row, _| {
    row.cells[0].style.bold = true
});
row_cache_transition!(row_cache_transition_italic, |row, _| {
    row.cells[0].style.italic = true
});
row_cache_transition!(row_cache_transition_faint, |row, _| {
    row.cells[0].style.faint = true
});
row_cache_transition!(row_cache_transition_blink, |row, _| {
    row.cells[0].style.blink = true
});
row_cache_transition!(row_cache_transition_invisible, |row, _| {
    row.cells[0].style.invisible = true
});
row_cache_transition!(row_cache_transition_selected, |row, _| {
    row.cells[0].style.selected = true
});
row_cache_transition!(row_cache_transition_strikethrough, |row, _| {
    row.cells[0].style.strikethrough = true
});
row_cache_transition!(row_cache_transition_overline, |row, _| {
    row.cells[0].style.overline = true
});
row_cache_transition!(row_cache_transition_protected, |row, _| {
    row.cells[0].style.protected = true
});
row_cache_transition!(row_cache_transition_background_default, |row, _| {
    row.cells[0].style.background_is_default = !row.cells[0].style.background_is_default
});
row_cache_transition!(row_cache_transition_underline, |row, _| {
    row.cells[0].style.underline = orbit_protocol::Underline::Single
});
row_cache_transition!(row_cache_transition_scale, |_, context| {
    context.metrics.scale += 0.25
});
row_cache_transition!(row_cache_transition_metric_width, |_, context| {
    context.metrics.width += 0.5
});
row_cache_transition!(row_cache_transition_metric_height, |_, context| {
    context.metrics.height += 0.5
});
row_cache_transition!(row_cache_transition_metric_font_size, |_, context| {
    context.metrics.font_size += 0.5
});
row_cache_transition!(row_cache_transition_padding, |_, context| {
    context.metrics.padding += 0.5
});
row_cache_transition!(row_cache_transition_font_size, |_, context| {
    context.cell_font.size += 0.5
});
row_cache_transition!(row_cache_transition_font_top, |_, context| {
    context.cell_font.top_offset += 0.5
});
row_cache_transition!(row_cache_transition_box_size, |_, context| {
    context.cell_font.box_size += 0.5
});
row_cache_transition!(row_cache_transition_box_spacing, |_, context| {
    context.cell_font.box_letter_spacing += 0.5
});
row_cache_transition!(row_cache_transition_family, |_, context| {
    context.family = None
});
row_cache_transition!(row_cache_transition_columns, |_, context| {
    context.columns += 1
});
row_cache_transition!(row_cache_transition_screen, |_, context| {
    context.screen = Screen::Alternate
});
