//! Eon workspace chrome drawing: tabs, pane headers, Eon Bar controls and quota chips, the
//! header tooltip and the native dialog, plus the pointer state that highlights them.

use super::{
    CellMetrics, CursorPoint, DrawStyleKind, FrameTitle, FrameTitleTab, PaneTitleTab, PlacedText,
    RectangleBatch, Renderer, SceneColor,
    cells::{blend_to_contrast, contrast_ratio},
    palette, physical_outline_width, shaping,
};
use crate::{
    DialogScene, Load, SceneRect, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceScene,
    scene::{
        HintSection, QuotaProvider, SIDEBAR_EMPTY, SIDEBAR_TITLE, ShortcutHintGroup,
        ShortcutHintsScene, TabBadge, WorkspaceControl,
        tokens::{self, chrome_title_metrics},
    },
};
use eon_workspace_protocol::v41::AttentionState;
use glyphon::{
    Attrs, Buffer, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, SwashCache, TextBounds, Wrap,
};
use winit::dpi::PhysicalSize;

const OPENAI_BLOSSOM_ID: u16 = 1;
const CLAUDE_SPLAT_ID: u16 = 2;
const EON_ICON_ID: u16 = 3;
const QUOTA_ICON_SIZE: usize = 104;
/// The sides an Eon icon image may have, in pixels.
const EON_ICON_SIDES: std::ops::RangeInclusive<usize> = 16..=512;
/// The last row of a keycap popup that cannot show every column: the Shortcuts viewer lists them.
static HINT_POPUP_MORE: std::sync::LazyLock<(String, &str)> = std::sync::LazyLock::new(|| {
    use eon_workspace_protocol::shortcuts::{Action, Platform, shortcut};
    (
        shortcut(Action::ToggleViewer)
            .trigger
            .display_label_for(Platform::CURRENT),
        "lists all",
    )
});

/// The edge of the bar a tooltip opens from: below the header at this top, or above the
/// shortcut hints with this bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TooltipEdge {
    Below(f32),
    Above(f32),
}

/// Where the tooltip of the bar item at `anchor` opens: above the hints for the version label,
/// which sits in them, and below the header for the rest.
fn tooltip_edge(
    workspace: &WorkspaceScene,
    anchor: SceneRect,
    metrics: CellMetrics,
) -> TooltipEdge {
    match &workspace.hints {
        Some(hints) if anchor.top >= hints.rect.top => {
            TooltipEdge::Above(hints.rect.top - tokens::tooltip_gap(metrics))
        }
        _ => TooltipEdge::Below(workspace.header.bottom() + tokens::tooltip_gap(metrics)),
    }
}

/// The widest and tallest a tooltip opening at `edge` in a window `size` may be: 40 ems at
/// most, and never past the window's padding, the header or the hints.
fn tooltip_room(
    size: PhysicalSize<u32>,
    metrics: CellMetrics,
    workspace: &WorkspaceScene,
    edge: TooltipEdge,
) -> (f32, f32) {
    let padding = metrics.padding;
    let width = (size.width as f32 - padding * 2.0).min(metrics.font_size * 40.0);
    let height = match edge {
        TooltipEdge::Below(top) => size.height as f32 - top - padding,
        TooltipEdge::Above(bottom) => {
            bottom - workspace.header.bottom() - tokens::tooltip_gap(metrics)
        }
    };
    (width, height)
}

/// A tooltip `width` by `height` at `anchor`'s left, kept inside a window `window` wide, on the
/// `edge` it names.
fn tooltip_rect(
    window: f32,
    padding: f32,
    anchor: f32,
    width: f32,
    height: f32,
    edge: TooltipEdge,
) -> SceneRect {
    SceneRect {
        left: anchor.clamp(padding, (window - width - padding).max(padding)),
        top: match edge {
            TooltipEdge::Below(top) => top,
            TooltipEdge::Above(bottom) => bottom - height,
        },
        width,
        height,
    }
}

/// One line of a keycap popup's column.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HintPopupLine<'a> {
    /// A section's title, or none above the rest of a section's rows.
    Title(Option<&'a str>),
    /// Keys and their short label, with the index of the group row they list; none for the
    /// pointer at full help.
    Row(&'a str, &'a str, Option<usize>),
}

/// One column of a keycap popup: its lines, the width of their keys, and its whole width.
#[derive(Clone, Debug, PartialEq)]
struct HintPopupColumn<'a> {
    lines: Vec<HintPopupLine<'a>>,
    keys: f32,
    width: f32,
}

/// A keycap popup's columns with room for `lines` lines: each section starts a column with its
/// title, and rows past the room continue in the next column under an empty title. None without
/// room for a title and a row.
fn hint_popup_columns(
    group: &ShortcutHintGroup,
    lines: f32,
) -> Option<Vec<Vec<HintPopupLine<'_>>>> {
    if !lines.is_finite() || lines < 2.0 {
        return None;
    }
    let lines = lines as usize;
    let mut columns: Vec<Vec<HintPopupLine<'_>>> = Vec::new();
    for section in HintSection::ALL {
        let mut title = Some(section.title());
        for (index, row) in group.rows.iter().enumerate() {
            if row.section != section {
                continue;
            }
            if let Some(title) = title.take() {
                columns.push(vec![HintPopupLine::Title(Some(title))]);
            } else if columns.last().is_some_and(|column| column.len() == lines) {
                columns.push(vec![HintPopupLine::Title(None)]);
            }
            if let Some(column) = columns.last_mut() {
                column.push(HintPopupLine::Row(&row.key, &row.short, Some(index)));
            }
        }
    }
    Some(columns)
}

/// A keycap popup laid out with room for `lines` lines and `room` pixels across, columns one
/// padding apart inside one padding of margin. Columns past the room give way to a last row
/// that points at full help. None without room for a title and a row.
fn hint_popup_layout<'a>(
    group: &'a ShortcutHintGroup,
    lines: f32,
    room: f32,
    padding: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> Option<Vec<HintPopupColumn<'a>>> {
    let mut column = |lines: Vec<HintPopupLine<'a>>| {
        let (mut keys, mut labels, mut titles) = (0.0_f32, 0.0_f32, 0.0_f32);
        for line in &lines {
            match *line {
                HintPopupLine::Title(title) => titles = titles.max(title.map_or(0.0, &mut measure)),
                HintPopupLine::Row(key, label, _) => {
                    keys = keys.max(measure(key));
                    labels = labels.max(measure(label));
                }
            }
        }
        HintPopupColumn {
            width: titles.max(keys + padding + labels),
            lines,
            keys,
        }
    };
    let most = lines as usize;
    let all = hint_popup_columns(group, lines)?
        .into_iter()
        .map(&mut column)
        .collect::<Vec<_>>();
    let across = |columns: &[HintPopupColumn<'_>]| {
        columns
            .iter()
            .map(|column| column.width + padding)
            .sum::<f32>()
            + padding
    };
    if all.len() < 2 || across(&all) <= room {
        return Some(all);
    }
    // The most columns that fit with the pointer ending the last, and at least the first.
    let mut count = all.len() - 1;
    loop {
        let mut lines = all[count - 1].lines.clone();
        if lines.len() == most {
            // The pointer takes the last row's place; above only a title, the title's.
            let last = lines.len() - 1;
            let title = matches!(lines[last - 1], HintPopupLine::Title(_));
            lines.remove(if title { last - 1 } else { last });
        }
        lines.push(HintPopupLine::Row(
            HINT_POPUP_MORE.0.as_str(),
            HINT_POPUP_MORE.1,
            None,
        ));
        let mut kept = all[..count - 1].to_vec();
        kept.push(column(lines));
        if count == 1 || across(&kept) <= room {
            return Some(kept);
        }
        count -= 1;
    }
}

fn scroll_thumb(track: SceneRect, viewport: f32, scroll: f32, max_scroll: f32) -> SceneRect {
    let height = (track.height * viewport / (viewport + max_scroll))
        .max(track.width * 2.0)
        .min(track.height);
    SceneRect {
        top: track.top + (track.height - height) * scroll / max_scroll,
        height,
        ..track
    }
}

fn contrast(a: SceneColor, b: SceneColor) -> f32 {
    let luminance = |color: SceneColor| {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `fill` at `opacity` over a white desktop: a translucent card's lightest case, the one that
/// bounds light text on it.
fn over_bright_desktop(fill: SceneColor, opacity: f32) -> SceneColor {
    let resolved = |value: u8| (f32::from(value) * opacity + 255.0 * (1.0 - opacity)).round() as u8;
    SceneColor {
        r: resolved(fill.r),
        g: resolved(fill.g),
        b: resolved(fill.b),
    }
}

/// A Stack label's colour on its column's terminal background `fill`: primary text, blended
/// toward black only as far as 4.5:1 needs when that background is itself light. Like the
/// terminal's own text, it is judged against the terminal's colour, not the translucent result:
/// what shows through depends on the desktop, and assuming a bright one would darken labels on a
/// dark terminal over a dark desktop.
fn stack_label_color(fill: SceneColor) -> SceneColor {
    super::cells::blend_to_contrast(palette::PRIMARY_TEXT, fill, 4.5)
}

fn pane_id_color(
    name: SceneColor,
    selected: bool,
    live: bool,
    fill: SceneColor,
    opacity: f32,
) -> SceneColor {
    if !live {
        return name;
    }
    let dim = if selected {
        palette::SECONDARY_TEXT
    } else {
        palette::PANE_ID
    };
    // A bright desktop bounds the translucent fill; frame and hover remain opaque.
    let resolved = over_bright_desktop(fill, opacity);
    if contrast(dim, resolved) >= 4.5 && contrast(dim, palette::BAR_BACKGROUND) >= 4.5 {
        dim
    } else {
        name
    }
}

fn color_label_identity(buffer: &mut Buffer, fonts: &mut FontSystem, id: &str, color: SceneColor) {
    let label = buffer
        .lines
        .iter()
        .map(|line| line.text())
        .collect::<String>();
    if !label.starts_with(id) {
        return;
    }
    let attrs = Attrs::new().family(Family::SansSerif);
    buffer.set_rich_text(
        [
            (
                &label[..id.len()],
                attrs.clone().color(Color::rgb(color.r, color.g, color.b)),
            ),
            (&label[id.len()..], attrs.clone()),
        ],
        &attrs,
        shaping(&label),
        None,
    );
    buffer.shape_until_scroll(fonts, false);
}

fn load_color(load: Load) -> SceneColor {
    match load {
        Load::Normal => palette::SECONDARY_TEXT,
        Load::Notice => palette::LOAD_NOTICE,
        Load::High => palette::LOAD_HIGH,
        Load::Critical => palette::LOAD_CRITICAL,
    }
}

fn quota_pace_color(shortfall: f32) -> SceneColor {
    let gradient = palette::QUOTA_PACE;
    if shortfall <= gradient[0].0 {
        return gradient[0].1;
    }
    gradient
        .windows(2)
        .find(|pair| shortfall <= pair[1].0)
        .map_or(gradient[gradient.len() - 1].1, |pair| {
            let ((from, start), (to, end)) = (pair[0], pair[1]);
            let share = (shortfall - from) / (to - from);
            let mix = |start: u8, end: u8| {
                (f32::from(start) + (f32::from(end) - f32::from(start)) * share).round() as u8
            };
            SceneColor {
                r: mix(start.r, end.r),
                g: mix(start.g, end.g),
                b: mix(start.b, end.b),
            }
        })
}

/// Recolors the judged chip's whole label without changing its shaped width.
fn color_quota_pace(buffer: &mut Buffer, fonts: &mut FontSystem, label: &str, shortfall: f32) {
    let color = quota_pace_color(shortfall);
    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .color(Color::rgb(color.r, color.g, color.b));
    buffer.set_text(label, &attrs, shaping(label), None);
    buffer.shape_until_scroll(fonts, false);
}

const OPENAI_BLOSSOM_ALPHA: &[u8; QUOTA_ICON_SIZE * QUOTA_ICON_SIZE] =
    include_bytes!("../../assets/openai-blossom-104.alpha");
const CLAUDE_SPLAT_ALPHA: &[u8; QUOTA_ICON_SIZE * QUOTA_ICON_SIZE] =
    include_bytes!("../../assets/claude-splat-104.alpha");

/// Pointer hover and press state for the workspace chrome, part of the content cache key.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct HeaderInteraction {
    hovered_header: Option<(WorkspaceFocus, String)>,
    quota_hovered: Option<usize>,
    version_hovered: bool,
    hint_group_hovered: Option<usize>,
    hint_popup: bool,
    /// The row of the hovered keycap's popup under the pointer.
    hint_row_hovered: Option<usize>,
    cpu_hovered: bool,
    ram_hovered: bool,
    disk_hovered: bool,
    clock_hovered: bool,
    pressed_header: Option<WorkspaceHeaderControl>,
    pub(super) pressed_pane: Option<(String, crate::PaneHeaderAction)>,
    pressed_heading: Option<crate::HeadingAction>,
}

/// One header button's interaction state.
#[derive(Clone, Copy)]
struct HeaderButtonState {
    hovered: bool,
    pressed: bool,
    focused: bool,
    enabled: bool,
}

impl HeaderInteraction {
    fn hovers(&self, focus: WorkspaceFocus, id: &str) -> bool {
        self.hovered_header
            .as_ref()
            .is_some_and(|(region, target)| *region == focus && target == id)
    }
}

/// The drawn keycap popup, for the pointer: its group, its box down to the bar, and each listed
/// row's line.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct HintPopupHit {
    group: usize,
    rect: SceneRect,
    rows: Vec<(SceneRect, usize)>,
}

impl HintPopupHit {
    /// Group `group`'s popup drawn at `popup` above a bar whose top is `bar`, with each row's
    /// line laid out from the popup's corner. The pointer keeps the popup from the bar up, across
    /// the gap below it.
    fn new(group: usize, popup: SceneRect, bar: f32, rows: Vec<(SceneRect, usize)>) -> Self {
        Self {
            group,
            rect: SceneRect {
                height: bar.max(popup.bottom()) - popup.top,
                ..popup
            },
            rows: rows
                .into_iter()
                .map(|(rect, row)| {
                    (
                        SceneRect {
                            left: rect.left + popup.left,
                            top: rect.top + popup.top,
                            ..rect
                        },
                        row,
                    )
                })
                .collect(),
        }
    }

    /// The popup's group and the row at `x`, `y`, when the pointer there is over the popup.
    fn at(&self, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        self.rect.contains(x, y).then(|| {
            (
                self.group,
                self.rows
                    .iter()
                    .find(|(rect, _)| rect.contains(x, y))
                    .map(|(_, row)| *row),
            )
        })
    }
}

/// The height of every line `text` wraps to in word-wrapped status typography `width` pixels
/// wide.
fn wrapped_text_height(
    font_system: &mut FontSystem,
    metrics: CellMetrics,
    cell_font: super::cells::CellFont,
    text: &str,
    width: f32,
) -> f32 {
    super::shape_styled_text(
        font_system,
        metrics,
        cell_font,
        text,
        width,
        f32::MAX,
        DrawStyleKind::Status(Wrap::Word),
    )
    .layout_runs()
    .map(|run| run.line_top + run.line_height)
    .fold(0.0, f32::max)
}

/// The box above a keycap popup at `popup` that holds a row's full-help text `text_height`
/// high: the popup's width, a third of a padding above it. None when it would reach within a
/// third of a padding of the header, whose bottom is `top`.
fn hint_detail_rect(
    popup: SceneRect,
    text_height: f32,
    metrics: CellMetrics,
    top: f32,
) -> Option<SceneRect> {
    let height = text_height.ceil() + metrics.padding * 2.0;
    let rect = SceneRect {
        top: popup.top - tokens::tooltip_gap(metrics) - height,
        height,
        ..popup
    };
    (text_height > 0.0 && rect.top >= top + tokens::tooltip_gap(metrics)).then_some(rect)
}

fn text_bounds(rect: SceneRect) -> TextBounds {
    TextBounds {
        left: rect.left.floor() as i32,
        top: rect.top.floor() as i32,
        right: rect.right().ceil() as i32,
        bottom: rect.bottom().ceil() as i32,
    }
}

/// Reads the Eon icon Eon names with `--eon-icon`: a square image, four bytes per pixel (red,
/// green, blue and premultiplied alpha) row by row, 16 to 512 pixels a side.
///
/// # Errors
///
/// Names the file when it cannot be read or is not such an image.
pub fn read_eon_icon(path: &std::path::Path) -> Result<std::sync::Arc<[u8]>, String> {
    use std::io::Read;
    let limit = EON_ICON_SIDES.end() * EON_ICON_SIDES.end() * 4;
    let mut image = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut image))
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let side = (image.len() / 4).isqrt();
    if side * side * 4 != image.len() || !EON_ICON_SIDES.contains(&side) {
        return Err(format!(
            "{} is not a square RGBA image of 16 to 512 pixels a side",
            path.display()
        ));
    }
    Ok(image.into())
}

/// Shapes `glyph` in the status font and returns it with the origin that puts its ink's centre on
/// `centre`, in whole physical pixels. A glyph's side bearings and baseline leave its ink off the
/// middle of its layout box, so the ink box comes from the rasterized glyphs, placed as glyphon
/// places them; a whole-pixel origin moves that ink by exactly the origin.
pub(super) fn ink_centred_glyph(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    metrics: CellMetrics,
    glyph: &str,
    centre: (f32, f32),
) -> (Buffer, f32, f32) {
    let (buffer, (left, top, right, bottom)) =
        status_glyph_ink(font_system, swash_cache, metrics, glyph);
    let origin_left = (centre.0 - (left + right) as f32 / 2.0).round();
    let origin_top = (centre.1 - (top + bottom) as f32 / 2.0).round();
    (buffer, origin_left, origin_top)
}

/// Shapes `glyph` like [`ink_centred_glyph`] and returns the origin that centres its ink
/// vertically on `centre_y` and leaves as many whole pixels between its ink and `target`'s right
/// edge as between its ink and the window's top. When that would put ink outside `target`, as in
/// a narrow window, the ink centres on `centre_x` instead, pulled inside `target` where it fits.
pub(super) fn ink_cornered_glyph(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    metrics: CellMetrics,
    glyph: &str,
    centre: (f32, f32),
    target: SceneRect,
) -> (Buffer, f32, f32) {
    let (buffer, (left, top, right, bottom)) =
        status_glyph_ink(font_system, swash_cache, metrics, glyph);
    let inset = (centre.1 - (bottom - top) as f32 / 2.0).round();
    let pixels = super::text_clip_bounds(target);
    let mut origin_left = target.right().round() - inset - right as f32;
    if origin_left as i32 + left < pixels.left || origin_left as i32 + right > pixels.right {
        origin_left = (centre.0 - (left + right) as f32 / 2.0)
            .round()
            .min((pixels.right - right) as f32)
            .max((pixels.left - left) as f32);
    }
    (buffer, origin_left, inset - top as f32)
}

/// The glyph of a header control whose ink Venus places itself, with its origin: Shortcuts' `?`
/// centred in its slot, and Close's `×` at the window's corner. None for the others, which
/// draw their layout box.
pub(super) fn placed_control_glyph(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    metrics: CellMetrics,
    workspace: &WorkspaceScene,
    control: &WorkspaceControl,
) -> Option<(Buffer, f32, f32)> {
    let square = workspace.control_glyph_rect(control, metrics);
    let centre = (
        square.left + square.width / 2.0,
        square.top + square.height / 2.0,
    );
    let glyph = control.kind.glyph();
    match control.kind {
        WorkspaceHeaderControl::Shortcuts => Some(ink_centred_glyph(
            font_system,
            swash_cache,
            metrics,
            glyph,
            centre,
        )),
        WorkspaceHeaderControl::CloseTab => Some(ink_cornered_glyph(
            font_system,
            swash_cache,
            metrics,
            glyph,
            (control.centre, centre.1),
            control.rect,
        )),
        _ => None,
    }
}

/// The label box of a tab or workspace chip in `slot`: inside its padding, as tall as its pill.
fn chip_label_box(workspace: &WorkspaceScene, slot: SceneRect, metrics: CellMetrics) -> SceneRect {
    SceneRect {
        left: slot.left + metrics.padding,
        width: (slot.width - metrics.padding * 2.0).max(1.0),
        ..workspace.bar_pill_rect(slot, metrics)
    }
}

/// The label box of hint text from `left`, `width` wide, on `row`: as tall as the hints bar and
/// centred on the text's terminal-height line at the row's top, so the text keeps its place and
/// clips at the bar, not at that line.
fn hint_label_box(
    hints: &ShortcutHintsScene,
    row: SceneRect,
    left: f32,
    width: f32,
    metrics: CellMetrics,
) -> SceneRect {
    let line = chrome_title_metrics(metrics).line_height;
    let height = hints.rect.height.max(line);
    SceneRect {
        left,
        top: row.top + (line - height) / 2.0,
        width,
        height,
    }
}

/// The pixels a chip label in `label` may draw: the chip's height, not its line's, within `clip`.
fn chip_label_bounds(label: SceneRect, clip: SceneRect) -> Option<TextBounds> {
    SceneRect {
        width: label.width.max(1.0),
        ..label
    }
    .intersection(clip)
    .map(super::text_clip_bounds)
}

/// The top at which `buffer`, one header line shaped `line_height` tall, draws in `chip`:
/// centred there. Only when the face's descent would pass the chip's bottom, as a face taller
/// than the chip at a large font size can, it moves by whole pixels to centre the label's own ink
/// in the chip instead, or to keep the descenders inside when that ink is taller than the chip.
fn chip_label_top(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    buffer: &mut Buffer,
    chip: SceneRect,
    line_height: f32,
) -> f32 {
    let top = chip.top + (chip.height - line_height) / 2.0;
    let line_y = buffer.layout_runs().next().map(|run| run.line_y);
    let descent = buffer
        .line_layout(font_system, 0)
        .and_then(|lines| lines.first())
        .map(|line| line.max_descent);
    let (Some(line_y), Some(descent)) = (line_y, descent) else {
        return top;
    };
    // Glyphon puts the baseline at the truncated top plus the rounded line offset.
    let baseline = top.trunc() + line_y.round();
    if baseline + descent <= chip.bottom() {
        return top;
    }
    // The label's ink rows from its baseline.
    let mut ink: Option<(i32, i32)> = None;
    for glyph in buffer.layout_runs().flat_map(|run| run.glyphs) {
        let physical = glyph.physical((0.0, 0.0), 1.0);
        let Some(image) = swash_cache.get_image(font_system, physical.cache_key) else {
            continue;
        };
        if image.placement.height == 0 {
            continue;
        }
        let glyph_top = physical.y - image.placement.top;
        let glyph_bottom = glyph_top + image.placement.height as i32;
        ink = Some(ink.map_or((glyph_top, glyph_bottom), |(top, bottom)| {
            (top.min(glyph_top), bottom.max(glyph_bottom))
        }));
    }
    let Some((ink_top, ink_bottom)) = ink else {
        return top;
    };
    let rows = super::text_clip_bounds(chip);
    let centred = (rows.top + rows.bottom - ink_top - ink_bottom).div_euclid(2);
    top + (centred.min(rows.bottom - ink_bottom) as f32 - baseline)
}

/// `glyph` shaped in the status font, and its ink box from an origin at zero in whole physical
/// pixels: left, top, right and bottom. A glyph without ink gives its layout box.
pub(super) fn status_glyph_ink(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    metrics: CellMetrics,
    glyph: &str,
) -> (Buffer, (i32, i32, i32, i32)) {
    let title = chrome_title_metrics(metrics);
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(title.font_size, title.line_height),
    );
    buffer.set_size(None, None);
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        glyph,
        &Attrs::new().family(Family::SansSerif),
        shaping(glyph),
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    let mut ink: Option<(i32, i32, i32, i32)> = None;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let Some(image) = swash_cache.get_image(font_system, physical.cache_key) else {
                continue;
            };
            let (width, height) = (image.placement.width as i32, image.placement.height as i32);
            if width == 0 || height == 0 {
                continue;
            }
            let left = physical.x + image.placement.left;
            let top = run.line_y.round() as i32 + physical.y - image.placement.top;
            ink = Some(ink.map_or((left, top, left + width, top + height), |ink| {
                (
                    ink.0.min(left),
                    ink.1.min(top),
                    ink.2.max(left + width),
                    ink.3.max(top + height),
                )
            }));
        }
    }
    let ink = ink.unwrap_or((
        0,
        0,
        buffer
            .layout_runs()
            .map(|run| run.line_w.ceil() as i32)
            .max()
            .unwrap_or(0),
        title.line_height.ceil() as i32,
    ));
    (buffer, ink)
}

/// Area-averages a provider mark, or the Eon icon when Eon supplied one, to the requested
/// physical pixels.
pub(super) fn rasterize_bar_icon(
    request: RasterizeCustomGlyphRequest,
    eon_icon: Option<&[u8]>,
) -> Option<RasterizedCustomGlyph> {
    let (source, channels, content_type): (&[u8], usize, _) = match request.id {
        OPENAI_BLOSSOM_ID => (OPENAI_BLOSSOM_ALPHA, 1, ContentType::Mask),
        CLAUDE_SPLAT_ID => (CLAUDE_SPLAT_ALPHA, 1, ContentType::Mask),
        // Premultiplied, so averaging never bleeds a clear pixel's colour into the mark.
        EON_ICON_ID => (eon_icon?, 4, ContentType::Color),
        _ => return None,
    };
    let size = (source.len() / channels).isqrt();
    let width = usize::from(request.width);
    let height = usize::from(request.height);
    if width == 0 || height == 0 {
        return None;
    }
    let mut data = vec![0; width * height * channels];
    for y in 0..height {
        let top = y * size / height;
        let bottom = ((y + 1) * size).div_ceil(height).max(top + 1).min(size);
        for x in 0..width {
            let left = x * size / width;
            let right = ((x + 1) * size).div_ceil(width).max(left + 1).min(size);
            let mut sums = [0_u64; 4];
            let count = ((right - left) * (bottom - top)) as u64;
            for source_y in top..bottom {
                for source_x in left..right {
                    let pixel = (source_y * size + source_x) * channels;
                    for (sum, value) in sums.iter_mut().zip(&source[pixel..pixel + channels]) {
                        *sum += u64::from(*value);
                    }
                }
            }
            let pixel = &mut data[(y * width + x) * channels..][..channels];
            if let [red, green, blue, alpha] = pixel {
                // Glyphon blends colour glyphs with straight alpha.
                let coverage = sums[3];
                for (channel, sum) in [red, green, blue].into_iter().zip(sums) {
                    *channel = (sum * 255 + coverage / 2)
                        .checked_div(coverage)
                        .map_or(0, |value| value.min(255) as u8);
                }
                *alpha = (coverage / count) as u8;
            } else {
                pixel[0] = (sums[0] / count) as u8;
            }
        }
    }
    Some(RasterizedCustomGlyph { data, content_type })
}

/// A working badge's phases, Nova's braille frames ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏, each as its offset from
/// U+2800: dot n, numbered down the left column (1–3) and then the right (4–6), is bit n − 1.
const SPINNER_FRAMES: [u8; 10] = [0x0B, 0x19, 0x39, 0x38, 0x3C, 0x34, 0x26, 0x27, 0x07, 0x0F];

/// A working badge's spinner within the static rectangle vertices: a tick rewrites only these
/// bytes, which keep their length in every phase. The badge lies wholly inside its clip, so the
/// rewrite needs none.
struct SpinnerSlot {
    offset: usize,
    badge: TabBadge,
    color: SceneColor,
}

/// The working badges in the current static vertices and the phase they are drawn at. On a
/// software adapter, where every turn re-rasterizes the whole frame, or when Eon turns the
/// animation off, there is no phase: working badges are drawn still and never tick (VEN-C21).
/// Working and background badges take the window's focus color.
pub(super) struct Spinners {
    slots: Vec<SpinnerSlot>,
    phase: Option<usize>,
    focus: SceneColor,
}

/// What an attention badge's glyph sits on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Backing {
    /// An opaque bar pill and the darker bar around its overhanging badge.
    BarPill(SceneColor),
    /// An opaque surface of this color, already painted.
    Surface(SceneColor),
    /// A translucent card's nominal color; its painted color also depends on the desktop.
    Translucent(SceneColor),
}

impl Backing {
    /// A card in `fill` at the terminal-background `opacity`.
    fn card(fill: SceneColor, opacity: f32) -> Self {
        if opacity >= 1.0 {
            Self::Surface(fill)
        } else {
            Self::Translucent(fill)
        }
    }
}

/// Whether working badges turn: only on a hardware adapter, and only while
/// `--animate-working-badge` is true (VEN-C21).
pub(super) fn spinners_turn(device: wgpu::DeviceType, animate: bool) -> bool {
    animate && device != wgpu::DeviceType::Cpu
}

impl Spinners {
    pub(super) fn new(turning: bool, focus_color: SceneColor) -> Self {
        Self {
            slots: Vec::new(),
            phase: turning.then_some(0),
            focus: focus_color,
        }
    }

    pub(super) fn clear(&mut self) {
        self.slots.clear();
    }

    /// Draws a bare glyph at 3:1 before stale dimming, or on a quiet opaque square when the
    /// translucent surface is unknown. Spinner ticks keep the same resolved color.
    fn push_badge(&mut self, badge: TabBadge, backing: Backing, rectangles: &mut RectangleBatch) {
        if !badge_inside(badge, rectangles.clip) {
            return;
        }
        let base = match badge.state {
            AttentionState::NeedsInput => palette::ATTENTION_NEEDS_INPUT,
            AttentionState::Error => palette::ATTENTION_ERROR,
            AttentionState::Finished => palette::ATTENTION_FINISHED,
            AttentionState::Working | AttentionState::Background => self.focus,
            AttentionState::Idle | AttentionState::Unavailable => palette::ATTENTION_IDLE,
        };
        let (base, under) = match backing {
            Backing::BarPill(color) => {
                (blend_to_contrast(base, palette::BAR_BACKGROUND, 3.0), color)
            }
            Backing::Surface(color) => (base, color),
            Backing::Translucent(color) => {
                let square = blend_to_contrast(color, color, 1.5);
                rectangles.push_rounded(
                    badge.rect,
                    tokens::badge_backing_radius(badge.rect.width),
                    square,
                );
                (base, square)
            }
        };
        let color = blend_to_contrast(base, under, 3.0);
        if badge.state == AttentionState::Working && self.phase.is_some() {
            self.slots.push(SpinnerSlot {
                offset: rectangles.bytes.len(),
                badge,
                color,
            });
        }
        push_badge_glyph(rectangles, badge, self.phase, color);
    }

