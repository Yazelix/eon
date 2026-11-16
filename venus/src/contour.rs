//! Venus-owned title fitting, contours, coverage and header bands shared by every host.

use crate::render::{shaping, uncovered};
use crate::scene::{
    SIDEBAR_TITLE, WorkspacePane,
    tokens::{self, card_gap, pill_gap},
};
use crate::{CellMetrics, DialogScene, PanelHeading, SceneRect, WorkspaceScene};
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, SwashCache, Wrap};
use std::ops::Range;

/// The single outer shape shared by a card's backdrop and terminal paint.
#[derive(Clone, Copy)]
pub(crate) struct CardFill {
    pub bounds: SceneRect,
    pub radius: f32,
}

/// A rounded frame and the part of it that belongs to one stacked edge and its joins.
pub(crate) struct PaneOutline {
    pub bounds: SceneRect,
    pub clip: SceneRect,
    pub radius: f32,
    pub title_y: Option<f32>,
    pub expanded: bool,
}

#[derive(Clone)]
pub(crate) struct FrameTitle {
    pub(crate) label: String,
    pub(crate) width: f32,
}

#[derive(Clone)]
pub(crate) struct FrameTitleTab {
    pub(crate) heading: PanelHeading,
    pub(crate) top: f32,
    pub(crate) start: f32,
    pub(crate) left_radius: f32,
    pub(crate) row: SceneRect,
    pub(crate) return_run: f32,
    pub(crate) title: FrameTitle,
    /// `start` of the mirrored action notch at the card's right, on headings that hold a button.
    pub(crate) action: Option<f32>,
}

#[derive(Clone)]
pub(crate) struct PaneTitleTab {
    pub(crate) card: CardFill,
    pub(crate) tab: FrameTitleTab,
}

// Fill and stroke share the contour, including the tangent return into the top rule.
pub(crate) fn title_tab_bands(card: CardFill, tab: &FrameTitleTab, inset: f32) -> Vec<SceneRect> {
    if tab.heading == PanelHeading::Connected {
        let cut = card.bounds.top + rounded_radius(card.bounds, card.radius);
        return CardFill {
            bounds: SceneRect {
                left: card.bounds.left + inset,
                top: tab.top + inset,
                width: (card.bounds.width - inset * 2.0).max(0.0),
                height: (card.bounds.bottom() - tab.top - inset * 2.0).max(0.0),
            },
            radius: (rounded_radius(card.bounds, card.radius) - inset).max(0.0),
        }
        .bands()
        // Outline sampling emits one raster row per band above the existing join.
        .flat_map(|band| {
            let rows = (cut - band.top).clamp(0.0, band.height) as u32;
            (0..rows)
                .map(move |row| SceneRect {
                    top: band.top + row as f32,
                    height: 1.0,
                    ..band
                })
                .chain((band.height > rows as f32).then_some(SceneRect {
                    top: band.top + rows as f32,
                    height: band.height - rows as f32,
                    ..band
                }))
        })
        .collect();
    }
    let (mut bands, body) = notch_bands(card, tab, tab.start, inset);
    if let Some(start) = tab.action {
        // The action notch is the title notch mirrored about the card's centre. Each row keeps
        // the title's band and adds only what the mirrored band reaches beyond it, starting at
        // that band's own right edge, so no pixel blends twice.
        let mirror = card.bounds.left + card.bounds.right();
        let (action, _) = notch_bands(card, tab, start, inset);
        let pieces = bands
            .iter()
            .zip(action)
            .filter_map(|(band, action)| {
                let left = (mirror - action.right()).max(band.right());
                let right = mirror - action.left;
                (right > left).then_some(SceneRect {
                    left,
                    width: right - left,
                    ..*band
                })
            })
            .collect::<Vec<_>>();
        bands.extend(pieces);
    }
    bands.extend(body);
    bands
}

