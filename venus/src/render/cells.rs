//! Terminal cell drawing: cell fonts and text runs, and the cell, preview and cursor
//! rectangles drawn from a scene.

use super::{
    CellMetrics, RectangleBatch, SceneColor, images::BandItem, preview_row_top, preview_rows,
    row_intersects_clip, shaping, valid_rect, vertex_count,
};
use crate::{DrawCursor, DrawRow, DrawStyle, GlyphRun, Scene, ScenePreview, SceneRect};
use glyphon::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap};
use orbit_protocol::{CellWidth, CursorShape, Underline};

pub(super) const BRAILLE_FAMILY: &str = "DejaVu Sans";
pub(super) const NERD_FONT_FAMILY: &str = "Symbols Nerd Font Mono";
const ACCELERANDO_FAMILY: &str = "Accelerando Terminal";
#[cfg(target_os = "macos")]
const EMOJI_FAMILY: &str = "Apple Color Emoji";
#[cfg(not(target_os = "macos"))]
const EMOJI_FAMILY: &str = "Noto Color Emoji";

#[derive(Clone, Copy)]
pub(super) struct CellFont {
    family: Option<&'static str>,
    pub(super) size: f32,
    pub(super) top_offset: f32,
    letter_spacing: f32,
    pub(super) box_size: f32,
    pub(super) box_letter_spacing: f32,
}

impl CellFont {
    pub(super) fn attrs(self) -> Attrs<'static> {
        Attrs::new()
            .family(self.family.map_or(Family::Monospace, Family::Name))
            .letter_spacing(self.letter_spacing)
    }
}

pub(super) fn fitted_cell_font(
    font_system: &mut FontSystem,
    metrics: CellMetrics,
    family: Option<&'static str>,
    configured: bool,
) -> CellFont {
    let top_offset = ((metrics.height - metrics.font_size) / 2.0)
        .round()
        .max(0.0);
    let mut buffer = Buffer::new(font_system, Metrics::new(metrics.font_size, metrics.height));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        " ",
        &Attrs::new().family(family.map_or(Family::Monospace, Family::Name)),
        Shaping::Advanced,
        None,
    );
    let advance = buffer
        .line_layout(font_system, 0)
        .and_then(|lines| lines.first())
        .map(|line| line.w)
        .filter(|width| width.is_finite() && *width > 0.0);
    advance.map_or(
        CellFont {
            family,
            size: metrics.font_size,
            top_offset,
            letter_spacing: 0.0,
            box_size: metrics.font_size,
            box_letter_spacing: 0.0,
        },
        |advance| {
            let fitted_size = (metrics.font_size * metrics.width / advance)
                .round()
                .max(1.0);
            let size = (fitted_size - 1.0).clamp(1.0, metrics.font_size);
            let box_size = if configured {
                (fitted_size + 1.0).max(metrics.height)
            } else {
                fitted_size + 1.0
            };
            CellFont {
                family,
                size,
                top_offset,
                letter_spacing: metrics.width / size - advance / metrics.font_size,
                box_size,
                box_letter_spacing: metrics.width / box_size - advance / metrics.font_size,
            }
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellTextKind {
    Text,
    Braille,
    Box,
}

fn cell_text_kind(character: char) -> CellTextKind {
    match character {
        '\u{2500}'..='\u{257f}' => CellTextKind::Box,
        '\u{2800}'..='\u{28ff}' => CellTextKind::Braille,
        _ => CellTextKind::Text,
    }
}

pub(super) fn cell_text_segments(text: &str) -> Vec<(&str, CellTextKind)> {
    let Some(first) = text.chars().next() else {
        return Vec::new();
    };
    let mut segments = Vec::new();
    let mut start = 0;
    let mut kind = cell_text_kind(first);
    for (index, character) in text.char_indices().skip(1) {
        let next_kind = cell_text_kind(character);
        if next_kind != kind {
            segments.push((&text[start..index], kind));
            start = index;
            kind = next_kind;
        }
    }
    segments.push((&text[start..], kind));
    segments
}

fn is_nerd_font_symbol(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{f2fff}'
        )
    })
}

pub(super) fn block_span(text: &str, narrow: bool) -> Option<(f32, f32)> {
    match (narrow, text) {
        (true, "█") => Some((0.0, 1.0)),
        (true, "▀") => Some((0.0, 0.5)),
        (true, "▄") => Some((0.5, 0.5)),
        _ => None,
    }
}

pub(super) fn cell_ink_right(run: &GlyphRun, next: Option<&GlyphRun>, columns: u16) -> u16 {
    let cell_right = run.column.saturating_add(run.columns);
    if is_nerd_font_symbol(&run.text) {
        next.map_or(columns, |next| next.column).max(cell_right)
    } else {
        cell_right
    }
}

pub(super) fn cell_text_attrs(
    cell_font: CellFont,
    text: &str,
    wide: bool,
    style: DrawStyle,
) -> Attrs<'static> {
    let mut attrs = cell_font.attrs();
    if cell_font.family.is_none()
        && text
            .chars()
            .any(|character| matches!(character, '\u{100000}'..='\u{101fff}'))
    {
        attrs = attrs.family(Family::Name(ACCELERANDO_FAMILY));
    }
    if is_nerd_font_symbol(text) {
        // The packaged symbol face is Regular-only; bold can select an ambient mono face.
        if cell_font.family.is_none() {
            attrs = attrs.family(Family::Name(NERD_FONT_FAMILY));
        }
        return attrs;
    }
    if wide
        && !text.contains('\u{fe0e}')
        && (text.chars().any(|character| {
            matches!(
                character,
                '\u{fe0f}'
                    | '\u{1f1e6}'..='\u{1f1ff}'
                    | '\u{1f3fb}'..='\u{1f3ff}'
                    | '\u{200d}'
                    | '\u{20e3}'
                    | '\u{e0020}'..='\u{e007f}'
            )
        }) || text.chars().next().is_some_and(
            |character| matches!(character, '\u{1f000}'..='\u{1faff}' | '\u{2300}'..='\u{2bff}'),
        ))
    {
        // Colour emoji faces are Regular-only, even in bold or italic cells.
        return attrs.family(Family::Name(EMOJI_FAMILY));
    }
    if style.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    attrs
}