    fn push_edge_badge(
        &mut self,
        badge: TabBadge,
        pills: impl IntoIterator<Item = (SceneRect, SceneColor)>,
        rectangles: &mut RectangleBatch,
    ) {
        let fill = pills
            .into_iter()
            .filter(|(pill, _)| {
                super::CardFill {
                    bounds: *pill,
                    radius: tokens::capsule_radius(pill.height),
                }
                .bands()
                .any(|band| band.intersection(badge.rect).is_some())
            })
            .map(|(_, fill)| fill)
            .max_by(|left, right| {
                contrast_ratio(*left, palette::BAR_BACKGROUND)
                    .total_cmp(&contrast_ratio(*right, palette::BAR_BACKGROUND))
            })
            .unwrap_or(palette::BAR_BACKGROUND);
        self.push_badge(badge, Backing::BarPill(fill), rectangles);
    }
}

/// Whether a badge lies wholly inside `clip`. One that crosses an edge of the tab viewport is
/// hidden past it, and that edge's badge stands for it; it is not drawn, so no clipped spinner
/// keeps ticking.
fn badge_inside(badge: TabBadge, clip: Option<SceneRect>) -> bool {
    clip.is_none_or(|clip| {
        badge.rect.left >= clip.left
            && badge.rect.right() <= clip.right()
            && badge.rect.top >= clip.top
            && badge.rect.bottom() <= clip.bottom()
    })
}

/// Stacked hollow windows distinguish popup agents from checkboxes and round attention glyphs.
fn push_popup_marker(
    rectangles: &mut RectangleBatch,
    rect: SceneRect,
    stroke: f32,
    color: SceneColor,
) {
    let size = rect.width.min(rect.height).floor();
    if size <= 0.0 {
        return;
    }
    let side = (size * 2.0 / 3.0).round();
    let offset = size - side;
    let front = SceneRect {
        left: rect.left.round() + offset,
        top: rect.top.round(),
        width: side,
        height: side,
    };
    let back = SceneRect {
        left: rect.left.round(),
        top: front.top + offset,
        ..front
    };
    let radius = tokens::popup_marker_radius(side);
    // Keep tiny squares hollow when the configured card stroke would fill them.
    let stroke = stroke.min(((side - 1.0) / 2.0).floor().max(1.0));
    let mask = super::CardFill {
        bounds: front,
        radius,
    }
    .bands()
    .collect::<Vec<_>>();
    for (rect, alpha) in super::rounded_outline_rects(back, radius, stroke) {
        for rect in super::uncovered(rect, &mask) {
            rectangles.push(rect.left, rect.top, rect.width, rect.height, color, alpha);
        }
    }
    rectangles.push_rounded_outline_width(front, radius, stroke, color);
}

/// Paint the row and badge together, returning text colours readable on that surface.
fn push_attention_row_surface(
    row: &crate::SidebarRow,
    hovered: bool,
    radius: f32,
    surface: Backing,
    background: SceneColor,
    spinners: &mut Spinners,
    rectangles: &mut RectangleBatch,
) -> (SceneColor, SceneColor) {
    let fill = if row.focused || hovered {
        let hover = palette::CHROME_HIGHLIGHT;
        Some(if contrast_ratio(hover, background) >= 1.5 {
            hover
        } else {
            blend_to_contrast(background, background, 1.5)
        })
    } else {
        None
    };
    if let Some(fill) = fill {
        rectangles.push_rounded(row.rect, radius, fill);
    }
    // Snap inward so fractional list clipping cannot soften the one-pixel strokes.
    let mut outline = SceneRect {
        left: row.rect.left.ceil(),
        top: row.rect.top.ceil(),
        width: row.rect.right().floor() - row.rect.left.ceil(),
        height: row.rect.bottom().floor() - row.rect.top.ceil(),
    };
    let mut outline_radius = radius.round();
    if row.current {
        let current = blend_to_contrast(spinners.focus, background, 3.0);
        rectangles.push_rounded_outline(outline, outline_radius, current);
        outline.left += 1.0;
        outline.top += 1.0;
        outline.width -= 2.0;
        outline.height -= 2.0;
        outline_radius = (outline_radius - 1.0).max(0.0);
    }
    if row.focused {
        let focus = blend_to_contrast(palette::ACCENT, fill.unwrap_or(background), 3.0);
        rectangles.push_rounded_outline(outline, outline_radius, focus);
    }
    let backing = fill.map_or(surface, Backing::Surface);
    spinners.push_badge(row.glyph, backing, rectangles);
    let (title, detail) = if row.current {
        (palette::PRIMARY_TEXT, palette::PRIMARY_TEXT)
    } else if row.glyph.dimmed {
        (palette::SECONDARY_TEXT, palette::TERTIARY_TEXT)
    } else {
        (palette::PRIMARY_TEXT, palette::SECONDARY_TEXT)
    };
    // Like Stack labels, resting rows follow the terminal color rather than guessing wallpaper.
    let background = fill.unwrap_or(background);
    (
        blend_to_contrast(title, background, 4.5),
        blend_to_contrast(detail, background, 4.5),
    )
}

/// Draws one attention badge's glyph in its resolved `color`: ◆ needs input, ✕ error, ✓ finished, the
/// braille frame at `phase` for working, or the first frame without one, three still dots for
/// background, and, in sidebar rows, ○ idle and · unavailable. Quads keep the
/// glyphs independent of fonts.
fn push_badge_glyph(
    rectangles: &mut RectangleBatch,
    badge: TabBadge,
    phase: Option<usize>,
    color: SceneColor,
) {
    // The ring yields entirely on the smallest badges, so the glyph never leaves the badge.
    let ring = (badge.rect.width / 12.0).floor();
    let glyph = SceneRect {
        left: badge.rect.left + ring,
        top: badge.rect.top + ring,
        width: badge.rect.width - ring * 2.0,
        height: badge.rect.height - ring * 2.0,
    };
    let alpha = if badge.dimmed { 0.45 } else { 1.0 };
    let at = |x: f32, y: f32| CursorPoint {
        x: glyph.left + glyph.width * x,
        y: glyph.top + glyph.height * y,
    };
    let stroke = (glyph.width / 4.0).max(1.0);
    match badge.state {
        AttentionState::NeedsInput => rectangles.push_quad(
            [at(0.5, 0.0), at(1.0, 0.5), at(0.5, 1.0), at(0.0, 0.5)],
            color,
            alpha,
        ),
        AttentionState::Error => {
            rectangles.push_stroke(at(0.1, 0.1), at(0.9, 0.9), stroke, color, alpha);
            rectangles.push_stroke(at(0.9, 0.1), at(0.1, 0.9), stroke, color, alpha);
        }
        AttentionState::Finished => {
            rectangles.push_stroke(at(0.05, 0.55), at(0.4, 0.88), stroke, color, alpha);
            rectangles.push_stroke(at(0.4, 0.88), at(0.95, 0.15), stroke, color, alpha);
        }
        AttentionState::Working => push_dots(
            rectangles,
            glyph,
            (2, 3),
            SPINNER_FRAMES[phase.unwrap_or(0)],
            color,
            alpha,
        ),
        // Background never animates (VEN-C21), so it takes no phase.
        AttentionState::Background => push_dots(rectangles, glyph, (3, 1), 0b111, color, alpha),
        AttentionState::Idle | AttentionState::Unavailable => {
            // A thin ring for idle and a small round dot for unavailable, each drawn as 16 joined
            // segments.
            let center = at(0.5, 0.5);
            let (outer, inner) = if badge.state == AttentionState::Idle {
                let outer = glyph.width / 2.0;
                (outer, (outer - stroke / 2.0).max(0.0))
            } else {
                (glyph.width * 0.15, 0.0)
            };
            let step = std::f32::consts::TAU / 16.0;
            let point = |radius: f32, angle: f32| CursorPoint {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
            };
            for segment in 0..16 {
                let start = segment as f32 * step;
                let end = start + step;
                rectangles.push_quad(
                    [
                        point(outer, start),
                        point(outer, end),
                        point(inner, end),
                        point(inner, start),
                    ],
                    color,
                    alpha,
                );
            }
        }
    }
}

/// Draws a `columns` by `rows` grid of square dots centred on the whole pixels inside `glyph`,
/// lighting the dots whose bit is set in `lit`, numbered down each column and then across, as a
/// braille cell numbers them: a working badge's 2x3 braille frame, or the background badge's
/// three dots in a row, in `color`. Dots are twice as wide as the gaps between them, two pixels
/// and one at the default badge, scaled by whole pixels with the glyph. Every dot is pushed, unlit
/// ones at zero alpha, so each spinner phase writes the same vertices and only alpha changes. A
/// glyph too small for the grid keeps the dots that fit; none leave `glyph`, so the badge's clip
/// check covers them.
fn push_dots(
    rectangles: &mut RectangleBatch,
    glyph: SceneRect,
    (columns, rows): (u8, u8),
    lit: u8,
    color: SceneColor,
    alpha: f32,
) {
    let left = glyph.left.ceil();
    let top = glyph.top.ceil();
    let area = SceneRect {
        left,
        top,
        width: (glyph.right().floor() - left).max(0.0),
        height: (glyph.bottom().floor() - top).max(0.0),
    };
    // A grid of n dots spans 3n - 1 units: n dots of two and n - 1 gaps of one.
    let (width, height) = (f32::from(columns), f32::from(rows));
    let unit = (area.height / (height * 3.0 - 1.0))
        .min(area.width / (width * 3.0 - 1.0))
        .floor();
    let (dot, gap) = if unit >= 1.0 {
        (unit * 2.0, unit)
    } else if area.height >= height * 2.0 - 1.0 && area.width >= width * 2.0 - 1.0 {
        (1.0, 1.0)
    } else {
        (1.0, 0.0)
    };
    let span = |count: f32| dot * count + gap * (count - 1.0);
    let x = left + ((area.width - span(width)) / 2.0).floor().max(0.0);
    let y = top + ((area.height - span(height)) / 2.0).floor().max(0.0);
    for bit in 0..columns * rows {
        let rect = SceneRect {
            left: x + f32::from(bit / rows) * (dot + gap),
            top: y + f32::from(bit % rows) * (dot + gap),
            width: dot,
            height: dot,
        };
        if let Some(rect) = rect.intersection(area) {
            rectangles.push(
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                color,
                if (lit >> bit) & 1 == 1 { alpha } else { 0.0 },
            );
        }
    }
}

/// The gap a heading's button leaves in its card's top edge where no notch holds it: its
/// target and half a padding each side, the margin a compact title's gap keeps around its text.
pub(super) fn heading_button_gap(target: SceneRect, metrics: CellMetrics) -> SceneRect {
    SceneRect {
        left: target.left - tokens::title_margin(metrics),
        width: target.width + metrics.padding,
        ..target
    }
}

impl FrameTitle {
    pub(super) fn fit(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        label: &str,
        available: f32,
    ) -> Self {
        let (label, width) = fit_header_text(fonts, metrics, label, available);
        Self { label, width }
    }

    pub(super) fn ink_width(&self, metrics: CellMetrics, stroke: f32) -> f32 {
        self.width + (tokens::title_margin(metrics) - stroke).max(0.0)
    }

    pub(super) fn border_gap(&self, row: SceneRect, metrics: CellMetrics) -> SceneRect {
        SceneRect {
            left: row.left + tokens::title_margin(metrics),
            top: row.top + 1.0,
            width: self.width + metrics.padding,
            height: (row.height - 2.0).max(0.0),
        }
    }
}

impl FrameTitleTab {
    fn with_heading(mut self, heading: super::PanelHeading) -> Self {
        self.heading = heading;
        self
    }

    pub(super) fn popup(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        workspace: &WorkspaceScene,
        stroke: f32,
    ) -> Option<Self> {
        let label = workspace.popup_label()?;
        let row = workspace.popup_title_row(metrics)?;
        let card = workspace.card_fill(workspace.chrome, metrics, stroke);
        // A clear raster row separates the cap from both the bar and the title box.
        let frame_top = workspace
            .tab_viewport
            .bottom()
            .max(workspace.header.bottom());
        Self::new(
            fonts,
            metrics,
            label,
            row,
            card,
            frame_top,
            stroke,
            workspace.heading_action(crate::HeadingAction::HidePopup),
        )
    }

    /// The tab over `row`; with an `action` button it also carries the mirrored right notch
    /// around that button, and its label stops before that notch.
    #[allow(clippy::too_many_arguments)]
    fn new(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        label: &str,
        row: SceneRect,
        card: crate::scene::CardFill,
        frame_top: f32,
        stroke: f32,
        action: Option<SceneRect>,
    ) -> Option<Self> {
        let extra = tokens::title_tab_rise(metrics).max(stroke + 1.0);
        let top = (row.top - extra).floor().max(frame_top.ceil() + 1.0);
        if top + stroke + 1.0 > row.top {
            return None;
        }
        let Some(target) = action else {
            return Self::at(fonts, metrics, label, row, card, top, row.top, stroke);
        };
        // Both notches return to the top edge over the same run; flat edge stays between them.
        let return_run = tokens::title_tab_return(card.bounds.top - top);
        let label_row = SceneRect {
            width: (target.left - metrics.padding - return_run - row.left).max(0.0),
            ..row
        };
        let mut tab = Self::at(fonts, metrics, label, label_row, card, top, row.top, stroke)?;
        // The title's return starts this far before the padded right edge of its label; the
        // mirrored one starts as far before the padded left edge of the button.
        let label_edge = card.bounds.left
            + metrics.padding
            + tab.title.ink_width(metrics, stroke)
            + tokens::title_margin(metrics);
        tab.action = Some(
            card.bounds.left + card.bounds.right() - target.left + tokens::title_margin(metrics)
                - (label_edge - tab.start),
        );
        Some(tab)
    }

    #[allow(clippy::too_many_arguments)]
    fn at(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        label: &str,
        row: SceneRect,
        card: crate::scene::CardFill,
        top: f32,
        ink_top: f32,
        stroke: f32,
    ) -> Option<Self> {
        if top + stroke + 1.0 > ink_top || top >= card.bounds.top {
            return None;
        }
        let rise = card.bounds.top - top;
        let join_radius = (card.radius / 2.0).min(rise / 2.0);
        let return_run = tokens::title_tab_return(rise);
        let right_padding = tokens::title_margin(metrics);
        if right_padding < stroke + 1.0 {
            return None;
        }
        // Keep antialiased corner pixels outside the title box, including at large fonts.
        let corner_limit = |padding: f32| {
            let clearance = (padding - stroke - 1.0).max(0.0);
            let extra = (ink_top - top - stroke).max(0.0);
            stroke + clearance + extra + (2.0 * clearance * extra).sqrt()
        };
        let left_radius = rise.min(corner_limit(metrics.padding));
        let right_radius = left_radius
            .min(rise - join_radius)
            .min(corner_limit(right_padding));
        let available = row
            .right()
            .min(card.bounds.right() - super::rounded_radius(card.bounds, card.radius))
            - card.bounds.left
            - (return_run - right_radius)
            - stroke
            - metrics.padding
            - right_padding
            - (right_padding - stroke);
        let title = FrameTitle::fit(fonts, metrics, label, available.max(0.0));
        let right =
            card.bounds.left + metrics.padding + title.ink_width(metrics, stroke) + right_padding;
        if title.label.is_empty() {
            return None;
        }
        Some(Self {
            heading: super::PanelHeading::Notch,
            top,
            start: right - right_radius,
            left_radius,
            row,
            return_run,
            title,
            action: None,
        })
    }

    /// The tab on the top line of a card allocated `area`, labelled `label`, with the right
    /// notch of its `action` button when it has one.
    #[allow(clippy::too_many_arguments)]
    fn card(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        workspace: &WorkspaceScene,
        area: SceneRect,
        label: &str,
        stroke: f32,
        card_stroke: f32,
        action: Option<SceneRect>,
    ) -> Option<Self> {
        let row = SceneRect {
            height: chrome_title_metrics(metrics).text_height,
            ..area
        };
        let card = crate::scene::CardFill {
            bounds: workspace.card_rect(area, metrics, card_stroke),
            radius: workspace.card_radius(),
        };
        if card.bounds.top != crate::scene::title_rule_y(row.top, row.height) {
            return None;
        }
        Self::new(
            fonts,
            metrics,
            label,
            row,
            card,
            workspace
                .header
                .bottom()
                .max(workspace.tab_viewport.bottom()),
            stroke,
            action,
        )
    }

    pub(super) fn sidebar(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        workspace: &WorkspaceScene,
        stroke: f32,
        card_stroke: f32,
    ) -> Option<Self> {
        let area = workspace.sidebar_area()?;
        Self::card(
            fonts,
            metrics,
            workspace,
            area,
            SIDEBAR_TITLE,
            stroke,
            card_stroke,
            workspace.heading_action(crate::HeadingAction::HideSidebar),
        )
    }

    pub(super) fn dialog(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        viewer: &DialogScene,
        stroke: f32,
    ) -> Option<(crate::scene::CardFill, Self)> {
        let height = chrome_title_metrics(metrics).text_height;
        let row = SceneRect {
            // Keep the lower chrome title inside the existing heading band, clear of bar chips.
            top: (viewer.title.bottom() - height).ceil(),
            height,
            ..viewer.bounds
        };
        let top = crate::scene::title_rule_y(row.top, row.height);
        if top >= viewer.bounds.bottom() {
            return None;
        }
        let card = crate::scene::CardFill {
            bounds: SceneRect {
                top,
                height: (viewer.bounds.bottom() - top).max(0.0),
                ..viewer.bounds
            },
            radius: tokens::card_radius(metrics),
        };
        Self::new(
            fonts,
            metrics,
            &viewer.kind.title(),
            row,
            card,
            viewer.bounds.top,
            stroke,
            None,
        )
        .map(|tab| (card, tab))
    }
    /// Each column's Stack tab, the selected column's first, or the popup's; then the sidebar's.
    pub(super) fn workspace(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        workspace: &WorkspaceScene,
        pane_frames: bool,
        widths: (f32, f32),
    ) -> Vec<Option<Self>> {
        let (border, focus) = widths;
        let card_stroke = border.max(focus);
        let mut tabs = if workspace.popup_label().is_some() {
            vec![FrameTitleTab::popup(fonts, metrics, workspace, border)]
        } else {
            workspace
                .column_cards()
                .map(|(chrome, label, panes, index)| {
                    label
                        .filter(|_| pane_frames && !panes.is_empty())
                        .and_then(|label| {
                            Self::card(
                                fonts,
                                metrics,
                                workspace,
                                chrome,
                                label,
                                border,
                                card_stroke,
                                workspace.heading_action(crate::HeadingAction::CloseStack(index)),
                            )
                        })
                })
                .collect()
        };
        if workspace.sidebar().is_some() {
            tabs.push(
                pane_frames
                    .then(|| FrameTitleTab::sidebar(fonts, metrics, workspace, border, card_stroke))
                    .flatten(),
            );
        }
        tabs
    }
}

impl PaneTitleTab {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fit(
        fonts: &mut FontSystem,
        swash: &mut SwashCache,
        metrics: CellMetrics,
        pane: &crate::scene::WorkspacePane,
        above: Option<&crate::scene::WorkspacePane>,
        outline: &crate::scene::PaneOutline,
        viewport: SceneRect,
        right: f32,
        stroke: f32,
    ) -> Option<Self> {
        let row = pane.rect;
        let card = crate::scene::CardFill {
            bounds: outline.bounds,
            radius: outline.radius,
        };
        let mut ink = |pane: &crate::scene::WorkspacePane| {
            let (_, (_, top, _, bottom)) = status_glyph_ink(fonts, swash, metrics, pane.label());
            let row = pane.rect;
            SceneRect {
                top: (row.top + (row.height - chrome_title_metrics(metrics).text_height) / 2.0)
                    .trunc()
                    + top as f32,
                height: (bottom - top) as f32,
                ..row
            }
        };
        let own_ink = ink(pane);
        let rise = tokens::title_tab_rise(metrics);
        // With at least a tab rise of room, keep the cap out of the previous row too. A short
        // window can shrink the gap below that threshold, leaving the ink and viewport bounds.
        // Fractional row sums can round just below a whole rise.
        let limit = above
            .map(|above| {
                let edge = above.rect.bottom();
                let ink = ink(above).bottom();
                if row.top - edge >= rise - 0.01 {
                    ink.max(edge)
                } else {
                    ink
                }
            })
            .unwrap_or(f32::NEG_INFINITY)
            .max(viewport.top);
        let top = (row.top - rise).floor().max(limit.ceil());
        let tab = FrameTitleTab::at(
            fonts,
            metrics,
            pane.label(),
            SceneRect {
                width: (right - row.left).max(0.0),
                ..row
            },
            card,
            top,
            own_ink.top,
            stroke,
        )?;
        Some(Self { card, tab })
    }

    pub(super) fn workspace(
        fonts: &mut FontSystem,
        swash: &mut SwashCache,
        metrics: CellMetrics,
        workspace: &WorkspaceScene,
        widths: (f32, f32),
        right_limit: Option<f32>,
    ) -> Vec<Vec<Option<PaneTitleTab>>> {
        if workspace.popup_label().is_some() {
            return Vec::new();
        }
        let (border, focus) = widths;
        let terminals = std::iter::once(workspace.terminal)
            .chain(workspace.other_columns.iter().map(|column| column.terminal));
        workspace
            .column_cards()
            .zip(terminals)
            .enumerate()
            .map(|(column, ((area, _, panes, _), terminal))| {
                let fill = workspace.card_fill(area, metrics, border.max(focus));
                let outlines = workspace.pane_outlines(
                    panes,
                    terminal,
                    fill,
                    border,
                    metrics,
                    border.max(focus),
                );
                panes
                    .iter()
                    .zip(&outlines)
                    .enumerate()
                    .map(|(index, (pane, outline))| {
                        // Only the expanded pane has a band; a collapsed title sits in its
                        // row's line.
                        if !pane.selected {
                            return None;
                        }
                        let current = column == 0 && pane.selected;
                        let right = if current {
                            right_limit
                                .unwrap_or(pane.header_content(metrics).right())
                                .min(pane.header_content(metrics).right())
                        } else {
                            pane.header_content(metrics).right()
                        };
                        let above = index.checked_sub(1).and_then(|above| panes.get(above));
                        let top = (workspace.header_viewport.top - tokens::title_tab_rise(metrics))
                            .max(fill.bounds.top);
                        let viewport = SceneRect {
                            top,
                            height: (workspace.header_viewport.bottom() - top).max(0.0),
                            ..workspace.header_viewport
                        };
                        PaneTitleTab::fit(
                            fonts,
                            swash,
                            metrics,
                            pane,
                            above,
                            outline,
                            viewport,
                            right,
                            if current { focus } else { border },
                        )
                    })
                    .collect()
            })
            .collect()
    }
}

/// Raised notches replace the rule under their buttons; compact groups need a gap.
pub(super) fn pane_rule_gaps(
    title: SceneRect,
    mut scrollback: SceneRect,
    actions: Option<[SceneRect; 2]>,
    notched: bool,
) -> [SceneRect; 2] {
    if !notched && let Some(actions) = actions {
        let left = if scrollback.width > 0.0 {
            scrollback.left
        } else {
            actions[0].left
        };
        scrollback = SceneRect {
            left,
            width: actions[1].right() - left,
            ..actions[0]
        };
    }
    [title, scrollback]
}

impl Renderer {
    pub(super) fn workspace_pane_tabs(
        &mut self,
        workspace: &WorkspaceScene,
    ) -> Vec<Vec<Option<PaneTitleTab>>> {
        if !self.pane_frames {
            return Vec::new();
        }
        let widths = (
            physical_outline_width(self.border_width, self.metrics.scale),
            physical_outline_width(self.focus_border_width, self.metrics.scale),
        );
        if let Some(cache) = &self.pane_titles
            && cache.matches(
                workspace,
                self.metrics,
                widths,
                &self.terminal.scrollback_label,
            )
        {
            return cache.tabs.clone();
        }
        let right = workspace
            .panes
            .iter()
            .find(|pane| pane.selected)
            .and_then(|pane| {
                self.scrollback_geometry(
                    SceneRect {
                        left: pane.rect.left + self.metrics.width * 4.0,
                        width: (pane.header_content(self.metrics).width - self.metrics.width * 4.0)
                            .max(0.0),
                        ..pane.rect
                    },
                    workspace.header_viewport,
                    true,
                    None,
                )
                .map(|(rect, _)| rect.left)
            });
        let tabs = PaneTitleTab::workspace(
            &mut self.fonts.font_system,
            &mut self.swash_cache,
            self.metrics,
            workspace,
            widths,
            right,
        );
        self.pane_titles = Some(super::PaneTitleCache {
            workspace: workspace.clone(),
            metrics: self.metrics,
            widths,
            scrollback: self.terminal.scrollback_label.clone(),
            tabs: tabs.clone(),
        });
        tabs
    }

    pub(super) fn workspace_title_tabs(
        &mut self,
        workspace: &WorkspaceScene,
    ) -> Vec<Option<FrameTitleTab>> {
        FrameTitleTab::workspace(
            &mut self.fonts.font_system,
            self.metrics,
            workspace,
            self.pane_frames,
            (
                physical_outline_width(self.border_width, self.metrics.scale),
                physical_outline_width(self.focus_border_width, self.metrics.scale),
            ),
        )
        .into_iter()
        .map(|tab| tab.map(|tab| tab.with_heading(self.panel_heading)))
        .collect()
    }

    /// Steps every drawn working badge one frame by rewriting only its own uploaded vertices,
    /// so the scene, text and terminal layers stay cached.
    pub fn turn_spinners(&mut self) {
        let Some(phase) = &mut self.spinners.phase else {
            return;
        };
        *phase = (*phase + 1) % SPINNER_FRAMES.len();
        for slot in &self.spinners.slots {
            let mut batch = RectangleBatch::new(self.config.width, self.config.height);
            push_badge_glyph(&mut batch, slot.badge, self.spinners.phase, slot.color);
            self.queue
                .write_buffer(&self.vertices, slot.offset as u64, &batch.bytes);
        }
    }

    /// Whether the current frame draws a working badge's spinner.
    #[must_use]
    pub fn spinning(&self) -> bool {
        !self.spinners.slots.is_empty()
    }
}

fn header_item_visual(rect: SceneRect, metrics: CellMetrics) -> SceneRect {
    let inset = tokens::chip_inset(metrics)
        .min(rect.width / 4.0)
        .min(rect.height / 4.0);
    SceneRect {
        left: rect.left + inset,
        top: rect.top + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        height: (rect.height - inset * 2.0).max(0.0),
    }
}

fn push_bar_widget(rectangles: &mut RectangleBatch, visual: SceneRect) {
    rectangles.push_rounded(
        visual,
        tokens::capsule_radius(visual.height),
        palette::TAB_IDLE,
    );
}

fn push_session_label_surface(
    rectangles: &mut RectangleBatch,
    visual: SceneRect,
    state: HeaderButtonState,
) {
    let radius = tokens::capsule_radius(visual.height);
    rectangles.push_rounded(visual, radius, session_label_fill(state));
    if state.enabled && state.focused {
        rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
    }
}

fn session_label_fill(state: HeaderButtonState) -> SceneColor {
    if state.enabled && state.pressed {
        palette::SESSION_PRESSED
    } else if state.enabled && state.hovered {
        palette::TAB_HOVER
    } else {
        palette::TAB_IDLE
    }
}

fn tab_fill(selected: bool, hovered: bool) -> SceneColor {
    if selected {
        palette::TAB_SELECTED
    } else if hovered {
        palette::TAB_HOVER
    } else {
        palette::TAB_IDLE
    }
}

fn push_bar_control_surface(
    rectangles: &mut RectangleBatch,
    visual: SceneRect,
    state: HeaderButtonState,
) {
    let fill = if state.pressed {
        palette::CHROME_HIGHLIGHT
    } else if state.hovered {
        palette::CONTROL_HOVER
    } else {
        palette::BAR_BACKGROUND
    };
    let radius = tokens::capsule_radius(visual.height);
    rectangles.push_rounded(visual, radius, fill);
    if state.focused {
        rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
    }
}

pub(super) fn measure_reading_width(fonts: &mut FontSystem, metrics: CellMetrics) -> f32 {
    (0..=100)
        .map(|percent| fit_header_text(fonts, metrics, &format!("{percent}%"), f32::INFINITY).1)
        .fold(0.0, f32::max)
}

/// Lines `text` takes in dialog row typography wrapped to `width` pixels.
pub(super) fn dialog_text_lines(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    cell_font: super::cells::CellFont,
    text: &str,
    width: f32,
) -> usize {
    super::shape_styled_text(
        fonts,
        metrics,
        cell_font,
        text,
        width,
        f32::MAX,
        DrawStyleKind::Status(Wrap::WordOrGlyph),
    )
    .layout_runs()
    .count()
    .max(1)
}

