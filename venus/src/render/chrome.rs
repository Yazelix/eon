//! Eon workspace chrome drawing: tabs, pane headers, Eon Bar controls and quota chips, the
//! header tooltip and the native dialog, plus the pointer state that highlights them.

use super::{
    CellMetrics, CursorPoint, DrawStyleKind, PlacedText, RectangleBatch, Renderer, SceneColor,
    cells::NERD_FONT_FAMILY, palette, physical_outline_width, shaping,
};
use crate::{
    DialogKind, DialogScene, Load, SceneRect, WorkspaceFocus, WorkspaceHeaderControl,
    WorkspaceScene,
    scene::{QuotaProvider, SIDEBAR_EMPTY, SIDEBAR_TITLE, TabBadge, chrome_title_metrics},
};
use eon_workspace_protocol::v26::AttentionState;
use glyphon::{
    Attrs, Buffer, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, TextBounds, Weight, Wrap, fontdb,
};

const OPENAI_BLOSSOM_ID: u16 = 1;
const CLAUDE_SPLAT_ID: u16 = 2;
const QUOTA_ICON_SIZE: usize = 104;

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

fn pane_id_color(
    name: SceneColor,
    selected: bool,
    live: bool,
    fill: SceneColor,
    opacity: f32,
    band: &[SceneColor],
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
    let resolved = |value: u8| (f32::from(value) * opacity + 255.0 * (1.0 - opacity)).round() as u8;
    let resolved = SceneColor {
        r: resolved(fill.r),
        g: resolved(fill.g),
        b: resolved(fill.b),
    };
    // An extended header band is opaque and sits under the label's lower half.
    if contrast(dim, resolved) >= 4.5
        && contrast(dim, palette::BAR_BACKGROUND) >= 4.5
        && band.iter().all(|&color| contrast(dim, color) >= 4.5)
    {
        dim
    } else {
        name
    }
}

fn color_pane_identity(buffer: &mut Buffer, fonts: &mut FontSystem, id: &str, color: SceneColor) {
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
    cpu_hovered: bool,
    ram_hovered: bool,
    clock_hovered: bool,
    pressed_header: Option<WorkspaceHeaderControl>,
}

impl HeaderInteraction {
    fn hovers(&self, focus: WorkspaceFocus, id: &str) -> bool {
        self.hovered_header
            .as_ref()
            .is_some_and(|(region, target)| *region == focus && target == id)
    }
}

pub(super) fn rasterize_quota_icon(
    request: RasterizeCustomGlyphRequest,
) -> Option<RasterizedCustomGlyph> {
    let source = match request.id {
        OPENAI_BLOSSOM_ID => OPENAI_BLOSSOM_ALPHA,
        CLAUDE_SPLAT_ID => CLAUDE_SPLAT_ALPHA,
        _ => return None,
    };
    let width = usize::from(request.width);
    let height = usize::from(request.height);
    if width == 0 || height == 0 {
        return None;
    }
    let mut data = vec![0; width * height];
    for y in 0..height {
        let top = y * QUOTA_ICON_SIZE / height;
        let bottom = ((y + 1) * QUOTA_ICON_SIZE)
            .div_ceil(height)
            .max(top + 1)
            .min(QUOTA_ICON_SIZE);
        for x in 0..width {
            let left = x * QUOTA_ICON_SIZE / width;
            let right = ((x + 1) * QUOTA_ICON_SIZE)
                .div_ceil(width)
                .max(left + 1)
                .min(QUOTA_ICON_SIZE);
            let mut alpha = 0_u64;
            let count = ((right - left) * (bottom - top)) as u64;
            for source_y in top..bottom {
                for source_x in left..right {
                    alpha += u64::from(source[source_y * QUOTA_ICON_SIZE + source_x]);
                }
            }
            data[y * width + x] = (alpha / count) as u8;
        }
    }
    Some(RasterizedCustomGlyph {
        data,
        content_type: ContentType::Mask,
    })
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
}

/// The working badges in the current static vertices and the phase they are drawn at. On a
/// software adapter, where every turn re-rasterizes the whole frame, or when Eon turns the
/// animation off, there is no phase: working badges are drawn still and never tick (VEN-C21).
pub(super) struct Spinners {
    slots: Vec<SpinnerSlot>,
    phase: Option<usize>,
}

/// Whether working badges turn: only on a hardware adapter, and only while
/// `--animate-working-badge` is true (VEN-C21).
pub(super) fn spinners_turn(device: wgpu::DeviceType, animate: bool) -> bool {
    animate && device != wgpu::DeviceType::Cpu
}

impl Spinners {
    pub(super) fn new(turning: bool) -> Self {
        Self {
            slots: Vec::new(),
            phase: turning.then_some(0),
        }
    }

    pub(super) fn clear(&mut self) {
        self.slots.clear();
    }

    /// Draws `badge`, inside a disc of `ring` (the color of what it sits on) when given.
    fn push_badge(
        &mut self,
        badge: TabBadge,
        ring: Option<SceneColor>,
        rectangles: &mut RectangleBatch,
    ) {
        if !badge_inside(badge, rectangles.clip) {
            return;
        }
        if let Some(ring) = ring {
            rectangles.push_rounded(badge.rect, badge.rect.width / 2.0, ring);
        }
        if badge.state == AttentionState::Working && self.phase.is_some() {
            self.slots.push(SpinnerSlot {
                offset: rectangles.bytes.len(),
                badge,
            });
        }
        push_badge_glyph(rectangles, badge, self.phase);
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

/// Where a shaped buffer's ink ends, from its left edge. Unlike a run's `line_w`, this counts the
/// offset that right-aligns a right-to-left line, such as a Hebrew directory name.
fn shaped_end(buffer: &Buffer) -> f32 {
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| glyph.x + glyph.w)
        .fold(0.0, f32::max)
}

/// Draws the agent sidebar's popup marker, a small window: an outlined square with a filled
/// title bar, distinct from every attention glyph. Quads keep it independent of fonts.
fn push_popup_marker(rectangles: &mut RectangleBatch, rect: SceneRect, color: SceneColor) {
    let stroke = (rect.width / 8.0).round().max(1.0);
    rectangles.push_hollow(rect.left, rect.top, rect.width, rect.height, stroke, color);
    let bar = (rect.height * 0.3).round().max(stroke);
    rectangles.push(rect.left, rect.top, rect.width, bar, color, 1.0);
}

/// Draws one attention badge's glyph inside its disc: ◆ needs input, ✕ error, ✓ finished, the
/// braille frame at `phase` for working, or the first frame without one, three still dots for
/// background, and, in sidebar rows, ○ idle and · unavailable. Quads keep the glyphs independent
/// of fonts.
fn push_badge_glyph(rectangles: &mut RectangleBatch, badge: TabBadge, phase: Option<usize>) {
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
            palette::ATTENTION_NEEDS_INPUT,
            alpha,
        ),
        AttentionState::Error => {
            rectangles.push_stroke(
                at(0.1, 0.1),
                at(0.9, 0.9),
                stroke,
                palette::ATTENTION_ERROR,
                alpha,
            );
            rectangles.push_stroke(
                at(0.9, 0.1),
                at(0.1, 0.9),
                stroke,
                palette::ATTENTION_ERROR,
                alpha,
            );
        }
        AttentionState::Finished => {
            rectangles.push_stroke(
                at(0.05, 0.55),
                at(0.4, 0.88),
                stroke,
                palette::ATTENTION_FINISHED,
                alpha,
            );
            rectangles.push_stroke(
                at(0.4, 0.88),
                at(0.95, 0.15),
                stroke,
                palette::ATTENTION_FINISHED,
                alpha,
            );
        }
        AttentionState::Working => push_dots(
            rectangles,
            glyph,
            (2, 3),
            SPINNER_FRAMES[phase.unwrap_or(0)],
            alpha,
        ),
        // Background never animates (VEN-C21), so it takes no phase.
        AttentionState::Background => push_dots(rectangles, glyph, (3, 1), 0b111, alpha),
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
                    palette::ATTENTION_IDLE,
                    alpha,
                );
            }
        }
    }
}

