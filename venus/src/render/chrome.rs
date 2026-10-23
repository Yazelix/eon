//! Eon workspace chrome drawing: tabs, pane headers, Eon Bar controls and quota chips, the
//! header tooltip and the native dialog, plus the pointer state that highlights them.

use super::{
    CellMetrics, CursorPoint, DEFAULT_BACKGROUND, DrawStyleKind, PlacedText, RectangleBatch,
    Renderer, SceneColor, shaping,
};
use crate::{
    DialogKind, DialogScene, SceneRect, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceScene,
    scene::{
        QuotaProvider, SIDEBAR_EMPTY, SIDEBAR_TITLE, TabBadge, inset_sides, pane_chrome_rect,
        sidebar_text_height,
    },
};
use eon_workspace_protocol::v19::AttentionState;
use glyphon::{
    Attrs, Buffer, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, TextBounds, Wrap,
};

const OPENAI_BLOSSOM_ID: u16 = 1;
const CLAUDE_SPLAT_ID: u16 = 2;
const QUOTA_ICON_SIZE: usize = 104;
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

/// The phases of a working badge's arc, each turned 45° clockwise from the one before.
const SPINNER_PHASES: usize = 8;
/// How much of the accent tints the agent sidebar row of the focused terminal (VEN-C30). It
/// follows the accent and becomes themeable with it (`eon-eon-themes-or3q`).
const CURRENT_ROW_TINT: f32 = 0.12;

/// A working badge's spinner within the static rectangle vertices: a tick rewrites only these
/// bytes, which keep their length in every phase. The badge lies wholly inside its clip, so the
/// rewrite needs none.
struct SpinnerSlot {
    offset: usize,
    badge: TabBadge,
}

/// The working badges in the current static vertices and the phase they are drawn at. On a
/// software adapter, where every turn re-rasterizes the whole frame, there is no phase: working
/// badges are drawn still and never tick (VEN-C21).
pub(super) struct Spinners {
    slots: Vec<SpinnerSlot>,
    phase: Option<usize>,
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
/// arc turned to `phase` for working, or at its first phase without one, and, in sidebar rows,
/// ○ idle and · unavailable. Quads keep the glyphs independent of fonts.
fn push_badge_glyph(rectangles: &mut RectangleBatch, badge: TabBadge, phase: Option<usize>) {
    // The ring yields entirely on the smallest badges, so the glyph never leaves the badge.
    let ring = (badge.rect.width / 12.0).floor();
    let glyph = SceneRect {
        left: badge.rect.left + ring,
        top: badge.rect.top + ring,
        width: badge.rect.width - ring * 2.0,
        height: badge.rect.height - ring * 2.0,
    };
    let muted = SceneColor {
        r: 162,
        g: 174,
        b: 190,
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
            SceneColor {
                r: 255,
                g: 196,
                b: 87,
            },
            alpha,
        ),
        AttentionState::Error => {
            let red = SceneColor {
                r: 255,
                g: 107,
                b: 107,
            };
            rectangles.push_stroke(at(0.1, 0.1), at(0.9, 0.9), stroke, red, alpha);
            rectangles.push_stroke(at(0.9, 0.1), at(0.1, 0.9), stroke, red, alpha);
        }
        AttentionState::Finished => {
            let green = SceneColor {
                r: 126,
                g: 231,
                b: 185,
            };
            rectangles.push_stroke(at(0.05, 0.55), at(0.4, 0.88), stroke, green, alpha);
            rectangles.push_stroke(at(0.4, 0.88), at(0.95, 0.15), stroke, green, alpha);
        }
        AttentionState::Working => {
            push_arc(rectangles, badge.rect, glyph, phase.unwrap_or(0), alpha)
        }
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
                    muted,
                    alpha,
                );
            }
        }
    }
}

