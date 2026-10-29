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
                    (
                        glyph.glyph_id,
                        &fonts.font_system.db().face(glyph.font_id).unwrap().families,
                    )
                })
                .collect();
            let matches = if emoji {
                glyphs.len() == 1
                    && glyphs.iter().all(|(id, families)| {
                        *id != 0 && families.iter().any(|(family, _)| family == emoji_family)
                    })
            } else {
                attrs.family != Family::Name(emoji_family)
            };
            all_match &= matches;
            observations.push(format!(
                "configured={configured} {text:?} wide={wide} bold={bold}: {} glyphs {glyphs:?}, matches={matches}",
                glyphs.len()
            ));
        }
    }
    assert!(
        all_match,
        "expected one nonzero glyph from {emoji_family} for each wide emoji and ordinary routing for controls:\n{}",
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