/// The rows of one notch of `tab` whose flat top ends at `start`, down to where the card's
/// own corner ends, and the card's bands below them.
fn notch_bands(
    card: CardFill,
    tab: &FrameTitleTab,
    start: f32,
    inset: f32,
) -> (Vec<SceneRect>, Vec<SceneRect>) {
    let bounds = SceneRect {
        left: card.bounds.left + inset,
        top: card.bounds.top + inset,
        width: (card.bounds.width - inset * 2.0).max(0.0),
        height: (card.bounds.height - inset * 2.0).max(0.0),
    };
    let radius = (rounded_radius(card.bounds, card.radius) - inset).max(0.0);
    let left_radius = (tab.left_radius - inset).max(0.0);
    let top = tab.top + inset;
    let return_start = start;
    let cut = bounds.top + radius;
    let circle =
        |radius: f32, distance: f32| (radius * radius - distance * distance).max(0.0).sqrt();
    let mut bands = Vec::new();
    for row in top.floor() as i32..cut.ceil() as i32 {
        let y = row as f32 + 0.5;
        let left = if y < top + left_radius {
            bounds.left + left_radius - circle(left_radius, top + left_radius - y)
        } else {
            bounds.left
        };
        let end = if y >= bounds.top {
            bounds.right() - radius + circle(radius, bounds.top + radius - y)
        } else {
            let rise = card.bounds.top - tab.top;
            let radius = (tab.return_run.powi(2) + rise.powi(2)) / (4.0 * rise);
            let join_y = tab.top + rise / 2.0 + inset * (1.0 - rise / (2.0 * radius));
            if y < join_y {
                return_start + circle((radius - inset).max(0.0), tab.top + radius - y)
            } else {
                return_start + tab.return_run
                    - circle(radius + inset, y - (card.bounds.top - radius))
            }
        };
        bands.push(SceneRect {
            left,
            top: row as f32,
            width: (end - left).max(0.0),
            height: 1.0,
        });
    }
    let body = CardFill { bounds, radius }
        .bands()
        .filter(|band| band.top >= cut)
        .collect();
    (bands, body)
}

pub(crate) fn title_tab_outline_rects(
    card: CardFill,
    tab: &FrameTitleTab,
    width: f32,
) -> Vec<(SceneRect, f32)> {
    let inner = title_tab_bands(card, tab, width);
    let cut = card.bounds.top + rounded_radius(card.bounds, card.radius);
    title_tab_bands(card, tab, 0.0)
        .into_iter()
        .filter(|band| band.top < cut)
        .flat_map(|band| uncovered(band, &inner))
        .flat_map(|piece| span_rects(piece.left, piece.right(), piece.top))
        .collect()
}

pub(crate) fn physical_outline_width(logical_width: u8, scale: f32) -> f32 {
    (f32::from(logical_width) * scale).round().max(1.0)
}

/// Each pixel row of a rounded rectangle's top and bottom corner bands, with how far the curve
/// sits in from the sides at that row.
pub(crate) fn corner_rows(rect: SceneRect, radius: f32) -> impl Iterator<Item = (f32, f32)> {
    (0..radius as u32).flat_map(move |row| {
        let dy = radius - row as f32 - 0.5;
        let inset = radius - (radius * radius - dy * dy).sqrt();
        [
            (rect.top + row as f32, inset),
            (rect.bottom() - row as f32 - 1.0, inset),
        ]
    })
}

impl CardFill {
    pub(crate) fn middle(self) -> SceneRect {
        let radius = rounded_radius(self.bounds, self.radius);
        SceneRect {
            top: self.bounds.top + radius,
            height: self.bounds.height - radius * 2.0,
            ..self.bounds
        }
    }

    pub(crate) fn bands(self) -> impl Iterator<Item = SceneRect> {
        let rect = self.bounds;
        std::iter::once(self.middle()).chain(
            corner_rows(rect, rounded_radius(rect, self.radius)).map(move |(top, inset)| {
                SceneRect {
                    left: rect.left + inset,
                    top,
                    width: rect.width - inset * 2.0,
                    height: 1.0,
                }
            }),
        )
    }
}