/// Draws a working badge's arc: a 270° band inside `glyph` that trails its head, which points up
/// at phase 0 and turns clockwise. Every pixel the glyph touches inside `badge` is one 1x1 quad
/// whose alpha is the arc's coverage of 8x8 samples, kept at zero outside it, so each phase writes
/// the same vertices and only alpha turns. Pixels past `badge` stay undrawn, so its clip check
/// covers them.
fn push_arc(
    rectangles: &mut RectangleBatch,
    badge: SceneRect,
    glyph: SceneRect,
    phase: usize,
    alpha: f32,
) {
    const SAMPLES: u16 = 8;
    let blue = SceneColor {
        r: 138,
        g: 180,
        b: 255,
    };
    let center_x = glyph.left + glyph.width / 2.0;
    let center_y = glyph.top + glyph.height / 2.0;
    let outer = glyph.width / 2.0;
    let inner = (outer - (glyph.width * 0.2).max(1.6)).max(0.0);
    let (sin, cos) = (phase as f32 * std::f32::consts::TAU / SPINNER_PHASES as f32
        - std::f32::consts::FRAC_PI_2)
        .sin_cos();
    let left = glyph.left.floor().max(badge.left.ceil());
    let right = glyph.right().ceil().min(badge.right().floor());
    let top = glyph.top.floor().max(badge.top.ceil());
    let bottom = glyph.bottom().ceil().min(badge.bottom().floor());
    let mut y = top;
    while y < bottom {
        let mut x = left;
        while x < right {
            let mut covered = 0u16;
            for sample in 0..SAMPLES * SAMPLES {
                let dx = x + (f32::from(sample % SAMPLES) + 0.5) / f32::from(SAMPLES) - center_x;
                let dy = y + (f32::from(sample / SAMPLES) + 0.5) / f32::from(SAMPLES) - center_y;
                let radius = dx * dx + dy * dy;
                // The quarter gap lies clockwise of the head, between it and 90° past it.
                let gap = cos * dy - sin * dx > 0.0 && cos * dx + sin * dy > 0.0;
                if radius <= outer * outer && radius >= inner * inner && !gap {
                    covered += 1;
                }
            }
            let coverage = f32::from(covered) / f32::from(SAMPLES * SAMPLES);
            rectangles.push(x, y, 1.0, 1.0, blue, alpha * coverage);
            x += 1.0;
        }
        y += 1.0;
    }
}