pub(super) fn fit_header_text(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    text: &str,
    width: f32,
) -> (String, f32) {
    let title = chrome_title_metrics(metrics);
    let mut buffer = Buffer::new(fonts, Metrics::new(title.font_size, title.line_height));
    buffer.set_size(None, Some(title.text_height));
    buffer.set_wrap(Wrap::None);
    let measure = |buffer: &mut Buffer, fonts: &mut FontSystem, text: &str| {
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            shaping(text),
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
    };
    let measured = measure(&mut buffer, fonts, text);
    if measured <= width {
        return (text.to_owned(), measured);
    }

    // Keep the positional prefix; cuts use the shaper's original cluster boundaries.
    let prefix = text.find("  ").map_or(0, |index| index + 2);
    let mut boundaries: Vec<_> = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .flat_map(|glyph| [glyph.start, glyph.end])
        .filter(|index| *index >= prefix)
        .collect();
    boundaries.extend([prefix, text.len()]);
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut low = 0;
    let mut high = boundaries.len().saturating_sub(2);
    let identity = text[..prefix].trim_end();
    let identity_width = measure(&mut buffer, fonts, identity);
    let mut best = if identity_width <= width {
        (identity.to_owned(), identity_width)
    } else {
        (String::new(), 0.0)
    };
    while low <= high {
        let kept = low + (high - low) / 2;
        let candidate = format!(
            "{}…{}",
            &text[..boundaries[kept.div_ceil(2)]],
            &text[boundaries[boundaries.len() - 1 - kept / 2]..]
        );
        let measured = measure(&mut buffer, fonts, &candidate);
        if measured <= width {
            best = (candidate, measured);
            low = kept + 1;
        } else if kept == 0 {
            break;
        } else {
            high = kept - 1;
        }
    }
    best
}

pub(super) fn bottom_title_row(row: SceneRect, bottom: f32, text_height: f32) -> SceneRect {
    SceneRect {
        top: (bottom - row.height / 2.0)
            .min(row.top)
            .min(row.bottom() - (row.height + text_height) / 2.0),
        ..row
    }
}

impl Renderer {
    pub fn set_hovered_header(&mut self, header: Option<(WorkspaceFocus, String)>) {
        self.header.hovered_header = header;
    }

    pub fn set_version_hovered(&mut self, hovered: bool) {
        self.header.version_hovered = hovered;
    }

    /// The shortcut hints keycap under the pointer, or whose popup it is over; whether that
    /// popup listing the group's hints shows (Escape or a press hides it until the pointer leaves
    /// them); and the popup row under the pointer.
    pub fn set_hint_group_hovered(
        &mut self,
        group: Option<usize>,
        popup: bool,
        row: Option<usize>,
    ) {
        self.header.hint_group_hovered = group;
        self.header.hint_popup = popup;
        self.header.hint_row_hovered = row;
    }

    /// The keycap popup drawn under the pointer at `x`, `y`: its group, and the row there.
    #[must_use]
    pub fn hint_popup_at(&self, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        self.hint_popup_hit.as_ref()?.at(x, y)
    }

    pub fn set_cpu_hovered(&mut self, hovered: bool) {
        self.header.cpu_hovered = hovered;
    }

    pub fn set_ram_hovered(&mut self, hovered: bool) {
        self.header.ram_hovered = hovered;
    }

    pub fn set_disk_hovered(&mut self, hovered: bool) {
        self.header.disk_hovered = hovered;
    }

    pub fn set_clock_hovered(&mut self, hovered: bool) {
        self.header.clock_hovered = hovered;
    }

    pub fn set_quota_hovered(&mut self, hovered: Option<usize>) {
        self.header.quota_hovered = hovered;
    }

    pub fn set_pressed_pane_action(&mut self, action: Option<(String, crate::PaneHeaderAction)>) {
        self.header.pressed_pane = action;
    }

    pub fn set_pressed_heading(&mut self, action: Option<crate::HeadingAction>) {
        self.header.pressed_heading = action;
    }

    pub fn set_pressed_header(&mut self, control: Option<WorkspaceHeaderControl>) {
        self.header.pressed_header = control;
    }

    /// Fit the scene's canonical label using the same typography as its header.
    pub fn fit_tab_text(&mut self, label: &str) -> (String, f32) {
        let width = WorkspaceScene::tab_text_width(self.size(), self.metrics);
        fit_header_text(&mut self.fonts.font_system, self.metrics, label, width)
    }

    /// Draws one sidebar or inbox row over `surface`, what its badge sits on unless the row is
    /// highlighted.
    fn push_attention_row(
        &mut self,
        row: &crate::SidebarRow,
        list: SceneRect,
        hovered: bool,
        surface: Backing,
        background: SceneColor,
        rectangles: &mut RectangleBatch,
    ) {
        let line = self.metrics.height;
        if row.rect.intersection(list).is_none() {
            return;
        }
        let (title, detail) = push_attention_row_surface(
            row,
            hovered,
            tokens::inner_radius(self.metrics),
            surface,
            background,
            &mut self.spinners,
            rectangles,
        );
        if let Some((provider, rect)) = row.icon {
            self.push_quota_icon(provider, rect, Some(list));
        }
        if let Some(marker) = row.popup_marker {
            push_popup_marker(
                rectangles,
                marker,
                physical_outline_width(self.border_width, self.metrics.scale),
                title,
            );
        }
        let text_width = (row.text_right - row.text_left).max(0.0);
        let text_top = row.glyph.rect.top - (line - row.glyph.rect.height) / 2.0;
        for (index, (text, color)) in [
            (Some(row.title.as_str()), title),
            (Some(row.detail.as_str()), detail),
            (row.message.as_deref(), title),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(text) = text else { continue };
            self.push_sidebar_line(
                text,
                row.text_left,
                text_top + line * index as f32,
                text_width,
                color,
                list,
            );
        }
    }

    /// One header line in `label`, whose height is its chip's, as `chip_label_top` places it.
    /// It clips at the chip and `clip`, not at its terminal-height line, which can be shorter
    /// than the face's ascent and descent.
    fn push_chip_label(
        &mut self,
        text: &str,
        label: SceneRect,
        color: SceneColor,
        clip: SceneRect,
    ) {
        let Some(bounds) = chip_label_bounds(label, clip) else {
            return;
        };
        let width = label.width.max(1.0);
        let line_height = chrome_title_metrics(self.metrics).line_height;
        let pushed = self.text.len();
        self.push_text(
            text,
            label.left,
            label.top,
            width,
            line_height,
            color,
            DrawStyleKind::Status(Wrap::None),
        );
        for placed in &mut self.text[pushed..] {
            placed.top = chip_label_top(
                &mut self.fonts.font_system,
                &mut self.swash_cache,
                placed.buffer.mutable(),
                label,
                line_height,
            );
            placed.bound_left = bounds.left;
            placed.bound_top = bounds.top;
            placed.right = bounds.right;
            placed.bottom = bounds.bottom;
        }
    }

    /// One line of agent sidebar text within the scene's reserved text area.
    #[allow(clippy::too_many_arguments)]
    fn push_sidebar_line(
        &mut self,
        text: &str,
        left: f32,
        top: f32,
        width: f32,
        color: SceneColor,
        clip: SceneRect,
    ) {
        if width <= 0.0 {
            return;
        }
        let height = chrome_title_metrics(self.metrics).text_height;
        self.push_text_clipped(
            text,
            left,
            top,
            width,
            width,
            height,
            color,
            DrawStyleKind::Status(Wrap::None),
            clip,
        );
    }

    /// The widest bar percentage for this renderer's current fonts, metrics and scale.
    pub fn reading_width(&self) -> f32 {
        self.reading_width
    }

    /// Measure an exact, non-truncated Eon Bar fact label with header typography.
    pub fn measure_header_text(&mut self, label: &str) -> (String, f32) {
        self.fit_header_text(label, f32::INFINITY)
    }

    /// Lines `text` takes in dialog row typography wrapped to `width` pixels, as a dialog row
    /// draws it.
    pub fn dialog_text_lines(&mut self, text: &str, width: f32) -> usize {
        dialog_text_lines(
            &mut self.fonts.font_system,
            self.metrics,
            self.cell_font,
            text,
            width,
        )
    }

    /// Fit header text to `width` pixels with header typography, as tab labels are.
    pub fn fit_header_text(&mut self, label: &str, width: f32) -> (String, f32) {
        fit_header_text(&mut self.fonts.font_system, self.metrics, label, width)
    }

    /// The tooltip of the hovered bar item, or the popup of the hovered shortcut hints keycap.
    pub(super) fn build_header_tooltip(
        &mut self,
        workspace: &WorkspaceScene,
        rectangles: &mut RectangleBatch,
    ) {
        let padding = self.metrics.padding;
        if let Some(hints) = &workspace.hints
            && self.header.hint_popup
            && let Some((index, group)) = self
                .header
                .hint_group_hovered
                .and_then(|index| Some((index, hints.groups.get(index)?)))
        {
            self.build_hint_popup(
                workspace,
                hints.rect.top - tokens::tooltip_gap(self.metrics),
                index,
                group,
                rectangles,
            );
            return;
        }
        let hovered_tab = workspace
            .tabs
            .iter()
            .find(|tab| self.header.hovers(WorkspaceFocus::Tabs, &tab.id));
        let hovered_pane = self.header.hovered_header.as_ref().and_then(|(focus, id)| {
            let WorkspaceFocus::PaneAction(action) = focus else {
                return None;
            };
            let (rect, enabled) = workspace.pane_action(id, *action)?;
            let pane = workspace
                .column_panes()
                .flat_map(|(panes, _)| panes)
                .find(|pane| pane.id == *id)?;
            pane.visible_actions()?;
            let mut label = action.label(pane.label(), workspace.zoomed_pane().is_some());
            if !enabled {
                label.push_str(" — unavailable in a one-pane tab");
            }
            Some((label, rect))
        });
        let hovered_pane = hovered_pane.or_else(|| {
            let (WorkspaceFocus::HeadingAction(action), _) = self.header.hovered_header.as_ref()?
            else {
                return None;
            };
            Some((
                workspace.heading_action_label(*action),
                workspace.heading_action(*action)?,
            ))
        });
        let (label, anchor) = if let Some((label, rect)) = &hovered_pane {
            (label.as_str(), *rect)
        } else if let Some(quota) = self
            .header
            .quota_hovered
            .and_then(|index| workspace.quotas.get(index))
        {
            (quota.tooltip.as_str(), quota.rect)
        } else if self.header.version_hovered
            && let Some(version) = &workspace.version
        {
            (version.tooltip.as_str(), version.rect)
        } else if self.header.cpu_hovered
            && let Some(cpu) = &workspace.cpu
        {
            (cpu.tooltip.as_str(), cpu.rect)
        } else if self.header.ram_hovered
            && let Some(ram) = &workspace.ram
        {
            (ram.tooltip.as_str(), ram.rect)
        } else if self.header.disk_hovered
            && let Some(disk) = &workspace.disk
        {
            (disk.tooltip.as_str(), disk.rect)
        } else if self.header.clock_hovered
            && let Some(clock) = &workspace.clock
        {
            (clock.tooltip.as_str(), clock.rect)
        } else if let Some(tab) = hovered_tab {
            (tab.accessible_label(), tab.rect)
        } else if let Some((label, tooltip)) = workspace
            .workspace_label
            .as_ref()
            .and_then(|label| Some((label, label.tooltip.as_deref()?)))
            .filter(|_| {
                self.header.hovers(
                    WorkspaceFocus::Header(WorkspaceHeaderControl::Workspaces),
                    "",
                )
            })
        {
            (tooltip, label.rect)
        } else if let Some(control) = workspace
            .header_controls()
            .find(|control| self.header.hovers(WorkspaceFocus::Header(control.kind), ""))
        {
            (control.label.as_str(), control.rect)
        } else {
            return;
        };
        let edge = if hovered_pane.is_some() {
            if anchor.top > self.config.height as f32 / 2.0 {
                TooltipEdge::Above(anchor.top - tokens::tooltip_gap(self.metrics))
            } else {
                TooltipEdge::Below(anchor.bottom() + tokens::tooltip_gap(self.metrics))
            }
        } else {
            tooltip_edge(workspace, anchor, self.metrics)
        };
        let (width, height) = self.tooltip_room(workspace, edge);
        if width <= padding * 2.0 || height <= padding * 2.0 {
            return;
        }
        let first = self.text.len();
        self.push_text(
            label,
            padding,
            padding,
            width - padding * 2.0,
            height - padding * 2.0,
            palette::PRIMARY_TEXT,
            DrawStyleKind::Status(Wrap::WordOrGlyph),
        );
        let Some(text) = self.text.get(first) else {
            return;
        };
        let mut text_width = 0.0_f32;
        let mut text_height = 0.0_f32;
        for run in text.buffer.layout_runs() {
            text_width = text_width.max(run.line_w);
            text_height = text_height.max(run.line_top + run.line_height);
        }
        self.place_tooltip(
            first,
            (text_width.ceil() + padding * 2.0).min(width),
            (text_height.ceil() + padding * 2.0).min(height),
            anchor.left,
            edge,
            rectangles,
        );
    }

    fn tooltip_room(&self, workspace: &WorkspaceScene, edge: TooltipEdge) -> (f32, f32) {
        tooltip_room(
            PhysicalSize::new(self.config.width, self.config.height),
            self.metrics,
            workspace,
            edge,
        )
    }

    /// Lists a hovered keycap's hints above it in sections side by side, each key with its
    /// short label; columns that do not fit give way to a pointer at full help. The row under the
    /// pointer shows what it does as full help says it in a box just above, when that says more.
    fn build_hint_popup(
        &mut self,
        workspace: &WorkspaceScene,
        bottom: f32,
        index: usize,
        group: &ShortcutHintGroup,
        rectangles: &mut RectangleBatch,
    ) {
        let padding = self.metrics.padding;
        let line = chrome_title_metrics(self.metrics).line_height;
        let edge = TooltipEdge::Above(bottom);
        let (room, height) = self.tooltip_room(workspace, edge);
        if room <= padding * 2.0 {
            return;
        }
        let Some(columns) = hint_popup_layout(
            group,
            (height - padding * 2.0) / line,
            room,
            padding,
            |text| self.measure_header_text(text).1.ceil(),
        ) else {
            return;
        };
        let width = (columns
            .iter()
            .map(|column| column.width + padding)
            .sum::<f32>()
            + padding)
            .min(room);
        let lines = columns
            .iter()
            .map(|column| column.lines.len())
            .max()
            .unwrap_or_default();
        let first = self.text.len();
        let mut rows = Vec::new();
        let mut left = padding;
        for column in &columns {
            for (number, entry) in column.lines.iter().enumerate() {
                let top = padding + line * number as f32;
                match *entry {
                    HintPopupLine::Title(None) => {}
                    HintPopupLine::Title(Some(title)) => {
                        self.push_text(
                            title,
                            left,
                            top,
                            column.width.max(1.0),
                            line,
                            palette::HINT_POPUP_TEXT,
                            DrawStyleKind::PopupLabel,
                        );
                    }
                    HintPopupLine::Row(key, label, row) => {
                        self.push_text(
                            key,
                            left,
                            top,
                            column.keys.max(1.0),
                            line,
                            palette::HINT_POPUP_TEXT,
                            DrawStyleKind::Status(Wrap::None),
                        );
                        self.push_text(
                            label,
                            left + column.keys + padding,
                            top,
                            (column.width - column.keys - padding).max(1.0),
                            line,
                            palette::PRIMARY_TEXT,
                            DrawStyleKind::Status(Wrap::None),
                        );
                        if let Some(row) = row {
                            rows.push((
                                SceneRect {
                                    left,
                                    top,
                                    width: column.width,
                                    height: line,
                                },
                                row,
                            ));
                        }
                    }
                }
            }
            left += column.width + padding;
        }
        let popup = self.place_tooltip(
            first,
            width,
            line * lines as f32 + padding * 2.0,
            group.rect.left,
            edge,
            rectangles,
        );
        let bar = workspace
            .hints
            .as_ref()
            .map_or(popup.bottom(), |hints| hints.rect.top);
        self.hint_popup_hit = Some(HintPopupHit::new(index, popup, bar, rows));
        let Some(row) = self
            .header
            .hint_row_hovered
            .and_then(|row| group.rows.get(row))
            .filter(|row| row.description != row.short)
        else {
            return;
        };
        let top = workspace.header.bottom();
        let inner = (width - padding * 2.0).max(1.0);
        // The whole wrapped text decides whether the box fits; none of it is cut.
        let text_height = wrapped_text_height(
            &mut self.fonts.font_system,
            self.metrics,
            self.cell_font,
            &row.description,
            inner,
        );
        let Some(rect) = hint_detail_rect(popup, text_height, self.metrics, top) else {
            return;
        };
        let detail = self.text.len();
        self.push_text(
            &row.description,
            padding,
            padding,
            inner,
            text_height.ceil().max(1.0),
            palette::PRIMARY_TEXT,
            DrawStyleKind::Status(Wrap::Word),
        );
        self.place_box(detail, rect, rectangles);
        // One cover over both boxes, which share their left edge and width.
        self.text_overlay = Some((
            first,
            vec![text_bounds(SceneRect {
                height: popup.bottom() - rect.top,
                ..rect
            })],
        ));
    }

    /// Box the texts pushed since `first`, laid out from the origin, as a tooltip `width` by
    /// `height` at `anchor`'s left, kept inside the window, on the `edge` it names. It covers the
    /// text beneath it and the scrollback pill yields to it.
    fn place_tooltip(
        &mut self,
        first: usize,
        width: f32,
        height: f32,
        anchor: f32,
        edge: TooltipEdge,
        rectangles: &mut RectangleBatch,
    ) -> SceneRect {
        let tooltip = tooltip_rect(
            self.config.width as f32,
            self.metrics.padding,
            anchor,
            width,
            height,
            edge,
        );
        self.place_box(first, tooltip, rectangles);
        self.text_overlay = Some((first, vec![text_bounds(tooltip)]));
        tooltip
    }

    /// Box the texts pushed since `first`, laid out from the origin, at `rect`; the scrollback
    /// pill yields to it.
    fn place_box(&mut self, first: usize, rect: SceneRect, rectangles: &mut RectangleBatch) {
        let padding = self.metrics.padding;
        let SceneRect {
            left,
            top,
            width,
            height,
        } = rect;
        if self
            .terminal
            .scrollback_rect
            .is_some_and(|pill| pill.intersection(rect).is_some())
        {
            self.terminal.scrollback_rect = None;
        }
        for text in &mut self.text[first..] {
            text.left += left;
            text.top += top;
            text.bound_left = (left + padding).floor() as i32;
            text.right = (left + width - padding).ceil() as i32;
            text.bound_top = (top + padding).floor() as i32;
            text.bottom = (top + height - padding).ceil() as i32;
        }
        rectangles.push_rounded(
            rect,
            tokens::inner_radius(self.metrics),
            palette::TOOLTIP_BACKGROUND,
        );
    }