pub(super) fn set_cell_layer(
    buffer: &mut Buffer,
    text: &str,
    segments: &[(&str, CellTextKind)],
    attrs: &Attrs<'static>,
    braille_attrs: &Attrs<'static>,
    box_attrs: &Attrs<'static>,
    box_drawing: bool,
) {
    let hidden = attrs.clone().color(Color::rgba(0, 0, 0, 0));
    buffer.set_rich_text(
        segments.iter().map(|(segment, kind)| {
            (
                *segment,
                match (*kind, box_drawing) {
                    (CellTextKind::Text, false) => attrs.clone(),
                    (CellTextKind::Braille, false) => braille_attrs.clone(),
                    (CellTextKind::Box, true) => box_attrs.clone(),
                    _ => hidden.clone(),
                },
            )
        }),
        attrs,
        shaping(text),
        None,
    );
}

pub(super) fn shape_cell_text(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cell_font: CellFont,
    text: &str,
    style: DrawStyle,
    attrs: &Attrs<'static>,
) {
    buffer.shape_until_scroll(font_system, false);
    if attrs.family == Family::Name(EMOJI_FAMILY)
        && buffer
            .layout_runs()
            .flat_map(|run| run.glyphs)
            .any(|glyph| {
                glyph.glyph_id == 0
                    || !font_system.db().face(glyph.font_id).is_some_and(|face| {
                        face.families
                            .iter()
                            .any(|(family, _)| family == EMOJI_FAMILY)
                    })
            })
    {
        // Restore the primary family and style for the whole cluster, not just missing glyphs.
        let attrs = cell_text_attrs(cell_font, text, false, style);
        let braille_attrs = if cell_font.family.is_some() {
            attrs.clone()
        } else {
            attrs.clone().family(Family::Name(BRAILLE_FAMILY))
        };
        set_cell_layer(
            buffer,
            text,
            &cell_text_segments(text),
            &attrs,
            &braille_attrs,
            &attrs,
            false,
        );
        buffer.shape_until_scroll(font_system, false);
    }
}

/// Draws one terminal's cell rectangles in its layers: every background of the live and preview
/// rows, then the terminal's image band, then foreground rectangles and the cursor (VEN-C31).
/// Returns the vertex count after the cursor, where a trailing cursor draws.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_scene_rectangles(
    rectangles: &mut RectangleBatch,
    scene: &Scene,
    band: &[BandItem],
    preview: Option<&ScenePreview>,
    blink_visible: bool,
    metrics: CellMetrics,
    origin: SceneRect,
    draw_cursor: bool,
) -> u32 {
    let grid = metrics.grid_rect(origin, scene.columns, scene.rows);
    let (left, top) = (grid.left, grid.top);
    let live = scene
        .content
        .iter()
        .enumerate()
        .map(|(index, row)| (row, top + index as f32 * metrics.height))
        .collect::<Vec<_>>();
    let previewed = preview_rows(preview)
        .into_iter()
        .flat_map(|(direction, rows)| {
            rows.iter().enumerate().map(move |(index, row)| {
                (
                    row,
                    preview_row_top(scene, direction, index, metrics, origin),
                )
            })
        })
        .filter(|(_, top)| {
            rectangles
                .clip
                .is_none_or(|clip| row_intersects_clip(*top, metrics.height, clip))
        })
        .collect::<Vec<_>>();
    for (row, top) in live.iter().chain(&previewed) {
        build_row_backgrounds(rectangles, row, metrics, left, *top);
    }
    // Images stay on the live rows' grid: never in the padding, nor over preview rows while
    // scrolling, even where the batch's clip falls back to the whole terminal.
    if let Some(image_clip) = rectangles
        .clip
        .map_or(Some(grid), |clip| clip.intersection(grid))
    {
        let clip = rectangles.clip.replace(image_clip);
        rectangles.push_band(band, left, top, metrics, scene.foreground);
        rectangles.clip = clip;
    }
    for (row, top) in &live {
        build_row_foregrounds(rectangles, row, blink_visible, metrics, left, *top);
    }
    if draw_cursor
        && let Some(cursor) = scene
            .cursor
            .filter(|cursor| cursor.visible && (blink_visible || !cursor.blinking))
        && let Some(bounds) = cursor_bounds(scene, cursor, metrics, origin)
    {
        push_cursor(rectangles, cursor, bounds, metrics, None);
    }
    let cursor_boundary = vertex_count(&rectangles.bytes);
    for (row, top) in &previewed {
        build_row_foregrounds(rectangles, row, blink_visible, metrics, left, *top);
    }
    cursor_boundary
}

fn build_row_backgrounds(
    rectangles: &mut RectangleBatch,
    row: &DrawRow,
    metrics: CellMetrics,
    row_left: f32,
    row_top: f32,
) {
    let mut start = 0_usize;
    while start < row.cells.len() {
        let background = row.cells[start].style.background;
        let background_is_default = row.cells[start].style.background_is_default;
        let mut end = start + 1;
        while end < row.cells.len()
            && row.cells[end].style.background == background
            && row.cells[end].style.background_is_default == background_is_default
        {
            end += 1;
        }
        if !background_is_default {
            rectangles.push(
                row_left + start as f32 * metrics.width,
                row_top,
                (end - start) as f32 * metrics.width,
                metrics.height,
                background,
                1.0,
            );
        }
        start = end;
    }
}