impl Renderer {
    /// Turns every drawn arc one phase by rewriting only its own uploaded vertices, so the
    /// scene, text and terminal layers stay cached.
    pub fn turn_spinners(&mut self) {
        let Some(phase) = &mut self.spinners.phase else {
            return;
        };
        *phase = (*phase + 1) % SPINNER_PHASES;
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

pub(super) fn fit_header_text(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    text: &str,
    width: f32,
) -> (String, f32) {
    let mut buffer = Buffer::new(fonts, Metrics::new(metrics.font_size, metrics.height));
    buffer.set_size(None, Some(metrics.height));
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
    pub fn fit_tab_text(&mut self, label: &str) -> (String, f32) {
        let width = WorkspaceScene::tab_text_width(self.size(), self.metrics);
        fit_header_text(&mut self.fonts.font_system, self.metrics, label, width)
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
        let height = sidebar_text_height(self.metrics);
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
        fit_header_text(
            &mut self.fonts.font_system,
            self.metrics,
            label,
            f32::INFINITY,
        )
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
            SceneColor {
                r: 239,
                g: 244,
                b: 248,
            },
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
        rectangles.push_rounded(
            tooltip,
            padding / 2.0,
            SceneColor {
                r: 37,
                g: 49,
                b: 64,
            },
        );
    }

    pub(super) fn build_dialog(&mut self, viewer: &DialogScene, rectangles: &mut RectangleBatch) {
        let first = self.text.len();
        let padding = self.metrics.padding.min(viewer.bounds.width / 8.0);
        rectangles.clip = None;
        rectangles.push_rounded(
            viewer.bounds,
            padding / 2.0,
            SceneColor {
                r: 16,
                g: 22,
                b: 32,
            },
        );
        rectangles.push_rounded_outline(
            viewer.bounds,
            padding / 2.0,
            SceneColor {
                r: 58,
                g: 75,
                b: 91,
            },
        );
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
            SceneColor {
                r: 239,
                g: 244,
                b: 248,
            },
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
            SceneColor {
                r: 162,
                g: 174,
                b: 190,
            },
            DrawStyleKind::PopupLabel,
            viewer.bounds,
        );

        rectangles.clip = Some(viewer.content);
        let key_cells = match viewer.kind {
            DialogKind::Shortcuts => 15.0,
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
                    SceneColor {
                        r: 126,
                        g: 231,
                        b: 185,
                    },
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
                    SceneColor {
                        r: 162,
                        g: 204,
                        b: 230,
                    },
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
                    SceneColor {
                        r: 219,
                        g: 226,
                        b: 234,
                    },
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
                track.width / 2.0,
                SceneColor {
                    r: 43,
                    g: 53,
                    b: 67,
                },
            );
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
                SceneColor {
                    r: 126,
                    g: 231,
                    b: 185,
                },
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
                SceneColor {
                    r: 162,
                    g: 174,
                    b: 190,
                },
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
    ) {
        let accent = SceneColor {
            r: 126,
            g: 231,
            b: 185,
        };
        let selected = SceneColor {
            r: 28,
            g: 43,
            b: 58,
        };
        let idle = SceneColor {
            r: 16,
            g: 22,
            b: 32,
        };
        rectangles.push(
            workspace.header.left,
            workspace.header.top,
            workspace.header.width,
            workspace.header.height,
            DEFAULT_BACKGROUND,
            1.0,
        );
        if let Some(sidebar) = workspace.sidebar() {
            // The card's translucent background and outline are drawn with the pane cards'.
            let padding = self.metrics.padding;
            let line = self.metrics.height;
            let text_height = sidebar_text_height(self.metrics);
            let width = (sidebar.width - padding * 2.0).max(0.0);
            let bright = SceneColor {
                r: 239,
                g: 244,
                b: 248,
            };
            let muted = SceneColor {
                r: 162,
                g: 174,
                b: 190,
            };
            let dim = SceneColor {
                r: 104,
                g: 114,
                b: 128,
            };
            // The title takes one line; the empty state wraps below it.
            self.push_text_clipped(
                SIDEBAR_TITLE,
                sidebar.left + padding,
                sidebar.top + padding,
                width,
                width,
                text_height,
                bright,
                DrawStyleKind::Status(Wrap::None),
                sidebar,
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
                    muted,
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
                    accent,
                    list,
                    rectangles,
                );
            }
            for row in workspace.sidebar_rows() {
                if row.rect.intersection(list).is_none() {
                    continue;
                }
                let hovered = self.header.hovers(WorkspaceFocus::Sidebar, &row.target);
                // The faint "you are here" tint; a focused or hovered row's highlight covers it.
                if row.current {
                    rectangles.push_rounded_alpha(
                        row.rect,
                        padding / 2.0,
                        accent,
                        CURRENT_ROW_TINT,
                    );
                }
                if row.focused || hovered {
                    rectangles.push_rounded(row.rect, padding / 2.0, selected);
                }
                if row.focused {
                    rectangles.push_rounded_outline(row.rect, padding / 2.0, accent);
                }
                // A glyph on the translucent card needs no disc; on a highlight it matches it.
                let ring = (row.focused || hovered).then_some(selected);
                self.spinners.push_badge(row.glyph, ring, rectangles);
                if let Some((provider, rect)) = row.icon {
                    self.push_quota_icon(provider, rect, Some(list));
                }
                let (title, detail) = if row.glyph.dimmed {
                    (muted, dim)
                } else {
                    (bright, muted)
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
                        selected,
                    );
                }
                for (text, left, width, color) in [
                    (
                        hint.modifiers.as_str(),
                        hint.rect.left + self.metrics.padding / 2.0,
                        hint.key_offset - self.metrics.padding * 1.5,
                        SceneColor {
                            r: 255,
                            g: 107,
                            b: 107,
                        },
                    ),
                    (
                        hint.key.as_str(),
                        hint.rect.left + hint.key_offset,
                        hint.label_offset - hint.key_offset,
                        accent,
                    ),
                    (
                        hint.label.as_str(),
                        hint.rect.left + hint.label_offset,
                        hint.rect.width - hint.label_offset,
                        SceneColor {
                            r: 162,
                            g: 174,
                            b: 190,
                        },
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
            let radius = tab.rect.height / 2.0;
            let fill = if tab.selected {
                selected
            } else if self.header.hovers(WorkspaceFocus::Tabs, &tab.id) {
                SceneColor {
                    r: 23,
                    g: 34,
                    b: 46,
                }
            } else {
                idle
            };
            rectangles.push_rounded(tab.rect, radius, fill);
            if tab.selected && focus == WorkspaceFocus::Tabs {
                rectangles.push_rounded_outline(tab.rect, radius, accent);
            }
            let label_width = (tab.rect.width - self.metrics.padding * 2.0).max(1.0);
            self.push_text_clipped(
                tab.label(),
                tab.rect.left + self.metrics.padding,
                tab.rect.top + (tab.rect.height - self.metrics.height) / 2.0,
                label_width,
                label_width,
                self.metrics.height,
                if tab.selected {
                    SceneColor {
                        r: 239,
                        g: 244,
                        b: 248,
                    }
                } else {
                    SceneColor {
                        r: 162,
                        g: 174,
                        b: 190,
                    }
                },
                DrawStyleKind::Status(Wrap::None),
                workspace.tab_label_clip,
            );
            if let Some(badge) = tab.badge {
                self.spinners
                    .push_badge(badge, Some(DEFAULT_BACKGROUND), rectangles);
            }
        }
        for badge in &workspace.edge_badges {
            self.spinners
                .push_badge(*badge, Some(DEFAULT_BACKGROUND), rectangles);
        }
        rectangles.clip = None;
        // Quota chips and readings share one pill; the version label stays plain.
        let chip = SceneColor {
            r: 23,
            g: 34,
            b: 46,
        };
        let reading = SceneColor {
            r: 162,
            g: 174,
            b: 190,
        };
        for quota in &workspace.quotas {
            let visual = header_item_visual(quota.rect, self.metrics.padding);
            rectangles.push_rounded(visual, visual.height / 2.0, chip);
            self.push_quota_icon(quota.provider, quota.logo, None);
            let label_left = quota.logo.right() + self.metrics.padding / 2.0;
            let label_width = (quota.rect.right() - self.metrics.padding - label_left).max(0.0);
            self.push_text_clipped(
                &quota.label,
                label_left,
                quota.rect.top + (quota.rect.height - self.metrics.height).max(0.0) / 2.0,
                label_width,
                label_width,
                self.metrics.height.min(quota.rect.height),
                reading,
                DrawStyleKind::Status(Wrap::None),
                quota.rect,
            );
        }
        for (label, color, pill) in [
            (&workspace.cpu, reading, true),
            (&workspace.ram, reading, true),
            (&workspace.clock, reading, true),
            (
                &workspace.version,
                SceneColor {
                    r: 124,
                    g: 196,
                    b: 216,
                },
                false,
            ),
        ] {
            let Some(label) = label else {
                continue;
            };
            if pill {
                let visual = header_item_visual(label.rect, self.metrics.padding);
                rectangles.push_rounded(visual, visual.height / 2.0, chip);
            }
            let width = (label.rect.width - self.metrics.padding * 2.0).max(0.0);
            // A label narrower than its chip sits in the middle; every label is ASCII in cells.
            let slack = (width - label.label.len() as f32 * self.metrics.width).max(0.0);
            self.push_text_clipped(
                &label.label,
                label.rect.left + self.metrics.padding + slack / 2.0,
                label.rect.top + (label.rect.height - self.metrics.height).max(0.0) / 2.0,
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
            let visual = header_item_visual(control.rect, self.metrics.padding);
            let hovered = self.header.hovers(WorkspaceFocus::Header(control.kind), "");
            let pressed = self.header.pressed_header == Some(control.kind);
            let focused = focus == WorkspaceFocus::Header(control.kind);
            let fill = if pressed {
                selected
            } else if hovered {
                SceneColor {
                    r: 23,
                    g: 34,
                    b: 46,
                }
            } else {
                DEFAULT_BACKGROUND
            };
            rectangles.push_rounded(visual, visual.height / 2.0, fill);
            if focused {
                rectangles.push_rounded_outline(visual, visual.height / 2.0, accent);
            }
            let glyph_width = self.metrics.width.min(control.rect.width);
            self.push_text_clipped(
                control.kind.glyph(),
                control.rect.left + (control.rect.width - glyph_width).max(0.0) / 2.0,
                control.rect.top + (control.rect.height - self.metrics.height).max(0.0) / 2.0,
                glyph_width,
                glyph_width,
                self.metrics.height.min(control.rect.height),
                if pressed || focused {
                    SceneColor {
                        r: 239,
                        g: 244,
                        b: 248,
                    }
                } else {
                    SceneColor {
                        r: 162,
                        g: 174,
                        b: 190,
                    }
                },
                DrawStyleKind::Status(Wrap::None),
                control.rect,
            );
        }
        for (panes, selected_column) in workspace.column_panes() {
            for pane in panes {
                // Only the selected column's expanded header shows selection and scrollback.
                let current = pane.selected && selected_column;
                let Some(rect) = pane.rect.intersection(workspace.pane_viewport) else {
                    continue;
                };
                // Framed fills, the header's base included, follow their card's corners.
                let card = SceneRect {
                    left: pane.rect.left,
                    width: pane.rect.width,
                    ..workspace.pane_viewport
                };
                rectangles.clip = Some(rect);
                if self.pane_frames {
                    rectangles.push_rounded(card, workspace.card_radius(), DEFAULT_BACKGROUND);
                } else {
                    rectangles.push(
                        rect.left,
                        rect.top,
                        rect.width,
                        rect.height,
                        DEFAULT_BACKGROUND,
                        1.0,
                    );
                }
                if pane.selected || self.header.hovers(WorkspaceFocus::Panes, &pane.id) {
                    // Frameless fills keep their own rounded, inset shape.
                    let shape = if self.pane_frames {
                        card
                    } else {
                        inset_sides(
                            pane_chrome_rect(
                                SceneRect {
                                    left: pane.rect.left,
                                    width: pane.rect.width,
                                    ..workspace.pane_viewport
                                },
                                self.metrics,
                            ),
                            self.metrics,
                        )
                    };
                    rectangles.push_rounded(
                        shape,
                        if self.pane_frames {
                            workspace.card_radius()
                        } else {
                            self.metrics.padding
                        },
                        if current { selected } else { idle },
                    );
                }
                rectangles.clip = None;
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
                let width = pane.rect.width - self.metrics.padding * 2.0 - scrollback_width;
                let label = (scrollback_width > 0.0).then(|| {
                    fit_header_text(
                        &mut self.fonts.font_system,
                        self.metrics,
                        pane.label(),
                        width,
                    )
                    .0
                });
                self.push_text_clipped(
                    label.as_deref().unwrap_or(pane.label()),
                    pane.rect.left + self.metrics.padding,
                    pane.rect.top + (pane.rect.height - self.metrics.height) / 2.0,
                    width,
                    width,
                    self.metrics.height,
                    if current {
                        SceneColor {
                            r: 239,
                            g: 244,
                            b: 248,
                        }
                    } else if !pane.live {
                        SceneColor {
                            r: 221,
                            g: 126,
                            b: 126,
                        }
                    } else {
                        SceneColor {
                            r: 162,
                            g: 174,
                            b: 190,
                        }
                    },
                    DrawStyleKind::Status(Wrap::None),
                    workspace.pane_viewport,
                );
            }
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
            QuotaProvider::Codex => (OPENAI_BLOSSOM_ID, Color::rgb(255, 255, 255)),
            QuotaProvider::Claude => (CLAUDE_SPLAT_ID, Color::rgb(217, 119, 87)),
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
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use crate::EonBar;
    use glyphon::cosmic_text::SubpixelBin;

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
    fn spinner_phases_rewrite_only_alpha_in_place() {
        // A tick rewrites without the tab viewport's clip, which leaves a drawn badge unchanged,
        // down to the one-pixel badge of the smallest typography, at the viewport's edge.
        let viewport = Some(SceneRect {
            width: 240.0,
            height: 40.0,
            ..SceneRect::default()
        });
        for size in [12.0, 1.0] {
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
            for phase in 0..SPINNER_PHASES {
                let turned = glyph(phase, None);
                assert_eq!(turned, glyph(phase, viewport));
                assert_eq!(turned.len(), first.len());
                // Each vertex is x, y, r, g, b, alpha: only alpha turns.
                for (vertex, first) in turned.chunks_exact(6).zip(first.chunks_exact(6)) {
                    assert_eq!(vertex[..5], first[..5]);
                }
                // A one-pixel arc covers its pixel equally at every quarter turn.
                if size > 1.0 {
                    assert_eq!(phase == 0, turned == first);
                }
            }
        }
    }

    #[test]
    fn working_badges_draw_an_antialiased_open_arc() {
        // The badges of the default typography at scales 1 and 2, as rows of coverage per pixel.
        let coverage = |size: f32, phase| {
            let badge = TabBadge {
                state: AttentionState::Working,
                dimmed: false,
                rect: SceneRect {
                    left: 4.0,
                    top: 4.0,
                    width: size,
                    height: size,
                },
            };
            let mut rectangles = RectangleBatch::new(800, 600);
            push_badge_glyph(&mut rectangles, badge, phase);
            // Each pixel is one quad of six vertices of x, y, r, g, b, alpha.
            let alphas = rectangles
                .bytes
                .chunks_exact(24 * 6)
                .map(|quad| f32::from_ne_bytes(quad[20..24].try_into().unwrap()))
                .collect::<Vec<_>>();
            let side = (alphas.len() as f32).sqrt() as usize;
            assert_eq!(side * side, alphas.len());
            alphas.chunks(side).map(<[f32]>::to_vec).collect::<Vec<_>>()
        };
        let art = |rows: Vec<Vec<f32>>| {
            rows.iter()
                .map(|row| {
                    row.iter()
                        .map(|alpha| [' ', '.', '+', '*', '#'][(alpha * 4.0).round() as usize])
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        // Phase 0, also the still form, opens a quarter between 12 and 3 o'clock.
        assert_eq!(
            art(coverage(12.0, None)),
            [
                "  .*#     ",
                " +###     ",
                ".##+      ",
                "*#+       ",
                "##        ",
                "##      ##",
                "*#+    +#*",
                ".##+  +##.",
                " +######+ ",
                "  .*##*.  ",
            ]
        );
        assert_eq!(
            art(coverage(24.0, None)),
            [
                "      .*##          ",
                "    .#####          ",
                "   +######          ",
                "  +#######          ",
                " .#####+.           ",
                " #####.             ",
                ".####.              ",
                "*###+               ",
                "####.               ",
                "####                ",
                "####            ####",
                "####.          .####",
                "*###+          +###*",
                ".####.        .####.",
                " #####.      .##### ",
                " .#####+.  .+#####. ",
                "  +##############+  ",
                "   +############+   ",
                "    .##########.    ",
                "      .*####*.      ",
            ]
        );
        // Every second phase turns the arc a quarter clockwise.
        for size in [12.0, 24.0] {
            assert_eq!(coverage(size, None), coverage(size, Some(0)));
            for phase in (0..SPINNER_PHASES).step_by(2) {
                let rows = coverage(size, Some(phase));
                let side = rows.len();
                let turned = (0..side)
                    .map(|y| (0..side).map(|x| rows[side - 1 - x][y]).collect())
                    .collect::<Vec<Vec<f32>>>();
                assert_eq!(turned, coverage(size, Some((phase + 2) % SPINNER_PHASES)));
            }
        }
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
        use eon_workspace_protocol::v19::{Pane, Snapshot, Tab};
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
                    None,
                    true,
                    FontSetup::new(&FontSettings::default()).unwrap(),
                ))
                .unwrap();
                let snapshot = Snapshot {
                    active_tab: "t2".into(),
                    geometry: eon_workspace_protocol::v19::PopupGeometry {
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
                            columns: vec![eon_workspace_protocol::v19::Column {
                                panes: vec![Pane {
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
                    sidebar: eon_workspace_protocol::v19::Sidebar::Open,
                };
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                renderer.build_workspace(
                    &workspace,
                    WorkspaceFocus::Terminal,
                    &mut RectangleBatch::new(900, 600),
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
                    id: "p3".into(),
                    session: "s3".into(),
                    endpoint: b"/tmp/p3.sock".to_vec(),
                    live: false,
                    attention: None,
                });
                snapshot.tabs[1].columns[0].panes[0].live = false;
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
                    pane_color("p2 offline"),
                    pane_color("p3 offline"),
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
                let metadata = crate::scene::PaneMetadata::Available {
                    working_directory: format!("/{}mountain/leaf", "W".repeat(46)),
                };
                let workspace = WorkspaceScene::from_snapshot_with_metadata(
                    &snapshot,
                    EonBar::default(),
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_| Some(&metadata),
                    |_, label| renderer.fit_tab_text(label),
                    None,
                );
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
                assert!(line.text.ends_with("/leaf"));
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