pub(crate) fn span_rects(
    left: f32,
    right: f32,
    top: f32,
) -> impl Iterator<Item = (SceneRect, f32)> {
    let parts = if left.floor() == right.floor() {
        [
            (left.floor(), 1.0, right - left),
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 0.0),
        ]
    } else {
        [
            (left.floor(), 1.0, left.ceil() - left),
            (left.ceil(), right.floor() - left.ceil(), 1.0),
            (right.floor(), 1.0, right - right.floor()),
        ]
    };
    parts
        .into_iter()
        .filter(|(_, width, alpha)| *width > 0.0 && *alpha > 0.0)
        .map(move |(left, width, alpha)| {
            (
                SceneRect {
                    left,
                    top,
                    width,
                    height: 1.0,
                },
                alpha,
            )
        })
}

pub(crate) fn rounded_outline_rects(
    rect: SceneRect,
    radius: f32,
    width: f32,
) -> Vec<(SceneRect, f32)> {
    let radius = rounded_radius(rect, radius);
    if rect.width <= width * 2.0 || rect.height <= width * 2.0 {
        let fill = CardFill {
            bounds: rect,
            radius,
        };
        let mut rects = vec![(fill.middle(), 1.0)];
        for (top, inset) in corner_rows(rect, radius) {
            rects.extend(span_rects(rect.left + inset, rect.right() - inset, top));
        }
        return rects;
    }
    let mut rects = Vec::new();
    for left in [rect.left, rect.right() - width] {
        rects.push((
            SceneRect {
                left,
                top: rect.top + radius,
                width,
                height: rect.height - radius * 2.0,
            },
            1.0,
        ));
    }
    if width > radius {
        for top in [rect.top + radius, rect.bottom() - width] {
            rects.push((
                SceneRect {
                    left: rect.left + width,
                    top,
                    width: rect.width - width * 2.0,
                    height: width - radius,
                },
                1.0,
            ));
        }
    }
    for row in 0..radius as u32 {
        let dy = radius - row as f32 - 0.5;
        let outer = radius - (radius * radius - dy * dy).sqrt();
        for top in [rect.top + row as f32, rect.bottom() - row as f32 - 1.0] {
            if (row as f32) < width {
                rects.extend(span_rects(rect.left + outer, rect.right() - outer, top));
            } else {
                let inner = radius - ((radius - width).powi(2) - dy * dy).sqrt();
                rects.extend(span_rects(rect.left + outer, rect.left + inner, top));
                rects.extend(span_rects(rect.right() - inner, rect.right() - outer, top));
            }
        }
    }
    rects
}