fn build_row_foregrounds(
    rectangles: &mut RectangleBatch,
    row: &DrawRow,
    blink_visible: bool,
    metrics: CellMetrics,
    row_left: f32,
    row_top: f32,
) {
    for (column, cell) in row.cells.iter().enumerate() {
        if !cell.style.foreground_visible(blink_visible) {
            continue;
        }
        let left = row_left + column as f32 * metrics.width;
        let top = row_top;
        if let Some((offset, height)) = block_span(&cell.text, cell.width == CellWidth::Narrow) {
            rectangles.push(
                left,
                top + metrics.height * offset,
                metrics.width,
                metrics.height * height,
                cell.style.foreground,
                f32::from(cell.style.foreground_alpha()) / 255.0,
            );
        }
        let thickness = (metrics.height / 14.0).max(1.0);
        match cell.style.underline {
            Underline::None => {}
            Underline::Single => rectangles.push(
                left,
                top + metrics.height - thickness * 2.0,
                metrics.width,
                thickness,
                cell.style.underline_color,
                1.0,
            ),
            Underline::Curly => {
                let segment = (metrics.width / 4.0).max(1.0);
                for part in 0..4 {
                    rectangles.push(
                        left + part as f32 * segment,
                        top + metrics.height - thickness * if part % 2 == 0 { 3.0 } else { 1.5 },
                        segment,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
            Underline::Dotted => {
                let dot = thickness.max(1.0);
                let mut x = left;
                while x < left + metrics.width {
                    rectangles.push(
                        x,
                        top + metrics.height - thickness * 2.0,
                        dot,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                    x += dot * 2.0;
                }
            }
            Underline::Dashed => {
                let dash = (metrics.width / 3.0).max(1.0);
                for part in [0.0, 2.0] {
                    rectangles.push(
                        left + part * dash,
                        top + metrics.height - thickness * 2.0,
                        dash,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
            Underline::Double => {
                for offset in [2.0, 4.0] {
                    rectangles.push(
                        left,
                        top + metrics.height - thickness * offset,
                        metrics.width,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
        }
        if cell.style.strikethrough {
            rectangles.push(
                left,
                top + metrics.height * 0.52,
                metrics.width,
                thickness,
                cell.style.foreground,
                1.0,
            );
        }
        if cell.style.overline {
            rectangles.push(
                left,
                top + thickness,
                metrics.width,
                thickness,
                cell.style.foreground,
                1.0,
            );
        }
    }
}

pub(super) fn cursor_bounds(
    scene: &Scene,
    cursor: DrawCursor,
    metrics: CellMetrics,
    viewport: SceneRect,
) -> Option<SceneRect> {
    if !valid_rect(viewport)
        || !metrics.width.is_finite()
        || !metrics.height.is_finite()
        || !metrics.padding.is_finite()
        || metrics.width <= 0.0
        || metrics.height <= 0.0
        || cursor.row >= scene.rows
    {
        return None;
    }
    let column = cursor.leading_column();
    let wide = cursor.at_wide_tail
        || scene
            .content
            .get(usize::from(cursor.row))
            .and_then(|row| row.cells.get(usize::from(column)))
            .is_some_and(|cell| cell.width == CellWidth::Wide);
    let columns = if wide { 2 } else { 1 };
    if column.checked_add(columns)? > scene.columns {
        return None;
    }
    Some(SceneRect {
        left: viewport.left + metrics.padding + f32::from(column) * metrics.width,
        top: viewport.top + metrics.padding + f32::from(cursor.row) * metrics.height,
        width: metrics.width * f32::from(columns),
        height: metrics.height,
    })
}

pub(super) fn push_cursor(
    rectangles: &mut RectangleBatch,
    cursor: DrawCursor,
    bounds: SceneRect,
    metrics: CellMetrics,
    outline: Option<SceneColor>,
) {
    let thickness = (metrics.width / 7.0).max(1.0);
    if let Some(color) = outline {
        match cursor.shape {
            CursorShape::Bar => rectangles.push(
                bounds.left,
                bounds.top,
                thickness + metrics.scale,
                bounds.height,
                color,
                1.0,
            ),
            CursorShape::Underline => rectangles.push(
                bounds.left,
                bounds.bottom() - thickness - metrics.scale,
                bounds.width,
                thickness + metrics.scale,
                color,
                1.0,
            ),
            CursorShape::BlockHollow => rectangles.push_hollow(
                bounds.left,
                bounds.top,
                bounds.width,
                bounds.height,
                thickness + metrics.scale,
                color,
            ),
            CursorShape::Block => {}
        }
    }
    let (x, y, width, height, alpha) = match cursor.shape {
        CursorShape::Bar => (bounds.left, bounds.top, thickness, bounds.height, 1.0),
        CursorShape::Underline => (
            bounds.left,
            bounds.bottom() - thickness,
            bounds.width,
            thickness,
            1.0,
        ),
        CursorShape::Block => (bounds.left, bounds.top, bounds.width, bounds.height, 0.55),
        CursorShape::BlockHollow => {
            rectangles.push_hollow(
                bounds.left,
                bounds.top,
                bounds.width,
                bounds.height,
                thickness,
                cursor.color,
            );
            return;
        }
    };
    rectangles.push(x, y, width, height, cursor.color, alpha);
    if cursor.shape == CursorShape::Block
        && let Some(color) = outline
    {
        rectangles.push_hollow(
            bounds.left,
            bounds.top,
            bounds.width,
            bounds.height,
            metrics.scale,
            color,
        );
    }
}

fn relative_luminance(color: SceneColor) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast_ratio(left: SceneColor, right: SceneColor) -> f64 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

pub(super) fn cursor_outline(fill: SceneColor) -> SceneColor {
    let black = SceneColor { r: 0, g: 0, b: 0 };
    let white = SceneColor {
        r: 255,
        g: 255,
        b: 255,
    };
    let target = if contrast_ratio(fill, black) >= contrast_ratio(fill, white) {
        black
    } else {
        white
    };
    (1..=255)
        .map(|amount| {
            let blend = |source: u8, target: u8| {
                ((u16::from(source) * (255 - amount) + u16::from(target) * amount + 127) / 255)
                    as u8
            };
            SceneColor {
                r: blend(fill.r, target.r),
                g: blend(fill.g, target.g),
                b: blend(fill.b, target.b),
            }
        })
        .find(|outline| contrast_ratio(fill, *outline) >= 3.0)
        .unwrap_or(target)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::cursor_trail::inset_cursor_quad;
    use super::super::*;
    use super::*;
    use crate::{DrawCell, SceneGraphics};
    use glyphon::fontdb;
    use orbit_protocol::Screen;

    #[test]
    fn accelerando_specialty_routing_preserves_explicit_fonts_and_styles() {
        let mut fonts = FontSystem::new();
        let cell_font = fitted_cell_font(&mut fonts, CellMetrics::for_scale(1.0), None, false);
        let style = DrawStyle {
            bold: true,
            italic: true,
            ..plain_style()
        };
        for codepoint in 0x100000..=0x101fff {
            let text = char::from_u32(codepoint).unwrap().to_string();
            let attrs = cell_text_attrs(cell_font, &text, false, style);
            assert_eq!(attrs.family, Family::Name("Accelerando Terminal"));
            assert_eq!(attrs.weight, Weight::BOLD);
            assert_eq!(attrs.style, Style::Italic);
            let configured = CellFont {
                family: Some("DejaVu Sans Mono"),
                ..cell_font
            };
            assert_eq!(
                cell_text_attrs(configured, &text, false, style).family,
                Family::Name("DejaVu Sans Mono")
            );
        }
        for text in ["A", "\u{fffff}", "\u{102000}"] {
            assert_eq!(
                cell_text_attrs(cell_font, text, false, style).family,
                Family::Monospace
            );
        }
    }

    #[test]
    fn configured_fonts_resolve_fallbacks_and_scale_one_grid() {
        let original = FontSystem::new();
        let named = |name: &str| {
            original
                .db()
                .face(
                    original
                        .db()
                        .query(&fontdb::Query {
                            families: &[Family::Name(name)],
                            ..Default::default()
                        })
                        .expect("the accepted font environment supplies the tested face"),
                )
                .unwrap()
                .clone()
        };
        let mono = named("DejaVu Sans Mono");
        let symbol = named(NERD_FONT_FAMILY);
        let make_system = || {
            let mut db = fontdb::Database::new();
            db.push_face_info(mono.clone());
            for name in ["Venus Fallback A", "Venus Fallback B"] {
                let mut face = symbol.clone();
                face.families.truncate(1);
                face.families[0].0 = name.into();
                db.push_face_info(face);
            }
            FontSystem::new_with_locale_and_db("en-US".into(), db)
        };
        let mut settings = FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            fallbacks: vec!["Venus Fallback A".into(), "Venus Fallback B".into()],
            size: 20.0,
            line_height: 1.5,
        };
        for expected in ["Venus Fallback A", "Venus Fallback B"] {
            let mut fonts = FontSetup::resolve(&settings, make_system()).unwrap();
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let metrics = fonts.metrics(scale);
                assert_eq!(metrics.height, (30.0 * scale as f32).round());
                assert_eq!(metrics.font_size, 20.0 * scale as f32);
                let cell_font =
                    fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, true);
                let mut buffer = Buffer::new(
                    &mut fonts.font_system,
                    Metrics::new(cell_font.size, metrics.height),
                );
                buffer.set_monospace_width(Some(metrics.width));
                buffer.set_text(
                    "\u{f015}",
                    &cell_text_attrs(
                        cell_font,
                        "\u{f015}",
                        false,
                        DrawStyle {
                            bold: true,
                            ..plain_style()
                        },
                    ),
                    Shaping::Advanced,
                    None,
                );
                buffer.shape_until_scroll(&mut fonts.font_system, false);
                let glyph = &buffer.layout_runs().next().unwrap().glyphs[0];
                assert_ne!(glyph.glyph_id, 0);
                assert_eq!(
                    fonts.font_system.db().face(glyph.font_id).unwrap().families[0].0,
                    expected
                );
                let buffer = grid_buffer(
                    &mut fonts.font_system,
                    metrics,
                    cell_font,
                    "abc",
                    false,
                    cell_font.attrs(),
                );
                for (column, glyph) in buffer
                    .layout_runs()
                    .next()
                    .unwrap()
                    .glyphs
                    .iter()
                    .enumerate()
                {
                    assert!((glyph.x - column as f32 * metrics.width).abs() < 0.01);
                }
            }
            settings.fallbacks.reverse();
        }
        settings.family = Some("Missing Venus Font".into());
        assert!(
            FontSetup::resolve(&settings, make_system())
                .err()
                .unwrap()
                .to_string()
                .contains("unavailable")
        );
        settings.family = None;
        settings.fallbacks = vec!["Missing Venus Fallback".into()];
        assert!(
            FontSetup::resolve(&settings, make_system())
                .err()
                .unwrap()
                .to_string()
                .contains("unavailable")
        );
        settings.family = Some("DejaVu Sans".into());
        settings.fallbacks.clear();
        assert!(
            FontSetup::resolve(&settings, FontSystem::new())
                .err()
                .unwrap()
                .to_string()
                .contains("not monospace")
        );
        let defaults = FontSetup::new(&FontSettings::default()).unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = defaults.metrics(scale);
            assert!(metrics.title_cap_top_em.is_some());
            assert_eq!(
                CellMetrics {
                    title_cap_top_em: None,
                    ..metrics
                },
                CellMetrics::for_scale(scale)
            );
        }
    }

    pub(in crate::render) fn plain_style() -> DrawStyle {
        DrawStyle {
            foreground: SceneColor::default(),
            background: palette::TERMINAL_FALLBACK,
            underline_color: SceneColor::default(),
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected: false,
            background_is_default: true,
            protected: false,
            underline: Underline::None,
        }
    }

    fn grid_buffer(
        font_system: &mut FontSystem,
        metrics: CellMetrics,
        cell_font: CellFont,
        text: &str,
        box_drawing: bool,
        attrs: Attrs<'static>,
    ) -> Buffer {
        let attrs = attrs.letter_spacing(cell_font.letter_spacing);
        let box_attrs = attrs
            .clone()
            .metrics(Metrics::new(cell_font.box_size, metrics.height))
            .letter_spacing(cell_font.box_letter_spacing);
        let braille_attrs = attrs.clone().family(Family::Name(BRAILLE_FAMILY));
        let mut buffer = Buffer::new(font_system, Metrics::new(cell_font.size, metrics.height));
        buffer.set_size(
            Some(metrics.width * text.chars().count() as f32),
            Some(metrics.height),
        );
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        set_cell_layer(
            &mut buffer,
            text,
            &cell_text_segments(text),
            &attrs,
            &braille_attrs,
            &box_attrs,
            box_drawing,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
    }

    #[test]
    fn concealed_cells_draw_no_foreground_elements() {
        let style = DrawStyle {
            blink: true,
            invisible: true,
            strikethrough: true,
            overline: true,
            underline: Underline::Single,
            ..plain_style()
        };
        let scene = Scene {
            revision: 1,
            columns: 1,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: palette::TERMINAL_FALLBACK,
            foreground: SceneColor::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![DrawCell {
                    width: CellWidth::Narrow,
                    text: "secret".into(),
                    hyperlink: String::new(),
                    style,
                }],
            }],
        };
        let mut rectangles = RectangleBatch::new(100, 100);

        build_scene_rectangles(
            &mut rectangles,
            &scene,
            &[],
            None,
            true,
            CellMetrics::for_scale(1.0),
            SceneRect {
                left: 0.0,
                top: 0.0,
                width: 100.0,
                height: 100.0,
            },
            true,
        );

        assert!(rectangles.bytes.is_empty());
        assert!(scene.glyph_runs().is_empty());
        assert!(!scene.has_blinking_content());
    }

    #[test]
    fn block_cells_use_exact_cell_rectangles() {
        let colors = [
            SceneColor { r: 255, g: 0, b: 0 },
            SceneColor { r: 255, g: 0, b: 0 },
            SceneColor { r: 0, g: 0, b: 255 },
            SceneColor {
                r: 255,
                g: 0,
                b: 255,
            },
        ];
        for (text, offset, height) in [("█", 0.0, 1.0), ("▀", 0.0, 0.5), ("▄", 0.5, 0.5)] {
            let scene = Scene {
                revision: 1,
                columns: 2,
                rows: 2,
                screen: Screen::Primary,
                title: String::new(),
                working_directory: String::new(),
                graphics: SceneGraphics::default(),
                background: palette::TERMINAL_FALLBACK,
                foreground: SceneColor::default(),
                cursor: None,
                content: colors
                    .chunks_exact(2)
                    .map(|row| DrawRow {
                        wrapped: false,
                        wrap_continuation: false,
                        cells: row
                            .iter()
                            .map(|foreground| DrawCell {
                                width: CellWidth::Narrow,
                                text: text.into(),
                                hyperlink: String::new(),
                                style: DrawStyle {
                                    foreground: *foreground,
                                    faint: *foreground == colors[2],
                                    ..plain_style()
                                },
                            })
                            .collect(),
                    })
                    .collect(),
            };

            let glyph_runs = scene.glyph_runs();
            assert_eq!(glyph_runs.len(), 4);
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let metrics = CellMetrics::for_scale(scale);
                let viewport = SceneRect {
                    left: 7.0,
                    top: 11.0,
                    width: 200.0,
                    height: 200.0,
                };
                let mut actual = RectangleBatch::new(300, 300);
                build_scene_rectangles(
                    &mut actual,
                    &scene,
                    &[],
                    None,
                    true,
                    metrics,
                    viewport,
                    true,
                );
                let mut expected = RectangleBatch::new(300, 300);
                for (index, color) in colors.into_iter().enumerate() {
                    expected.push(
                        viewport.left + metrics.padding + (index % 2) as f32 * metrics.width,
                        viewport.top
                            + metrics.padding
                            + ((index / 2) as f32 + offset) * metrics.height,
                        metrics.width,
                        metrics.height * height,
                        color,
                        if index == 2 { 150.0 / 255.0 } else { 1.0 },
                    );
                }
                assert_eq!(actual.bytes, expected.bytes, "{text}, scale {scale}");
            }
        }
    }

    #[test]
    fn half_blocks_preserve_clipped_preview_styles() {
        let foreground = SceneColor { r: 255, g: 0, b: 0 };
        let background = SceneColor { r: 0, g: 0, b: 255 };
        let row = DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: [
                ("▄", true, false, false),
                ("▀", false, true, false),
                ("▄", false, false, true),
            ]
            .into_iter()
            .map(|(text, faint, blink, invisible)| DrawCell {
                width: CellWidth::Narrow,
                text: text.into(),
                hyperlink: String::new(),
                style: DrawStyle {
                    foreground,
                    background,
                    background_is_default: false,
                    faint,
                    blink,
                    invisible,
                    ..plain_style()
                },
            })
            .collect(),
        };
        let scene = Scene {
            revision: 1,
            columns: 3,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: palette::TERMINAL_FALLBACK,
            foreground,
            cursor: None,
            content: vec![row.clone()],
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let origin = SceneRect {
                left: 7.0,
                top: 50.0,
                width: 200.0,
                height: 200.0,
            };
            for direction in [VerticalDirection::Up, VerticalDirection::Down] {
                let preview = ScenePreview::Viewport {
                    frame_revision: 1,
                    direction,
                    edge_reached: false,
                    rows: vec![row.clone()],
                };
                let left = origin.left + metrics.padding;
                let top = origin.top
                    + metrics.padding
                    + if direction == VerticalDirection::Up {
                        -metrics.height
                    } else {
                        metrics.height
                    };
                let clip = SceneRect {
                    left: left + metrics.width * 0.5,
                    top: top + metrics.height * 0.25,
                    width: metrics.width * 2.0,
                    height: metrics.height * 0.5,
                };
                for blink_visible in [false, true] {
                    let mut actual = RectangleBatch::new(300, 300);
                    actual.clip = Some(clip);
                    build_scene_rectangles(
                        &mut actual,
                        &scene,
                        &[],
                        Some(&preview),
                        blink_visible,
                        metrics,
                        origin,
                        false,
                    );
                    let mut expected = RectangleBatch::new(300, 300);
                    expected.clip = Some(clip);
                    expected.push(
                        left,
                        top,
                        metrics.width * 3.0,
                        metrics.height,
                        background,
                        1.0,
                    );
                    expected.push(
                        left,
                        top + metrics.height * 0.5,
                        metrics.width,
                        metrics.height * 0.5,
                        foreground,
                        150.0 / 255.0,
                    );
                    if blink_visible {
                        expected.push(
                            left + metrics.width,
                            top,
                            metrics.width,
                            metrics.height * 0.5,
                            foreground,
                            1.0,
                        );
                    }
                    assert_eq!(
                        actual.bytes, expected.bytes,
                        "{direction:?}, {scale}, blink {blink_visible}"
                    );
                }
            }
        }
        for (text, narrow) in [
            ("▀\u{301}", true),
            ("▄▄", true),
            ("▌", true),
            ("text", true),
            ("▀", false),
            ("▄", false),
        ] {
            assert!(
                block_span(text, narrow).is_none(),
                "{text}, narrow {narrow}"
            );
        }
    }

    #[test]
    fn wide_cell_cursors_cover_the_complete_glyph() {
        let style = plain_style();
        let mut scene = Scene {
            revision: 1,
            columns: 3,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: palette::TERMINAL_FALLBACK,
            foreground: SceneColor::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: [CellWidth::Narrow, CellWidth::Wide, CellWidth::SpacerTail]
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
        let metrics = CellMetrics::for_scale(1.0);
        let cursor = DrawCursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            column: 0,
            row: 0,
            at_wide_tail: false,
            color: SceneColor::default(),
            explicit_color: false,
        };
        let mut draw_cursor = |cursor| {
            scene.cursor = Some(cursor);
            let mut rectangles = RectangleBatch::new(100, 100);
            build_scene_rectangles(
                &mut rectangles,
                &scene,
                &[],
                None,
                true,
                metrics,
                SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                true,
            );
            rectangles.bytes
        };
        let expected = |column: f32, columns: f32| {
            let mut rectangles = RectangleBatch::new(100, 100);
            rectangles.push(
                metrics.padding + column * metrics.width,
                metrics.padding,
                columns * metrics.width,
                metrics.height,
                cursor.color,
                0.55,
            );
            rectangles.bytes
        };

        assert_eq!(draw_cursor(cursor), expected(0.0, 1.0));
        assert_eq!(
            draw_cursor(DrawCursor {
                column: 1,
                ..cursor
            }),
            expected(1.0, 2.0)
        );
        assert_eq!(
            draw_cursor(DrawCursor {
                column: 2,
                at_wide_tail: true,
                ..cursor
            }),
            expected(1.0, 2.0)
        );
    }

    #[test]
    fn cursor_outline_is_contrasting_and_one_logical_pixel() {
        for fill in [
            SceneColor { r: 0, g: 0, b: 0 },
            SceneColor {
                r: 0x7f,
                g: 0x7f,
                b: 0x7f,
            },
            SceneColor {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
        ]
        .into_iter()
        .chain(
            [
                0xff3b30, 0xffd23f, 0xb7f34a, 0x35c978, 0x7ddcff, 0x5271ff, 0xa970ff, 0xff5da2,
            ]
            .map(|value| SceneColor {
                r: (value >> 16) as u8,
                g: (value >> 8) as u8,
                b: value as u8,
            }),
        ) {
            let outline = cursor_outline(fill);
            assert!(contrast_ratio(fill, outline) >= 3.0, "{fill:?} {outline:?}");
            if relative_luminance(fill) > 0.5 {
                assert!(relative_luminance(outline) < relative_luminance(fill));
            } else if relative_luminance(fill) < 0.05 {
                assert!(relative_luminance(outline) > relative_luminance(fill));
            }
        }

        assert_eq!(
            inset_cursor_quad(
                [
                    CursorPoint { x: 10.0, y: 20.0 },
                    CursorPoint { x: 30.0, y: 20.0 },
                    CursorPoint { x: 30.0, y: 50.0 },
                    CursorPoint { x: 10.0, y: 50.0 },
                ],
                2.0,
            ),
            [
                CursorPoint { x: 12.0, y: 22.0 },
                CursorPoint { x: 28.0, y: 22.0 },
                CursorPoint { x: 28.0, y: 48.0 },
                CursorPoint { x: 12.0, y: 48.0 },
            ]
        );
    }

    #[test]
    fn default_font_matches_nova_at_supported_scales() {
        let mut font_system = FontSystem::new();
        for (scale, width, height, font_size, padding) in [
            (1.0, 10.0, 18.0, 16.0, 12.0),
            (1.25, 13.0, 23.0, 20.0, 15.0),
            (1.5, 15.0, 27.0, 24.0, 18.0),
            (2.0, 20.0, 36.0, 32.0, 24.0),
        ] {
            let metrics = CellMetrics::for_scale(scale);
            assert_eq!(
                (
                    metrics.width,
                    metrics.height,
                    metrics.font_size,
                    metrics.padding
                ),
                (width, height, font_size, padding)
            );
            assert_eq!(
                fitted_cell_font(&mut font_system, metrics, None, false).size,
                font_size
            );
        }
    }

    #[test]
    fn background_opacity_skips_only_default_cell_backgrounds() {
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
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: (0..4)
                    .map(|_| DrawCell {
                        width: CellWidth::Narrow,
                        text: String::new(),
                        hyperlink: String::new(),
                        style: plain_style(),
                    })
                    .collect(),
            }],
        };
        scene.content[0].cells[1].style.background_is_default = false;
        scene.content[0].cells[2].style.background_is_default = false;
        scene.content[0].cells[3].style.background_is_default = false;

        let metrics = CellMetrics::for_scale(1.0);
        let viewport = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let mut actual = RectangleBatch::new(100, 100);
        build_scene_rectangles(
            &mut actual,
            &scene,
            &[],
            None,
            true,
            metrics,
            viewport,
            true,
        );
        let mut expected = RectangleBatch::new(100, 100);
        expected.push(
            metrics.padding + metrics.width,
            metrics.padding,
            metrics.width * 3.0,
            metrics.height,
            palette::TERMINAL_FALLBACK,
            1.0,
        );

        assert_eq!(actual.bytes, expected.bytes);
    }

    #[test]
    fn text_runs_follow_the_cell_grid_and_box_borders_connect() {
        let mut font_system = FontSystem::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for (text, box_drawing, attrs) in [
                ("narrow text", false, cell_font.attrs()),
                ("EON 9.8.7-alpha.2+proof", false, cell_font.attrs()),
                (
                    "┌──┬──┐",
                    true,
                    cell_font
                        .attrs()
                        .weight(Weight::BOLD)
                        .color(Color::rgb(80, 200, 160)),
                ),
                ("italic text", false, cell_font.attrs().style(Style::Italic)),
            ] {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    text,
                    box_drawing,
                    attrs,
                );
                let glyphs = buffer
                    .layout_runs()
                    .next()
                    .expect("the terminal run should shape")
                    .glyphs;
                assert_eq!(glyphs.len(), text.chars().count());
                for (column, glyph) in glyphs.iter().enumerate() {
                    assert!((glyph.x - column as f32 * metrics.width).abs() < 0.01);
                    assert!(
                        (glyph.w - metrics.width).abs() < 0.01,
                        "{text:?} column {column} at scale {scale} has width {} instead of {}",
                        glyph.w,
                        metrics.width
                    );
                }
            }

            let mixed = "e\u{301}│abc│";
            let segments = cell_text_segments(mixed);
            assert_eq!(
                segments,
                [
                    ("e\u{301}", CellTextKind::Text),
                    ("│", CellTextKind::Box),
                    ("abc", CellTextKind::Text),
                    ("│", CellTextKind::Box)
                ]
            );
            let layer_widths = [false, true].map(|box_drawing| {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    mixed,
                    box_drawing,
                    cell_font.attrs(),
                );
                shaped_preedit_placement(&buffer).1
            });
            for layer_width in layer_widths {
                assert!(
                    (layer_width - metrics.width * 6.0).abs() < 0.01,
                    "mixed combining and box text at scale {scale} occupies {layer_width} instead of {}",
                    metrics.width * 6.0
                );
            }
            let fallback_widths = [false, true].map(|box_drawing| {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    "אבג│",
                    box_drawing,
                    cell_font.attrs(),
                );
                shaped_preedit_placement(&buffer).1
            });
            assert!(
                (fallback_widths[0] - fallback_widths[1]).abs() < 0.01,
                "mixed fallback and box layers diverge at scale {scale}: {fallback_widths:?}"
            );

            let buffer = grid_buffer(
                &mut font_system,
                metrics,
                cell_font,
                "│",
                true,
                cell_font.attrs(),
            );
            let run = buffer
                .layout_runs()
                .next()
                .expect("the vertical border should shape");
            let glyph = run
                .glyphs
                .first()
                .expect("the vertical border should exist");
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let image = SwashCache::new()
                .get_image_uncached(&mut font_system, physical.cache_key)
                .expect("the vertical border glyph should rasterize");
            let top = run.line_y.round() as i32 + physical.y - image.placement.top;
            let bottom = top + image.placement.height as i32;
            assert!(
                top <= 0 && bottom >= metrics.height as i32,
                "vertical border at scale {scale} occupies {top}..{bottom} in a {} px row",
                metrics.height
            );
        }

        let metrics = CellMetrics::for_scale(1.0);
        let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
        let buffer = grid_buffer(
            &mut font_system,
            metrics,
            cell_font,
            "─────",
            true,
            cell_font.attrs(),
        );
        let glyphs = buffer
            .layout_runs()
            .next()
            .expect("the table border should shape")
            .glyphs;
        let mut swash_cache = SwashCache::new();
        let ink = glyphs
            .iter()
            .map(|glyph| {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the table border glyph should rasterize");
                let left = physical.x + image.placement.left;
                (left, left + image.placement.width as i32)
            })
            .collect::<Vec<_>>();
        for pair in ink.windows(2) {
            assert!(
                pair[0].1 >= pair[1].0,
                "adjacent table-border cells must not expose a gap: {pair:?}"
            );
        }
    }

    #[test]
    fn braille_progress_uses_distinct_packaged_glyphs_on_the_cell_grid() {
        const SPINNER: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
        assert_eq!(
            cell_text_segments(&format!("{SPINNER}│X")),
            [
                (SPINNER, CellTextKind::Braille),
                ("│", CellTextKind::Box),
                ("X", CellTextKind::Text),
            ]
        );

        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for frame in SPINNER.chars() {
                let text = format!("{frame}│X");
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    &text,
                    false,
                    cell_font.attrs(),
                );
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the Braille frame should shape");
                assert_eq!(run.glyphs.len(), 3);
                let glyph = &run.glyphs[0];
                let face = font_system
                    .db()
                    .face(glyph.font_id)
                    .expect("the Braille face should remain loaded");
                assert!(
                    face.families.iter().any(|(name, _)| name == BRAILLE_FAMILY),
                    "{frame} at scale {scale} used {}",
                    face.post_script_name
                );
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the Braille frame should rasterize");
                let left = physical.x + image.placement.left;
                let right = left + image.placement.width as i32;
                let top = run.line_y.round() as i32 + physical.y
                    - image.placement.top
                    - cell_font.top_offset as i32;
                let bottom = top + image.placement.height as i32;
                assert!(
                    left >= 0
                        && right <= metrics.width as i32
                        && top >= 0
                        && bottom <= metrics.height as i32,
                    "{frame} at scale {scale} occupies {left}..{right} by {top}..{bottom} in a {} by {} cell",
                    metrics.width,
                    metrics.height
                );
            }
        }
    }

    #[test]
    fn descender_rasters_fit_the_cell_row_at_supported_scales() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let mut failures = Vec::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for (style, attrs) in [
                ("regular", cell_font.attrs()),
                ("bold", cell_font.attrs().weight(Weight::BOLD)),
                ("italic", cell_font.attrs().style(Style::Italic)),
            ] {
                let text = "gjpqy";
                let mut buffer = Buffer::new(
                    &mut font_system,
                    Metrics::new(cell_font.size, metrics.height),
                );
                buffer.set_size(
                    Some(metrics.width * text.chars().count() as f32),
                    Some(metrics.height),
                );
                buffer.set_wrap(Wrap::None);
                buffer.set_monospace_width(Some(metrics.width));
                buffer.set_text(text, &attrs, shaping(text), None);
                buffer.shape_until_scroll(&mut font_system, false);
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the descender corpus should shape");
                assert_eq!(run.glyphs.len(), text.chars().count());
                for (character, glyph) in text.chars().zip(run.glyphs) {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    let image = swash_cache
                        .get_image_uncached(&mut font_system, physical.cache_key)
                        .expect("the descender glyph should rasterize");
                    let top = run.line_y.round() as i32 + physical.y
                        - image.placement.top
                        - cell_font.top_offset as i32;
                    let bottom = top + image.placement.height as i32;
                    if top < 0 || bottom > metrics.height as i32 {
                        failures.push(format!(
                            "{style} {character:?} at scale {scale} occupies {top}..{bottom} in a {} px row",
                            metrics.height
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn nerd_font_symbols_use_the_packaged_face_and_bounded_blank_space() {
        assert!(is_nerd_font_symbol("\u{e000}"));
        assert!(is_nerd_font_symbol("\u{f8ff}"));
        assert!(is_nerd_font_symbol("\u{f0000}"));
        assert!(is_nerd_font_symbol("\u{f2fff}"));
        assert!(!is_nerd_font_symbol("\u{f900}"));
        assert!(!is_nerd_font_symbol("\u{f3000}"));

        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let metrics = CellMetrics::for_scale(1.0);
        let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
        let mut observed_overhang = false;
        for symbol in ["\u{e5ff}", "\u{f015}", "\u{f15b}", "\u{f489}", "\u{f0868}"] {
            let geometry = [false, true].map(|bold| {
                let attrs = cell_text_attrs(
                    cell_font,
                    symbol,
                    false,
                    DrawStyle {
                        bold,
                        ..plain_style()
                    },
                );
                assert_eq!(attrs.weight, Weight::NORMAL);
                let buffer =
                    grid_buffer(&mut font_system, metrics, cell_font, symbol, false, attrs);
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the Nerd Font symbol should shape");
                let glyph = run
                    .glyphs
                    .first()
                    .expect("the Nerd Font symbol should emit one glyph");
                let face = font_system
                    .db()
                    .face(glyph.font_id)
                    .expect("the selected symbol face should remain loaded");
                assert!(
                    face.families
                        .iter()
                        .any(|(family, _)| family == NERD_FONT_FAMILY),
                    "{symbol:?} used {}",
                    face.post_script_name
                );
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the Nerd Font symbol should rasterize");
                let left = physical.x + image.placement.left;
                let right = left + image.placement.width as i32;
                observed_overhang |= right > metrics.width as i32;
                assert!(
                    left >= 0 && right <= (metrics.width * 2.0) as i32,
                    "{symbol:?} occupies {left}..{right} with one blank cell available"
                );
                (
                    glyph.font_id,
                    glyph.font_size.to_bits(),
                    glyph.w.to_bits(),
                    left,
                    right,
                )
            });
            assert_eq!(geometry[0], geometry[1]);
        }
        assert!(
            observed_overhang,
            "the corpus must exercise the clipping defect"
        );

        let symbol = GlyphRun {
            column: 0,
            row: 0,
            columns: 1,
            text: "\u{f015}".into(),
            style: plain_style(),
        };
        let label = GlyphRun {
            column: 2,
            text: "x".into(),
            ..symbol.clone()
        };
        assert_eq!(cell_ink_right(&symbol, Some(&label), 4), 2);
        assert_eq!(
            cell_ink_right(
                &symbol,
                Some(&GlyphRun {
                    column: 1,
                    ..label.clone()
                }),
                4,
            ),
            1
        );
        assert_eq!(cell_ink_right(&label, None, 4), 3);
    }
}