    pub(super) fn build_dialog(&mut self, viewer: &DialogScene, rectangles: &mut RectangleBatch) {
        let first = self.text.len();
        let padding = tokens::dialog_padding(self.metrics, viewer.bounds.width);
        rectangles.clip = None;
        let stroke = physical_outline_width(self.border_width, self.metrics.scale);
        let tab = FrameTitleTab::dialog(&mut self.fonts.font_system, self.metrics, viewer, stroke)
            .map(|(card, tab)| (card, tab.with_heading(self.panel_heading)));
        let occlusion = rectangles.push_dialog_frame(viewer.bounds, padding, tab.as_ref(), stroke);
        self.text_overlay = Some((first, occlusion));

        if let Some((_, tab)) = &tab {
            self.build_frame_title(
                &tab.title,
                tab.row,
                viewer.bounds,
                palette::PRIMARY_TEXT,
                stroke,
            );
        } else {
            self.build_border_title(
                &viewer.kind.title(),
                SceneRect {
                    left: viewer.bounds.left,
                    width: viewer.bounds.width,
                    ..viewer.title
                },
                viewer.bounds,
                (viewer.bounds.width - self.metrics.padding * 2.0).max(0.0),
                palette::PRIMARY_TEXT,
            );
        }
        self.push_text_clipped(
            &viewer.kind.subtitle(),
            viewer.subtitle.left,
            viewer.subtitle.top,
            viewer.subtitle.width,
            viewer.subtitle.width,
            viewer.subtitle.height,
            palette::SECONDARY_TEXT,
            DrawStyleKind::PopupLabel,
            viewer.bounds,
        );

        rectangles.clip = Some(viewer.bounds);
        for chip in &viewer.filters {
            if chip.selected {
                let radius = tokens::dialog_radius(padding);
                rectangles.push_rounded(chip.rect, radius, palette::CHROME_HIGHLIGHT);
                rectangles.push_rounded_outline(chip.rect, radius, palette::ACCENT);
            }
            self.push_text_clipped(
                &format!("{} {}", chip.filter.label(), chip.count),
                chip.rect.left + tokens::dialog_chip_label_inset(padding),
                chip.rect.top + (chip.rect.height - self.metrics.height) / 2.0,
                (chip.rect.width - tokens::dialog_chip_label_margins(padding)).max(0.0),
                chip.rect.width,
                chrome_title_metrics(self.metrics).text_height,
                if chip.selected {
                    palette::PRIMARY_TEXT
                } else {
                    palette::SECONDARY_TEXT
                },
                DrawStyleKind::Status(Wrap::None),
                chip.rect,
            );
        }
        rectangles.clip = Some(viewer.content);
        if let Some(empty) = viewer.empty {
            self.push_text_clipped(
                empty,
                viewer.content.left,
                viewer.content.top,
                viewer.content.width,
                viewer.content.width,
                viewer.content.height,
                palette::SECONDARY_TEXT,
                DrawStyleKind::Status(Wrap::WordOrGlyph),
                viewer.content,
            );
        }
        for row in &viewer.attention_rows {
            self.push_attention_row(
                row,
                viewer.content,
                false,
                Backing::Surface(palette::DIALOG_BACKGROUND),
                palette::DIALOG_BACKGROUND,
                rectangles,
            );
        }
        let headings = viewer
            .groups
            .iter()
            .map(|group| (group, group.heading))
            .chain(
                viewer
                    .continued
                    .iter()
                    .map(|(group, heading)| (&viewer.groups[*group], *heading)),
            );
        for (group, heading) in headings {
            if !group.title.is_empty() {
                self.push_text_clipped(
                    &group.title,
                    heading.left,
                    heading.top,
                    heading.width,
                    heading.width,
                    heading.height,
                    palette::SECONDARY_TEXT,
                    DrawStyleKind::PopupLabel,
                    viewer.content,
                );
            }
        }
        for group in &viewer.groups {
            for row in &group.rows {
                if row.focused {
                    rectangles.push_rounded(
                        row.rect,
                        tokens::dialog_row_radius(self.metrics),
                        palette::CONTROL_HOVER,
                    );
                }
                self.push_text_clipped(
                    &row.shortcut,
                    row.rect.left,
                    row.rect.top,
                    row.label_width,
                    row.label_width,
                    row.rect.height,
                    palette::SHORTCUT_KEY_TEXT,
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    viewer.content,
                );
                let action_left = row.rect.left + row.label_width + viewer.label_gap;
                self.push_text_clipped(
                    &row.action,
                    action_left,
                    row.rect.top,
                    (row.rect.right() - action_left).max(1.0),
                    (row.rect.right() - action_left).max(1.0),
                    row.rect.height,
                    palette::SHORTCUT_ACTION_TEXT,
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    viewer.content,
                );
            }
        }
        if viewer.max_scroll > 0.0 {
            let track = SceneRect {
                left: viewer.content.right() - self.metrics.scale * 3.0,
                top: viewer.content.top,
                width: self.metrics.scale * 2.0,
                height: viewer.content.height,
            };
            rectangles.push_rounded(
                track,
                tokens::capsule_radius(track.width),
                palette::SCROLLBAR_TRACK,
            );
            rectangles.push_rounded(
                scroll_thumb(
                    track,
                    viewer.content.height,
                    viewer.scroll,
                    viewer.max_scroll,
                ),
                tokens::capsule_radius(track.width),
                palette::DIALOG_SCROLL_THUMB,
            );
        }
        rectangles.clip = None;
        for (index, line) in viewer.footer().iter().enumerate() {
            self.push_text_clipped(
                line,
                viewer.content.left,
                viewer.content.bottom() + self.metrics.height * (0.35 + index as f32),
                viewer.content.width,
                viewer.content.width,
                self.metrics.height,
                palette::SECONDARY_TEXT,
                DrawStyleKind::PopupLabel,
                viewer.bounds,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_workspace(
        &mut self,
        workspace: &WorkspaceScene,
        focus: WorkspaceFocus,
        rectangles: &mut RectangleBatch,
        backgrounds: &[SceneColor],
        title_tabs: &[Option<FrameTitleTab>],
        pane_tabs: &[Vec<Option<PaneTitleTab>>],
    ) {
        rectangles.push(
            workspace.header.left,
            workspace.header.top,
            workspace.header.width,
            workspace.header.height,
            palette::BAR_BACKGROUND,
            1.0,
        );
        if let Some(sidebar) = workspace.sidebar() {
            // The card's translucent background and outline are drawn with the pane cards'.
            let padding = self.metrics.padding;
            let text_height = chrome_title_metrics(self.metrics).text_height;
            let width = (sidebar.width - padding * 2.0).max(0.0);
            let area = workspace.sidebar_area().unwrap();
            let row = SceneRect {
                height: text_height,
                ..area
            };
            let stroke = physical_outline_width(self.border_width, self.metrics.scale);
            let tab = title_tabs.last().and_then(Option::as_ref);
            let hide = workspace.heading_action(crate::HeadingAction::HideSidebar);
            let gap = if let Some(tab) = tab {
                self.build_frame_title(&tab.title, row, area, palette::PRIMARY_TEXT, stroke)
            } else {
                // A compact title stops before the button.
                let width = hide.map_or(width, |hide| {
                    (hide.left - row.left - padding * 2.0).max(0.0)
                });
                self.build_border_title(SIDEBAR_TITLE, row, area, width, palette::PRIMARY_TEXT)
            };
            // Without its notch the button sits in a gap in the top edge.
            let button_gap = hide
                .filter(|_| tab.is_none_or(|tab| tab.action.is_none()))
                .map(|hide| heading_button_gap(hide, self.metrics));
            if self.pane_frames {
                let card = crate::scene::CardFill {
                    bounds: workspace.card_rect(area, self.metrics, self.card_stroke_width()),
                    radius: workspace.card_radius(),
                };
                let mut clip = area;
                if let Some(tab) = tab {
                    rectangles.push_title_tab_outline(card, tab, stroke, palette::CARD_OUTLINE);
                    clip.top = card.bounds.top + super::rounded_radius(card.bounds, card.radius);
                    clip.height = (area.bottom() - clip.top).max(0.0);
                }
                rectangles.clip = Some(clip);
                rectangles.push_card_outline(
                    card.bounds,
                    card.radius,
                    &[Some(gap), button_gap]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>(),
                    stroke,
                    palette::CARD_OUTLINE,
                );
                rectangles.clip = None;
            }
            self.push_heading_button(
                workspace,
                focus,
                rectangles,
                crate::HeadingAction::HideSidebar,
                "–",
                palette::PRIMARY_TEXT,
                area,
                backgrounds
                    .last()
                    .copied()
                    .unwrap_or(palette::TERMINAL_FALLBACK),
            );
            let list = workspace.sidebar_list;
            if workspace.sidebar_empty() {
                self.push_text_clipped(
                    SIDEBAR_EMPTY,
                    sidebar.left + padding,
                    list.top,
                    width,
                    width,
                    list.height,
                    palette::SECONDARY_TEXT,
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    sidebar,
                );
            }
            rectangles.clip = Some(list);
            let background = backgrounds
                .last()
                .copied()
                .unwrap_or(palette::TERMINAL_FALLBACK);
            let heading_name = blend_to_contrast(palette::PRIMARY_TEXT, background, 4.5);
            let heading_number = blend_to_contrast(palette::PANE_ID, background, 4.5);
            for heading in &workspace.sidebar_headings {
                let first = self.text.len();
                self.push_sidebar_line(
                    &heading.label,
                    heading.rect.left,
                    heading.rect.top,
                    heading.rect.width,
                    heading_name,
                    list,
                );
                if let Some(index) = workspace.tabs.iter().position(|tab| tab.id == heading.tab)
                    && let Some(text) = self.text.get_mut(first)
                {
                    color_label_identity(
                        text.buffer.mutable(),
                        &mut self.fonts.font_system,
                        &(index + 1).to_string(),
                        heading_number,
                    );
                }
            }
            let card = Backing::card(background, self.background_opacity);
            for row in workspace.sidebar_rows() {
                let hovered = self.header.hovers(WorkspaceFocus::Sidebar, &row.target);
                self.push_attention_row(row, list, hovered, card, background, rectangles);
            }
            rectangles.clip = None;
            if let Some(track) = workspace.sidebar_scroll_track(self.metrics, stroke) {
                rectangles.push_rounded(
                    scroll_thumb(
                        track,
                        list.height,
                        workspace.sidebar_scroll(),
                        workspace.sidebar_max_scroll(),
                    ),
                    tokens::capsule_radius(track.width),
                    blend_to_contrast(palette::DIALOG_SCROLL_THUMB, background, 3.0),
                );
            }
        }
        if let Some(hints) = &workspace.hints {
            // The hints sit bare in the margin below the cards; only keycaps have a fill.
            let padding = self.metrics.padding;
            for (index, group) in hints.groups.iter().enumerate() {
                rectangles.push_rounded(
                    group.rect,
                    tokens::inner_radius(self.metrics),
                    if self.header.hint_group_hovered == Some(index) {
                        palette::CHROME_HIGHLIGHT
                    } else {
                        palette::HINT_KEYCAP
                    },
                );
                let width = (group.rect.width - padding).max(0.0);
                self.push_text_clipped(
                    &group.legend,
                    group.rect.left + tokens::inner_inset(self.metrics),
                    group.rect.top,
                    width,
                    width,
                    group.rect.height,
                    palette::HINT_MODIFIER_TEXT,
                    DrawStyleKind::Status(Wrap::None),
                    group.rect,
                );
            }
            for hint in &hints.pieces {
                for (text, left, width, color) in [
                    (
                        hint.key.as_str(),
                        hint.rect.left,
                        hint.label_offset,
                        palette::TAB_TEXT,
                    ),
                    (
                        hint.label.as_str(),
                        hint.rect.left + hint.label_offset,
                        hint.rect.width - hint.label_offset,
                        palette::SECONDARY_TEXT,
                    ),
                ] {
                    let label = hint_label_box(hints, hint.rect, left, width, self.metrics);
                    self.push_chip_label(
                        text,
                        label,
                        color,
                        SceneRect {
                            top: label.top,
                            height: label.height,
                            ..hint.rect
                        },
                    );
                }
            }
            // The clock and the version label are plain text in one colour at the hints' two
            // ends, each one padding from the window edge.
            for label in [&workspace.clock, &workspace.version].into_iter().flatten() {
                self.push_text_clipped(
                    &label.label,
                    label.rect.left,
                    label.rect.top,
                    label.rect.width,
                    label.rect.width,
                    label.rect.height,
                    palette::VERSION_TEXT,
                    DrawStyleKind::Monospace,
                    SceneRect {
                        left: label.rect.left,
                        width: label.rect.width,
                        ..hints.rect
                    },
                );
            }
        }
        rectangles.clip = Some(workspace.tab_viewport);
        for (index, tab) in workspace.tabs.iter().enumerate() {
            if tab.rect.intersection(workspace.tab_viewport).is_none() {
                continue;
            }
            let visual = workspace.bar_pill_rect(tab.rect, self.metrics);
            let radius = tokens::capsule_radius(visual.height);
            // Each tab state steps up from the last so that the pills stand out from the bar.
            let fill = tab_fill(
                tab.selected,
                self.header.hovers(WorkspaceFocus::Tabs, &tab.id),
            );
            rectangles.push_rounded(visual, radius, fill);
            if tab.selected && focus == WorkspaceFocus::Tabs {
                rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
            }
            let first = self.text.len();
            self.push_chip_label(
                tab.label(),
                chip_label_box(workspace, tab.rect, self.metrics),
                if tab.selected {
                    palette::PRIMARY_TEXT
                } else {
                    palette::TAB_TEXT
                },
                workspace.tab_label_clip,
            );
            if !tab.selected
                && let Some(text) = self.text.get_mut(first)
            {
                color_label_identity(
                    text.buffer.mutable(),
                    &mut self.fonts.font_system,
                    &(index + 1).to_string(),
                    blend_to_contrast(palette::PANE_ID, fill, 4.5),
                );
            }
            if let Some(badge) = tab.badge {
                self.spinners
                    .push_badge(badge, Backing::BarPill(fill), rectangles);
            }
        }
        for badge in &workspace.edge_badges {
            let pills = workspace.tabs.iter().map(|tab| {
                (
                    workspace.bar_pill_rect(tab.rect, self.metrics),
                    tab_fill(
                        tab.selected,
                        self.header.hovers(WorkspaceFocus::Tabs, &tab.id),
                    ),
                )
            });
            self.spinners.push_edge_badge(*badge, pills, rectangles);
        }
        rectangles.clip = None;
        if let Some(icon) = workspace.eon_icon {
            let control = WorkspaceHeaderControl::WindowMenu;
            if workspace.has_control(control) {
                let visual = workspace.bar_pill_rect(icon, self.metrics);
                let radius = tokens::capsule_radius(visual.height);
                if self.header.pressed_header == Some(control) {
                    rectangles.push_rounded(visual, radius, palette::CHROME_HIGHLIGHT);
                } else if self.header.hovers(WorkspaceFocus::Header(control), "") {
                    rectangles.push_rounded(visual, radius, palette::CONTROL_HOVER);
                }
                if focus == WorkspaceFocus::Header(control) {
                    rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
                }
            }
            self.push_bar_icon(EON_ICON_ID, palette::VERSION_TEXT, icon, None);
        }
        // The session label shares the tab fills for hover and press, with the header controls'
        // keyboard focus and Tab order while it opens the picker.
        if let Some(label) = &workspace.workspace_label {
            let control = WorkspaceHeaderControl::Workspaces;
            let visual = workspace.bar_pill_rect(label.rect, self.metrics);
            let state = HeaderButtonState {
                hovered: self.header.hovers(WorkspaceFocus::Header(control), ""),
                pressed: self.header.pressed_header == Some(control),
                focused: focus == WorkspaceFocus::Header(control),
                enabled: workspace.has_control(control),
            };
            push_session_label_surface(rectangles, visual, state);
            self.push_chip_label(
                &label.label,
                chip_label_box(workspace, label.rect, self.metrics),
                palette::PRIMARY_TEXT,
                SceneRect {
                    top: workspace.header.top,
                    height: workspace.header.height,
                    ..label.rect
                },
            );
            if let Some(badge) = label.badge {
                self.spinners.push_badge(
                    badge,
                    Backing::BarPill(session_label_fill(state)),
                    rectangles,
                );
            }
        }
        // Quota chips and readings share the tabs' band.
        for quota in &workspace.quotas {
            let visual =
                workspace.bar_pill_rect(header_item_visual(quota.rect, self.metrics), self.metrics);
            push_bar_widget(rectangles, visual);
            self.push_quota_icon(quota.provider, quota.logo, None);
            let label_left = quota.logo.right() + tokens::icon_gap(self.metrics);
            let label_width = (quota.rect.right() - self.metrics.padding - label_left).max(0.0);
            let first_text = self.text.len();
            self.push_chip_label(
                &quota.label,
                SceneRect {
                    left: label_left,
                    width: label_width,
                    ..visual
                },
                palette::SECONDARY_TEXT,
                quota.rect,
            );
            if let Some(shortfall) = quota.pace
                && let Some(text) = self.text.get_mut(first_text)
            {
                color_quota_pace(
                    text.buffer.mutable(),
                    &mut self.fonts.font_system,
                    &quota.label,
                    shortfall,
                );
            }
        }
        for label in [&workspace.cpu, &workspace.ram, &workspace.disk] {
            let Some(label) = label else {
                continue;
            };
            let color = load_color(label.load);
            let content = workspace.bar_item_rect(label.rect, self.metrics);
            let visual =
                workspace.bar_pill_rect(header_item_visual(label.rect, self.metrics), self.metrics);
            push_bar_widget(rectangles, visual);
            let icon = label.label.chars().next().map_or(0, char::len_utf8);
            let (icon, value) = label.label.split_at(icon);
            let left = label.rect.left + self.metrics.padding;
            let width = (label.rect.width - self.metrics.padding * 2.0).max(0.0);
            self.push_text_clipped(
                icon,
                left,
                content.top + (content.height - self.metrics.height).max(0.0) / 2.0,
                width,
                width,
                self.metrics.height.min(label.rect.height),
                color,
                DrawStyleKind::Monospace,
                label.rect,
            );
            let value_left = left + self.metrics.font_size + tokens::icon_gap(self.metrics);
            let first_text = self.text.len();
            self.push_chip_label(
                value.trim_start(),
                SceneRect {
                    left: value_left,
                    width: (label.rect.right() - self.metrics.padding - value_left).max(0.0),
                    ..visual
                },
                color,
                label.rect,
            );
            if let Some(text) = self.text.get_mut(first_text)
                && let Some(line) = text.buffer.layout_runs().next()
            {
                text.left = text
                    .left
                    .max(label.rect.right() - self.metrics.padding - line.line_w);
            }
        }
        for control in &workspace.controls {
            if control.rect.width <= 0.0 {
                continue;
            }
            // The button centres on the glyph, which Close keeps one bar edge from the window's.
            let half =
                (control.centre - control.rect.left).min(control.rect.right() - control.centre);
            let button = SceneRect {
                left: control.centre - half,
                width: half * 2.0,
                ..control.rect
            };
            let content = workspace.bar_item_rect(button, self.metrics);
            let visual = header_item_visual(content, self.metrics);
            let hovered = self.header.hovers(WorkspaceFocus::Header(control.kind), "");
            let pressed = self.header.pressed_header == Some(control.kind);
            let focused = focus == WorkspaceFocus::Header(control.kind);
            push_bar_control_surface(
                rectangles,
                visual,
                HeaderButtonState {
                    hovered,
                    pressed,
                    focused,
                    enabled: true,
                },
            );
            // Shortcuts centres its glyph's ink; Close keeps equal corner insets.
            let placed = placed_control_glyph(
                &mut self.fonts.font_system,
                &mut self.swash_cache,
                self.metrics,
                workspace,
                control,
            );
            if let Some((buffer, left, top)) = placed {
                let bounds = super::text_clip_bounds(control.rect);
                let color = if pressed || focused {
                    palette::PRIMARY_TEXT
                } else {
                    palette::SECONDARY_TEXT
                };
                self.text.push(PlacedText {
                    buffer: buffer.into(),
                    custom_glyph: None,
                    left,
                    top,
                    right: bounds.right,
                    bottom: bounds.bottom,
                    bound_left: bounds.left,
                    bound_top: bounds.top,
                    color: Color::rgba(color.r, color.g, color.b, 255),
                });
                continue;
            }
            let glyph_width = self.metrics.width.min(button.width);
            self.push_text_clipped(
                control.kind.glyph(),
                control.centre - glyph_width / 2.0,
                content.top + (content.height - self.metrics.height).max(0.0) / 2.0,
                glyph_width,
                glyph_width,
                self.metrics.height.min(control.rect.height),
                if pressed || focused {
                    palette::PRIMARY_TEXT
                } else {
                    palette::SECONDARY_TEXT
                },
                DrawStyleKind::Status(Wrap::None),
                control.rect,
            );
        }
        if let Some(label) = workspace.popup_label()
            && workspace.visible_terminal().is_some()
        {
            let stroke = physical_outline_width(self.border_width, self.metrics.scale);
            let card =
                workspace.card_fill(workspace.chrome, self.metrics, self.card_stroke_width());
            let mut clip = workspace.chrome;
            let mut gaps = Vec::new();
            if let Some(row) = workspace.popup_title_row(self.metrics) {
                let hide = workspace.heading_action(crate::HeadingAction::HidePopup);
                let title = if let Some(tab) = FrameTitleTab::popup(
                    &mut self.fonts.font_system,
                    self.metrics,
                    workspace,
                    stroke,
                )
                .map(|tab| tab.with_heading(self.panel_heading))
                {
                    rectangles.push_title_tab_outline(card, &tab, stroke, palette::CARD_OUTLINE);
                    let cut = card.bounds.top + super::rounded_radius(card.bounds, card.radius);
                    clip.top = cut;
                    clip.height = (workspace.chrome.bottom() - cut).max(0.0);
                    tab.title
                } else {
                    // A compact title stops before the button, which sits in its own gap.
                    let title = FrameTitle::fit(
                        &mut self.fonts.font_system,
                        self.metrics,
                        label,
                        (hide.map_or(row.width, |hide| hide.left - row.left)
                            - self.metrics.padding * 2.0)
                            .max(0.0),
                    );
                    gaps.push(title.border_gap(row, self.metrics));
                    gaps.extend(hide.map(|hide| heading_button_gap(hide, self.metrics)));
                    title
                };
                self.build_frame_title(
                    &title,
                    row,
                    workspace.chrome,
                    palette::PRIMARY_TEXT,
                    stroke,
                );
                self.push_heading_button(
                    workspace,
                    focus,
                    rectangles,
                    crate::HeadingAction::HidePopup,
                    "–",
                    palette::PRIMARY_TEXT,
                    workspace.chrome,
                    backgrounds
                        .first()
                        .copied()
                        .unwrap_or(palette::TERMINAL_FALLBACK),
                );
            }
            rectangles.clip = Some(clip);
            rectangles.push_card_outline(
                card.bounds,
                card.radius,
                &gaps,
                stroke,
                palette::CARD_OUTLINE,
            );
            rectangles.clip = None;
        }
        let terminals = std::iter::once(workspace.terminal)
            .chain(workspace.other_columns.iter().map(|column| column.terminal));
        // `column_index` counts cards with the selected column's first, the order of
        // `backgrounds` and `title_tabs`; `stack_index` is the column's place from the left.
        for (column_index, ((card, stack_label, panes, stack_index), terminal)) in
            workspace.column_cards().zip(terminals).enumerate()
        {
            let selected_column = column_index == 0;
            if panes.is_empty() {
                continue;
            }
            let empty_gap = SceneRect {
                left: card.right(),
                ..SceneRect::default()
            };
            let fill = workspace.card_fill(card, self.metrics, self.card_stroke_width());
            let mut rules = workspace
                .pane_outlines(
                    panes,
                    terminal,
                    fill,
                    physical_outline_width(self.border_width, self.metrics.scale),
                    self.metrics,
                    self.card_stroke_width(),
                )
                .into_iter()
                .map(|outline| (outline, [empty_gap; 2]))
                .collect::<Vec<_>>();
            for (index, (pane, (outline, rule_gaps))) in panes.iter().zip(&mut rules).enumerate() {
                // Only the selected column's expanded header shows selection and scrollback.
                let current = pane.selected && selected_column;
                let Some(rect) = pane.rect.intersection(workspace.header_viewport) else {
                    continue;
                };
                let scrollback_width = if current {
                    self.build_scrollback(
                        SceneRect {
                            left: pane.rect.left + self.metrics.width * 4.0,
                            width: (pane.header_content(self.metrics).width
                                - self.metrics.width * 4.0)
                                .max(0.0),
                            ..pane.rect
                        },
                        workspace.header_viewport,
                        true,
                        None,
                        rectangles,
                    )
                } else {
                    0.0
                };
                let width = (pane.header_content(self.metrics).width
                    - self.metrics.padding * 2.0
                    - scrollback_width)
                    .max(0.0)
                    .floor();
                let color = if current {
                    palette::PRIMARY_TEXT
                } else if !pane.live {
                    palette::DEAD_PANE_TEXT
                } else {
                    palette::SECONDARY_TEXT
                };
                let row = if self.pane_frames && index + 1 == panes.len() && !pane.selected {
                    bottom_title_row(
                        pane.rect,
                        outline.bounds.bottom(),
                        chrome_title_metrics(self.metrics).text_height,
                    )
                } else {
                    pane.rect
                };
                let first_text = self.text.len();
                let notch = pane_tabs
                    .get(column_index)
                    .and_then(|tabs| tabs.get(index))
                    .and_then(Option::as_ref);
                let pill = if let Some(notch) = notch {
                    self.build_frame_title(
                        &notch.tab.title,
                        row,
                        rect,
                        color,
                        physical_outline_width(
                            if current {
                                self.focus_border_width
                            } else {
                                self.border_width
                            },
                            self.metrics.scale,
                        ),
                    )
                } else {
                    self.build_border_title(pane.label(), row, rect, width, color)
                };
                let id_color = pane_id_color(
                    color,
                    current,
                    pane.live,
                    backgrounds
                        .get(column_index)
                        .copied()
                        .unwrap_or(palette::TERMINAL_FALLBACK),
                    self.background_opacity,
                );
                if id_color != color
                    && let Some(text) = self.text.get_mut(first_text)
                {
                    color_label_identity(
                        text.buffer.mutable(),
                        &mut self.fonts.font_system,
                        &pane.id,
                        id_color,
                    );
                }
                *rule_gaps = pane_rule_gaps(
                    pill,
                    if scrollback_width > 0.0 {
                        self.terminal.scrollback_rect.unwrap()
                    } else {
                        empty_gap
                    },
                    pane.visible_actions(),
                    notch.is_some(),
                );
                rectangles.clip = Some(rect);
                if pill.width > self.metrics.padding {
                    if self.header.hovers(WorkspaceFocus::Panes, &pane.id) {
                        rectangles.push_rounded(
                            pill,
                            tokens::inner_radius(self.metrics),
                            palette::CHROME_HIGHLIGHT,
                        );
                    }
                    if notch.is_none() && current && focus == WorkspaceFocus::Panes {
                        rectangles.push_rounded_outline(
                            pill,
                            tokens::inner_radius(self.metrics),
                            palette::ACCENT,
                        );
                    }
                }
                rectangles.clip = Some(rect);
                if let Some(actions) = pane.visible_actions() {
                    for action in crate::PaneHeaderAction::ALL {
                        let target = actions[action.index()];
                        let enabled = action != crate::PaneHeaderAction::Zoom || pane.zoom_enabled;
                        let hovered = enabled
                            && self
                                .header
                                .hovers(WorkspaceFocus::PaneAction(action), &pane.id);
                        let pressed = self
                            .header
                            .pressed_pane
                            .as_ref()
                            .is_some_and(|(id, held)| id == &pane.id && *held == action);
                        let glyph = match action {
                            crate::PaneHeaderAction::Close => "×",
                            crate::PaneHeaderAction::Zoom if workspace.zoomed_pane().is_some() => {
                                tokens::RESTORE_GLYPH
                            }
                            crate::PaneHeaderAction::Zoom => tokens::ZOOM_GLYPH,
                        };
                        self.push_header_button(
                            rectangles,
                            target,
                            row,
                            rect,
                            glyph,
                            HeaderButtonState {
                                hovered,
                                pressed,
                                focused: current
                                    && focus == WorkspaceFocus::PaneAction(action)
                                    && enabled,
                                enabled,
                            },
                            color,
                            backgrounds
                                .get(column_index)
                                .copied()
                                .unwrap_or(palette::TERMINAL_FALLBACK),
                        );
                    }
                }
                rectangles.clip = None;
            }
            // The Stack tab, or a compact title on the card's top line, always neutral.
            let mut stack_close = None;
            if let Some(stack_label) = stack_label
                && let Some((first, _)) = rules.first()
            {
                let mut stack = workspace.stack_outline(first, fill);
                let stroke = physical_outline_width(self.border_width, self.metrics.scale);
                let color = stack_label_color(
                    backgrounds
                        .get(column_index)
                        .copied()
                        .unwrap_or(palette::TERMINAL_FALLBACK),
                );
                let row = SceneRect {
                    height: chrome_title_metrics(self.metrics).text_height,
                    ..card
                };
                let close = crate::HeadingAction::CloseStack(stack_index);
                let target = workspace.heading_action(close);
                let tab = title_tabs.get(column_index).and_then(Option::as_ref);
                let gap = if let Some(tab) = tab {
                    rectangles.push_title_tab_outline(fill, tab, stroke, palette::CARD_OUTLINE);
                    let cut = fill.bounds.top + super::rounded_radius(fill.bounds, fill.radius);
                    let bottom = stack.clip.bottom();
                    stack.clip.top = cut;
                    stack.clip.height = (bottom - cut).max(0.0);
                    self.build_frame_title(&tab.title, row, card, color, stroke)
                } else {
                    // A compact title stops before the button.
                    let width = target.map_or(card.width - self.metrics.padding * 2.0, |close| {
                        close.left - row.left - self.metrics.padding * 2.0
                    });
                    self.build_border_title(stack_label, row, card, width.max(0.0), color)
                };
                // Without its notch the button sits in a gap in the top edge.
                let button_gap = target
                    .filter(|_| tab.is_none_or(|tab| tab.action.is_none()))
                    .map_or(empty_gap, |close| heading_button_gap(close, self.metrics));
                rules.push((stack, [gap, button_gap]));
                stack_close = Some((close, color));
            }
            rectangles.push_pane_outlines(
                fill,
                workspace.pane_viewport,
                &rules,
                pane_tabs.get(column_index).map_or(&[], Vec::as_slice),
                (selected_column && focus == WorkspaceFocus::Panes)
                    .then(|| panes.iter().position(|pane| pane.selected))
                    .flatten(),
                self.pane_frames.then_some(if selected_column {
                    self.focus_color
                } else {
                    palette::CARD_OUTLINE
                }),
                physical_outline_width(self.border_width, self.metrics.scale),
                physical_outline_width(
                    if selected_column {
                        self.focus_border_width
                    } else {
                        self.border_width
                    },
                    self.metrics.scale,
                ),
            );
            // The Stack's Close, in the label's colour, over its card's outline.
            if let Some((close, color)) = stack_close {
                self.push_heading_button(
                    workspace,
                    focus,
                    rectangles,
                    close,
                    "×",
                    color,
                    card,
                    backgrounds
                        .get(column_index)
                        .copied()
                        .unwrap_or(palette::TERMINAL_FALLBACK),
                );
            }
        }
    }

    /// Draws one header button: its hover pill, accent while held, focus outline and
    /// ink-centred `glyph`, readable over `background`. `clip` bounds the glyph.
    #[allow(clippy::too_many_arguments)]
    fn push_header_button(
        &mut self,
        rectangles: &mut RectangleBatch,
        target: SceneRect,
        row: SceneRect,
        clip: SceneRect,
        glyph: &str,
        state: HeaderButtonState,
        color: SceneColor,
        background: SceneColor,
    ) {
        let pill = if state.pressed {
            palette::ACCENT
        } else {
            palette::CHROME_HIGHLIGHT
        };
        let radius = tokens::inner_radius(self.metrics);
        if state.hovered {
            rectangles.push_rounded(target, radius, pill);
        }
        if state.focused {
            rectangles.push_rounded_outline(target, radius, palette::ACCENT);
        }
        let (buffer, left, top) = ink_centred_glyph(
            &mut self.fonts.font_system,
            &mut self.swash_cache,
            self.metrics,
            glyph,
            (target.left + target.width / 2.0, row.top + row.height / 2.0),
        );
        let c = blend_to_contrast(
            if state.enabled {
                color
            } else {
                palette::DEAD_PANE_TEXT
            },
            if state.hovered { pill } else { background },
            if state.enabled { 4.5 } else { 3.0 },
        );
        self.text.push(PlacedText {
            buffer: buffer.into(),
            custom_glyph: None,
            left,
            top,
            right: target.right().floor() as i32,
            bottom: clip.bottom().floor() as i32,
            bound_left: target.left.ceil() as i32,
            bound_top: clip.top.ceil() as i32,
            color: Color::rgb(c.r, c.g, c.b),
        });
    }

    /// Draws a heading's button, which is as tall as its title row: a popup's or the Agents
    /// heading's Hide, or a Stack's Close, as its `glyph` in `color`.
    #[allow(clippy::too_many_arguments)]
    fn push_heading_button(
        &mut self,
        workspace: &WorkspaceScene,
        focus: WorkspaceFocus,
        rectangles: &mut RectangleBatch,
        action: crate::HeadingAction,
        glyph: &str,
        color: SceneColor,
        clip: SceneRect,
        background: SceneColor,
    ) {
        let Some(target) = workspace.heading_action(action) else {
            return;
        };
        rectangles.clip = Some(clip);
        self.push_header_button(
            rectangles,
            target,
            target,
            clip,
            glyph,
            HeaderButtonState {
                hovered: self
                    .header
                    .hovers(WorkspaceFocus::HeadingAction(action), ""),
                pressed: self.header.pressed_heading == Some(action),
                focused: focus == WorkspaceFocus::HeadingAction(action),
                enabled: true,
            },
            color,
            background,
        );
        rectangles.clip = None;
    }

    /// Pane and sidebar titles share shaping, placement and their measured border gap.
    pub(super) fn build_border_title(
        &mut self,
        label: &str,
        row: SceneRect,
        clip: SceneRect,
        width: f32,
        color: SceneColor,
    ) -> SceneRect {
        let title = FrameTitle::fit(&mut self.fonts.font_system, self.metrics, label, width);
        self.build_frame_title(&title, row, clip, color, 1.0)
    }

    fn build_frame_title(
        &mut self,
        title: &FrameTitle,
        row: SceneRect,
        clip: SceneRect,
        color: SceneColor,
        stroke: f32,
    ) -> SceneRect {
        let metrics = chrome_title_metrics(self.metrics);
        self.push_text_clipped(
            &title.label,
            row.left + self.metrics.padding,
            row.top + (row.height - metrics.text_height) / 2.0,
            title.width,
            title.ink_width(self.metrics, stroke),
            metrics.text_height,
            color,
            DrawStyleKind::Status(Wrap::None),
            clip,
        );
        title.border_gap(row, self.metrics)
    }

    /// Draws a provider icon in `rect`, cut to `clip` when one is given.
    fn push_quota_icon(
        &mut self,
        provider: QuotaProvider,
        rect: SceneRect,
        clip: Option<SceneRect>,
    ) {
        let (id, color) = match provider {
            QuotaProvider::Codex => (OPENAI_BLOSSOM_ID, palette::QUOTA_ICON_CODEX),
            QuotaProvider::Claude => (CLAUDE_SPLAT_ID, palette::QUOTA_ICON_CLAUDE),
        };
        self.push_bar_icon(id, color, rect, clip);
    }

    /// Draws the custom glyph `id` in `rect`, a mask tinted `color` or a colour image as it is,
    /// cut to `clip` when one is given.
    fn push_bar_icon(
        &mut self,
        id: u16,
        color: SceneColor,
        rect: SceneRect,
        clip: Option<SceneRect>,
    ) {
        let Some(bounds) = clip.map_or(Some(rect), |clip| rect.intersection(clip)) else {
            return;
        };
        let mut buffer = Buffer::new_empty(Metrics::new(rect.height, rect.height));
        buffer.set_size(Some(rect.width), Some(rect.height));
        self.text.push(PlacedText {
            buffer: buffer.into(),
            custom_glyph: Some(CustomGlyph {
                id,
                width: rect.width,
                height: rect.height,
                snap_to_physical_pixel: true,
                ..CustomGlyph::default()
            }),
            left: rect.left,
            top: rect.top,
            right: bounds.right().ceil() as i32,
            bottom: bounds.bottom().ceil() as i32,
            bound_left: bounds.left.floor() as i32,
            bound_top: bounds.top.floor() as i32,
            color: Color::rgb(color.r, color.g, color.b),
        });
    }
}

impl RectangleBatch {
    pub(super) fn push_dialog_frame(
        &mut self,
        bounds: SceneRect,
        padding: f32,
        tab: Option<&(crate::scene::CardFill, FrameTitleTab)>,
        stroke: f32,
    ) -> Vec<TextBounds> {
        if let Some((card, tab)) = tab {
            let fill = super::title_tab_bands(*card, tab, 0.0);
            for band in &fill {
                self.push(
                    band.left,
                    band.top,
                    band.width,
                    band.height,
                    palette::DIALOG_BACKGROUND,
                    1.0,
                );
            }
            self.push_title_tab_outline(*card, tab, stroke, palette::CARD_OUTLINE);
            let cut = card.bounds.top + super::rounded_radius(card.bounds, card.radius);
            self.clip = Some(SceneRect {
                top: cut,
                height: (bounds.bottom() - cut).max(0.0),
                ..bounds
            });
            self.push_card_outline(card.bounds, card.radius, &[], stroke, palette::CARD_OUTLINE);
            self.clip = None;
            fill.into_iter().map(super::text_clip_bounds).collect()
        } else {
            let radius = tokens::dialog_radius(padding);
            self.push_rounded(bounds, radius, palette::DIALOG_BACKGROUND);
            self.push_rounded_outline_width(bounds, radius, stroke, palette::CARD_OUTLINE);
            vec![text_bounds(bounds)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::cells::NERD_FONT_FAMILY;
    use super::super::cells::contrast_ratio;
    use super::super::*;
    use super::*;
    use glyphon::{cosmic_text::SubpixelBin, fontdb};
    #[cfg(target_os = "linux")]
    use std::sync::Arc;

    #[test]
    fn scroll_thumb_tracks_viewport_and_clamps_its_minimum_height() {
        let track = SceneRect {
            left: 30.0,
            top: 10.0,
            width: 2.0,
            height: 100.0,
        };
        let start = scroll_thumb(track, 100.0, 0.0, 300.0);
        assert_eq!(start.top, track.top);
        assert_eq!(start.height, 25.0);
        let end = scroll_thumb(track, 100.0, 300.0, 300.0);
        assert_eq!(end.bottom(), track.bottom());
        assert_eq!(
            scroll_thumb(track, 100.0, 150.0, 300.0).top,
            (start.top + end.top) / 2.0
        );
        assert_eq!(
            scroll_thumb(track, 100.0, 0.0, 1_000_000.0).height,
            track.width * 2.0
        );
        let tiny = SceneRect {
            height: 1.0,
            ..track
        };
        assert_eq!(scroll_thumb(tiny, 1.0, 0.0, 300.0).height, tiny.height);
    }

    /// Venus's focus color when Eon passes none (`--focus-color`), readable on the dark bar.
    const DEFAULT_FOCUS: SceneColor = SceneColor {
        r: 0xb2,
        g: 0x07,
        b: 0xfb,
    };

    const FOCUS_COLORS: [SceneColor; 7] = [
        DEFAULT_FOCUS,
        palette::ACCENT,
        SceneColor { r: 0, g: 0, b: 0 },
        SceneColor {
            r: 255,
            g: 255,
            b: 255,
        },
        SceneColor { r: 255, g: 0, b: 0 },
        SceneColor { r: 0, g: 255, b: 0 },
        SceneColor { r: 0, g: 0, b: 255 },
    ];

    fn vertex_color(vertex: &[u8]) -> SceneColor {
        let rgb = [2, 3, 4].map(|i| {
            (f32::from_ne_bytes(vertex[i * 4..i * 4 + 4].try_into().unwrap()) * 255.0).round() as u8
        });
        SceneColor {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
        }
    }

    /// The ink box of `buffer` as glyphon draws it from `left` and `top`: left, top, right and
    /// bottom in physical pixels.
    fn drawn_ink(
        font_system: &mut FontSystem,
        buffer: &Buffer,
        left: f32,
        top: f32,
    ) -> (i32, i32, i32, i32) {
        let mut swash_cache = SwashCache::new();
        let mut ink = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((left, top), 1.0);
                let image = swash_cache
                    .get_image_uncached(font_system, physical.cache_key)
                    .expect("the bar glyph should rasterize");
                let x = physical.x + image.placement.left;
                let y = run.line_y.round() as i32 + physical.y - image.placement.top;
                ink = (
                    ink.0.min(x),
                    ink.1.min(y),
                    ink.2.max(x + image.placement.width as i32),
                    ink.3.max(y + image.placement.height as i32),
                );
            }
        }
        ink
    }

    #[test]
    fn close_glyph_ink_keeps_equal_right_and_top_insets() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let glyph = WorkspaceHeaderControl::CloseTab.glyph();
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            // The bar's centre line on whole, half and quarter pixels; the window's right edge.
            for (centre_y, right) in [(25.0, 1920.0), (31.5, 1600.0), (49.25, 3840.0)] {
                // A corner target as wide as the bar is tall.
                let target = SceneRect {
                    left: right - centre_y * 2.0,
                    top: 0.0,
                    width: centre_y * 2.0,
                    height: centre_y * 2.0,
                };
                let (buffer, left, top) = ink_cornered_glyph(
                    &mut font_system,
                    &mut swash_cache,
                    metrics,
                    glyph,
                    (right - centre_y, centre_y),
                    target,
                );
                assert_eq!((left.fract(), top.fract()), (0.0, 0.0));
                let ink = drawn_ink(&mut font_system, &buffer, left, top);
                assert!(ink.2 > ink.0 && ink.3 > ink.1, "{scale}: the glyph has ink");
                assert_eq!(
                    right as i32 - ink.2,
                    ink.1,
                    "{scale}: right and top ink insets"
                );
                let ink_centre_y = (ink.1 + ink.3) as f32 / 2.0;
                assert!(
                    (ink_centre_y - centre_y).abs() <= 0.5,
                    "{scale}: ink centre {ink_centre_y}, bar centre {centre_y}"
                );
            }
        }
    }

    /// Alt+Shift's fourteen default hints, seven actions then seven popups, after one Alt hint.
    fn default_popup_hints() -> Vec<crate::ShortcutHint> {
        let hint = |modifiers: &str, names: &str, key: char, section| crate::ShortcutHint {
            modifiers: modifiers.into(),
            modifier_names: names.into(),
            key: key.into(),
            label: key.to_lowercase().collect(),
            short: format!("Do {key}"),
            description: format!("Run the {key} action in full"),
            section,
            accessible_label: format!("{names}+{key} — Run the {key} action in full"),
            priority: 0,
        };
        std::iter::once(hint("A", "Alt", 'M', HintSection::Actions))
            .chain(
                "TWZNDBH"
                    .chars()
                    .map(|key| hint("AS", "Alt+Shift", key, HintSection::Actions)),
            )
            .chain(
                "JLYAMSE"
                    .chars()
                    .map(|key| hint("AS", "Alt+Shift", key, HintSection::Popups)),
            )
            .collect()
    }

    #[test]
    fn hint_popup_sits_above_its_keycap_inside_the_window() {
        use crate::ShortcutHintsScene;
        let snapshot = crate::scene::tests::bar::one_pane_snapshot();
        let hints = default_popup_hints();
        let metrics = CellMetrics::for_scale(1.0);
        let measure = |text: &str| text.chars().count() as f32 * metrics.width;
        let line = chrome_title_metrics(metrics).line_height;
        let padding = metrics.padding;
        // Whether Alt+Shift's sections wrap into more columns, and whether they then point at
        // full help.
        for (width, height, wraps, cut) in [
            (1280, 800, false, false),
            (800, 600, false, false),
            (400, 300, false, false),
            (330, 300, false, false),
            (330, 200, true, true),
        ] {
            let size = PhysicalSize::new(width, height);
            let workspace = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                crate::EonBar {
                    version: Some("9.8.7"),
                    ..crate::EonBar::default()
                },
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.to_owned(), measure(text)),
                Some(ShortcutHintsScene::new(
                    hints.clone(),
                    size,
                    metrics,
                    Some(measure("EON 9.8.7")),
                    None,
                    measure,
                )),
            );
            let footer = workspace.hints.as_ref().unwrap();
            for group in &footer.groups {
                let edge = TooltipEdge::Above(footer.rect.top - padding / 3.0);
                let (room, tall) = tooltip_room(size, metrics, &workspace, edge);
                let lines = (tall - padding * 2.0) / line;
                let columns = hint_popup_layout(group, lines, room, padding, measure).unwrap();
                let popup_height = line
                    * columns
                        .iter()
                        .map(|column| column.lines.len())
                        .max()
                        .unwrap() as f32
                    + padding * 2.0;
                let popup_width = columns
                    .iter()
                    .map(|column| column.width + padding)
                    .sum::<f32>()
                    + padding;
                assert!(popup_width <= room, "{width}: {popup_width} > {room}");
                let popup = tooltip_rect(
                    width as f32,
                    padding,
                    group.rect.left,
                    popup_width,
                    popup_height,
                    edge,
                );
                assert!(
                    (popup.bottom() - (footer.rect.top - padding / 3.0)).abs() < 0.01,
                    "{width}"
                );
                assert!(popup.top >= workspace.header.bottom(), "{width}: {popup:?}");
                assert!(popup.left >= padding && popup.right() <= width as f32 - padding);
                // A popup that fits sits at its keycap's left.
                let narrow = tooltip_rect(width as f32, padding, group.rect.left, 1.0, 1.0, edge);
                assert_eq!(narrow.left, group.rect.left);
                let shown = columns
                    .iter()
                    .flat_map(|column| &column.lines)
                    .filter_map(|line| match line {
                        HintPopupLine::Row(_, _, row) => Some(*row),
                        HintPopupLine::Title(_) => None,
                    })
                    .collect::<Vec<_>>();
                let cut_here = shown.last() == Some(&None);
                let alt_shift = group.name == "Alt+Shift";
                assert_eq!(cut_here, cut && alt_shift, "{width}x{height}");
                assert_eq!(
                    hint_popup_columns(group, lines).unwrap().len() > 2,
                    wraps && alt_shift,
                    "{width}x{height}"
                );
                // The rows show in bar order, all of them unless the last points at full help.
                let rows = shown.iter().flatten().copied().collect::<Vec<_>>();
                assert_eq!(rows, (0..rows.len()).collect::<Vec<_>>());
                assert_eq!(rows.len() == group.rows.len(), !cut_here, "{width}");
                if alt_shift && !wraps {
                    // Actions beside Popups, eight lines high.
                    assert_eq!(columns.len(), 2, "{width}");
                    assert_eq!(columns[0].lines[0], HintPopupLine::Title(Some("Actions")));
                    assert_eq!(columns[1].lines[0], HintPopupLine::Title(Some("Popups")));
                    assert_eq!(popup_height, line * 8.0 + padding * 2.0);
                }
            }
            // The version's tooltip opens above it; a header item's below the header.
            let version = workspace.version.as_ref().unwrap();
            assert_eq!(
                tooltip_edge(&workspace, version.rect, metrics),
                TooltipEdge::Above(footer.rect.top - padding / 3.0)
            );
            assert_eq!(
                tooltip_edge(&workspace, workspace.tabs[0].rect, metrics),
                TooltipEdge::Below(workspace.header.bottom() + padding / 3.0)
            );
        }
    }

