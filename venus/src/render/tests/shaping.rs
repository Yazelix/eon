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