/// Draws a `columns` by `rows` grid of square dots centred on the whole pixels inside `glyph`,
/// lighting the dots whose bit is set in `lit`, numbered down each column and then across, as a
/// braille cell numbers them: a working badge's 2x3 braille frame, or the background badge's
/// three dots in a row. Dots are twice as wide as the gaps between them, two pixels and one at
/// the default badge, scaled by whole pixels with the glyph. Every dot is pushed, unlit ones at
/// zero alpha, so each spinner phase writes the same vertices and only alpha changes. A glyph too
/// small for the grid keeps the dots that fit; none leave `glyph`, so the badge's clip check
/// covers them.
fn push_dots(
    rectangles: &mut RectangleBatch,
    glyph: SceneRect,
    (columns, rows): (u8, u8),
    lit: u8,
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
                palette::ATTENTION_WORKING,
                if (lit >> bit) & 1 == 1 { alpha } else { 0.0 },
            );
        }
    }
}

impl Renderer {
    /// Steps every drawn working badge one frame by rewriting only its own uploaded vertices,
    /// so the scene, text and terminal layers stay cached.
    pub fn turn_spinners(&mut self) {
        let Some(phase) = &mut self.spinners.phase else {
            return;
        };
        *phase = (*phase + 1) % SPINNER_FRAMES.len();
        for slot in &self.spinners.slots {
            let mut batch = RectangleBatch::new(self.config.width, self.config.height);
            push_badge_glyph(&mut batch, slot.badge, self.spinners.phase);
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

fn header_item_visual(rect: SceneRect, padding: f32) -> SceneRect {
    let inset = (padding / 3.0).min(rect.width / 4.0).min(rect.height / 4.0);
    SceneRect {
        left: rect.left + inset,
        top: rect.top + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        height: (rect.height - inset * 2.0).max(0.0),
    }
}

fn push_bar_widget(
    rectangles: &mut RectangleBatch,
    visual: SceneRect,
    border_width: u8,
    scale: f32,
) {
    rectangles.push_rounded_outline_width(
        visual,
        visual.height / 2.0,
        physical_outline_width(border_width, scale),
        palette::CHIP_OUTLINE,
    );
}

fn fit_tab_text(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    text: &str,
    zoomed: bool,
    width: f32,
) -> (String, f32) {
    let fallback = text
        .strip_suffix(" \u{f00e}")
        .filter(|_| {
            zoomed
                && fonts
                    .db()
                    .query(&fontdb::Query {
                        families: &[Family::Name(NERD_FONT_FAMILY)],
                        ..Default::default()
                    })
                    .and_then(|id| fonts.get_font(id, Weight::NORMAL))
                    .is_none()
        })
        .map(|label| format!("{label} · zoom"));
    fit_header_text(fonts, metrics, fallback.as_deref().unwrap_or(text), width)
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

    pub fn set_cpu_hovered(&mut self, hovered: bool) {
        self.header.cpu_hovered = hovered;
    }

    pub fn set_ram_hovered(&mut self, hovered: bool) {
        self.header.ram_hovered = hovered;
    }

    pub fn set_clock_hovered(&mut self, hovered: bool) {
        self.header.clock_hovered = hovered;
    }

    pub fn set_quota_hovered(&mut self, hovered: Option<usize>) {
        self.header.quota_hovered = hovered;
    }

    pub fn set_pressed_header(&mut self, control: Option<WorkspaceHeaderControl>) {
        self.header.pressed_header = control;
    }

    /// Fit the scene's canonical label using the same typography as its header.
    pub fn fit_tab_text(&mut self, label: &str, zoomed: bool) -> (String, f32) {
        let width = WorkspaceScene::tab_text_width(self.size(), self.metrics);
        fit_tab_text(
            &mut self.fonts.font_system,
            self.metrics,
            label,
            zoomed,
            width,
        )
    }

    fn push_attention_row(
        &mut self,
        row: &crate::SidebarRow,
        list: SceneRect,
        hovered: bool,
        rectangles: &mut RectangleBatch,
    ) {
        let padding = self.metrics.padding;
        let line = self.metrics.height;
        if row.rect.intersection(list).is_none() {
            return;
        }
        if row.focused || hovered {
            rectangles.push_rounded(row.rect, padding / 2.0, palette::CHROME_HIGHLIGHT);
        }
        // The "you are here" outline, in the pane cards' outline color, over any
        // highlight and under a focused row's outline.
        if row.current {
            rectangles.push_rounded_outline(row.rect, padding / 2.0, palette::CARD_OUTLINE);
        }
        if row.focused {
            rectangles.push_rounded_outline(row.rect, padding / 2.0, palette::ACCENT);
        }
        // A glyph on the translucent card needs no disc; on a highlight it matches it.
        let ring = (row.focused || hovered).then_some(palette::CHROME_HIGHLIGHT);
        self.spinners.push_badge(row.glyph, ring, rectangles);
        if let Some((provider, rect)) = row.icon {
            self.push_quota_icon(provider, rect, Some(list));
        }
        let (title, detail) = if row.glyph.dimmed {
            (palette::SECONDARY_TEXT, palette::TERTIARY_TEXT)
        } else {
            (palette::PRIMARY_TEXT, palette::SECONDARY_TEXT)
        };
        let text_width = (row.rect.right() - padding / 2.0 - row.text_left).max(0.0);
        let text_top = row.glyph.rect.top - (line - row.glyph.rect.height) / 2.0;
        for (index, (text, marker, color)) in [
            (Some(row.title.as_str()), None, title),
            (Some(row.detail.as_str()), row.popup_marker, detail),
            (row.message.as_deref(), None, title),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(text) = text else { continue };
            self.push_sidebar_line(
                text,
                marker,
                row.text_left,
                text_top + line * index as f32,
                text_width,
                color,
                list,
                rectangles,
            );
        }
    }

    /// One line of agent sidebar text. With `marker`, the popup marker follows the text before
    /// that byte offset, at the end of the line if that text overflows, and the rest of the text
    /// follows the marker.
    #[allow(clippy::too_many_arguments)]
    fn push_sidebar_line(
        &mut self,
        text: &str,
        marker: Option<usize>,
        left: f32,
        top: f32,
        width: f32,
        color: SceneColor,
        clip: SceneRect,
        rectangles: &mut RectangleBatch,
    ) {
        let height = chrome_title_metrics(self.metrics).text_height;
        let style = DrawStyleKind::Status(Wrap::None);
        let Some(marker) = marker else {
            self.push_text_clipped(text, left, top, width, width, height, color, style, clip);
            return;
        };
        let (before, after) = text.split_at(marker);
        // The marker is as large as a row's state glyph, half a cell from the text before it.
        let size = (self.metrics.font_size * 0.75).round().max(1.0);
        let spacing = (self.metrics.width / 2.0).round();
        let ink = (width - size - spacing).max(0.0);
        let pushed = self.text.len();
        self.push_text_clipped(before, left, top, ink, ink, height, color, style, clip);
        let end = self.text[pushed..]
            .first()
            .map_or(0.0, |text| shaped_end(&text.buffer));
        let marker = SceneRect {
            left: (left + end + spacing).min(left + width - size).max(left),
            top: top + (self.metrics.height - size) / 2.0,
            width: size,
            height: size,
        };
        push_popup_marker(rectangles, marker, color);
        let rest = left + width - marker.right();
        if rest > 0.0 {
            self.push_text_clipped(
                after,
                marker.right(),
                top,
                rest,
                rest,
                height,
                color,
                style,
                clip,
            );
        }
    }

    /// Measure an exact, non-truncated Eon Bar fact label with header typography.
    pub fn measure_header_text(&mut self, label: &str) -> (String, f32) {
        self.fit_header_text(label, f32::INFINITY)
    }

    /// Fit header text to `width` pixels with header typography, as tab labels are.
    pub fn fit_header_text(&mut self, label: &str, width: f32) -> (String, f32) {
        fit_header_text(&mut self.fonts.font_system, self.metrics, label, width)
    }

    pub(super) fn build_header_tooltip(
        &mut self,
        workspace: &WorkspaceScene,
        rectangles: &mut RectangleBatch,
    ) {
        let hovered_tab = workspace
            .tabs
            .iter()
            .find(|tab| self.header.hovers(WorkspaceFocus::Tabs, &tab.id));
        let (label, anchor) = if let Some(quota) = self
            .header
            .quota_hovered
            .and_then(|index| workspace.quotas.get(index))
        {
            (quota.description.as_str(), quota.rect)
        } else if self.header.version_hovered
            && let Some(version) = &workspace.version
        {
            (version.description.as_str(), version.rect)
        } else if self.header.cpu_hovered
            && let Some(cpu) = &workspace.cpu
        {
            (cpu.description.as_str(), cpu.rect)
        } else if self.header.ram_hovered
            && let Some(ram) = &workspace.ram
        {
            (ram.description.as_str(), ram.rect)
        } else if self.header.clock_hovered
            && let Some(clock) = &workspace.clock
        {
            (clock.description.as_str(), clock.rect)
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
            .controls
            .iter()
            .find(|control| self.header.hovers(WorkspaceFocus::Header(control.kind), ""))
        {
            (control.label, control.rect)
        } else {
            return;
        };
        let padding = self.metrics.padding;
        let top = workspace.header.bottom() + padding / 3.0;
        let width = (self.config.width as f32 - padding * 2.0).min(self.metrics.font_size * 40.0);
        let height = self.config.height as f32 - top - padding;
        if width <= padding * 2.0 || height <= padding * 2.0 {
            return;
        }
        let before = self.text.len();
        self.push_text(
            label,
            0.0,
            top + padding,
            width - padding * 2.0,
            height - padding * 2.0,
            palette::PRIMARY_TEXT,
            DrawStyleKind::Status(Wrap::WordOrGlyph),
        );
        let Some(text) = self.text.get_mut(before) else {
            return;
        };
        let mut text_width = 0.0_f32;
        let mut text_height = 0.0_f32;
        for run in text.buffer.layout_runs() {
            text_width = text_width.max(run.line_w);
            text_height = text_height.max(run.line_top + run.line_height);
        }
        let width = (text_width.ceil() + padding * 2.0).min(width);
        let height = (text_height.ceil() + padding * 2.0).min(height);
        let left = anchor.left.clamp(
            padding,
            (self.config.width as f32 - width - padding).max(padding),
        );
        let tooltip = SceneRect {
            left,
            top,
            width,
            height,
        };
        if self
            .terminal
            .scrollback_rect
            .is_some_and(|pill| pill.intersection(tooltip).is_some())
        {
            self.terminal.scrollback_rect = None;
        }
        text.left = left + padding;
        text.bound_left = text.left.floor() as i32;
        text.right = (left + width - padding).ceil() as i32;
        text.bound_top = (top + padding).floor() as i32;
        text.bottom = (top + height - padding).ceil() as i32;
        self.text_overlay = Some((
            before,
            TextBounds {
                left: left.floor() as i32,
                top: top.floor() as i32,
                right: (left + width).ceil() as i32,
                bottom: (top + height).ceil() as i32,
            },
        ));
        rectangles.push_rounded(tooltip, padding / 2.0, palette::TOOLTIP_BACKGROUND);
    }

    pub(super) fn build_dialog(&mut self, viewer: &DialogScene, rectangles: &mut RectangleBatch) {
        let first = self.text.len();
        let padding = self.metrics.padding.min(viewer.bounds.width / 8.0);
        rectangles.clip = None;
        rectangles.push_rounded(viewer.bounds, padding / 2.0, palette::DIALOG_BACKGROUND);
        rectangles.push_rounded_outline(viewer.bounds, padding / 2.0, palette::CARD_OUTLINE);
        self.text_overlay = Some((
            first,
            TextBounds {
                left: viewer.bounds.left.floor() as i32,
                top: viewer.bounds.top.floor() as i32,
                right: viewer.bounds.right().ceil() as i32,
                bottom: viewer.bounds.bottom().ceil() as i32,
            },
        ));

        self.push_text_clipped(
            viewer.kind.title(),
            viewer.title.left,
            viewer.title.top,
            viewer.title.width,
            viewer.title.width,
            viewer.title.height,
            palette::PRIMARY_TEXT,
            DrawStyleKind::Heading,
            viewer.bounds,
        );
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
                rectangles.push_rounded(chip.rect, padding / 2.0, palette::CHROME_HIGHLIGHT);
                rectangles.push_rounded_outline(chip.rect, padding / 2.0, palette::ACCENT);
            }
            self.push_text_clipped(
                &format!("{} {}", chip.filter.label(), chip.count),
                chip.rect.left + padding / 3.0,
                chip.rect.top + (chip.rect.height - self.metrics.height) / 2.0,
                (chip.rect.width - padding * 2.0 / 3.0).max(0.0),
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
            self.push_attention_row(row, viewer.content, false, rectangles);
        }
        let key_cells = match viewer.kind {
            DialogKind::Shortcuts | DialogKind::AttentionInbox => 15.0,
            DialogKind::CloseLastTab { .. } => 6.0,
        };
        let key_width = (self.metrics.font_size * key_cells).min(viewer.content.width * 0.4);
        let gap = padding.min(self.metrics.width);
        for group in &viewer.groups {
            if !group.title.is_empty() {
                self.push_text_clipped(
                    &group.title,
                    group.heading.left,
                    group.heading.top,
                    group.heading.width,
                    group.heading.width,
                    group.heading.height,
                    palette::ACCENT,
                    DrawStyleKind::PopupLabel,
                    viewer.content,
                );
            }
            for row in &group.rows {
                self.push_text_clipped(
                    &row.shortcut,
                    row.rect.left,
                    row.rect.top,
                    key_width,
                    key_width,
                    row.rect.height,
                    palette::SHORTCUT_KEY_TEXT,
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    viewer.content,
                );
                let action_left = row.rect.left + key_width + gap;
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
            rectangles.push_rounded(track, track.width / 2.0, palette::SCROLLBAR_TRACK);
            let thumb_height = (track.height * viewer.content.height
                / (viewer.content.height + viewer.max_scroll))
                .max(track.width * 2.0)
                .min(track.height);
            let thumb_top =
                track.top + (track.height - thumb_height) * viewer.scroll / viewer.max_scroll;
            rectangles.push_rounded(
                SceneRect {
                    top: thumb_top,
                    height: thumb_height,
                    ..track
                },
                track.width / 2.0,
                palette::ACCENT,
            );
        }
        rectangles.clip = None;
        for (index, line) in viewer.kind.footer().iter().enumerate() {
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

    pub(super) fn build_workspace(
        &mut self,
        workspace: &WorkspaceScene,
        focus: WorkspaceFocus,
        rectangles: &mut RectangleBatch,
        backgrounds: &[SceneColor],
        bands: &[Vec<SceneColor>],
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
            let gap = self.build_border_title(
                SIDEBAR_TITLE,
                SceneRect {
                    height: text_height,
                    ..area
                },
                area,
                width,
                palette::PRIMARY_TEXT,
            );
            if self.pane_frames {
                rectangles.clip = Some(area);
                rectangles.push_card_outline(
                    workspace.card_rect(area, self.metrics, self.card_stroke_width()),
                    workspace.card_radius(),
                    &[gap],
                    physical_outline_width(self.border_width, self.metrics.scale),
                    palette::CARD_OUTLINE,
                );
                rectangles.clip = None;
            }
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
            for heading in &workspace.sidebar_headings {
                self.push_sidebar_line(
                    &heading.label,
                    None,
                    heading.rect.left,
                    heading.rect.top,
                    heading.rect.width,
                    palette::ACCENT,
                    list,
                    rectangles,
                );
            }
            for row in workspace.sidebar_rows() {
                let hovered = self.header.hovers(WorkspaceFocus::Sidebar, &row.target);
                self.push_attention_row(row, list, hovered, rectangles);
            }
            rectangles.clip = None;
        }
        if let Some(hints) = &workspace.hints {
            // The hints sit bare in the margin below the cards.
            for hint in &hints.pieces {
                if !hint.modifiers.is_empty() {
                    rectangles.push_rounded(
                        SceneRect {
                            width: hint.key_offset - self.metrics.padding,
                            ..hint.rect
                        },
                        self.metrics.padding / 2.0,
                        palette::HINT_KEYCAP,
                    );
                }
                for (text, left, width, color) in [
                    (
                        hint.modifiers.as_str(),
                        hint.rect.left + self.metrics.padding / 2.0,
                        hint.key_offset - self.metrics.padding * 1.5,
                        palette::HINT_MODIFIER_TEXT,
                    ),
                    (
                        hint.key.as_str(),
                        hint.rect.left + hint.key_offset,
                        hint.label_offset - hint.key_offset,
                        palette::ACCENT,
                    ),
                    (
                        hint.label.as_str(),
                        hint.rect.left + hint.label_offset,
                        hint.rect.width - hint.label_offset,
                        palette::SECONDARY_TEXT,
                    ),
                ]
                .into_iter()
                .filter(|(text, _, _, _)| !text.is_empty())
                {
                    self.push_text_clipped(
                        text,
                        left,
                        hint.rect.top,
                        width,
                        width,
                        hint.rect.height,
                        color,
                        DrawStyleKind::Status(Wrap::None),
                        hint.rect,
                    );
                }
            }
        }
        rectangles.clip = Some(workspace.tab_viewport);
        for tab in &workspace.tabs {
            if tab.rect.intersection(workspace.tab_viewport).is_none() {
                continue;
            }
            let visual = workspace.bar_pill_rect(tab.rect, self.metrics);
            let radius = visual.height / 2.0;
            // Each tab state steps up from the last so that the pills stand out from the bar.
            let fill = if tab.selected {
                palette::TAB_SELECTED
            } else if self.header.hovers(WorkspaceFocus::Tabs, &tab.id) {
                palette::TAB_HOVER
            } else {
                palette::TAB_IDLE
            };
            rectangles.push_rounded(visual, radius, fill);
            if tab.selected && focus == WorkspaceFocus::Tabs {
                rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
            }
            let label_width = (tab.rect.width - self.metrics.padding * 2.0).max(1.0);
            self.push_text_clipped(
                tab.label(),
                tab.rect.left + self.metrics.padding,
                visual.top + (visual.height - self.metrics.height) / 2.0,
                label_width,
                label_width,
                self.metrics.height,
                if tab.selected {
                    palette::PRIMARY_TEXT
                } else {
                    palette::TAB_TEXT
                },
                DrawStyleKind::Status(Wrap::None),
                workspace.tab_label_clip,
            );
            if let Some(badge) = tab.badge {
                self.spinners
                    .push_badge(badge, Some(palette::BAR_BACKGROUND), rectangles);
            }
        }
        for badge in &workspace.edge_badges {
            self.spinners
                .push_badge(*badge, Some(palette::BAR_BACKGROUND), rectangles);
        }
        rectangles.clip = None;
        // The workspace label shares the tabs' band but reads as a bar widget, with the header
        // controls' hover, pressed and focus states while it opens the picker.
        if let Some(label) = &workspace.workspace_label {
            let control = WorkspaceHeaderControl::Workspaces;
            let visual = workspace.bar_pill_rect(label.rect, self.metrics);
            let radius = visual.height / 2.0;
            if workspace.has_control(control) {
                if self.header.pressed_header == Some(control) {
                    rectangles.push_rounded(visual, radius, palette::CHROME_HIGHLIGHT);
                } else if self.header.hovers(WorkspaceFocus::Header(control), "") {
                    rectangles.push_rounded(visual, radius, palette::CONTROL_HOVER);
                }
            }
            push_bar_widget(rectangles, visual, self.border_width, self.metrics.scale);
            if focus == WorkspaceFocus::Header(control) && workspace.has_control(control) {
                rectangles.push_rounded_outline(visual, radius, palette::ACCENT);
            }
            let label_width = (label.rect.width - self.metrics.padding * 2.0).max(1.0);
            self.push_text_clipped(
                &label.label,
                label.rect.left + self.metrics.padding,
                visual.top + (visual.height - self.metrics.height) / 2.0,
                label_width,
                label_width,
                self.metrics.height,
                palette::PRIMARY_TEXT,
                DrawStyleKind::Status(Wrap::None),
                SceneRect {
                    top: workspace.header.top,
                    height: workspace.header.height,
                    ..label.rect
                },
            );
            if let Some(badge) = label.badge {
                self.spinners
                    .push_badge(badge, Some(palette::BAR_BACKGROUND), rectangles);
            }
        }
        // Quota chips and readings share the tabs' band; the version label stays plain.
        for quota in &workspace.quotas {
            let visual = workspace.bar_pill_rect(
                header_item_visual(quota.rect, self.metrics.padding),
                self.metrics,
            );
            push_bar_widget(rectangles, visual, self.border_width, self.metrics.scale);
            let content = workspace.bar_item_rect(quota.rect, self.metrics);
            self.push_quota_icon(quota.provider, quota.logo, None);
            let label_left = quota.logo.right() + self.metrics.padding / 2.0;
            let label_width = (quota.rect.right() - self.metrics.padding - label_left).max(0.0);
            let first_text = self.text.len();
            self.push_text_clipped(
                &quota.label,
                label_left,
                content.top + (content.height - self.metrics.height).max(0.0) / 2.0,
                label_width,
                label_width,
                self.metrics.height.min(quota.rect.height),
                palette::SECONDARY_TEXT,
                DrawStyleKind::Status(Wrap::None),
                quota.rect,
            );
            if let Some(shortfall) = quota.pace
                && let Some(text) = self.text.get_mut(first_text)
            {
                color_quota_pace(
                    &mut text.buffer,
                    &mut self.fonts.font_system,
                    &quota.label,
                    shortfall,
                );
            }
        }
        for (label, color, pill) in [
            (&workspace.cpu, palette::SECONDARY_TEXT, true),
            (&workspace.ram, palette::SECONDARY_TEXT, true),
            (&workspace.clock, palette::SECONDARY_TEXT, true),
            (&workspace.version, palette::VERSION_TEXT, false),
        ] {
            let Some(label) = label else {
                continue;
            };
            let color = match label.load {
                Load::Normal => color,
                Load::High => palette::LOAD_HIGH,
                Load::Critical => palette::LOAD_CRITICAL,
            };
            let content = workspace.bar_item_rect(label.rect, self.metrics);
            if pill {
                let visual = workspace.bar_pill_rect(
                    header_item_visual(label.rect, self.metrics.padding),
                    self.metrics,
                );
                push_bar_widget(rectangles, visual, self.border_width, self.metrics.scale);
            }
            let width = (label.rect.width - self.metrics.padding * 2.0).max(0.0);
            // A label narrower than its chip sits in the middle; every label is ASCII in cells.
            let slack = (width - label.label.len() as f32 * self.metrics.width).max(0.0);
            self.push_text_clipped(
                &label.label,
                label.rect.left + self.metrics.padding + slack / 2.0,
                content.top + (content.height - self.metrics.height).max(0.0) / 2.0,
                width,
                width,
                self.metrics.height.min(label.rect.height),
                color,
                DrawStyleKind::Monospace,
                label.rect,
            );
        }
        for control in workspace.controls {
            if control.rect.width <= 0.0 {
                continue;
            }
            let content = workspace.bar_item_rect(control.rect, self.metrics);
            let visual = header_item_visual(content, self.metrics.padding);
            let hovered = self.header.hovers(WorkspaceFocus::Header(control.kind), "");
            let pressed = self.header.pressed_header == Some(control.kind);
            let focused = focus == WorkspaceFocus::Header(control.kind);
            let fill = if pressed {
                palette::CHROME_HIGHLIGHT
            } else if hovered {
                palette::CONTROL_HOVER
            } else {
                palette::BAR_BACKGROUND
            };
            rectangles.push_rounded(visual, visual.height / 2.0, fill);
            if focused {
                rectangles.push_rounded_outline(visual, visual.height / 2.0, palette::ACCENT);
            }
            let glyph_width = self.metrics.width.min(control.rect.width);
            self.push_text_clipped(
                control.kind.glyph(),
                control.rect.left + (control.rect.width - glyph_width).max(0.0) / 2.0,
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
            && let Some(terminal) = workspace.visible_terminal()
        {
            let card = workspace
                .card_fill(workspace.chrome, self.metrics, self.card_stroke_width())
                .bounds;
            let title = chrome_title_metrics(self.metrics);
            let row = SceneRect {
                height: title.text_height,
                ..workspace.chrome
            };
            let gaps = if terminal.top + self.metrics.padding >= row.bottom()
                && terminal.height >= self.metrics.padding * 2.0 + self.metrics.height
                && terminal.width >= self.metrics.padding * 2.0 + self.metrics.width * 4.0
                && card.top == crate::scene::title_rule_y(row.top, row.height)
            {
                vec![self.build_border_title(
                    label,
                    row,
                    workspace.chrome,
                    (row.width - self.metrics.padding * 2.0).max(0.0),
                    palette::PRIMARY_TEXT,
                )]
            } else {
                Vec::new()
            };
            rectangles.clip = Some(workspace.chrome);
            rectangles.push_card_outline(
                card,
                workspace.card_radius(),
                &gaps,
                physical_outline_width(self.border_width, self.metrics.scale),
                palette::CARD_OUTLINE,
            );
            rectangles.clip = None;
        }
        for (column_index, (panes, terminal, selected_column)) in
            std::iter::once((workspace.panes.as_slice(), workspace.terminal, true))
                .chain(
                    workspace
                        .other_columns
                        .iter()
                        .map(|column| (column.panes.as_slice(), column.terminal, false)),
                )
                .enumerate()
        {
            let Some(first) = panes.first() else { continue };
            let card = SceneRect {
                left: first.rect.left,
                width: first.rect.width,
                ..workspace.pane_viewport
            };
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
                let Some(rect) = pane.rect.intersection(workspace.pane_viewport) else {
                    continue;
                };
                let scrollback_width = if current {
                    self.build_scrollback(
                        SceneRect {
                            left: pane.rect.left + self.metrics.width * 4.0,
                            width: (pane.rect.width - self.metrics.width * 4.0).max(0.0),
                            ..pane.rect
                        },
                        workspace.pane_viewport,
                        true,
                        None,
                        rectangles,
                    )
                } else {
                    0.0
                };
                let width = (pane.rect.width - self.metrics.padding * 2.0 - scrollback_width)
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
                let pill = self.build_border_title(pane.label(), row, rect, width, color);
                let id_color = pane_id_color(
                    color,
                    current,
                    pane.live,
                    backgrounds
                        .get(column_index)
                        .copied()
                        .unwrap_or(palette::TERMINAL_FALLBACK),
                    self.background_opacity,
                    bands
                        .get(column_index)
                        .filter(|_| outline.expanded)
                        .map_or(&[], Vec::as_slice),
                );
                if id_color != color
                    && let Some(text) = self.text.get_mut(first_text)
                {
                    color_pane_identity(
                        &mut text.buffer,
                        &mut self.fonts.font_system,
                        &pane.id,
                        id_color,
                    );
                }
                let gaps = [
                    pill,
                    if scrollback_width > 0.0 {
                        self.terminal.scrollback_rect.unwrap()
                    } else {
                        empty_gap
                    },
                ];
                *rule_gaps = gaps;
                rectangles.clip = Some(rect);
                if pill.width > self.metrics.padding {
                    if self.header.hovers(WorkspaceFocus::Panes, &pane.id) {
                        rectangles.push_rounded(
                            pill,
                            self.metrics.padding / 2.0,
                            palette::CHROME_HIGHLIGHT,
                        );
                    }
                    if current && focus == WorkspaceFocus::Panes {
                        rectangles.push_rounded_outline(
                            pill,
                            self.metrics.padding / 2.0,
                            palette::ACCENT,
                        );
                    }
                }
                rectangles.clip = None;
            }
            rectangles.push_pane_outlines(
                fill,
                workspace.pane_viewport,
                &rules,
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
        }
    }

    /// Pane, popup and sidebar titles share shaping, placement and their measured border gap.
    pub(super) fn build_border_title(
        &mut self,
        label: &str,
        row: SceneRect,
        clip: SceneRect,
        width: f32,
        color: SceneColor,
    ) -> SceneRect {
        let (label, measured) =
            fit_header_text(&mut self.fonts.font_system, self.metrics, label, width);
        let title = chrome_title_metrics(self.metrics);
        self.push_text_clipped(
            &label,
            row.left + self.metrics.padding,
            row.top + (row.height - title.text_height) / 2.0,
            width,
            width,
            title.text_height,
            color,
            DrawStyleKind::Status(Wrap::None),
            clip,
        );
        SceneRect {
            left: row.left + self.metrics.padding / 2.0,
            top: row.top + 1.0,
            width: measured + self.metrics.padding,
            height: (row.height - 2.0).max(0.0),
        }
    }

    /// Draws a provider icon in `rect`, cut to `clip` when one is given.
    fn push_quota_icon(
        &mut self,
        provider: QuotaProvider,
        rect: SceneRect,
        clip: Option<SceneRect>,
    ) {
        let Some(bounds) = clip.map_or(Some(rect), |clip| rect.intersection(clip)) else {
            return;
        };
        let (id, color) = match provider {
            QuotaProvider::Codex => (OPENAI_BLOSSOM_ID, palette::QUOTA_ICON_CODEX),
            QuotaProvider::Claude => (CLAUDE_SPLAT_ID, palette::QUOTA_ICON_CLAUDE),
        };
        let mut buffer = Buffer::new_empty(Metrics::new(rect.height, rect.height));
        buffer.set_size(Some(rect.width), Some(rect.height));
        self.text.push(PlacedText {
            buffer,
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

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use glyphon::cosmic_text::SubpixelBin;

    #[test]
    fn quota_pace_gradient_is_ordered_readable_and_colors_only_judged_windows() {
        for (shortfall, color) in palette::QUOTA_PACE {
            assert_eq!(quota_pace_color(shortfall), color);
        }
        assert_eq!(quota_pace_color(-1.0), palette::QUOTA_PACE[0].1);
        assert_eq!(quota_pace_color(1.0), palette::QUOTA_PACE[4].1);
        // More shortfall is never greener, and every shade stays readable on the bar.
        let mut previous = quota_pace_color(-1.0);
        for step in -100..=100 {
            let color = quota_pace_color(step as f32 / 100.0);
            assert!(color.r >= previous.r && color.g <= previous.g, "{step}");
            assert!(contrast(color, palette::BAR_BACKGROUND) >= 4.5, "{step}");
            previous = color;
        }

        for label in ["2h30m/5h 50%", "2d15h/7d 50%"] {
            let mut fonts = FontSystem::new();
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 18.0));
            buffer.set_size(Some(600.0), Some(40.0));
            let attrs = Attrs::new().family(Family::SansSerif);
            buffer.set_text(label, &attrs, shaping(label), None);
            buffer.shape_until_scroll(&mut fonts, false);
            let before = buffer.layout_runs().next().unwrap().line_w;
            color_quota_pace(&mut buffer, &mut fonts, label, 0.5);
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
    fn pane_identity_dimming_preserves_contrast_and_shaped_geometry() {
        let name = SceneColor {
            r: 239,
            g: 244,
            b: 248,
        };
        let black = SceneColor { r: 0, g: 0, b: 0 };
        let dim = pane_id_color(name, true, true, black, 0.8, &[]);
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
        assert_eq!(pane_id_color(name, true, true, black, 0.3, &[]), name);
        let grey = SceneColor {
            r: 110,
            g: 110,
            b: 110,
        };
        assert_eq!(pane_id_color(name, true, true, black, 0.8, &[grey]), name);
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
                1.0,
                &[]
            ),
            name
        );
        let mut fonts = FontSystem::new();
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
        color_pane_identity(&mut buffer, &mut fonts, "p7", dim);
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
    fn eon_bar_widgets_paint_only_scaled_neutral_outlines() {
        let coverage = |batch: &RectangleBatch, x: f32, y: f32| {
            batch
                .bytes
                .chunks_exact(VERTEX_SIZE as usize * 6)
                .map(|quad| {
                    let value = |vertex: usize, component: usize| {
                        let offset = vertex * VERTEX_SIZE as usize + component * 4;
                        f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                    };
                    let rect = SceneRect {
                        left: (value(0, 0) + 1.0) * 64.0,
                        top: (1.0 - value(0, 1)) * 64.0,
                        width: (value(2, 0) - value(0, 0)) * 64.0,
                        height: (value(0, 1) - value(2, 1)) * 64.0,
                    };
                    if rect.contains(x, y) {
                        value(0, 5)
                    } else {
                        0.0
                    }
                })
                .sum::<f32>()
        };
        for scale in [1.0, 1.3] {
            let visual = SceneRect {
                left: 16.0,
                top: 13.0,
                width: 96.0,
                height: 38.0,
            };
            for border in 1..=3 {
                let mut batch = RectangleBatch::new(128, 128);
                push_bar_widget(&mut batch, visual, border, scale);
                let width = physical_outline_width(border, scale);
                for row in 0..visual.height as u32 {
                    let expected = if (row as f32) < width || row as f32 >= visual.height - width {
                        1.0
                    } else {
                        0.0
                    };
                    assert_eq!(
                        coverage(&batch, 64.5, visual.top + row as f32 + 0.5),
                        expected,
                        "widget interior/stroke at border {border}, scale {scale}, row {row}"
                    );
                }
                assert_eq!(coverage(&batch, visual.left + 0.5, visual.top + 0.5), 0.0);
                for vertex in batch.bytes.chunks_exact(VERTEX_SIZE as usize) {
                    for (component, channel) in [
                        palette::CHIP_OUTLINE.r,
                        palette::CHIP_OUTLINE.g,
                        palette::CHIP_OUTLINE.b,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let offset = (component + 2) * 4;
                        let actual =
                            f32::from_ne_bytes(vertex[offset..offset + 4].try_into().unwrap());
                        assert!((actual - f32::from(channel) / 255.0).abs() < 0.001);
                    }
                }
            }
            let mut tab = RectangleBatch::new(128, 128);
            tab.push_rounded(visual, visual.height / 2.0, palette::BAR_BACKGROUND);
            assert_eq!(coverage(&tab, 64.5, visual.top + visual.height / 2.0), 1.0);
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
                push_badge_glyph(&mut rectangles, badge, Some(phase));
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
        push_badge_glyph(&mut rectangles, badge, phase);
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
        let mut spinners = Spinners::new(true);
        let mut rectangles = RectangleBatch::new(800, 600);
        spinners.push_badge(badge, None, &mut rectangles);
        assert!(
            spinners.slots.is_empty(),
            "a background badge schedules no tick"
        );
        // A phase never changes it.
        let mut turned = RectangleBatch::new(800, 600);
        push_badge_glyph(&mut turned, badge, Some(4));
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
            let mut spinners = Spinners::new(turning);
            let mut rectangles = RectangleBatch::new(800, 600);
            spinners.push_badge(badge, None, &mut rectangles);
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
                let image = rasterize_quota_icon(RasterizeCustomGlyphRequest {
                    id,
                    width: size,
                    height: size,
                    x_bin: SubpixelBin::Zero,
                    y_bin: SubpixelBin::Zero,
                    scale: f32::from(size) / 16.0,
                })
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
            rasterize_quota_icon(RasterizeCustomGlyphRequest {
                id: CLAUDE_SPLAT_ID + 1,
                width: 16,
                height: 16,
                x_bin: SubpixelBin::Zero,
                y_bin: SubpixelBin::Zero,
                scale: 1.0,
            })
            .is_none()
        );
    }

    #[test]
    fn sidebar_marker_follows_the_shaped_ink_of_either_direction() {
        let mut fonts = FontSystem::new();
        let metrics = CellMetrics::for_scale(1.0);
        let width = 200.0;
        let mut end = |text: &str| {
            let mut buffer =
                Buffer::new(&mut fonts, Metrics::new(metrics.font_size, metrics.height));
            buffer.set_size(Some(width), Some(metrics.height));
            buffer.set_wrap(Wrap::None);
            buffer.set_text(
                text,
                &Attrs::new().family(Family::SansSerif),
                shaping(text),
                None,
            );
            buffer.shape_until_scroll(&mut fonts, false);
            let line = buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0, f32::max);
            (shaped_end(&buffer), line)
        };
        // A left-to-right label ends where its shaped width does.
        let (ltr, line) = end("2  site");
        assert!(ltr > 0.0 && (ltr - line).abs() < 0.5 && ltr < width / 2.0);
        // A right-to-left label is right-aligned, so its ink ends at the line's end, not its width.
        let (rtl, line) = end("2  שלום");
        assert!((rtl - width).abs() < 1.0 && line < width / 2.0);
    }

    #[test]
    fn zoom_marker_uses_bundled_face_or_text_fallback_and_fits() {
        let original = FontSystem::new();
        for available in [true, false] {
            let mut db = fontdb::Database::new();
            for name in ["DejaVu Sans", NERD_FONT_FAMILY] {
                if !available && name == NERD_FONT_FAMILY {
                    continue;
                }
                let id = original
                    .db()
                    .query(&fontdb::Query {
                        families: &[Family::Name(name)],
                        ..Default::default()
                    })
                    .expect("the accepted font environment supplies the tested face");
                db.push_face_info(original.db().face(id).unwrap().clone());
            }
            db.set_sans_serif_family("DejaVu Sans");
            let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
            for scale in [1.0, 1.3, 2.0] {
                let metrics = CellMetrics::for_scale(scale);
                let plain = "1  eon";
                let label = "1  eon \u{f00e}";
                let unzoomed = fit_tab_text(&mut fonts, metrics, label, false, f32::INFINITY);
                assert_eq!(unzoomed.0, label, "directory text must not imply zoom");
                let (fitted, width) = fit_tab_text(&mut fonts, metrics, label, true, f32::INFINITY);
                assert_eq!(fitted, if available { label } else { "1  eon · zoom" });
                let plain_width = fit_header_text(&mut fonts, metrics, plain, f32::INFINITY).1;
                assert!(width > plain_width, "the marker must consume tab width");
                let narrow = fit_tab_text(&mut fonts, metrics, label, true, plain_width);
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
                assert_eq!(runs.len(), 1, "label and marker share one baseline");
                assert!((runs[0].line_w - width).abs() < 0.01);
                assert!(runs[0].glyphs.iter().all(|glyph| glyph.glyph_id != 0));
                if available {
                    let glyph = runs[0]
                        .glyphs
                        .iter()
                        .find(|glyph| fitted[glyph.start..glyph.end].contains('\u{f00e}'))
                        .unwrap();
                    assert!(
                        fonts
                            .db()
                            .face(glyph.font_id)
                            .unwrap()
                            .families
                            .iter()
                            .any(|(name, _)| name == NERD_FONT_FAMILY),
                        "the marker resolves through the bundled symbol face"
                    );
                }
            }
        }
    }

    #[test]
    fn tab_text_fits_shaped_width_without_splitting_combining_clusters() {
        let mut fonts = FontSystem::new();
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
        use eon_workspace_protocol::v26::{Pane, Snapshot, Tab};
        use winit::{
            application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop,
            platform::wayland::EventLoopBuilderExtWayland, window::WindowId,
        };

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
                    SceneColor {
                        r: 178,
                        g: 7,
                        b: 251,
                    },
                    1,
                    2,
                    true,
                    FontSetup::new(&FontSettings::default()).unwrap(),
                ))
                .unwrap();
                let snapshot = Snapshot {
                    workspace: "w1".into(),
                    active_workspace: "w1".into(),
                    workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
                        id: "w1".into(),
                        name: None,
                        tab_count: 1,
                        attention: None,
                        recency: 0,
                    }],
                    active_tab: "t2".into(),
                    geometry: eon_workspace_protocol::v26::PopupGeometry {
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
                            columns: vec![eon_workspace_protocol::v26::Column {
                                panes: vec![Pane {
                                    label: format!("p{}", index + 1),
                                    pinned_name: None,
                                    id: format!("p{}", index + 1),
                                    session: format!("session-{}", index + 1),
                                    endpoint: format!("/tmp/session-{}.sock", index + 1)
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
                    sidebar: eon_workspace_protocol::v26::Sidebar::Open,
                    attention_queue: Vec::new(),
                };
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label, false),
                );
                renderer.build_workspace(
                    &workspace,
                    WorkspaceFocus::Terminal,
                    &mut RectangleBatch::new(900, 600),
                    &[],
                    &[],
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
                let (first, overlay) = renderer.text_overlay.unwrap();
                let areas: Vec<_> =
                    text_areas(&renderer.text[..first], renderer.text_overlay).collect();
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
                    |_, label| renderer.fit_tab_text(label, false),
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
                    |_, label| renderer.fit_tab_text(label, false),
                    None,
                );
                // Exercise the real shaped title and rectangle path in this existing native probe.
                for frames in [true, false] {
                    renderer.pane_frames = frames;
                    renderer.text.clear();
                    renderer.set_hovered_header(Some((WorkspaceFocus::Panes, "p3".into())));
                    let mut batch = RectangleBatch::new(900, 600);
                    renderer.build_workspace(
                        &workspace,
                        WorkspaceFocus::Panes,
                        &mut batch,
                        &[],
                        &[],
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
                    let rule = colors_at(pane.rect.right() - 20.0, rule_y);
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
                        colors_at(label.left + line.line_w / 2.0, pane.rect.top + 1.5),
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
                let (first, overlay) = renderer.text_overlay.unwrap();
                assert_eq!(
                    renderer.text[first].buffer.lines[0].text(),
                    "↓ 18446744073709551615 rows"
                );
                assert!(
                    text_areas(&renderer.text[..first], renderer.text_overlay).all(|area| {
                        let b = area.bounds;
                        b.right <= overlay.left
                            || b.left >= overlay.right
                            || b.bottom <= overlay.top
                            || b.top >= overlay.bottom
                    })
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
                    .push(eon_workspace_protocol::v26::PopupEntry {
                        id: "files".into(),
                        label: "Files".into(),
                        shortcut: eon_workspace_protocol::v26::PopupKey::Bound(
                            eon_workspace_protocol::v26::Shortcut {
                                modifiers: eon_workspace_protocol::v26::ALT,
                                key: "KeyF".into(),
                            },
                        ),
                    });
                snapshot.tabs[1]
                    .popups
                    .push(eon_workspace_protocol::v26::Popup {
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
                        |_, label| renderer.fit_tab_text(label, false),
                    );
                    renderer.text.clear();
                    let mut batch = RectangleBatch::new(1600, 900);
                    renderer.build_workspace(
                        &workspace,
                        WorkspaceFocus::Terminal,
                        &mut batch,
                        &[],
                        &[],
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
}