    #[test]
    fn hint_popup_keeps_the_pointer_and_shows_a_row_in_full_above() {
        let padding = 9.0;
        let metrics = CellMetrics {
            padding,
            ..CellMetrics::for_scale(1.0)
        };
        let popup = SceneRect {
            left: 100.0,
            top: 400.0,
            width: 160.0,
            height: 80.0,
        };
        let bar = popup.bottom() + padding / 3.0;
        let line = |top| SceneRect {
            left: padding,
            top,
            width: 70.0,
            height: 17.0,
        };
        let hit = HintPopupHit::new(
            2,
            popup,
            bar,
            vec![(line(padding), 0), (line(padding + 17.0), 5)],
        );
        // Over a row, over the popup's margin, and across the gap down to the bar.
        assert_eq!(
            hit.at(100.0 + padding + 1.0, 400.0 + padding + 18.0),
            Some((2, Some(5)))
        );
        assert_eq!(
            hit.at(100.0 + padding + 1.0, 400.0 + padding + 1.0),
            Some((2, Some(0)))
        );
        assert_eq!(hit.at(101.0, 401.0), Some((2, None)));
        assert_eq!(hit.at(150.0, bar - 0.5), Some((2, None)));
        // Not at the bar, beside the popup or above it.
        assert_eq!(hit.at(150.0, bar), None);
        assert_eq!(hit.at(99.0, 450.0), None);
        assert_eq!(hit.at(150.0, 399.0), None);
        // The full text sits just above, the popup's width; never into the header.
        let detail = hint_detail_rect(popup, 33.2, metrics, 40.0).unwrap();
        assert_eq!(detail.left, popup.left);
        assert_eq!(detail.width, popup.width);
        assert_eq!(detail.height, 34.0 + padding * 2.0);
        assert!((popup.top - detail.bottom() - padding / 3.0).abs() < 0.01);
        let header = detail.top - padding / 3.0;
        assert_eq!(hint_detail_rect(popup, 33.2, metrics, header), Some(detail));
        assert_eq!(hint_detail_rect(popup, 33.2, metrics, header + 1.0), None);
        assert_eq!(hint_detail_rect(popup, 0.0, metrics, 40.0), None);
    }

    #[test]
    fn hint_popup_full_text_counts_every_wrapped_line() {
        let mut fonts = FontSystem::new();
        let metrics = CellMetrics::for_scale(1.0);
        let cell_font = fitted_cell_font(&mut fonts, metrics, None, false);
        let text = "Detach: hide the window, keep terminals running";
        let one_line = wrapped_text_height(&mut fonts, metrics, cell_font, text, 10_000.0);
        assert!(one_line > 0.0);
        // Half its one-line width wraps it onto more lines, and every one counts, where a
        // buffer cut to the room for one line lays out only that line.
        let width = shape_styled_text(
            &mut fonts,
            metrics,
            cell_font,
            text,
            10_000.0,
            f32::MAX,
            DrawStyleKind::Status(Wrap::Word),
        )
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
            / 2.0;
        let wrapped = wrapped_text_height(&mut fonts, metrics, cell_font, text, width);
        assert!(
            wrapped >= one_line * 2.0,
            "{wrapped} for one line {one_line}"
        );
        let cut = shape_styled_text(
            &mut fonts,
            metrics,
            cell_font,
            text,
            width,
            one_line,
            DrawStyleKind::Status(Wrap::Word),
        )
        .layout_runs()
        .map(|run| run.line_top + run.line_height)
        .fold(0.0, f32::max);
        assert!(cut < wrapped, "{cut} against {wrapped}");
        // With room above for the cut text but not the whole of it, no box shows.
        let padding = metrics.padding;
        let popup = SceneRect {
            left: 0.0,
            top: 500.0,
            width,
            height: 100.0,
        };
        let room = hint_detail_rect(popup, cut, metrics, 0.0).unwrap();
        let header = room.top - padding / 3.0;
        assert!(hint_detail_rect(popup, cut, metrics, header).is_some());
        assert_eq!(hint_detail_rect(popup, wrapped, metrics, header), None);
    }

    #[test]
    fn hint_popup_sections_wrap_into_columns() {
        use crate::scene::HintRow;
        use HintPopupLine::{Row, Title};
        let row = |key: &str, section| HintRow {
            key: key.into(),
            short: format!("Do {key}"),
            description: format!("Run the {key} action in full"),
            section,
        };
        let group = ShortcutHintGroup {
            name: "Alt+Shift".into(),
            legend: "Alt+Shift".into(),
            rect: SceneRect::default(),
            hover: SceneRect::default(),
            rows: "TWZ"
                .chars()
                .map(|key| row(&key.to_string(), HintSection::Actions))
                .chain(
                    "JL".chars()
                        .map(|key| row(&key.to_string(), HintSection::Popups)),
                )
                .collect(),
            accessible_label: String::new(),
        };
        let key = |index: usize| group.rows[index].key.as_str();
        let short = |index: usize| group.rows[index].short.as_str();
        let at = |index: usize| Row(key(index), short(index), Some(index));
        // With room, one column per section.
        assert_eq!(
            hint_popup_columns(&group, 9.0),
            Some(vec![
                vec![Title(Some("Actions")), at(0), at(1), at(2)],
                vec![Title(Some("Popups")), at(3), at(4)],
            ])
        );
        // A section past the room goes on under an empty title, so rows line up.
        assert_eq!(
            hint_popup_columns(&group, 2.0),
            Some(vec![
                vec![Title(Some("Actions")), at(0)],
                vec![Title(None), at(1)],
                vec![Title(None), at(2)],
                vec![Title(Some("Popups")), at(3)],
                vec![Title(None), at(4)],
            ])
        );
        // A group with one section has one column.
        let actions = ShortcutHintGroup {
            rows: group.rows[..2].to_vec(),
            ..group.clone()
        };
        assert_eq!(
            hint_popup_columns(&actions, 3.5),
            Some(vec![vec![Title(Some("Actions")), at(0), at(1)]])
        );
        // No popup without room for a title and a row.
        assert_eq!(hint_popup_columns(&group, 1.9), None);
        assert_eq!(hint_popup_columns(&group, f32::NAN), None);
        // Columns past the room give way to a last row pointing at full help. One character is
        // one pixel and the padding one: Actions is its title's 7 wide, Popups its rows' 1 + 1 +
        // 4, so both take 1 + 7 + 1 + 6 + 1.
        let measure = |text: &str| text.chars().count() as f32;
        let more = Row(HINT_POPUP_MORE.0.as_str(), HINT_POPUP_MORE.1, None);
        fn lines(columns: Vec<HintPopupColumn<'_>>) -> Vec<Vec<HintPopupLine<'_>>> {
            columns.into_iter().map(|column| column.lines).collect()
        }
        let all = hint_popup_layout(&group, 9.0, 16.0, 1.0, measure).unwrap();
        assert_eq!(all[0].keys, 1.0);
        assert_eq!(all[0].width, "Actions".len() as f32);
        assert_eq!(lines(all), hint_popup_columns(&group, 9.0).unwrap());
        assert_eq!(
            lines(hint_popup_layout(&group, 9.0, 15.0, 1.0, measure).unwrap()),
            vec![vec![Title(Some("Actions")), at(0), at(1), at(2), more]]
        );
        // In a full column, the pointer takes the last row's place.
        assert_eq!(
            lines(hint_popup_layout(&group, 4.0, 15.0, 1.0, measure).unwrap()),
            vec![vec![Title(Some("Actions")), at(0), at(1), more]]
        );
        // With room for one row under a title, the pointer takes the title's place, not the
        // row's.
        assert_eq!(
            lines(hint_popup_layout(&group, 2.0, 7.0, 1.0, measure).unwrap()),
            vec![vec![at(0), more]]
        );
        // The first column stays, however narrow the room.
        assert_eq!(
            lines(hint_popup_layout(&actions, 9.0, 0.0, 1.0, measure).unwrap()),
            vec![vec![Title(Some("Actions")), at(0), at(1)]]
        );
    }

    #[test]
    fn placed_control_glyphs_stay_inside_their_targets() {
        use eon_workspace_protocol::v41::{Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab};
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: Vec::new(),
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: vec![Column {
                    panes: vec![Pane {
                        label: "p1".into(),
                        pinned_name: None,
                        id: "p1".into(),
                        session: "s1".into(),
                        endpoint: b"/tmp/p1.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                    expanded: "p1".into(),
                }],
                popups: Vec::new(),
                attention_rollup: None,
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
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for logical_width in [100, 120, 160, 1280] {
                let width = (f64::from(logical_width) * scale) as u32;
                let workspace = WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    crate::EonBar {
                        version: Some("9.8.7"),
                        icon: true,
                        ..crate::EonBar::default()
                    },
                    PhysicalSize::new(width, 600),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| {
                        (
                            text.to_owned(),
                            text.chars().count() as f32 * 10.0 * scale as f32,
                        )
                    },
                    None,
                );
                let mut placed = 0;
                for control in &workspace.controls {
                    if control.rect.width <= 0.0 {
                        continue;
                    }
                    let Some((buffer, left, top)) = placed_control_glyph(
                        &mut font_system,
                        &mut swash_cache,
                        metrics,
                        &workspace,
                        control,
                    ) else {
                        continue;
                    };
                    placed += 1;
                    // Every pixel of ink lies inside the target the glyph is clipped to.
                    let ink = drawn_ink(&mut font_system, &buffer, left, top);
                    let bounds = super::super::text_clip_bounds(control.rect);
                    let at = format!("{:?} at {scale}, {logical_width} px", control.kind);
                    assert!(ink.2 > ink.0 && ink.3 > ink.1, "{at}: the glyph has ink");
                    assert!(
                        ink.0 >= bounds.left
                            && ink.2 <= bounds.right
                            && ink.1 >= bounds.top
                            && ink.3 <= bounds.bottom,
                        "{at}: ink {ink:?} outside {:?}",
                        (bounds.left, bounds.top, bounds.right, bounds.bottom)
                    );
                    // With room, Close keeps equal right and top ink insets.
                    if control.kind == WorkspaceHeaderControl::CloseTab && logical_width >= 160 {
                        assert_eq!(width as i32 - ink.2, ink.1, "{at}");
                    }
                }
                assert_eq!(
                    placed, 2,
                    "Shortcuts and Close at {scale}, {logical_width} px"
                );
            }
        }
    }

    #[test]
    fn shortcuts_glyph_centres_its_ink_in_the_slot() {
        let mut font_system = test_sans();
        let mut swash_cache = SwashCache::new();
        let glyph = WorkspaceHeaderControl::Shortcuts.glyph();
        for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            // Slot centres fall on whole, half and quarter pixels.
            for centre in [(16.0, 20.0), (16.5, 20.5), (40.25, 23.75), (1203.5, 19.0)] {
                let (buffer, left, top) =
                    ink_centred_glyph(&mut font_system, &mut swash_cache, metrics, glyph, centre);
                assert_eq!((left.fract(), top.fract()), (0.0, 0.0));
                let ink = drawn_ink(&mut font_system, &buffer, left, top);
                assert!(ink.2 > ink.0 && ink.3 > ink.1, "{scale}: the glyph has ink");
                let ink_centre = ((ink.0 + ink.2) as f32 / 2.0, (ink.1 + ink.3) as f32 / 2.0);
                assert!(
                    (ink_centre.0 - centre.0).abs() <= 0.5
                        && (ink_centre.1 - centre.1).abs() <= 0.5,
                    "{scale}: ink centre {ink_centre:?}, slot centre {centre:?}"
                );
            }
        }
    }