/// `glyph` shaped in the status font, and its ink box from an origin at zero in whole physical
/// pixels: left, top, right and bottom. A glyph without ink gives its layout box.
pub(crate) fn status_glyph_ink(
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
    let ink = buffer_ink(font_system, swash_cache, &buffer).unwrap_or((
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

pub(crate) fn buffer_ink(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    buffer: &Buffer,
) -> Option<(i32, i32, i32, i32)> {
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
    ink
}

/// The gap a heading's button leaves in its card's top edge where no notch holds it: its
/// target and half a padding each side, the margin a compact title's gap keeps around its text.
pub(crate) fn heading_button_gap(target: SceneRect, metrics: CellMetrics) -> SceneRect {
    SceneRect {
        left: target.left - title_margin(metrics),
        width: target.width + metrics.padding,
        ..target
    }
}

impl FrameTitle {
    pub(crate) fn fit(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        label: &str,
        available: f32,
    ) -> Self {
        let (label, width) = fit_header_text(fonts, metrics, label, available);
        Self { label, width }
    }

    pub(crate) fn ink_width(&self, metrics: CellMetrics, stroke: f32) -> f32 {
        self.width + (title_margin(metrics) - stroke).max(0.0)
    }

    pub(crate) fn border_gap(&self, row: SceneRect, metrics: CellMetrics) -> SceneRect {
        SceneRect {
            left: row.left + title_margin(metrics),
            top: row.top + 1.0,
            width: self.width + metrics.padding,
            height: (row.height - 2.0).max(0.0),
        }
    }
}

impl FrameTitleTab {
    pub(crate) fn with_heading(mut self, heading: PanelHeading) -> Self {
        self.heading = heading;
        self
    }

    pub(crate) fn popup(
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
        card: CardFill,
        frame_top: f32,
        stroke: f32,
        action: Option<SceneRect>,
    ) -> Option<Self> {
        let extra = title_tab_rise(metrics).max(stroke + 1.0);
        let top = (row.top - extra).floor().max(frame_top.ceil() + 1.0);
        if top + stroke + 1.0 > row.top {
            return None;
        }
        let Some(target) = action else {
            return Self::at(fonts, metrics, label, row, card, top, row.top, stroke);
        };
        // Both notches return to the top edge over the same run; flat edge stays between them.
        let return_run = title_tab_return(card.bounds.top - top);
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
            + title_margin(metrics);
        tab.action = Some(
            card.bounds.left + card.bounds.right() - target.left + title_margin(metrics)
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
        card: CardFill,
        top: f32,
        ink_top: f32,
        stroke: f32,
    ) -> Option<Self> {
        if top + stroke + 1.0 > ink_top || top >= card.bounds.top {
            return None;
        }
        let rise = card.bounds.top - top;
        let join_radius = (card.radius / 2.0).min(rise / 2.0);
        let return_run = title_tab_return(rise);
        let right_padding = title_margin(metrics);
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
            .min(card.bounds.right() - rounded_radius(card.bounds, card.radius))
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
            heading: PanelHeading::Notch,
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
        let card = CardFill {
            bounds: workspace.card_rect(area, metrics, card_stroke),
            radius: workspace.card_radius(),
        };
        if card.bounds.top != title_rule_y(row.top, row.height) {
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

    pub(crate) fn sidebar(
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

    pub(crate) fn dialog(
        fonts: &mut FontSystem,
        metrics: CellMetrics,
        viewer: &DialogScene,
        stroke: f32,
    ) -> Option<(CardFill, Self)> {
        let height = chrome_title_metrics(metrics).text_height;
        let row = SceneRect {
            // Keep the lower chrome title inside the existing heading band, clear of bar chips.
            top: (viewer.title.bottom() - height).ceil(),
            height,
            ..viewer.bounds
        };
        let top = title_rule_y(row.top, row.height);
        if top >= viewer.bounds.bottom() {
            return None;
        }
        let card = CardFill {
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
    pub(crate) fn workspace(
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
    pub(crate) fn bands(&self, inset: f32) -> Vec<SceneRect> {
        let cut = self.card.bounds.top + rounded_radius(self.card.bounds, self.card.radius);
        title_tab_bands(self.card, &self.tab, inset)
            .into_iter()
            .map(|mut band| {
                if band.top < cut {
                    band.width = (self.card.bounds.left + self.card.bounds.right()
                        - 2.0 * band.left)
                        .max(0.0);
                }
                band
            })
            .collect()
    }

    pub(crate) fn outline_rects(&self, width: f32) -> Vec<(SceneRect, f32)> {
        let inner = self.bands(width);
        let replacement = self.replacement();
        self.bands(0.0)
            .into_iter()
            .filter_map(|band| band.intersection(replacement))
            .flat_map(|band| uncovered(band, &inner))
            .flat_map(|piece| span_rects(piece.left, piece.right(), piece.top))
            .collect()
    }

    pub(crate) fn replacement(&self) -> SceneRect {
        SceneRect {
            top: self.tab.top,
            height: self.card.bounds.top + rounded_radius(self.card.bounds, self.card.radius)
                - self.tab.top,
            ..self.card.bounds
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fit(
        fonts: &mut FontSystem,
        swash: &mut SwashCache,
        metrics: CellMetrics,
        pane: &WorkspacePane,
        above: Option<&WorkspacePane>,
        outline: &PaneOutline,
        viewport: SceneRect,
        right: f32,
        stroke: f32,
    ) -> Option<Self> {
        let row = pane.rect;
        let card = CardFill {
            bounds: outline.bounds,
            radius: outline.radius,
        };
        let mut ink = |pane: &WorkspacePane| {
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
        let rise = title_tab_rise(metrics);
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

    pub(crate) fn workspace(
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
                let outlines = pane_outlines(
                    workspace,
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
                        let top = (workspace.header_viewport.top - title_tab_rise(metrics))
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
pub(crate) fn pane_rule_gaps(
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

pub(crate) fn fit_header_text(
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

pub(crate) fn bottom_title_row(row: SceneRect, bottom: f32, text_height: f32) -> SceneRect {
    SceneRect {
        top: (bottom - row.height / 2.0)
            .min(row.top)
            .min(row.bottom() - (row.height + text_height) / 2.0),
        ..row
    }
}

pub(crate) fn rounded_radius(rect: SceneRect, radius: f32) -> f32 {
    radius
        .min(rect.width / 2.0)
        .min(rect.height / 2.0)
        .max(0.0)
        .floor()
}

fn grid_safe_bottom(bottom: f32, grid_bottom: Option<f32>, stroke_width: f32) -> f32 {
    grid_bottom.map_or(bottom, |grid| bottom.max(grid.ceil() + stroke_width))
}

pub(crate) fn painted_bottom(
    bottom: f32,
    metrics: CellMetrics,
    stroke_width: f32,
    grid_bottom: Option<f32>,
) -> f32 {
    let inset =
        (chrome_title_metrics(metrics).text_height / 2.0).min(metrics.padding - stroke_width);
    grid_safe_bottom((bottom - inset).floor(), grid_bottom, stroke_width)
}

pub(crate) fn pane_outlines(
    workspace: &WorkspaceScene,
    panes: &[WorkspacePane],
    terminal: SceneRect,
    fill: CardFill,
    border_width: f32,
    metrics: CellMetrics,
    stroke_width: f32,
) -> Vec<PaneOutline> {
    let viewport = workspace.pane_viewport;
    let Some(expanded) = panes.iter().position(|pane| pane.selected) else {
        return Vec::new();
    };
    let grid_bottom = workspace.grid_bottom(terminal, metrics);
    let mut expanded_frame = panes[expanded].frame(terminal);
    let next = panes.get(expanded + 1);
    // The bottom stroke can use the space before the next header, including the reserve
    // after a first pane's terminal.
    let bottom_limit = next.map_or(expanded_frame.bottom(), |next| next.rect.top);
    // Scrolling may leave no room for a bottom stroke inside the fixed card.
    let bottom_limit = grid_safe_bottom(
        bottom_limit.min(fill.bounds.bottom()),
        grid_bottom,
        stroke_width,
    );
    let bottom = next.map_or(bottom_limit, |next| {
        let target = title_rule_y(next.rect.top, next.rect.height) - next.rect.height;
        target
            .max(painted_bottom(
                terminal.bottom(),
                metrics,
                stroke_width,
                grid_bottom,
            ))
            .min(bottom_limit)
    });
    expanded_frame.height = (bottom - expanded_frame.top).max(0.0);
    let mut outlines = Vec::with_capacity(panes.len());
    for (index, pane) in panes.iter().enumerate() {
        let mut frame = pane.frame(terminal);
        if pane.selected {
            frame = expanded_frame;
        } else if index + 1 == panes.len() {
            // Move paint with the scrolled row, keeping its allocated title band unchanged.
            frame.top += fill.bounds.bottom()
                - title_rule_y(viewport.bottom() - pane.rect.height, pane.rect.height);
        }
        let radius = rounded_radius(frame, fill.radius);
        let (bounds, clip) = if pane.selected {
            (frame, frame)
        } else if index < expanded {
            let next = if index + 1 == expanded {
                expanded_frame
            } else {
                panes[index + 1].frame(terminal)
            };
            let join = next.top + rounded_radius(next, fill.radius);
            (
                SceneRect {
                    height: join + radius - frame.top,
                    ..frame
                },
                SceneRect {
                    height: join - frame.top,
                    ..frame
                },
            )
        } else {
            let previous = &panes[index - 1];
            let previous_frame = if previous.selected {
                expanded_frame
            } else {
                previous.frame(terminal)
            };
            let join = if previous.selected {
                previous_frame.bottom()
            } else {
                previous_frame.top + border_width
            } - rounded_radius(previous_frame, fill.radius);
            let bottom = if index + 1 == panes.len() {
                frame.top
            } else {
                frame.top + border_width
            };
            (
                SceneRect {
                    top: join - radius,
                    height: bottom - join + radius,
                    ..frame
                },
                SceneRect {
                    top: join,
                    height: bottom - join,
                    ..frame
                },
            )
        };
        outlines.push(PaneOutline {
            bounds,
            // Scrolled headers pass under the Stack band.
            clip: clip
                .intersection(workspace.header_viewport)
                .unwrap_or(SceneRect {
                    height: 0.0,
                    ..clip
                }),
            radius,
            title_y: Some(if index + 1 == panes.len() && !pane.selected {
                frame.top - border_width
            } else {
                frame.top
            }),
            expanded: pane.selected,
        });
    }
    outlines
}

/// The column card's segment above `first`, its first pane's outline: the Stack tab's top
/// line and the sides down to where that pane's top corners turn into them, or to the header
/// viewport's top while that pane is scrolled under the Stack band.
pub(crate) fn stack_outline(
    workspace: &WorkspaceScene,
    first: &PaneOutline,
    fill: CardFill,
) -> PaneOutline {
    let top = fill.bounds.top;
    let join = (first.bounds.top + first.radius)
        .max(workspace.header_viewport.top)
        .max(top);
    let radius = rounded_radius(
        SceneRect {
            height: join - top + fill.radius,
            ..fill.bounds
        },
        fill.radius,
    );
    PaneOutline {
        bounds: SceneRect {
            height: join - top + radius,
            ..fill.bounds
        },
        clip: SceneRect {
            height: join - top,
            ..fill.bounds
        },
        radius,
        title_y: Some(top),
        expanded: false,
    }
}

pub(crate) fn header_heights(metrics: CellMetrics) -> (f32, f32) {
    // Preserve header allocation independently of the painted bar band.
    let row = pill_row(metrics);
    (
        row + (card_gap(metrics) - tab_gap(metrics, row)).max(0.0) * 2.0,
        chrome_title_metrics(metrics).text_height,
    )
}

/// The row of tab pills and header buttons: the pills plus one tab gap above and below them.
pub(crate) fn pill_row(metrics: CellMetrics) -> f32 {
    (metrics.height * 1.75).round().max(1.0)
}

/// The pill row inside a header `tab_height` tall, which yields only after the header's inset.
pub(crate) fn tab_row(metrics: CellMetrics, tab_height: f32) -> f32 {
    pill_row(metrics).min(tab_height)
}

/// The header's height in a window `height` tall whose content above any hints ends at
/// `content_height`: the header's extra inset yields first, keeping `minimum_height` for the
/// cards, and the pill row only when the window itself is shorter.
pub(crate) fn header_height(
    metrics: CellMetrics,
    height: f32,
    content_height: f32,
    minimum_height: f32,
) -> f32 {
    header_heights(metrics)
        .0
        .min((content_height - minimum_height).max(pill_row(metrics)))
        .min(height)
}

/// The space between neighbouring tab pills.
pub(crate) fn tab_gap(metrics: CellMetrics, tab_height: f32) -> f32 {
    pill_gap(metrics).min(tab_row(metrics, tab_height) / 4.0)
}

/// The tab hit targets' inset: one card gap at full header height. The first tab still lines
/// up with the first card horizontally; its painted vertical band is independent.
pub(crate) fn tab_inset(metrics: CellMetrics, tab_height: f32) -> f32 {
    tab_gap(metrics, tab_height) + (tab_height - tab_row(metrics, tab_height)) / 2.0
}

/// The tab pills' height in a header with room for its full height.
fn nominal_pill_height(metrics: CellMetrics) -> f32 {
    header_heights(metrics).0 - (chrome_title_metrics(metrics).text_height / 2.0).floor()
}

/// The painted bar band, independent of its hover and accessibility rectangles.
pub(crate) fn bar_pill_band(metrics: CellMetrics, tab_height: f32, card_top: f32) -> Range<f32> {
    let end = card_top.min(header_heights(metrics).0);
    let height = nominal_pill_height(metrics).min(end);
    if height >= metrics.height {
        end - height..end
    } else {
        // Keep the old tiny-window band, clipped to the space left above the cards.
        let inset = tab_inset(metrics, tab_height);
        let end = (tab_height - inset).min(card_top);
        inset.min(end)..end
    }
}

/// The bar's edge: how far the painted band's centre lies below the window's top. The close
/// glyph's centre lies as far from the window's right edge, and the Eon icon's from its left.
pub(crate) fn bar_edge(metrics: CellMetrics, tab_height: f32, card_top: f32) -> f32 {
    let band = bar_pill_band(metrics, tab_height, card_top);
    (band.start + band.end) / 2.0
}

/// Give the cap one snapped title pitch, crediting the rise already supplied by the Stack band.
fn band_cap_clearance(metrics: CellMetrics, provided_rise: f32) -> f32 {
    let title_height = chrome_title_metrics(metrics).text_height;
    (title_height.floor() - title_height / 2.0 + (title_tab_rise(metrics) - provided_rise)).ceil()
}

pub(crate) fn first_band_clearance(metrics: CellMetrics) -> f32 {
    band_cap_clearance(metrics, title_tab_rise(metrics))
}

pub(crate) fn band_cap_space(metrics: CellMetrics, panes: usize) -> f32 {
    match panes {
        0 => 0.0,
        1 => first_band_clearance(metrics),
        _ => band_cap_clearance(metrics, 0.0),
    }
}

/// The space between the bar's pills and the card areas, which puts the title tabs' caps one card
/// gap below the pills.
pub(crate) fn bar_card_margin(metrics: CellMetrics) -> f32 {
    card_gap(metrics) + title_tab_rise(metrics)
}

/// The band at the top of each column card that carries its Stack tab: one title row plus a tab's
/// rise, so the first pane's title row clears the Stack tab's row as the tab's cap clears its
/// label.
pub(crate) fn stack_band(metrics: CellMetrics) -> f32 {
    chrome_title_metrics(metrics).text_height + title_tab_rise(metrics)
}

/// The widest tab pill: it fits the viewport after the pills' left inset and keeps one tab gap
/// before the viewport's end.
pub(crate) fn tab_max_width(metrics: CellMetrics, viewport_width: f32) -> f32 {
    (metrics.font_size * 17.5)
        .min((viewport_width - card_gap(metrics) - pill_gap(metrics)).max(1.0))
}

/// Snap the complete title midpoint once, in physical pixels.
pub(crate) fn title_rule_y(top: f32, height: f32) -> f32 {
    (top + height / 2.0).floor()
}

/// The margin a card's title keeps around its text where the title interrupts the card's edge,
/// and a heading's button keeps on each side of its target there.
pub(crate) const fn title_margin(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// How far a title tab's cap rises above its title row: a third of the title padding.
pub(crate) const fn title_tab_rise(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

/// The run along a card's top edge over which a title tab that rose `rise` returns to it.
pub(crate) const fn title_tab_return(rise: f32) -> f32 {
    rise * 1.5
}

pub(crate) const fn chrome_title_metrics(metrics: CellMetrics) -> ChromeTitleMetrics {
    let text_height = metrics.height.max(metrics.font_size * 1.25);
    ChromeTitleMetrics {
        font_size: metrics.font_size,
        line_height: metrics.height,
        text_height,
    }
}

/// Shared typography for chrome titles and their descender-safe clip box.
pub(crate) struct ChromeTitleMetrics {
    pub font_size: f32,
    pub line_height: f32,
    pub text_height: f32,
}
