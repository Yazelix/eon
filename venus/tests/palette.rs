use yazelix_venus::{Color, Palette, PaletteMode, contrast_ratio};

#[test]
fn palette_contrast_is_available_to_other_consumers() {
    let black = Color { r: 0, g: 0, b: 0 };
    let white = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    assert_eq!(contrast_ratio(black, white), 21.0);
    assert_eq!(contrast_ratio(white, white), 1.0);
    for mode in [PaletteMode::Dark, PaletteMode::Light] {
        let palette = Palette::builtin(mode);
        assert!(contrast_ratio(palette.primary_text, palette.bar_background) >= 4.5);
        let pairs: Vec<_> = palette.contrasts().collect();
        assert!(pairs.iter().any(|p| p.adapted));
        assert!(
            pairs.iter().any(|pair| {
                pair.foreground == "accent"
                    && pair.background == "session_pressed"
                    && !pair.adapted
                    && pair.target >= 3.0
                    && pair.ratio >= pair.target
            }),
            "pressed session keyboard-focus outline must be guarded"
        );
        assert!(pairs.iter().all(|p| p.ratio >= p.target));
        for pair in pairs {
            assert_eq!(
                pair.ratio,
                contrast_ratio(pair.foreground_color, pair.background_color)
            );
        }
    }
}