    #[test]
    fn quota_pace_gradient_is_ordered_readable_and_colors_only_judged_windows() {
        assert_eq!(load_color(Load::Notice), palette::LOAD_NOTICE);
        for load in [Load::Normal, Load::Notice, Load::High, Load::Critical] {
            assert!(contrast(load_color(load), palette::TAB_IDLE) >= 4.5);
        }
        assert!(contrast(palette::PRIMARY_TEXT, palette::TAB_IDLE) >= 4.5);
        for (shortfall, color) in palette::QUOTA_PACE {
            assert_eq!(quota_pace_color(shortfall), color);
            assert!(contrast(color, palette::TAB_IDLE) >= 4.5, "{shortfall}");
        }
        assert_eq!(quota_pace_color(-1.0), palette::QUOTA_PACE[0].1);
        assert_eq!(quota_pace_color(1.0), palette::QUOTA_PACE[4].1);
        // On pace is a light green; a window a quarter early is already the full red.
        let rgb = |r, g, b| SceneColor { r, g, b };
        assert_eq!(quota_pace_color(0.0), rgb(144, 196, 93));
        assert_eq!(quota_pace_color(0.05), rgb(255, 196, 87));
        assert_eq!(quota_pace_color(0.10), rgb(255, 150, 70));
        assert_eq!(quota_pace_color(0.25), rgb(255, 75, 75));
        assert_eq!(palette::LOAD_CRITICAL, rgb(255, 75, 75));
        // More shortfall is never greener, and every shade stays readable on the chip fill.
        let mut previous = quota_pace_color(-1.0);
        for step in -2500..=2500 {
            let color = quota_pace_color(step as f32 / 10000.0);
            assert!(color.r >= previous.r && color.g <= previous.g, "{step}");
            assert!(contrast(color, palette::TAB_IDLE) >= 4.5, "{step}");
            previous = color;
        }

        for label in ["2h30m/5h 50%", "2d15h/7d 50%"] {
            let mut fonts = test_sans();
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 18.0));
            buffer.set_size(Some(600.0), Some(40.0));
            let attrs = Attrs::new().family(Family::SansSerif);
            buffer.set_text(label, &attrs, shaping(label), None);
            buffer.shape_until_scroll(&mut fonts, false);
            let before = buffer.layout_runs().next().unwrap().line_w;
            color_quota_pace(&mut buffer, &mut fonts, label, 0.25);
            let run = buffer.layout_runs().next().unwrap();
            assert_eq!(run.line_w, before);
            let red = palette::QUOTA_PACE[4].1;
            for glyph in run.glyphs {
                let expected = Some(Color::rgb(red.r, red.g, red.b));
                assert_eq!(
                    glyph.color_opt, expected,
                    "{label}: glyph at {}",
                    glyph.start
                );
            }
        }
    }

    #[test]
    fn stack_labels_keep_primary_text_unless_a_light_background_needs_contrast() {
        let gray = |value| SceneColor {
            r: value,
            g: value,
            b: value,
        };
        // Dark terminals keep primary text at any `background_opacity`: the label is judged
        // against the terminal's own colour, so a black terminal at 0.5, or the default at 0.3,
        // over a dark desktop never darkens it.
        for fill in [palette::TERMINAL_FALLBACK, gray(0), gray(40)] {
            assert_eq!(stack_label_color(fill), palette::PRIMARY_TEXT);
        }
        // Light ones darken the label just enough to read at 4.5:1.
        for fill in [gray(255), gray(244), gray(170)] {
            let color = stack_label_color(fill);
            assert_ne!(color, palette::PRIMARY_TEXT);
            assert!(contrast_ratio(color, fill) >= 4.5, "{fill:?}");
            assert!(
                contrast_ratio(color, fill) < 4.7,
                "{fill:?} darkens no further than needed"
            );
        }
    }

    #[test]
    fn pane_identity_dimming_preserves_contrast_and_shaped_geometry() {
        let name = SceneColor {
            r: 239,
            g: 244,
            b: 248,
        };
        let black = SceneColor { r: 0, g: 0, b: 0 };
        let dim = pane_id_color(name, true, true, black, 0.8);
        assert_ne!(dim, name);
        assert!(
            contrast(
                dim,
                SceneColor {
                    r: 51,
                    g: 51,
                    b: 51
                }
            ) >= 4.5
        );
        assert_eq!(pane_id_color(name, true, true, black, 0.3), name);
        assert_eq!(
            pane_id_color(
                name,
                true,
                true,
                SceneColor {
                    r: 230,
                    g: 220,
                    b: 200
                },
                1.0
            ),
            name
        );
        let mut fonts = test_sans();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 18.0));
        buffer.set_size(Some(600.0), Some(40.0));
        buffer.set_text(
            "p7  Review pane labels",
            &Attrs::new().family(Family::SansSerif),
            shaping("p7  Review pane labels"),
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let before = buffer.layout_runs().next().unwrap().line_w;
        color_label_identity(&mut buffer, &mut fonts, "p7", dim);
        let run = buffer.layout_runs().next().unwrap();
        assert_eq!(run.line_w, before);
        assert!(
            run.glyphs
                .iter()
                .filter(|glyph| glyph.start < 2)
                .all(|glyph| glyph.color_opt == Some(Color::rgb(dim.r, dim.g, dim.b)))
        );
        assert!(
            run.glyphs
                .iter()
                .filter(|glyph| glyph.start >= 4)
                .all(|glyph| glyph.color_opt.is_none())
        );
        let width = fit_header_text(&mut fonts, CellMetrics::for_scale(1.0), "p7", f32::INFINITY).1;
        let fitted = fit_header_text(
            &mut fonts,
            CellMetrics::for_scale(1.0),
            "p7  e\u{301} Review labels",
            width,
        );
        assert_eq!(fitted.0, "p7");
    }

    #[test]
    fn chrome_text_roles_keep_grey_labels_and_readable_identity_spans() {
        let light_blue = SceneColor {
            r: 162,
            g: 204,
            b: 230,
        };
        for color in [
            palette::HINT_MODIFIER_TEXT,
            palette::HINT_POPUP_TEXT,
            palette::SHORTCUT_KEY_TEXT,
            palette::SECONDARY_TEXT,
            palette::PRIMARY_TEXT,
            palette::PANE_ID,
            palette::DIALOG_SCROLL_THUMB,
        ] {
            assert_ne!(color, palette::ACCENT);
            assert_ne!(color, light_blue);
        }
        assert!(contrast_ratio(palette::DIALOG_SCROLL_THUMB, palette::SCROLLBAR_TRACK) >= 3.0);
        for (text, background) in [
            (palette::HINT_POPUP_TEXT, palette::TOOLTIP_BACKGROUND),
            (palette::HINT_MODIFIER_TEXT, palette::HINT_KEYCAP),
            (palette::HINT_MODIFIER_TEXT, palette::CHROME_HIGHLIGHT),
            (palette::SHORTCUT_KEY_TEXT, palette::DIALOG_BACKGROUND),
            (palette::SHORTCUT_KEY_TEXT, palette::CONTROL_HOVER),
            (palette::SECONDARY_TEXT, palette::DIALOG_BACKGROUND),
        ] {
            assert!(
                contrast_ratio(text, background) >= 4.5,
                "{text:?} on {background:?}"
            );
        }
        let black = SceneColor { r: 0, g: 0, b: 0 };
        assert!(
            contrast_ratio(palette::TAB_IDLE, black) < contrast_ratio(palette::TAB_HOVER, black)
        );
        assert!(
            contrast_ratio(palette::TAB_HOVER, black)
                < contrast_ratio(palette::TAB_SELECTED, black)
        );
        assert!(contrast_ratio(palette::TAB_SELECTED, palette::BAR_BACKGROUND) >= 3.0);
        assert!(contrast_ratio(palette::PRIMARY_TEXT, palette::TAB_SELECTED) >= 4.5);
        let metrics = CellMetrics::for_scale(1.0);
        let mut fonts = test_sans();
        for (label, id) in [
            ("12  e\u{301}on", "12"),
            ("p7  Review pane", "p7"),
            ("12", "12"),
        ] {
            let mut buffer =
                Buffer::new(&mut fonts, Metrics::new(metrics.font_size, metrics.height));
            buffer.set_wrap(Wrap::None);
            buffer.set_text(
                label,
                &Attrs::new().family(Family::SansSerif),
                shaping(label),
                None,
            );
            buffer.shape_until_scroll(&mut fonts, false);
            let width = buffer.layout_runs().next().unwrap().line_w;
            for background in [
                palette::TAB_IDLE,
                palette::TAB_HOVER,
                SceneColor {
                    r: 40,
                    g: 42,
                    b: 54,
                },
                SceneColor {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ] {
                let identity = blend_to_contrast(palette::PANE_ID, background, 4.5);
                let name = blend_to_contrast(palette::PRIMARY_TEXT, background, 4.5);
                assert!(contrast_ratio(identity, background) >= 4.5);
                assert!(contrast_ratio(name, background) >= 4.5);
                color_label_identity(&mut buffer, &mut fonts, id, identity);
                let run = buffer.layout_runs().next().unwrap();
                assert_eq!(run.line_w, width);
                assert_eq!(run.text, label);
                assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id != 0));
                for glyph in run.glyphs {
                    assert_eq!(
                        glyph.color_opt,
                        (glyph.start < id.len())
                            .then_some(Color::rgb(identity.r, identity.g, identity.b))
                    );
                }
            }
            let before = buffer.lines[0].attrs_list().clone();
            color_label_identity(&mut buffer, &mut fonts, "absent", palette::ACCENT);
            assert_eq!(buffer.lines[0].attrs_list(), &before);
        }
    }

    #[test]
    fn eon_bar_widgets_fill_the_same_capsule_as_idle_tabs() {
        for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
            let visual = SceneRect {
                left: 8.0 * scale,
                top: 6.0 * scale,
                width: 40.0 * scale,
                height: 16.0 * scale,
            };
            let mut widget = RectangleBatch::new(128, 128);
            push_bar_widget(&mut widget, visual);
            let mut tab = RectangleBatch::new(128, 128);
            tab.push_rounded(
                visual,
                tokens::capsule_radius(visual.height),
                palette::TAB_IDLE,
            );
            assert_eq!(widget.bytes, tab.bytes, "scale {scale}");
            // A filled capsule includes an opaque interior, unlike the old outline.
            assert!(
                widget
                    .bytes
                    .chunks_exact(VERTEX_SIZE as usize)
                    .any(|vertex| {
                        let value = |component: usize| {
                            f32::from_ne_bytes(
                                vertex[component * 4..component * 4 + 4].try_into().unwrap(),
                            )
                        };
                        (value(1) - (1.0 - (visual.top + visual.height / 2.0) / 64.0)).abs() < 0.1
                            && value(5) == 1.0
                    })
            );
        }
    }

    #[test]
    fn bar_surfaces_keep_distinct_session_hover_press_and_mint_focus() {
        assert_ne!(palette::TAB_IDLE, palette::TAB_HOVER);
        type Surface = fn(&mut RectangleBatch, SceneRect, HeaderButtonState);
        for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
            let visual = SceneRect {
                left: 8.0 * scale,
                top: 6.0 * scale,
                width: 40.0 * scale,
                height: 16.0 * scale,
            };
            let state = HeaderButtonState {
                hovered: false,
                pressed: false,
                focused: false,
                enabled: true,
            };
            let bytes = |draw: fn(&mut RectangleBatch, SceneRect, HeaderButtonState), state| {
                let mut batch = RectangleBatch::new(128, 128);
                draw(&mut batch, visual, state);
                batch.bytes
            };
            let idle = bytes(push_session_label_surface, state);
            let hover = bytes(
                push_session_label_surface,
                HeaderButtonState {
                    hovered: true,
                    ..state
                },
            );
            let press = bytes(
                push_session_label_surface,
                HeaderButtonState {
                    pressed: true,
                    ..state
                },
            );
            assert_ne!(idle, hover);
            assert_ne!(hover, press);
            for (draw, fills) in [
                (
                    push_session_label_surface as Surface,
                    [
                        palette::TAB_IDLE,
                        palette::TAB_HOVER,
                        palette::SESSION_PRESSED,
                    ],
                ),
                (
                    push_bar_control_surface as Surface,
                    [
                        palette::BAR_BACKGROUND,
                        palette::CONTROL_HOVER,
                        palette::CHROME_HIGHLIGHT,
                    ],
                ),
            ] {
                for (state, fill) in [
                    (state, fills[0]),
                    (
                        HeaderButtonState {
                            hovered: true,
                            ..state
                        },
                        fills[1],
                    ),
                    (
                        HeaderButtonState {
                            hovered: true,
                            pressed: true,
                            ..state
                        },
                        fills[2],
                    ),
                ] {
                    let surface = bytes(draw, state);
                    let expected = [fill.r, fill.g, fill.b].map(|v| f32::from(v) / 255.0);
                    let last = surface.chunks_exact(VERTEX_SIZE as usize).last().unwrap();
                    for (i, value) in expected.into_iter().enumerate() {
                        let offset = (i + 2) * 4;
                        assert_eq!(
                            f32::from_ne_bytes(last[offset..offset + 4].try_into().unwrap()),
                            value
                        );
                    }
                    let focused = bytes(
                        draw,
                        HeaderButtonState {
                            focused: true,
                            ..state
                        },
                    );
                    let mut outline = RectangleBatch::new(128, 128);
                    outline.push_rounded_outline(
                        visual,
                        tokens::capsule_radius(visual.height),
                        palette::ACCENT,
                    );
                    assert!(focused.starts_with(&surface));
                    assert!(focused.ends_with(&outline.bytes));
                }
            }
            assert_eq!(
                idle,
                bytes(
                    push_session_label_surface,
                    HeaderButtonState {
                        hovered: true,
                        pressed: true,
                        focused: true,
                        enabled: false,
                    }
                )
            );
        }
    }

    #[test]
    fn only_badges_wholly_inside_the_tab_viewport_are_drawn() {
        let badge = |left| TabBadge {
            state: AttentionState::Working,
            dimmed: false,
            rect: SceneRect {
                left,
                top: 4.0,
                width: 12.0,
                height: 12.0,
            },
        };
        let viewport = Some(SceneRect {
            width: 240.0,
            height: 40.0,
            ..SceneRect::default()
        });
        assert!(badge_inside(badge(228.0), viewport));
        // A fractional scroll that leaves a sliver inside still hides it past the edge.
        assert!(!badge_inside(badge(228.01), viewport));
        assert!(!badge_inside(badge(238.99), viewport));
        assert!(!badge_inside(badge(-0.01), viewport));
        assert!(badge_inside(badge(238.99), None));
    }

    #[test]
    fn braille_phases_rewrite_only_alpha_in_place_and_light_their_dots() {
        // A tick rewrites without the tab viewport's clip, which leaves a drawn badge unchanged,
        // down to the one-pixel badge of the smallest typography, at the viewport's edge.
        let viewport = Some(SceneRect {
            width: 240.0,
            height: 40.0,
            ..SceneRect::default()
        });
        for size in [24.0, 12.0, 6.0, 1.0] {
            let badge = TabBadge {
                state: AttentionState::Working,
                dimmed: false,
                rect: SceneRect {
                    left: 240.0 - size,
                    top: 4.0,
                    width: size,
                    height: size,
                },
            };
            let glyph = |phase, clip| {
                let mut rectangles = RectangleBatch::new(800, 600);
                rectangles.clip = clip;
                push_badge_glyph(&mut rectangles, badge, Some(phase), DEFAULT_FOCUS);
                rectangles
                    .bytes
                    .chunks_exact(4)
                    .map(|value| f32::from_ne_bytes(value.try_into().unwrap()))
                    .collect::<Vec<_>>()
            };
            assert!(badge_inside(badge, viewport));
            let first = glyph(0, None);
            for (phase, character) in "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".chars().enumerate() {
                let bits = SPINNER_FRAMES[phase];
                assert_eq!(char::from_u32(0x2800 + u32::from(bits)), Some(character));
                let turned = glyph(phase, None);
                assert_eq!(turned, glyph(phase, viewport));
                assert_eq!(turned.len(), first.len());
                // Each vertex is x, y, r, g, b, alpha: only alpha changes.
                for (vertex, first) in turned.chunks_exact(6).zip(first.chunks_exact(6)) {
                    assert_eq!(vertex[..5], first[..5]);
                }
                if size < 6.0 {
                    continue;
                }
                // Six dots of six vertices each; a lit dot's column and row name its braille dot.
                // Vertices are in clip space, whose y points up.
                let dots = turned
                    .chunks_exact(36)
                    .map(|dot| (dot[0], -dot[1], dot[5]))
                    .collect::<Vec<_>>();
                assert_eq!(dots.len(), 6);
                let mut columns = dots.iter().map(|dot| dot.0).collect::<Vec<_>>();
                let mut rows = dots.iter().map(|dot| dot.1).collect::<Vec<_>>();
                for axis in [&mut columns, &mut rows] {
                    axis.sort_by(f32::total_cmp);
                    axis.dedup();
                }
                assert_eq!((columns.len(), rows.len()), (2, 3));
                let lit = dots
                    .iter()
                    .filter(|dot| dot.2 > 0.0)
                    .map(|dot| {
                        let column = columns.iter().position(|x| *x == dot.0).unwrap();
                        let row = rows.iter().position(|y| *y == dot.1).unwrap();
                        1u32 << (column * 3 + row)
                    })
                    .sum::<u32>();
                assert_eq!(lit, character as u32 - 0x2800, "{character} at {size}");
            }
        }
    }

    /// A badge's dots as one character per pixel: # a lit dot, . an unlit one.
    fn badge_art(
        state: AttentionState,
        left: f32,
        top: f32,
        size: f32,
        phase: Option<usize>,
    ) -> Vec<String> {
        let badge = TabBadge {
            state,
            dimmed: false,
            rect: SceneRect {
                left,
                top,
                width: size,
                height: size,
            },
        };
        let mut rectangles = RectangleBatch::new(800, 600);
        push_badge_glyph(&mut rectangles, badge, phase, DEFAULT_FOCUS);
        let side = size as usize;
        let mut rows = vec![vec![' '; side]; side];
        // Each dot is one quad of six vertices of x, y, r, g, b, alpha, in clip space.
        for dot in rectangles.bytes.chunks_exact(24 * 6) {
            let value = |at: usize| f32::from_ne_bytes(dot[at..at + 4].try_into().unwrap());
            let xs = [0, 24, 48].map(|at| (value(at) + 1.0) * 400.0);
            let ys = [4, 28, 52].map(|at| (1.0 - value(at)) * 300.0);
            let low = |values: [f32; 3]| values.into_iter().fold(f32::MAX, f32::min);
            let high = |values: [f32; 3]| values.into_iter().fold(0.0, f32::max);
            let (dot_left, dot_top, dot_right, dot_bottom) = (low(xs), low(ys), high(xs), high(ys));
            for edge in [dot_left, dot_top, dot_right, dot_bottom] {
                assert!(
                    (edge - edge.round()).abs() < 1e-3,
                    "dots keep to whole pixels"
                );
            }
            let [dot_left, dot_top, dot_right, dot_bottom] =
                [dot_left, dot_top, dot_right, dot_bottom].map(f32::round);
            assert!(dot_left >= left && dot_right <= left + size);
            assert!(dot_top >= top && dot_bottom <= top + size);
            let mark = if value(20) > 0.0 { '#' } else { '.' };
            let (x, y) = (left.ceil(), top.ceil());
            for row in &mut rows[(dot_top - y) as usize..(dot_bottom - y) as usize] {
                row[(dot_left - x) as usize..(dot_right - x) as usize].fill(mark);
            }
        }
        rows.into_iter()
            .map(|row| row.into_iter().collect::<String>())
            .collect()
    }

    #[test]
    fn working_badges_draw_braille_dots_on_whole_pixels() {
        // The badges of the default typography at scales 1 and 2.
        let art =
            |left, top, size, phase| badge_art(AttentionState::Working, left, top, size, phase);
        // Frame 0, ⠋, is also the still form.
        assert_eq!(
            art(4.0, 4.0, 12.0, None),
            [
                "            ",
                "            ",
                "   ## ##    ",
                "   ## ##    ",
                "            ",
                "   ## ..    ",
                "   ## ..    ",
                "            ",
                "   .. ..    ",
                "   .. ..    ",
                "            ",
                "            ",
            ]
        );
        assert_eq!(
            art(4.0, 4.0, 24.0, None),
            [
                "                        ",
                "                        ",
                "                        ",
                "                        ",
                "       ####  ####       ",
                "       ####  ####       ",
                "       ####  ####       ",
                "       ####  ####       ",
                "                        ",
                "                        ",
                "       ####  ....       ",
                "       ####  ....       ",
                "       ####  ....       ",
                "       ####  ....       ",
                "                        ",
                "                        ",
                "       ....  ....       ",
                "       ....  ....       ",
                "       ....  ....       ",
                "       ....  ....       ",
                "                        ",
                "                        ",
                "                        ",
                "                        ",
            ]
        );
        for size in [12.0, 24.0] {
            assert_eq!(art(4.0, 4.0, size, None), art(4.0, 4.0, size, Some(0)));
            // A badge between pixels still draws whole-pixel dots inside it.
            art(4.5, 3.25, size, Some(4));
        }
    }

    #[test]
    fn background_badges_draw_three_still_dots_that_never_tick() {
        let art = |size| badge_art(AttentionState::Background, 4.0, 4.0, size, None);
        let mut small = vec!["            "; 12];
        small[5..7].fill("  ## ## ##  ");
        assert_eq!(art(12.0), small);
        let mut large = vec!["                        "; 24];
        large[10..14].fill("    ####  ####  ####    ");
        assert_eq!(art(24.0), large);
        let badge = TabBadge {
            state: AttentionState::Background,
            dimmed: false,
            rect: SceneRect {
                width: 12.0,
                height: 12.0,
                ..SceneRect::default()
            },
        };
        let mut spinners = Spinners::new(true, DEFAULT_FOCUS);
        let mut rectangles = RectangleBatch::new(800, 600);
        spinners.push_badge(
            badge,
            Backing::Surface(palette::BAR_BACKGROUND),
            &mut rectangles,
        );
        assert!(
            spinners.slots.is_empty(),
            "a background badge schedules no tick"
        );
        // A phase never changes it.
        let mut turned = RectangleBatch::new(800, 600);
        push_badge_glyph(&mut turned, badge, Some(4), DEFAULT_FOCUS);
        assert_eq!(turned.bytes, rectangles.bytes);
    }

    #[test]
    fn software_adapters_draw_working_badges_still_and_never_tick() {
        let badge = TabBadge {
            state: AttentionState::Working,
            dimmed: false,
            rect: SceneRect {
                width: 12.0,
                height: 12.0,
                ..SceneRect::default()
            },
        };
        let draw = |turning| {
            let mut spinners = Spinners::new(turning, DEFAULT_FOCUS);
            let mut rectangles = RectangleBatch::new(800, 600);
            spinners.push_badge(
                badge,
                Backing::Surface(palette::BAR_BACKGROUND),
                &mut rectangles,
            );
            let alphas = rectangles
                .bytes
                .chunks_exact(24)
                .map(|vertex| f32::from_ne_bytes(vertex[20..].try_into().unwrap()))
                .collect::<Vec<_>>();
            (spinners.slots.len(), alphas)
        };
        let (turning_slots, turning) = draw(true);
        let (still_slots, still) = draw(false);
        // Only a turning badge is recorded, so a software adapter's frame reports no spinner and
        // `update_spinner` schedules no tick.
        assert_eq!((turning_slots, still_slots), (1, 0));
        assert_eq!(turning, still);
    }

    fn popup_sidebar_snapshot() -> eon_workspace_protocol::v41::Snapshot {
        use eon_workspace_protocol::v41::{
            Attention, AttentionRecord, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey,
            Sidebar, Snapshot, Tab,
        };

        let attention = Attention {
            state: AttentionState::Working,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: "codex".into(),
                name: String::new(),
                title: "Review output".into(),
                message: String::new(),
                changed_at: 1,
                stale: false,
                others: 0,
            }),
        };
        let pane = |id: &str, attention| Pane {
            id: id.into(),
            label: id.into(),
            pinned_name: None,
            session: format!("terminal-{id}"),
            endpoint: format!("/tmp/{id}.sock").into_bytes(),
            live: true,
            attention,
        };
        Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v41::WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: Some(AttentionState::Working),
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
                shortcut: PopupKey::Unbound,
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp".to_vec(),
                pending: false,
                selected_pane: Some("p2".into()),
                selected_popup: None,
                columns: vec![Column {
                    panes: vec![pane("p1", Some(attention.clone())), pane("p2", None)],
                    expanded: "p2".into(),
                }],
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "agent".into(),
                    session: "terminal-u1".into(),
                    endpoint: b"/tmp/u1.sock".to_vec(),
                    attention: Some(attention),
                }],
                attention_rollup: Some(AttentionState::Working),
            }],
            codex_quota: None,
            claude_quota: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: Sidebar::Open,
            attention_queue: Vec::new(),
        }
    }

    #[test]
    fn popup_and_pane_rows_share_surfaces_in_every_state() {
        let snapshot = popup_sidebar_snapshot();
        let metrics = CellMetrics::for_scale(1.0);
        let scene = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1600, 900),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        );
        assert_eq!(scene.sidebar_rows().len(), 2);
        let mut row = scene.sidebar_rows()[0].clone();
        assert!(row.popup_marker.is_some());
        for focus in FOCUS_COLORS {
            for (focused, current, hovered, dimmed) in
                (0..16).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
            {
                row.focused = focused;
                row.current = current;
                row.glyph.dimmed = dimmed;
                let white = SceneColor {
                    r: 255,
                    g: 255,
                    b: 255,
                };
                for (surface, background) in [
                    (
                        Backing::Translucent(palette::TERMINAL_FALLBACK),
                        palette::TERMINAL_FALLBACK,
                    ),
                    (Backing::Translucent(white), white),
                    (
                        Backing::Surface(palette::TERMINAL_FALLBACK),
                        palette::TERMINAL_FALLBACK,
                    ),
                    (Backing::Surface(white), white),
                ] {
                    let paint = |row: &crate::SidebarRow| {
                        let mut rectangles = RectangleBatch::new(1600, 900);
                        let colors = push_attention_row_surface(
                            row,
                            hovered,
                            metrics.padding / 2.0,
                            surface,
                            background,
                            &mut Spinners::new(false, focus),
                            &mut rectangles,
                        );
                        (colors, rectangles.bytes)
                    };
                    let popup = paint(&row);
                    let mut pane = row.clone();
                    pane.popup_marker = None;
                    assert_eq!(popup, paint(&pane));
                    let fill = if focused || hovered {
                        let rgb = [2, 3, 4].map(|i| {
                            (f32::from_ne_bytes(popup.1[i * 4..i * 4 + 4].try_into().unwrap())
                                * 255.0)
                                .round() as u8
                        });
                        SceneColor {
                            r: rgb[0],
                            g: rgb[1],
                            b: rgb[2],
                        }
                    } else {
                        background
                    };
                    for text in [popup.0.0, popup.0.1] {
                        assert!(contrast_ratio(text, fill) >= 4.5);
                    }
                    if current {
                        let primary = blend_to_contrast(palette::PRIMARY_TEXT, fill, 4.5);
                        assert_eq!(popup.0, (primary, primary));
                    }
                }
            }
        }
    }

    #[test]
    fn sidebar_hover_and_keyboard_fill_remain_distinct_from_rest() {
        let snapshot = popup_sidebar_snapshot();
        let metrics = CellMetrics::for_scale(1.0);
        let scene = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(1600, 900),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
        );
        let mut row = scene.sidebar_rows()[0].clone();
        for background in [palette::BAR_BACKGROUND, palette::CHROME_HIGHLIGHT]
            .into_iter()
            .chain((0..=255).map(|value| SceneColor {
                r: value,
                g: value,
                b: value,
            }))
        {
            for current in [false, true] {
                row.current = current;
                for (focused, hovered) in [(false, true), (true, false), (true, true)] {
                    row.focused = focused;
                    let mut batch = RectangleBatch::new(1600, 900);
                    push_attention_row_surface(
                        &row,
                        hovered,
                        tokens::inner_radius(metrics),
                        Backing::Surface(background),
                        background,
                        &mut Spinners::new(false, DEFAULT_FOCUS),
                        &mut batch,
                    );
                    let rgb = [2, 3, 4].map(|i| {
                        (f32::from_ne_bytes(batch.bytes[i * 4..i * 4 + 4].try_into().unwrap())
                            * 255.0)
                            .round() as u8
                    });
                    let fill = SceneColor {
                        r: rgb[0],
                        g: rgb[1],
                        b: rgb[2],
                    };
                    assert!(contrast_ratio(fill, background) >= 1.5);
                    let expected = if contrast_ratio(palette::CHROME_HIGHLIGHT, background) >= 1.5 {
                        palette::CHROME_HIGHLIGHT
                    } else {
                        blend_to_contrast(background, background, 1.5)
                    };
                    assert_eq!(fill, expected, "{background:?}, current={current}");
                }
            }
        }
    }

    #[test]
    fn sidebar_rows_keep_current_and_keyboard_outlines_distinct_and_content_readable() {
        let snapshot = popup_sidebar_snapshot();

        // The approved outline stays the same on hover; 3:1 is against the sidebar background.
        assert!(contrast_ratio(DEFAULT_FOCUS, palette::CHROME_HIGHLIGHT) < 3.0);
        assert_ne!(DEFAULT_FOCUS, palette::CHROME_HIGHLIGHT);
        for scale in [1.0, 1.125, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let size = PhysicalSize::new((1600.0 * scale) as u32, (900.0 * scale) as u32);
            let scene =
                WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
                    (text.into(), text.len() as f32 * metrics.width)
                });
            let mut row = scene.sidebar_rows()[0].clone();
            row.glyph.dimmed = false;
            // Exercise fractional placement and the clipping used by the shared list painter.
            row.rect.left += 0.25;
            row.rect.top += 0.25;
            let outer = SceneRect {
                left: row.rect.left.ceil(),
                top: row.rect.top.ceil(),
                width: row.rect.right().floor() - row.rect.left.ceil(),
                height: row.rect.bottom().floor() - row.rect.top.ceil(),
            };
            let inner = SceneRect {
                left: outer.left + 1.0,
                top: outer.top + 1.0,
                width: outer.width - 2.0,
                height: outer.height - 2.0,
            };
            for background in [
                palette::BAR_BACKGROUND,
                palette::CHROME_HIGHLIGHT,
                SceneColor { r: 4, g: 5, b: 7 },
                SceneColor {
                    r: 80,
                    g: 80,
                    b: 80,
                },
                SceneColor {
                    r: 100,
                    g: 100,
                    b: 100,
                },
                SceneColor {
                    r: 128,
                    g: 128,
                    b: 128,
                },
                SceneColor {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ] {
                for focus in FOCUS_COLORS {
                    let current_color = blend_to_contrast(focus, background, 3.0);
                    assert!(contrast_ratio(current_color, background) >= 3.0);
                    for opacity in [0.8, 1.0] {
                        let surface = Backing::card(background, opacity);
                        for (current, focused, hovered) in
                            (0..8).map(|bits| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0))
                        {
                            row.current = current;
                            row.focused = focused;
                            let fill = (focused || hovered).then(|| {
                                if contrast_ratio(palette::CHROME_HIGHLIGHT, background) >= 1.5 {
                                    palette::CHROME_HIGHLIGHT
                                } else {
                                    blend_to_contrast(background, background, 1.5)
                                }
                            });
                            let under = fill.unwrap_or(background);
                            for state in [
                                AttentionState::NeedsInput,
                                AttentionState::Error,
                                AttentionState::Finished,
                                AttentionState::Working,
                                AttentionState::Background,
                                AttentionState::Idle,
                                AttentionState::Unavailable,
                            ] {
                                row.glyph.state = state;
                                let mut batch = RectangleBatch::new(size.width, size.height);
                                batch.clip = Some(row.rect);
                                let colors = push_attention_row_surface(
                                    &row,
                                    hovered,
                                    tokens::inner_radius(metrics),
                                    surface,
                                    background,
                                    &mut Spinners::new(false, focus),
                                    &mut batch,
                                );
                                let mut expected = RectangleBatch::new(size.width, size.height);
                                expected.clip = Some(row.rect);
                                if let Some(fill) = fill {
                                    expected.push_rounded(
                                        row.rect,
                                        tokens::inner_radius(metrics),
                                        fill,
                                    );
                                    assert_ne!(current_color, fill);
                                }
                                let outline_start = expected.bytes.len();
                                if current {
                                    expected.push_rounded_outline(
                                        outer,
                                        tokens::inner_radius(metrics).round(),
                                        current_color,
                                    );
                                }
                                if focused {
                                    let mint = blend_to_contrast(palette::ACCENT, under, 3.0);
                                    assert!(contrast_ratio(mint, under) >= 3.0);
                                    expected.push_rounded_outline(
                                        if current { inner } else { outer },
                                        (tokens::inner_radius(metrics).round()
                                            - if current { 1.0 } else { 0.0 })
                                        .max(0.0),
                                        mint,
                                    );
                                }
                                let outline_end = expected.bytes.len();
                                let backing = fill.map_or(surface, Backing::Surface);
                                let mut badge = RectangleBatch::new(size.width, size.height);
                                badge.clip = Some(row.rect);
                                Spinners::new(false, focus)
                                    .push_badge(row.glyph, backing, &mut badge);
                                expected.bytes.extend_from_slice(&badge.bytes);
                                assert_eq!(
                                    batch.bytes, expected.bytes,
                                    "scale={scale}, {background:?}, {focus:?}, {state:?}, current={current}, focused={focused}, hovered={hovered}, opacity={opacity}"
                                );
                                for vertex in batch.bytes[outline_start..outline_end]
                                    .chunks_exact(VERTEX_SIZE as usize)
                                {
                                    let ndc = [0, 1].map(|i| {
                                        f32::from_ne_bytes(
                                            vertex[i * 4..i * 4 + 4].try_into().unwrap(),
                                        )
                                    });
                                    for pixel in [
                                        (ndc[0] + 1.0) * size.width as f32 / 2.0,
                                        (1.0 - ndc[1]) * size.height as f32 / 2.0,
                                    ] {
                                        assert!(
                                            (pixel - pixel.round()).abs() < 0.001,
                                            "scale={scale}, fractional outline edge {pixel}"
                                        );
                                    }
                                }
                                for text in [colors.0, colors.1] {
                                    assert!(
                                        contrast_ratio(text, under) >= 4.5,
                                        "{background:?}, {text:?}"
                                    );
                                }
                                let under = match backing {
                                    Backing::Surface(color) | Backing::BarPill(color) => color,
                                    Backing::Translucent(color) => {
                                        blend_to_contrast(color, color, 1.5)
                                    }
                                };
                                for vertex in badge.bytes.chunks_exact(VERTEX_SIZE as usize) {
                                    let color = vertex_color(vertex);
                                    if color != under {
                                        assert!(
                                            contrast_ratio(color, under) >= 3.0,
                                            "{state:?}: {color:?} on {under:?}"
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

    #[test]
    fn popup_marker_is_two_crisp_hollow_windows_with_the_back_occluded() {
        let mut snapshot = popup_sidebar_snapshot();
        let attention = snapshot.tabs[0].popups[0].attention.as_mut().unwrap();
        attention.state = AttentionState::NeedsInput;
        attention.record.as_mut().unwrap().message = "A question".into();
        let color = palette::SECONDARY_TEXT;
        let mut fractional_top = false;
        for (scale, font_size, height, padding) in [
            (1.0_f32, 16.0_f32, 19.0, 12.0),
            (1.125, 18.0, 21.0, 14.0),
            (2.0, 32.0, 38.0, 24.0),
            (1.0, 16.0, 18.0, 13.0),
            (1.0, 6.0, 7.0, 5.0),
            (1.125, 6.75, 8.0, 6.0),
            (2.0, 12.0, 14.0, 10.0),
        ] {
            let metrics = CellMetrics {
                scale,
                width: (font_size * 0.625).round(),
                height,
                font_size,
                padding,
            };
            let scene = WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new((1600.0 * scale) as u32, (900.0 * scale) as u32),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
            );
            let row = &scene.sidebar_rows()[0];
            let marker = row.popup_marker.unwrap();
            fractional_top |=
                marker.top.fract() != 0.0 && (row.rect.height - marker.height) % 2.0 != 0.0;
            let rect = SceneRect {
                left: marker.left.round(),
                top: marker.top.round(),
                ..marker
            };
            let side = (rect.width * 2.0 / 3.0).round();
            let offset = rect.width - side;
            let front = SceneRect {
                left: rect.left + offset,
                width: side,
                height: side,
                ..rect
            };
            let back = SceneRect {
                left: rect.left,
                top: rect.top + offset,
                ..front
            };
            for border in 1..=3 {
                let full_stroke = physical_outline_width(border, scale);
                let stroke = full_stroke.min(((side - 1.0) / 2.0).floor());
                let mut batch = RectangleBatch::new(2048, 2048);
                push_popup_marker(&mut batch, marker, full_stroke, color);
                let ink = batch
                    .bytes
                    .chunks_exact(VERTEX_SIZE as usize * 6)
                    .map(|quad| {
                        let value = |vertex: usize, component: usize| {
                            let offset = vertex * VERTEX_SIZE as usize + component * 4;
                            f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                        };
                        let quad = SceneRect {
                            left: (value(0, 0) + 1.0) * 1024.0,
                            top: (1.0 - value(0, 1)) * 1024.0,
                            width: (value(2, 0) - value(0, 0)) * 1024.0,
                            height: (value(0, 1) - value(2, 1)) * 1024.0,
                        };
                        assert_eq!((quad.top.fract(), quad.bottom().fract()), (0.0, 0.0));
                        if quad.height > 1.0 {
                            assert_eq!((quad.left.fract(), quad.right().fract()), (0.0, 0.0));
                        }
                        assert!(quad.left >= rect.left && quad.right() <= rect.right());
                        assert!(quad.top >= rect.top && quad.bottom() <= rect.bottom());
                        (quad, value(0, 5))
                    })
                    .collect::<Vec<_>>();
                let coverage = |x, y| {
                    ink.iter()
                        .filter(|(quad, _)| quad.contains(x, y))
                        .map(|(_, alpha)| alpha)
                        .sum::<f32>()
                };
                let old_top = rect.left + (rect.width / 4.0).round() + 0.5;
                assert_eq!(
                    coverage(old_top, rect.top + 0.5),
                    0.0,
                    "single checkbox outline"
                );
                for pixel in 0..side as u32 {
                    let position = pixel as f32 + 0.5;
                    let expected = if position < stroke || position >= side - stroke {
                        1.0
                    } else {
                        0.0
                    };
                    assert_eq!(
                        coverage(front.left + side / 2.0, front.top + position),
                        expected,
                        "front vertical at scale {scale}, border {border}, pixel {pixel}"
                    );
                    assert_eq!(
                        coverage(front.left + position, front.top + side / 2.0),
                        expected,
                        "front horizontal at scale {scale}, border {border}, pixel {pixel}"
                    );
                }
                let radius = (side / 4.0).round();
                for x in stroke as u32..(side - stroke) as u32 {
                    for y in stroke as u32..(side - stroke) as u32 {
                        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                        let dx = (radius - x + 0.5).max(x + 0.5 - (side - radius)).max(0.0);
                        let dy = (radius - y + 0.5).max(y + 0.5 - (side - radius)).max(0.0);
                        if dx * dx + dy * dy <= (radius - stroke).max(0.0).powi(2) {
                            assert_eq!(
                                coverage(front.left + x, front.top + y),
                                0.0,
                                "rear stroke inside front at scale {scale}, border {border}"
                            );
                        }
                    }
                }
                assert_eq!(
                    coverage(back.left + stroke / 2.0, back.top + side / 2.0),
                    1.0
                );
                assert_eq!(
                    coverage(back.left + side / 2.0, back.bottom() - stroke / 2.0),
                    1.0
                );
                assert_eq!(coverage(back.left + side / 2.0, back.top + side / 2.0), 0.0);
                for (x, y) in [
                    (front.right() - 0.5, front.top + 0.5),
                    (back.left + 0.5, back.bottom() - 0.5),
                ] {
                    assert!(coverage(x, y) < 1.0, "square corner remains");
                }
                for vertex in batch.bytes.chunks_exact(VERTEX_SIZE as usize) {
                    for (component, channel) in [color.r, color.g, color.b].into_iter().enumerate()
                    {
                        let offset = (component + 2) * 4;
                        let actual =
                            f32::from_ne_bytes(vertex[offset..offset + 4].try_into().unwrap());
                        assert!((actual - f32::from(channel) / 255.0).abs() < 0.001);
                    }
                }
            }
            let mut empty = RectangleBatch::new(2048, 2048);
            push_popup_marker(
                &mut empty,
                SceneRect {
                    width: 0.0,
                    height: 0.0,
                    ..marker
                },
                1.0,
                color,
            );
            assert!(empty.bytes.is_empty());
        }
        assert!(
            fractional_top,
            "scene geometry must exercise odd row/marker parity"
        );
    }

    #[test]
    fn attention_badges_use_bare_opaque_surfaces_and_quiet_square_fallbacks() {
        for (state, base) in [
            (AttentionState::NeedsInput, palette::ATTENTION_NEEDS_INPUT),
            (AttentionState::Error, palette::ATTENTION_ERROR),
            (AttentionState::Finished, palette::ATTENTION_FINISHED),
            (AttentionState::Working, DEFAULT_FOCUS),
            (AttentionState::Background, DEFAULT_FOCUS),
            (AttentionState::Idle, palette::ATTENTION_IDLE),
            (AttentionState::Unavailable, palette::ATTENTION_IDLE),
        ] {
            for focus in FOCUS_COLORS {
                let base = if matches!(state, AttentionState::Working | AttentionState::Background)
                {
                    focus
                } else {
                    base
                };
                for background in [
                    palette::BAR_BACKGROUND,
                    palette::TAB_IDLE,
                    palette::TAB_HOVER,
                    palette::TAB_SELECTED,
                    palette::SESSION_PRESSED,
                    palette::DIALOG_BACKGROUND,
                    SceneColor {
                        r: 119,
                        g: 119,
                        b: 119,
                    },
                    SceneColor {
                        r: 255,
                        g: 255,
                        b: 255,
                    },
                ] {
                    for opacity in [0.8, 1.0] {
                        for dimmed in [false, true] {
                            let badge = TabBadge {
                                state,
                                dimmed,
                                rect: SceneRect {
                                    width: 12.0,
                                    height: 12.0,
                                    ..SceneRect::default()
                                },
                            };
                            let mut expected_square = RectangleBatch::new(800, 600);
                            let under = if opacity < 1.0 {
                                let square = blend_to_contrast(background, background, 1.5);
                                assert_ne!(square, palette::BAR_BACKGROUND);
                                expected_square.push_rounded(badge.rect, 3.0, square);
                                square
                            } else {
                                background
                            };
                            let color = blend_to_contrast(base, under, 3.0);
                            assert!(contrast_ratio(color, under) >= 3.0);
                            let mut glyph = RectangleBatch::new(800, 600);
                            push_badge_glyph(&mut glyph, badge, Some(0), color);
                            let mut actual = RectangleBatch::new(800, 600);
                            let mut spinners = Spinners::new(true, focus);
                            spinners.push_badge(
                                badge,
                                Backing::card(background, opacity),
                                &mut actual,
                            );
                            let offset = expected_square.bytes.len();
                            let valid = actual.bytes.len() == offset + glyph.bytes.len()
                                && actual.bytes.starts_with(&expected_square.bytes)
                                && actual.bytes[offset..].chunks_exact(24).all(|vertex| {
                                    let alpha =
                                        f32::from_ne_bytes(vertex[20..24].try_into().unwrap());
                                    vertex_color(vertex) == color
                                        && (alpha == 0.0
                                            || alpha == if dimmed { 0.45 } else { 1.0 })
                                });
                            assert!(
                                valid,
                                "{state:?}, focus={focus:?}, background={background:?}, opacity={opacity}, dimmed={dimmed}"
                            );
                            assert_eq!(
                                spinners.slots.first().map(|slot| (slot.offset, slot.color)),
                                (state == AttentionState::Working).then_some((offset, color)),
                                "{state:?}, focus={focus:?}, background={background:?}, opacity={opacity}, dimmed={dimmed}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn overflow_badges_adapt_only_to_pills_beneath_them() {
        let pill = SceneRect {
            left: 10.0,
            top: 10.0,
            width: 100.0,
            height: 24.0,
        };
        let badge = TabBadge {
            state: AttentionState::Error,
            dimmed: false,
            rect: SceneRect {
                left: 40.0,
                top: 8.0,
                width: 12.0,
                height: 12.0,
            },
        };
        let far = SceneRect {
            left: 200.0,
            ..pill
        };
        let check = |badge, pills, color| {
            let mut actual = RectangleBatch::new(800, 600);
            Spinners::new(false, DEFAULT_FOCUS).push_edge_badge(badge, pills, &mut actual);
            let mut expected = RectangleBatch::new(800, 600);
            push_badge_glyph(&mut expected, badge, None, color);
            assert_eq!(
                actual.bytes, expected.bytes,
                "only intersecting pill fills may adapt the glyph"
            );
        };
        for fill in [palette::TAB_IDLE, palette::TAB_HOVER, palette::TAB_SELECTED] {
            check(
                badge,
                [(pill, fill), (far, palette::TAB_SELECTED)],
                blend_to_contrast(palette::ATTENTION_ERROR, fill, 3.0),
            );
        }
        check(
            badge,
            [(far, palette::TAB_SELECTED); 2],
            palette::ATTENTION_ERROR,
        );
        // A rounded pill's empty corner is bar, even where its bounding box intersects.
        check(
            TabBadge {
                rect: SceneRect {
                    left: 4.0,
                    top: 4.0,
                    ..badge.rect
                },
                ..badge
            },
            [
                (
                    SceneRect {
                        height: 48.0,
                        ..pill
                    },
                    palette::TAB_SELECTED,
                ),
                (far, palette::TAB_SELECTED),
            ],
            palette::ATTENTION_ERROR,
        );
    }

    #[test]
    fn bar_badges_contrast_with_both_the_pill_and_the_bar() {
        for focus in FOCUS_COLORS {
            for fill in [
                palette::TAB_IDLE,
                palette::TAB_HOVER,
                palette::TAB_SELECTED,
                palette::SESSION_PRESSED,
            ] {
                for state in [
                    AttentionState::NeedsInput,
                    AttentionState::Error,
                    AttentionState::Finished,
                    AttentionState::Working,
                    AttentionState::Background,
                ] {
                    let badge = TabBadge {
                        state,
                        dimmed: false,
                        rect: SceneRect {
                            width: 12.0,
                            height: 12.0,
                            ..SceneRect::default()
                        },
                    };
                    let mut rectangles = RectangleBatch::new(800, 600);
                    Spinners::new(true, focus).push_badge(
                        badge,
                        Backing::BarPill(fill),
                        &mut rectangles,
                    );
                    for vertex in rectangles.bytes.chunks_exact(24) {
                        let color = vertex_color(vertex);
                        for surface in [fill, palette::BAR_BACKGROUND] {
                            assert!(
                                contrast_ratio(color, surface) >= 3.0,
                                "{state:?}, focus={focus:?}, {color:?} on {surface:?}"
                            );
                        }
                        if fill == palette::TAB_SELECTED {
                            for surface in [palette::TAB_IDLE, palette::TAB_HOVER] {
                                assert!(contrast_ratio(color, surface) >= 3.0);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn focus_color_adaptation_reaches_three_to_one_without_unnecessary_blending() {
        let gray = |value| SceneColor {
            r: value,
            g: value,
            b: value,
        };
        // Any focus color, black and white included, reads at 3:1 on any surface: one that
        // already does is kept, any other blends no further than it needs, which the search over
        // increasing blends guarantees.
        let steps = (0..=255).step_by(17).map(|value| value as u8);
        for r in steps.clone() {
            for g in steps.clone() {
                for b in steps.clone() {
                    let focus = SceneColor { r, g, b };
                    for surface in [
                        palette::BAR_BACKGROUND,
                        palette::CHROME_HIGHLIGHT,
                        palette::DIALOG_BACKGROUND,
                        gray(25),
                        gray(119),
                        gray(255),
                    ] {
                        let color = blend_to_contrast(focus, surface, 3.0);
                        let ratio = contrast_ratio(color, surface);
                        assert!(ratio >= 3.0, "{focus:?} on {surface:?}");
                        if contrast_ratio(focus, surface) >= 3.0 {
                            assert_eq!(color, focus);
                        } else {
                            assert!(ratio < 3.1, "{focus:?} on {surface:?} at {ratio}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn working_badges_turn_only_on_hardware_adapters_with_animation_on() {
        use wgpu::DeviceType;
        for device in [
            DeviceType::Other,
            DeviceType::IntegratedGpu,
            DeviceType::DiscreteGpu,
            DeviceType::VirtualGpu,
            DeviceType::Cpu,
        ] {
            assert_eq!(spinners_turn(device, true), device != DeviceType::Cpu);
            // `--animate-working-badge false` keeps every adapter's badge still.
            assert!(!spinners_turn(device, false));
        }
    }

    #[test]
    fn quota_icons_preserve_transparency_and_shape_across_scales() {
        for (id, original) in [
            (OPENAI_BLOSSOM_ID, OPENAI_BLOSSOM_ALPHA),
            (CLAUDE_SPLAT_ID, CLAUDE_SPLAT_ALPHA),
        ] {
            for size in [16, 20, 32, 104] {
                let image = rasterize_bar_icon(
                    RasterizeCustomGlyphRequest {
                        id,
                        width: size,
                        height: size,
                        x_bin: SubpixelBin::Zero,
                        y_bin: SubpixelBin::Zero,
                        scale: f32::from(size) / 16.0,
                    },
                    None,
                )
                .unwrap();
                assert_eq!(image.content_type, ContentType::Mask);
                assert_eq!(image.data.len(), usize::from(size * size));
                assert_eq!(image.data[0], 0, "transparent corner remains clear");
                assert!(
                    image.data.iter().any(|alpha| *alpha > 200),
                    "the mark remains visible"
                );
                if usize::from(size) == QUOTA_ICON_SIZE {
                    assert_eq!(image.data.as_slice(), original);
                }
            }
        }
        assert!(
            rasterize_bar_icon(
                RasterizeCustomGlyphRequest {
                    id: EON_ICON_ID + 1,
                    width: 16,
                    height: 16,
                    x_bin: SubpixelBin::Zero,
                    y_bin: SubpixelBin::Zero,
                    scale: 1.0,
                },
                Some(&[255; 256]),
            )
            .is_none()
        );
    }

    #[test]
    fn eon_icon_loads_only_bounded_square_rgba_images() {
        let root = std::env::temp_dir().join(format!("venus-eon-icon-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let image = |name: &str, length: usize| {
            let path = root.join(name);
            std::fs::write(&path, vec![7; length]).unwrap();
            path
        };
        for side in [16, 256, 512] {
            let icon = read_eon_icon(&image("square", side * side * 4)).unwrap();
            assert_eq!(icon.len(), side * side * 4);
            assert!(icon.iter().all(|byte| *byte == 7));
        }
        for path in [
            image("small", 15 * 15 * 4),
            image("large", 513 * 513 * 4),
            image("oblong", 256 * 255 * 4),
            image("ragged", 256 * 256 * 4 - 1),
            image("empty", 0),
            root.join("missing"),
            root.clone(),
        ] {
            let error = read_eon_icon(&path).unwrap_err();
            assert!(error.contains(path.to_str().unwrap()), "{error}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn eon_icon_rasterizes_in_its_own_colours_at_the_requested_pixels() {
        // A 16-pixel image, clear but for its lower right quarter: solid orange, and orange at a
        // fifth of full coverage in its last row, both premultiplied.
        let image = (0..256)
            .flat_map(|index| match (index / 16, index % 16) {
                (15, 8..) => [40, 20, 10, 51],
                (8.., 8..) => [200, 100, 50, 255],
                _ => [0; 4],
            })
            .collect::<Vec<u8>>();
        let request = |width, height| RasterizeCustomGlyphRequest {
            id: EON_ICON_ID,
            width,
            height,
            x_bin: SubpixelBin::Zero,
            y_bin: SubpixelBin::Zero,
            scale: 1.0,
        };
        for side in [1, 8, 11, 16, 22, 40] {
            let glyph = rasterize_bar_icon(request(side, side), Some(&image)).unwrap();
            assert_eq!(glyph.content_type, ContentType::Color);
            assert_eq!(glyph.data.len(), usize::from(side) * usize::from(side) * 4);
            // Glyphon blends straight alpha, so every coloured pixel keeps the colour whatever
            // its coverage.
            for pixel in glyph.data.chunks(4).filter(|pixel| pixel[3] > 0) {
                assert!(pixel[0].abs_diff(200) <= 2, "{pixel:?}");
                assert!(pixel[1].abs_diff(100) <= 2, "{pixel:?}");
                assert!(pixel[2].abs_diff(50) <= 2, "{pixel:?}");
            }
            if side >= 8 {
                assert_eq!(glyph.data[..4], [0; 4], "the clear corner stays clear");
            }
            if (8..=16).contains(&side) {
                let row = usize::from(side) * 4;
                let above_last = glyph.data.len() - row - 4;
                assert_eq!(glyph.data[above_last..above_last + 4], [200, 100, 50, 255]);
            }
            if side == 16 {
                assert_eq!(glyph.data[glyph.data.len() - 4..], [200, 100, 50, 51]);
            }
        }
        assert!(rasterize_bar_icon(request(11, 11), None).is_none());
        // The provider marks keep their ids beside the Eon icon.
        for id in [OPENAI_BLOSSOM_ID, CLAUDE_SPLAT_ID] {
            let quota = |eon_icon| {
                let glyph = rasterize_bar_icon(
                    RasterizeCustomGlyphRequest {
                        id,
                        ..request(16, 16)
                    },
                    eon_icon,
                )
                .unwrap();
                assert_eq!(glyph.content_type, ContentType::Mask);
                glyph.data
            };
            assert_eq!(quota(Some(&image)), quota(None));
        }
    }

    fn test_sans() -> FontSystem {
        let font = std::env::var_os("EON_VENUS_TEST_SANS_FONT")
            .expect("run in nix develop with the exact packaged sans fixture");
        let mut db = fontdb::Database::new();
        db.load_font_file(std::path::Path::new(&font)).unwrap();
        db.set_sans_serif_family("DejaVu Sans");
        db.set_monospace_family("DejaVu Sans");
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    }

    #[test]
    fn readings_keep_bundled_symbol_attrs_and_interface_percentage_widths() {
        // No host font discovery: the explicit database contains one known sans file.
        // Symbols need their named attributes even when that face is absent from a test DB.
        let mut fonts = test_sans();
        for size in [6.0, 16.0, 96.0] {
            for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
                let metrics = CellMetrics {
                    font_size: size * scale,
                    height: size * 1.125 * scale,
                    ..CellMetrics::for_scale(f64::from(scale))
                };
                let cell_font = fitted_cell_font(&mut fonts, metrics, None, false);
                for icon in [
                    crate::system::CPU_ICON,
                    crate::system::RAM_ICON,
                    crate::system::DISK_ICON,
                ] {
                    let buffer = shape_styled_text(
                        &mut fonts,
                        metrics,
                        cell_font,
                        &icon.to_string(),
                        1000.0,
                        metrics.height,
                        DrawStyleKind::Monospace,
                    );
                    assert_eq!(
                        buffer.lines[0].attrs_list().get_span(0).family,
                        Family::Name(NERD_FONT_FAMILY)
                    );
                }
                let widest = measure_reading_width(&mut fonts, metrics);
                let mut measured_max: f32 = 0.0;
                for percent in 0..=100 {
                    let value = format!("{percent}%");
                    let buffer = shape_styled_text(
                        &mut fonts,
                        metrics,
                        cell_font,
                        &value,
                        2000.0,
                        metrics.height,
                        DrawStyleKind::Status(Wrap::None),
                    );
                    assert_eq!(
                        buffer.lines[0].attrs_list().get_span(0).family,
                        Family::SansSerif
                    );
                    let run = buffer.layout_runs().next().unwrap();
                    assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id != 0));
                    assert!(run.line_w <= widest, "{value} at {size}, {scale}");
                    measured_max = measured_max.max(run.line_w);
                }
                assert_eq!(measured_max, widest);
            }
        }
    }

    #[test]
    fn zoom_marker_uses_the_shared_restore_glyph_and_fits_without_symbol_fonts() {
        let mut fonts = test_sans();
        for scale in [1.0, 1.3, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let plain = "1  eon";
            let label = format!("{plain} {}", tokens::RESTORE_GLYPH);
            let (fitted, width) = fit_header_text(&mut fonts, metrics, &label, f32::INFINITY);
            assert_eq!(fitted, label);
            let plain_width = fit_header_text(&mut fonts, metrics, plain, f32::INFINITY).1;
            assert!(width > plain_width, "the marker consumes tab width");
            let narrow = fit_header_text(&mut fonts, metrics, &label, plain_width);
            assert!(narrow.1 <= plain_width);
            assert_ne!(narrow.0, fitted);
            let title = chrome_title_metrics(metrics);
            let mut buffer =
                Buffer::new(&mut fonts, Metrics::new(title.font_size, title.line_height));
            buffer.set_wrap(Wrap::None);
            buffer.set_text(
                &fitted,
                &Attrs::new().family(Family::SansSerif),
                shaping(&fitted),
                None,
            );
            buffer.shape_until_scroll(&mut fonts, false);
            let runs: Vec<_> = buffer.layout_runs().collect();
            assert_eq!(runs.len(), 1);
            assert!((runs[0].line_w - width).abs() < 0.01);
            assert!(runs[0].glyphs.iter().all(|glyph| glyph.glyph_id != 0));
        }
    }

    #[test]
    fn tab_text_fits_shaped_width_without_splitting_combining_clusters() {
        let mut fonts = test_sans();
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let available = (crate::scene::tab_max_width(metrics, 1000.0 * scale as f32)
                - metrics.padding * 2.0)
                .floor();
            for label in ["1  eon", "2  machine_vs_aliens"] {
                let fitted = fit_header_text(&mut fonts, metrics, label, available);
                assert_eq!(fitted.0, label);
                assert!(fitted.1 > 0.0 && fitted.1 <= available);
            }
            let label = format!("3  {}", "e\u{301}".repeat(80));
            let (fitted, measured) = fit_header_text(&mut fonts, metrics, &label, available);
            assert!(measured <= available);
            let (head, tail) = fitted.split_once('…').expect("long label must elide");
            assert!(head.starts_with("3  ") && head.ends_with('\u{301}'));
            assert!(tail.starts_with('e') && tail.ends_with('\u{301}'));
            assert!(label.starts_with(head) && label.ends_with(tail));
            let exact = fit_header_text(&mut fonts, metrics, &fitted, f32::INFINITY);
            assert!((exact.1 - measured).abs() < 0.01);

            let identity_width = fit_header_text(&mut fonts, metrics, "64", f32::INFINITY).1;
            let fitted =
                fit_header_text(&mut fonts, metrics, "64  machine_vs_aliens", identity_width);
            assert_eq!(
                fitted.0, "64",
                "keep a fitting identity when the name cannot fit"
            );
            assert!(fitted.1 <= identity_width);
            assert_eq!(
                fit_header_text(&mut fonts, metrics, "64  eon", 0.0),
                (String::new(), 0.0)
            );
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
    fn long_workspace_labels_stay_on_the_visible_line() {
        use crate::EonBar;
        use eon_workspace_protocol::v41::{Pane, Snapshot, Tab};
        use winit::{
            application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop,
            platform::wayland::EventLoopBuilderExtWayland, window::WindowId,
        };

        fn assert_reading_alignment(renderer: &mut Renderer) {
            let snapshot = popup_sidebar_snapshot();
            for scale in [1.0, 1.25, 2.0] {
                let size = PhysicalSize::new((1600.0 * scale) as u32, (900.0 * scale) as u32);
                renderer.resize(size, scale);
                let metrics = renderer.metrics();
                let mut previous = None;
                for percent in [5, 50, 100] {
                    let cpu = crate::CpuUse {
                        percent,
                        label: format!("{} {percent}%", crate::system::CPU_ICON),
                        tooltip: "CPU".into(),
                        description: "CPU".into(),
                        load: Load::Normal,
                    };
                    let ram = crate::RamUse::parse(&format!(
                        "MemTotal: 100000 kB\nMemAvailable: {} kB\n",
                        (100 - percent) * 1000
                    ))
                    .unwrap();
                    let disk =
                        crate::DiskUse::of(100, 100 - percent, 100 - percent, 4096, "/").unwrap();
                    let workspace = WorkspaceScene::from_snapshot_with_bar(
                        &snapshot,
                        EonBar {
                            cpu: Some(&cpu),
                            ram: Some(&ram),
                            disk: Some(&disk),
                            ..EonBar::default()
                        },
                        size,
                        metrics,
                        0.0,
                        0.0,
                        |kind, text| match kind {
                            crate::HeaderText::Reading => {
                                (text.to_owned(), renderer.reading_width())
                            }
                            _ => renderer.fit_tab_text(text),
                        },
                        None,
                    );
                    renderer.text.clear();
                    let title_tabs = renderer.workspace_title_tabs(&workspace);
                    let pane_tabs = renderer.workspace_pane_tabs(&workspace);
                    renderer.build_workspace(
                        &workspace,
                        WorkspaceFocus::Terminal,
                        &mut RectangleBatch::new(size.width, size.height),
                        &[],
                        &title_tabs,
                        &pane_tabs,
                    );
                    let positions = [&workspace.cpu, &workspace.ram, &workspace.disk].map(|chip| {
                        let chip = chip.as_ref().expect("room for all three reading chips");
                        let icon = chip.label.chars().next().unwrap().to_string();
                        let index = renderer
                            .text
                            .iter()
                            .position(|text| {
                                text.buffer
                                    .lines
                                    .first()
                                    .is_some_and(|line| line.text() == icon)
                            })
                            .expect("painted reading icon");
                        let icon = &renderer.text[index];
                        let value = &renderer.text[index + 1];
                        let line = value.buffer.layout_runs().next().unwrap();
                        assert_eq!(line.text, format!("{percent}%"));
                        let right = value.left + line.line_w;
                        let padded_right = chip.rect.right() - metrics.padding;
                        let value_left = icon.left + metrics.font_size + tokens::icon_gap(metrics);
                        assert!(
                            (right - padded_right).abs() < 0.01,
                            "{percent}% at scale {scale}: right {right}, expected {padded_right}"
                        );
                        assert_eq!(icon.left, chip.rect.left + metrics.padding);
                        assert!(
                            value.left + 0.01 >= value_left,
                            "value overlaps the icon gap"
                        );
                        assert_eq!(value.bound_left, (value_left - 0.5).ceil() as i32);
                        if percent == 100 {
                            // In packaged DejaVu Sans, 100% fills the reserved percentage slot.
                            assert!((value.left - value_left).abs() < 0.01);
                        }
                        (chip.rect, icon.left, padded_right)
                    });
                    if let Some(previous) = previous {
                        assert_eq!(positions, previous, "reading changes must not move chips");
                    }
                    previous = Some(positions);
                }
            }
            renderer.resize(PhysicalSize::new(1600, 900), 1.0);
        }

        fn assert_chrome_text(renderer: &mut Renderer) {
            fn color(color: SceneColor) -> Color {
                Color::rgb(color.r, color.g, color.b)
            }
            fn source(text: &PlacedText) -> &str {
                text.buffer.lines.first().map_or("", |line| line.text())
            }
            fn identity(text: &PlacedText, id: &str, number: SceneColor, name: SceneColor) {
                let run = text.buffer.layout_runs().next().unwrap();
                assert!(run.text.starts_with(id));
                assert!(run.glyphs.iter().any(|glyph| glyph.start < id.len()));
                assert!(run.glyphs.iter().any(|glyph| glyph.start > id.len()));
                for glyph in run.glyphs {
                    assert_eq!(
                        glyph.color_opt.unwrap_or(text.color),
                        color(if glyph.start < id.len() { number } else { name }),
                        "{} at {}",
                        run.text,
                        glyph.start
                    );
                }
            }
            fn grey_text(texts: &[PlacedText]) {
                for text in texts.iter().filter(|text| text.custom_glyph.is_none()) {
                    for ink in std::iter::once(text.color).chain(
                        text.buffer
                            .layout_runs()
                            .flat_map(|run| run.glyphs.iter().filter_map(|glyph| glyph.color_opt)),
                    ) {
                        assert_ne!(ink, color(palette::ACCENT), "mint text: {}", source(text));
                        assert_ne!(
                            ink,
                            Color::rgb(162, 204, 230),
                            "blue text: {}",
                            source(text)
                        );
                    }
                }
            }
            let size = PhysicalSize::new(1600, 900);
            renderer.resize(size, 1.0);
            renderer.header = HeaderInteraction::default();
            renderer.set_scrollback_label(None);
            let metrics = renderer.metrics();
            let mut snapshot = popup_sidebar_snapshot();
            snapshot.tabs[0].directory = b"/tmp/first".to_vec();
            let mut second = snapshot.tabs[0].clone();
            second.id = "t2".into();
            second.directory = b"/tmp/second".to_vec();
            second.selected_pane = Some("p4".into());
            second.columns[0].expanded = "p4".into();
            for (index, pane) in second.columns[0].panes.iter_mut().enumerate() {
                pane.id = format!("p{}", index + 3);
            }
            second.popups[0].id = "u2".into();
            snapshot.tabs.push(second);
            snapshot.workspaces[0].tab_count = 2;
            for tab in &mut snapshot.tabs {
                for pane in &mut tab.columns[0].panes {
                    pane.label = format!("{}  pane", pane.id);
                }
            }
            let mut hint_definitions = default_popup_hints();
            hint_definitions.push(crate::ShortcutHint {
                modifiers: String::new(),
                modifier_names: String::new(),
                key: "F6".into(),
                label: "focus".into(),
                short: "Focus chrome".into(),
                description: "Focus chrome".into(),
                section: HintSection::Actions,
                accessible_label: "F6 — Focus chrome".into(),
                priority: 0,
            });
            let hints =
                ShortcutHintsScene::new(hint_definitions, size, metrics, None, None, |text| {
                    renderer.measure_header_text(text).1
                });
            let workspace = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| renderer.fit_tab_text(text),
                Some(hints),
            );
            for background in [
                SceneColor { r: 4, g: 5, b: 7 },
                SceneColor {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ] {
                renderer.text.clear();
                let title_tabs = renderer.workspace_title_tabs(&workspace);
                let pane_tabs = renderer.workspace_pane_tabs(&workspace);
                renderer.build_workspace(
                    &workspace,
                    WorkspaceFocus::Terminal,
                    &mut RectangleBatch::new(1600, 900),
                    &[background, background],
                    &title_tabs,
                    &pane_tabs,
                );
                for (index, tab) in workspace.tabs.iter().enumerate() {
                    let text = renderer
                        .text
                        .iter()
                        .find(|text| {
                            source(text) == tab.label() && text.top < workspace.header.bottom()
                        })
                        .expect("painted tab label");
                    identity(
                        text,
                        &(index + 1).to_string(),
                        if tab.selected {
                            palette::PRIMARY_TEXT
                        } else {
                            palette::PANE_ID
                        },
                        if tab.selected {
                            palette::PRIMARY_TEXT
                        } else {
                            palette::TAB_TEXT
                        },
                    );
                }
                for (index, heading) in workspace.sidebar_headings.iter().enumerate() {
                    let text = renderer
                        .text
                        .iter()
                        .find(|text| {
                            source(text) == heading.label && text.top >= workspace.header.bottom()
                        })
                        .expect("painted sidebar heading");
                    identity(
                        text,
                        &(index + 1).to_string(),
                        blend_to_contrast(palette::PANE_ID, background, 4.5),
                        blend_to_contrast(palette::PRIMARY_TEXT, background, 4.5),
                    );
                }
                let hints = workspace.hints.as_ref().unwrap();
                assert!(!hints.pieces.is_empty());
                for hint in &hints.pieces {
                    for (label, expected) in [
                        (&hint.key, palette::TAB_TEXT),
                        (&hint.label, palette::SECONDARY_TEXT),
                    ] {
                        let text = renderer
                            .text
                            .iter()
                            .find(|text| {
                                source(text) == label.as_str() && text.top >= hints.rect.top
                            })
                            .expect("painted hint key or label");
                        assert_eq!(text.color, color(expected), "bar hint {label}");
                    }
                }
                for group in &hints.groups {
                    let text = renderer
                        .text
                        .iter()
                        .find(|text| source(text) == group.legend)
                        .expect("painted keycap legend");
                    assert_eq!(text.color, color(palette::HINT_MODIFIER_TEXT));
                }
                grey_text(&renderer.text);
            }
            let hints = workspace.hints.as_ref().unwrap();
            let group = &hints.groups[1];
            let first = renderer.text.len();
            renderer.build_hint_popup(
                &workspace,
                hints.rect.top - tokens::tooltip_gap(metrics),
                1,
                group,
                &mut RectangleBatch::new(1600, 900),
            );
            let painted = &renderer.text[first..];
            for section in HintSection::ALL {
                assert!(painted.iter().any(|text| source(text) == section.title()
                    && text.color == color(palette::HINT_POPUP_TEXT)));
            }
            for row in &group.rows {
                assert!(
                    painted.iter().any(|text| source(text) == row.key
                        && text.color == color(palette::HINT_POPUP_TEXT))
                );
            }
            grey_text(painted);
            let dialog = DialogScene::new(
                crate::DialogKind::Shortcuts,
                vec![crate::ShortcutGroup::new(
                    "Chrome group",
                    (0..200)
                        .map(|_| crate::ShortcutRow::new("Ctrl+Q", "Chrome action"))
                        .collect(),
                )],
                size,
                metrics,
                0.0,
                &crate::CellText(metrics),
            );
            assert!(dialog.max_scroll > 0.0);
            renderer.text.clear();
            let mut batch = RectangleBatch::new(1600, 900);
            renderer.build_dialog(&dialog, &mut batch);
            for (label, expected) in [
                ("Chrome group", palette::SECONDARY_TEXT),
                ("Ctrl+Q", palette::SHORTCUT_KEY_TEXT),
                ("Chrome action", palette::SHORTCUT_ACTION_TEXT),
            ] {
                assert!(
                    renderer
                        .text
                        .iter()
                        .any(|text| source(text) == label && text.color == color(expected)),
                    "painted dialog {label}"
                );
            }
            grey_text(&renderer.text);
            let mut neutral_thumb = false;
            for vertex in batch.bytes.chunks_exact(VERTEX_SIZE as usize) {
                let rgb = [2, 3, 4].map(|i| {
                    (f32::from_ne_bytes(vertex[i * 4..i * 4 + 4].try_into().unwrap()) * 255.0)
                        .round() as u8
                });
                assert_ne!(
                    rgb,
                    [palette::ACCENT.r, palette::ACCENT.g, palette::ACCENT.b],
                    "dialog thumb is not focus"
                );
                neutral_thumb |= rgb
                    == [
                        palette::DIALOG_SCROLL_THUMB.r,
                        palette::DIALOG_SCROLL_THUMB.g,
                        palette::DIALOG_SCROLL_THUMB.b,
                    ];
            }
            assert!(neutral_thumb, "painted neutral scroll thumb");
        }

        struct Probe(bool);
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let window = Arc::new(
                    event_loop
                        .create_window(Window::default_attributes())
                        .unwrap(),
                );
                let mut renderer = pollster::block_on(Renderer::new(
                    window,
                    event_loop,
                    1.0,
                    crate::PaddingColor::Extend,
                    None,
                    true,
                    crate::PanelHeading::Notch,
                    true,
                    SceneColor {
                        r: 178,
                        g: 7,
                        b: 251,
                    },
                    1,
                    2,
                    true,
                    None,
                    FontSetup::resolve(&FontSettings::default(), test_sans()).unwrap(),
                ))
                .unwrap();
                let snapshot = Snapshot {
                    workspace: "s1".into(),
                    active_workspace: "s1".into(),
                    workspaces: vec![eon_workspace_protocol::v41::WorkspaceInfo {
                        id: "s1".into(),
                        persistent_id: Some("s1".into()),
                        registration_error: None,
                        name: None,
                        tab_count: 1,
                        attention: None,
                        recency: 0,
                    }],
                    active_tab: "t2".into(),
                    geometry: eon_workspace_protocol::v41::PopupGeometry {
                        side_margin: 8.0,
                        vertical_margin: 4.0,
                    },
                    entries: Vec::new(),
                    tabs: ["eon", "machines_vs_aliens"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, directory)| Tab {
                            pending: false,
                            selected_popup: None,
                            popups: Vec::new(),
                            id: format!("t{}", index + 1),
                            directory: format!("/tmp/{directory}").into_bytes(),
                            selected_pane: Some(format!("p{}", index + 1)),
                            columns: vec![eon_workspace_protocol::v41::Column {
                                panes: vec![Pane {
                                    label: format!("p{}", index + 1),
                                    pinned_name: None,
                                    id: format!("p{}", index + 1),
                                    session: format!("terminal-{}", index + 1),
                                    endpoint: format!("/tmp/terminal-{}.sock", index + 1)
                                        .into_bytes(),
                                    live: true,
                                    attention: None,
                                }],
                                expanded: format!("p{}", index + 1),
                            }],
                            attention_rollup: None,
                        })
                        .collect(),
                    codex_quota: None,
                    claude_quota: None,
                    codex_quota_failure: None,
                    claude_quota_failure: None,
                    running: None,
                    busy: None,
                    sidebar: eon_workspace_protocol::v41::Sidebar::Open,
                    attention_queue: Vec::new(),
                };
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                let title_tabs = renderer.workspace_title_tabs(&workspace);
                let pane_tabs = renderer.workspace_pane_tabs(&workspace);
                renderer.build_workspace(
                    &workspace,
                    WorkspaceFocus::Terminal,
                    &mut RectangleBatch::new(900, 600),
                    &[],
                    &title_tabs,
                    &pane_tabs,
                );
                let label = &renderer.text[1];
                let line = label.buffer.layout_runs().next().unwrap();
                assert_eq!(
                    line.glyphs.last().unwrap().end,
                    workspace.tabs[1].label().len(),
                    "the directory name wrapped below the visible tab header"
                );
                assert!(workspace.tabs[1].rect.width > workspace.tabs[0].rect.width);
                assert!(line.line_w <= (label.right - label.bound_left) as f32);
                let underscore = line
                    .glyphs
                    .iter()
                    .find(|glyph| &line.text[glyph.start..glyph.end] == "_")
                    .unwrap()
                    .physical((label.left, label.top), 1.0);
                let ink = renderer
                    .swash_cache
                    .get_image(&mut renderer.fonts.font_system, underscore.cache_key)
                    .as_ref()
                    .unwrap();
                let top = line.line_y.round() as i32 + underscore.y - ink.placement.top;
                assert!(ink.placement.height > 0);
                assert!(
                    top >= label.bound_top && top + ink.placement.height as i32 <= label.bottom,
                    "the directory underscore is clipped outside the header"
                );
                renderer.set_hovered_header(Some((WorkspaceFocus::Tabs, "t2".into())));
                renderer.build_header_tooltip(&workspace, &mut RectangleBatch::new(900, 600));
                let (first, masks) = renderer.text_overlay.as_ref().unwrap();
                let first = *first;
                let overlay = masks[0];
                let areas: Vec<_> =
                    text_areas(&renderer.text[..first], renderer.text_overlay.as_ref()).collect();
                assert!(
                    areas.iter().all(|area| {
                        let b = area.bounds;
                        b.right <= overlay.left
                            || b.left >= overlay.right
                            || b.bottom <= overlay.top
                            || b.top >= overlay.bottom
                    }),
                    "underlying text must not paint over the hover path"
                );
                assert_eq!(
                    renderer.text[first].buffer.lines[0].text(),
                    "Tab 2 of 2  /tmp/machines_vs_aliens"
                );
                let mut snapshot = snapshot;
                snapshot.tabs[1].columns[0].panes.push(Pane {
                    label: "p3  offline".into(),
                    pinned_name: None,
                    id: "p3".into(),
                    session: "s3".into(),
                    endpoint: b"/tmp/p3.sock".to_vec(),
                    live: false,
                    attention: None,
                });
                snapshot.tabs[1].columns[0].panes[0].live = false;
                snapshot.tabs[1].columns[0].panes[0].label = "p2  offline".into();
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                renderer.pane_frames = false;
                renderer.set_hovered_header(None);
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    1,
                );
                let pane_color = |label| {
                    renderer
                        .text
                        .iter()
                        .find(|text| text.buffer.lines[0].text() == label)
                        .unwrap()
                        .color
                };
                assert_ne!(
                    pane_color("p2  offline"),
                    pane_color("p3  offline"),
                    "offline pane selection disappears with decorative frames off"
                );
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    None,
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    &"Sessions unavailable. ".repeat(32),
                    false,
                    "",
                    1,
                );
                assert!(renderer.text[1].buffer.layout_runs().count() > 1);
                snapshot.tabs[1].columns[0].panes[0].live = true;
                for pane in &mut snapshot.tabs[1].columns[0].panes {
                    pane.label = format!("{}  /{}mountain/gyp_leaf", pane.id, "W".repeat(42));
                }
                let workspace = WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    EonBar::default(),
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                    None,
                );
                // Exercise the real shaped title and rectangle path in this existing native probe.
                for frames in [true, false] {
                    renderer.pane_frames = frames;
                    renderer.text.clear();
                    renderer.set_hovered_header(Some((WorkspaceFocus::Panes, "p3".into())));
                    let mut batch = RectangleBatch::new(900, 600);
                    let title_tabs = renderer.workspace_title_tabs(&workspace);
                    let pane_tabs = renderer.workspace_pane_tabs(&workspace);
                    renderer.build_workspace(
                        &workspace,
                        WorkspaceFocus::Panes,
                        &mut batch,
                        &[],
                        &title_tabs,
                        &pane_tabs,
                    );
                    let pane = &workspace.panes[0];
                    let label = renderer
                        .text
                        .iter()
                        .find(|text| text.buffer.lines[0].text().starts_with("p2  "))
                        .unwrap();
                    let line = label.buffer.layout_runs().next().unwrap();
                    for glyph in line.glyphs {
                        let physical = glyph.physical((label.left, label.top), 1.0);
                        let Some(ink) = renderer
                            .swash_cache
                            .get_image(&mut renderer.fonts.font_system, physical.cache_key)
                            .as_ref()
                        else {
                            continue;
                        };
                        let top = line.line_y.round() as i32 + physical.y - ink.placement.top;
                        assert!(
                            top >= label.bound_top
                                && top + ink.placement.height as i32 <= label.bottom,
                            "pane title descender is clipped: {}",
                            &line.text[glyph.start..glyph.end]
                        );
                    }
                    let colors_at = |x: f32, y: f32| {
                        batch
                            .bytes
                            .chunks_exact(144)
                            .filter_map(|quad| {
                                let value = |vertex: usize, component: usize| {
                                    let offset = vertex * 24 + component * 4;
                                    f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                                };
                                let rect = SceneRect {
                                    left: (value(0, 0) + 1.0) * 450.0,
                                    top: (1.0 - value(0, 1)) * 300.0,
                                    width: (value(2, 0) - value(0, 0)) * 450.0,
                                    height: (value(0, 1) - value(2, 1)) * 300.0,
                                };
                                rect.contains(x, y)
                                    .then(|| [2, 3, 4].map(|c| (value(0, c) * 255.0).round() as u8))
                            })
                            .collect::<Vec<_>>()
                    };
                    let rule_y = (pane.rect.top + pane.rect.height / 2.0).floor() + 0.5;
                    let band_y = pane_tabs
                        .first()
                        .and_then(|tabs| tabs.first())
                        .and_then(Option::as_ref)
                        .map_or(rule_y, |tab| tab.tab.top + 0.5);
                    let rule = colors_at(pane.rect.right() - 20.0, band_y);
                    assert_eq!(
                        rule,
                        if frames { vec![[178, 7, 251]] } else { vec![] },
                        "first top rule takes the focus colour; frameless has none"
                    );
                    assert!(
                        colors_at(label.left + line.line_w / 2.0, rule_y).is_empty(),
                        "rule enters the label gap"
                    );
                    assert!(
                        colors_at(pane.rect.right() - 20.0, pane.rect.top + 0.5).is_empty(),
                        "extra top stroke"
                    );
                    assert_eq!(
                        colors_at(
                            label.left + line.line_w / 2.0,
                            if frames { band_y } else { pane.rect.top + 1.5 }
                        ),
                        vec![[126, 231, 185]],
                        "focus stays local to the label"
                    );
                    let hovered = &workspace.panes[1];
                    assert_eq!(
                        colors_at(
                            hovered.rect.left + renderer.metrics.padding + 8.0,
                            hovered.rect.top + hovered.rect.height / 2.0
                        ),
                        vec![[40, 54, 70]],
                        "hover pill remains visible in both modes"
                    );
                }
                // Keyboard selection can move while the pointer rests on the old Close button.
                let mut changed = snapshot.clone();
                changed.tabs[1].selected_pane = Some("p3".into());
                changed.tabs[1].columns[0].expanded = "p3".into();
                let changed = WorkspaceScene::from_snapshot(
                    &changed,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                for frames in [true, false] {
                    renderer.pane_frames = frames;
                    for (scene, visible) in [(&workspace, true), (&changed, false)] {
                        renderer.text.clear();
                        renderer.text_overlay = None;
                        renderer.set_hovered_header(Some((
                            WorkspaceFocus::PaneAction(crate::PaneHeaderAction::Close),
                            "p2".into(),
                        )));
                        let title_tabs = renderer.workspace_title_tabs(scene);
                        let pane_tabs = renderer.workspace_pane_tabs(scene);
                        let mut hovered = RectangleBatch::new(900, 600);
                        renderer.build_workspace(
                            scene,
                            WorkspaceFocus::Terminal,
                            &mut hovered,
                            &[],
                            &title_tabs,
                            &pane_tabs,
                        );
                        assert_eq!(
                            renderer
                                .text
                                .iter()
                                .filter(|text| {
                                    text.top >= scene.header_viewport.top
                                        && matches!(text.buffer.lines[0].text(), "□" | "×")
                                })
                                .count(),
                            2,
                            "only the selected pane emits button glyphs"
                        );
                        renderer.build_header_tooltip(scene, &mut hovered);
                        assert_eq!(renderer.text_overlay.is_some(), visible);
                        if !visible {
                            renderer.text.clear();
                            renderer.set_hovered_header(None);
                            let mut resting = RectangleBatch::new(900, 600);
                            renderer.build_workspace(
                                scene,
                                WorkspaceFocus::Terminal,
                                &mut resting,
                                &[],
                                &title_tabs,
                                &pane_tabs,
                            );
                            assert_eq!(
                                hovered.bytes, resting.bytes,
                                "a hidden button retains neither hover fill nor tooltip"
                            );
                        }
                    }
                }
                renderer.pane_frames = false;
                renderer.set_hovered_header(None);
                renderer.set_scrollback_label(Some("↓ 240 rows".into()));
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    2,
                );
                let indicator = renderer
                    .text
                    .iter()
                    .find(|text| text.buffer.lines[0].text() == "↓ 240 rows")
                    .unwrap();
                let pill = renderer.terminal.scrollback_rect.unwrap();
                assert!(pill.contains(pill.left + 1.0, pill.top + 1.0));
                let pane = workspace.panes.iter().find(|pane| pane.selected).unwrap();
                assert!(
                    fit_header_text(
                        &mut renderer.fonts.font_system,
                        renderer.metrics,
                        pane.label(),
                        f32::INFINITY
                    )
                    .1 <= pane.rect.width - renderer.metrics.padding * 2.0,
                    "the label fits before reserving the scrollback count"
                );
                assert!(indicator.top >= pane.rect.top);
                assert!(indicator.bottom as f32 <= pane.rect.bottom());
                let pane_text = renderer
                    .text
                    .iter()
                    .find(|text| text.buffer.lines[0].text().starts_with("p2  "))
                    .unwrap();
                assert!(pane_text.right <= indicator.bound_left);
                let line = pane_text.buffer.layout_runs().next().unwrap();
                assert!(line.text.ends_with("/gyp_leaf"));
                assert!(
                    line.line_w <= (pane_text.right - pane_text.bound_left) as f32,
                    "the scrollback count clips the directory leaf"
                );
                renderer.set_scrollback_label(None);
                assert!(renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    &[],
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    2
                ));
                assert!(
                    renderer
                        .text
                        .iter()
                        .all(|text| !text.buffer.lines[0].text().starts_with('↓'))
                );
                assert!(renderer.terminal.scrollback_rect.is_none());
                renderer.set_scrollback_label(Some("↓ 18446744073709551615 rows".into()));
                let viewport = SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: 900.0,
                    height: 600.0,
                };
                let mut rectangles = RectangleBatch::new(900, 600);
                assert!(
                    renderer.build_scrollback(viewport, viewport, false, None, &mut rectangles)
                        > 0.0
                );
                let (first, masks) = renderer.text_overlay.as_ref().unwrap();
                let first = *first;
                let overlay = masks[0];
                assert_eq!(
                    renderer.text[first].buffer.lines[0].text(),
                    "↓ 18446744073709551615 rows"
                );
                assert!(
                    text_areas(&renderer.text[..first], renderer.text_overlay.as_ref()).all(
                        |area| {
                            let b = area.bounds;
                            b.right <= overlay.left
                                || b.left >= overlay.right
                                || b.bottom <= overlay.top
                                || b.top >= overlay.bottom
                        }
                    )
                );
                assert_eq!(
                    renderer.build_scrollback(
                        viewport,
                        viewport,
                        false,
                        Some(viewport),
                        &mut rectangles
                    ),
                    0.0,
                    "the indicator must yield to an overlapping terminal cursor"
                );
                let narrow = SceneRect {
                    width: 40.0,
                    ..viewport
                };
                let count = renderer.text.len();
                assert_eq!(
                    renderer.build_scrollback(narrow, narrow, false, None, &mut rectangles),
                    0.0
                );
                assert_eq!(
                    renderer.text.len(),
                    count,
                    "never clip digits into a different number"
                );
                // Shared popup/sidebar titles use the pane metric and interrupt their top edge.
                snapshot
                    .entries
                    .push(eon_workspace_protocol::v41::PopupEntry {
                        id: "files".into(),
                        label: "Files".into(),
                        shortcut: eon_workspace_protocol::v41::PopupKey::Bound(
                            eon_workspace_protocol::v41::Shortcut {
                                modifiers: eon_workspace_protocol::v41::ALT,
                                key: "KeyF".into(),
                            },
                        ),
                    });
                snapshot.tabs[1]
                    .popups
                    .push(eon_workspace_protocol::v41::Popup {
                        id: "u1".into(),
                        entry: "files".into(),
                        session: "files-session".into(),
                        endpoint: b"/tmp/files.sock".to_vec(),
                        attention: None,
                    });
                snapshot.tabs[1].selected_popup = Some("u1".into());
                renderer.set_scrollback_label(None);
                renderer.set_hovered_header(None);
                renderer.pane_frames = true;
                for scale in [1.0, 1.3] {
                    renderer.resize(PhysicalSize::new(1600, 900), scale);
                    let workspace = WorkspaceScene::from_snapshot(
                        &snapshot,
                        PhysicalSize::new(1600, 900),
                        renderer.metrics(),
                        0.0,
                        0.0,
                        |_, label| renderer.fit_tab_text(label),
                    );
                    renderer.text.clear();
                    let mut batch = RectangleBatch::new(1600, 900);
                    let title_tabs = renderer.workspace_title_tabs(&workspace);
                    let pane_tabs = renderer.workspace_pane_tabs(&workspace);
                    renderer.build_workspace(
                        &workspace,
                        WorkspaceFocus::Terminal,
                        &mut batch,
                        &[],
                        &title_tabs,
                        &pane_tabs,
                    );
                    let title = chrome_title_metrics(renderer.metrics);
                    for (name, card) in [
                        ("Agents", workspace.sidebar().unwrap()),
                        (
                            "Files",
                            crate::scene::popup_card_rect(workspace.chrome, renderer.metrics),
                        ),
                    ] {
                        let text = renderer
                            .text
                            .iter()
                            .find(|text| text.buffer.lines[0].text() == name)
                            .unwrap();
                        assert_eq!(text.left, card.left + renderer.metrics.padding);
                        assert!(
                            (text.top + title.text_height / 2.0 - card.top).abs() < 1.0,
                            "title is not in its border"
                        );
                        assert_eq!(text.buffer.metrics().font_size, title.font_size);
                        assert_eq!(text.buffer.metrics().line_height, title.line_height);
                        let line = text.buffer.layout_runs().next().unwrap();
                        for glyph in line.glyphs {
                            let physical = glyph.physical((text.left, text.top), 1.0);
                            let Some(ink) = renderer
                                .swash_cache
                                .get_image(&mut renderer.fonts.font_system, physical.cache_key)
                                .as_ref()
                            else {
                                continue;
                            };
                            let top = line.line_y.round() as i32 + physical.y - ink.placement.top;
                            assert!(
                                top >= text.bound_top
                                    && top + ink.placement.height as i32 <= text.bottom,
                                "{name} descender clipped"
                            );
                        }
                        let x = text.left + line.line_w / 2.0;
                        assert!(
                            !batch.bytes.chunks_exact(144).any(|quad| {
                                let value = |vertex: usize, component: usize| {
                                    let offset = vertex * 24 + component * 4;
                                    f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                                };
                                SceneRect {
                                    left: (value(0, 0) + 1.0) * 800.0,
                                    top: (1.0 - value(0, 1)) * 450.0,
                                    width: (value(2, 0) - value(0, 0)) * 800.0,
                                    height: (value(0, 1) - value(2, 1)) * 450.0,
                                }
                                .contains(x, card.top + 0.5)
                            }),
                            "{name} border crosses its label"
                        );
                    }
                }
                self.0 = true;
                // Popup text must stay clear of its marker, including RTL and zero-width lines.
                use eon_workspace_protocol::v41::{
                    Attention, AttentionRecord, Popup, PopupEntry, PopupKey,
                };
                let attention = Attention {
                    state: AttentionState::NeedsInput,
                    record: Some(AttentionRecord {
                        kind: None,
                        progress: None,
                        app: "codex".into(),
                        name: String::new(),
                        title: "Long popup agent title ".repeat(20),
                        message: "Long question ".repeat(20),
                        changed_at: 1,
                        stale: true,
                        others: 2,
                    }),
                };
                snapshot.entries.push(PopupEntry {
                    id: "agent".into(),
                    label: "חלון ".repeat(40),
                    shortcut: PopupKey::Unbound,
                });
                snapshot.tabs[1].popups.push(Popup {
                    id: "u2".into(),
                    entry: "agent".into(),
                    session: "popup".into(),
                    endpoint: b"/tmp/popup.sock".to_vec(),
                    attention: Some(attention),
                });
                snapshot.tabs[1].attention_rollup = Some(AttentionState::NeedsInput);
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(1600, 900),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                let row = workspace.sidebar_row("u2").unwrap();
                let marker = row.popup_marker.unwrap();
                for width in [0.0, 1.0, row.text_right - row.text_left] {
                    let mut row = row.clone();
                    row.text_left = row.text_right - width;
                    renderer.text.clear();
                    renderer.push_attention_row(
                        &row,
                        workspace.sidebar_list,
                        false,
                        Backing::Translucent(palette::TERMINAL_FALLBACK),
                        palette::TERMINAL_FALLBACK,
                        &mut RectangleBatch::new(1600, 900),
                    );
                    let lines: Vec<_> = renderer
                        .text
                        .iter()
                        .filter(|text| text.custom_glyph.is_none())
                        .collect();
                    if width == 0.0 {
                        assert!(lines.is_empty());
                    } else {
                        assert_eq!(lines.len(), 3);
                        for text in lines {
                            assert!(text.right as f32 <= row.text_right.ceil());
                            assert!((text.right as f32) < marker.left);
                            assert_eq!(text.buffer.layout_runs().count(), 1);
                        }
                    }
                }
                assert_reading_alignment(&mut renderer);
                assert_chrome_text(&mut renderer);
                event_loop.exit();
            }

            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::builder()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let mut probe = Probe(false);
        event_loop.run_app(&mut probe).unwrap();
        assert!(probe.0);
    }

    /// Tab and workspace chip labels and bottom-bar hint text with descenders, shaped in Eon's
    /// packaged DejaVu Sans, whose ascent and descent (1.164 em) exceed a terminal line of 1 em
    /// and nearly fill the default 1.125 em: their ink lies inside their clip bounds, drawn where
    /// their terminal-height line always drew it. In a chip shorter than the face, the line lifts
    /// by whole pixels to keep the descenders inside.
    #[test]
    fn chip_label_descenders_stay_inside_their_clip_and_chip() {
        use eon_workspace_protocol::v41::{
            Column, Pane, PopupGeometry, Sidebar, Snapshot, Tab, WorkspaceInfo,
        };

        let font = std::env::var_os("EON_VENUS_TEST_SANS_FONT").expect(
            "EON_VENUS_TEST_SANS_FONT names Eon's packaged DejaVu Sans; run in nix develop",
        );
        let tab = |index: usize, directory: &str| Tab {
            id: format!("t{index}"),
            directory: directory.as_bytes().to_vec(),
            pending: false,
            selected_pane: Some(format!("p{index}")),
            selected_popup: None,
            columns: vec![Column {
                panes: vec![Pane {
                    label: format!("p{index}"),
                    pinned_name: None,
                    id: format!("p{index}"),
                    session: format!("s{index}"),
                    endpoint: format!("/tmp/s{index}.sock").into_bytes(),
                    live: true,
                    attention: None,
                }],
                expanded: format!("p{index}"),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        };
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: Some("gpy-jq".into()),
                tab_count: 2,
                attention: None,
                recency: 0,
            }],
            active_tab: "t2".into(),
            tabs: vec![tab(1, "/tmp/eon"), tab(2, "/tmp/rota-gtp")],
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
            sidebar: Sidebar::Closed,
            attention_queue: Vec::new(),
        };
        for (line_height, scale) in [(1.0, 1.0), (1.0, 2.0), (1.125, 1.0), (1.125, 2.0)] {
            let mut fonts = FontSetup::new(&FontSettings {
                line_height,
                ..FontSettings::default()
            })
            .unwrap();
            fonts
                .set_header_font_file(std::path::Path::new(&font))
                .unwrap();
            let metrics = fonts.metrics(scale);
            let case = format!("line height {line_height} at scale {scale}");
            let size = PhysicalSize::new((1280.0 * scale) as u32, (800.0 * scale) as u32);
            let width = WorkspaceScene::tab_text_width(size, metrics);
            let workspace =
                WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, label| {
                    fit_header_text(&mut fonts.font_system, metrics, label, width)
                });
            let tab = &workspace.tabs[1];
            assert_eq!(tab.label(), "2  rota-gtp");
            let name = workspace.workspace_label.as_ref().unwrap();
            assert!(name.label.contains("gpy-jq"), "{}", name.label);
            let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
            let line = chrome_title_metrics(metrics).line_height;
            let mut swash_cache = SwashCache::new();
            let mut place = |font_system: &mut FontSystem, text: &str, label: SceneRect| {
                let mut buffer = shape_styled_text(
                    font_system,
                    metrics,
                    cell_font,
                    text,
                    label.width,
                    line,
                    DrawStyleKind::Status(Wrap::None),
                );
                let top = chip_label_top(font_system, &mut swash_cache, &mut buffer, label, line);
                let ink = drawn_ink(font_system, &buffer, label.left, top);
                (buffer, top, ink)
            };
            let header_clip = SceneRect {
                top: workspace.header.top,
                height: workspace.header.height,
                ..name.rect
            };
            for (text, slot, clip) in [
                (tab.label(), tab.rect, workspace.tab_label_clip),
                (name.label.as_str(), name.rect, header_clip),
            ] {
                let label = chip_label_box(&workspace, slot, metrics);
                let (buffer, top, ink) = place(&mut fonts.font_system, text, label);
                let bounds = chip_label_bounds(label, clip).unwrap();
                assert!(
                    ink.1 >= bounds.top && ink.3 <= bounds.bottom,
                    "{text} at {case}: ink rows {}..{} leave the clip rows {}..{}",
                    ink.1,
                    ink.3,
                    bounds.top,
                    bounds.bottom
                );
                let pill = workspace.bar_pill_rect(slot, metrics);
                assert_eq!((label.top, label.height), (pill.top, pill.height));
                // The label draws where the line centred in the pill always put it, and at 1 em
                // the descenders reach below that line, the box that used to clip them.
                let centred = pill.top + (pill.height - line) / 2.0;
                assert_eq!(top, centred, "{text} at {case}");
                assert_eq!(
                    ink,
                    drawn_ink(&mut fonts.font_system, &buffer, label.left, centred)
                );
                if line_height == 1.0 {
                    assert!(ink.3 as f32 > centred + line, "{text} at {case}");
                }
            }
            // Bare hint text in the bottom bar, as Left/Right and its label show in tabs focus.
            let hints = ShortcutHintsScene::new(
                vec![crate::ShortcutHint {
                    modifiers: String::new(),
                    modifier_names: String::new(),
                    key: "Left/Right".into(),
                    label: "gpy-jq".into(),
                    short: "gpy-jq".into(),
                    description: "gpy-jq".into(),
                    section: crate::HintSection::Actions,
                    accessible_label: "Left/Right gpy-jq".into(),
                    priority: 0,
                }],
                size,
                metrics,
                None,
                None,
                |text| fit_header_text(&mut fonts.font_system, metrics, text, f32::INFINITY).1,
            );
            let piece = &hints.pieces[0];
            for (text, left, width) in [
                (piece.key.as_str(), piece.rect.left, piece.label_offset),
                (
                    piece.label.as_str(),
                    piece.rect.left + piece.label_offset,
                    piece.rect.width - piece.label_offset,
                ),
            ] {
                let label = hint_label_box(&hints, piece.rect, left, width, metrics);
                let (_, top, ink) = place(&mut fonts.font_system, text, label);
                let clip = SceneRect {
                    top: label.top,
                    height: label.height,
                    ..piece.rect
                };
                let bounds = chip_label_bounds(label, clip).unwrap();
                assert!(
                    ink.1 >= bounds.top && ink.3 <= bounds.bottom,
                    "hint {text} at {case}: ink rows {}..{} leave the clip rows {}..{}",
                    ink.1,
                    ink.3,
                    bounds.top,
                    bounds.bottom
                );
                assert_eq!(top, piece.rect.top, "hint {text} at {case}");
                if line_height == 1.0 && text.contains('g') {
                    assert!(ink.3 as f32 > piece.rect.top + line, "hint at {case}");
                }
            }
            // A chip shorter than the face lifts the line by whole pixels, keeping g and p in.
            let short = SceneRect {
                height: metrics.font_size.round(),
                ..chip_label_box(&workspace, tab.rect, metrics)
            };
            let (_, top, ink) = place(&mut fonts.font_system, tab.label(), short);
            let bounds = chip_label_bounds(short, short).unwrap();
            assert!(
                ink.1 >= bounds.top && ink.3 <= bounds.bottom,
                "short chip at {case}: ink rows {}..{} leave the clip rows {}..{}",
                ink.1,
                ink.3,
                bounds.top,
                bounds.bottom
            );
            let centred = short.top + (short.height - line) / 2.0;
            assert!(top != centred && top.fract() == centred.fract(), "{case}");
        }
        // At font size 96, a chip shorter than the face but taller than "É-gp"'s ink keeps both
        // the accent and the descenders; one shorter than that ink keeps the descenders.
        let mut fonts = FontSetup::new(&FontSettings {
            size: 96.0,
            line_height: 1.0,
            ..FontSettings::default()
        })
        .unwrap();
        fonts
            .set_header_font_file(std::path::Path::new(&font))
            .unwrap();
        let metrics = fonts.metrics(1.0);
        let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, false);
        let line = metrics.height;
        let mut buffer = shape_styled_text(
            &mut fonts.font_system,
            metrics,
            cell_font,
            "É-gp",
            1000.0,
            line,
            DrawStyleKind::Status(Wrap::None),
        );
        let free = drawn_ink(&mut fonts.font_system, &buffer, 0.0, 0.0);
        let face = buffer.line_layout(&mut fonts.font_system, 0).unwrap()[0].clone();
        let ink_height = (free.3 - free.1) as f32;
        assert!(ink_height + 2.0 < face.max_ascent + face.max_descent);
        let mut swash_cache = SwashCache::new();
        for (height, fits) in [(ink_height + 1.0, true), (ink_height - 6.0, false)] {
            let chip = SceneRect {
                left: 0.0,
                top: 50.0,
                width: 1000.0,
                height,
            };
            let top = chip_label_top(
                &mut fonts.font_system,
                &mut swash_cache,
                &mut buffer,
                chip,
                line,
            );
            let ink = drawn_ink(&mut fonts.font_system, &buffer, 0.0, top);
            let bounds = chip_label_bounds(chip, chip).unwrap();
            assert!(
                ink.3 <= bounds.bottom && (ink.1 >= bounds.top) == fits,
                "É-gp in a {height}-pixel chip: ink rows {}..{}, clip rows {}..{}",
                ink.1,
                ink.3,
                bounds.top,
                bounds.bottom
            );
        }
    }
}
