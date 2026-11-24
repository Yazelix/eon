//! Venus's chrome radii, gaps, insets and shared text sizes, one name per role, named for what
//! the value is for and defined once from the cell metrics. Two roles may share a value today
//! and still change apart. Layout (`scene`) and paint (`render`) both read these. Colors
//! are in `render/palette.rs`; each text style's face and weight stay with `render`'s
//! `DrawStyleKind`.

use crate::render::CellMetrics;

/// Bar reading status outlines use one logical pixel, rounded to whole physical pixels.
pub(crate) const CHIP_OUTLINE_WIDTH: u8 = 1;

/// One shared reading recipe: a half-padding dot with a third-padding text gap.
pub(crate) fn chip_dot_size(metrics: CellMetrics) -> f32 {
    (metrics.padding / 2.0).round().max(1.0)
}

pub(crate) fn chip_dot_gap(metrics: CellMetrics) -> f32 {
    (metrics.padding / 3.0).round()
}

pub(crate) fn chip_dot_reserve(metrics: CellMetrics) -> f32 {
    chip_dot_size(metrics) + chip_dot_gap(metrics)
}

/// Background's three-second pulse on the existing 100ms badge clock.
pub(crate) const BACKGROUND_PULSE_FRAMES: usize = 30;
pub(crate) const BACKGROUND_PULSE_MIN_ALPHA: f32 = 0.65;

// Radii.
/// The radius that rounds a shape `thickness` across into a pill or a disc: tab pills, the bar's
/// chips and controls, the scrollback pill, and the dialog's scroll track.
pub(crate) const fn capsule_radius(thickness: f32) -> f32 {
    thickness / 2.0
}

/// A card's corner radius: pane cards, popups, the agent sidebar and the dialog. It is the gap
/// between cards today.
pub(crate) const fn card_radius(metrics: CellMetrics) -> f32 {
    card_gap(metrics)
}

/// Quarter-size rounded-square corners for popup markers and monogram tiles, snapped to physical pixels.
pub(crate) const fn rounded_square_radius(size: f32) -> f32 {
    (size / 4.0).round()
}

/// The corner radius of what sits inside a card or in the frame around the cards: sidebar and
/// inbox rows, tooltips and hint popups, hint keycaps, pane title highlights, and pane and
/// heading buttons.
pub(crate) const fn inner_radius(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// The corner radius of a dialog's focused row.
pub(crate) const fn dialog_row_radius(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

// Gaps and insets.
/// The even gap around and between pane cards, which is also their corner radius: 8 logical
/// pixels at default typography, always less than the label padding so no label meets a curve.
pub(crate) const fn card_gap(metrics: CellMetrics) -> f32 {
    let radius = (metrics.padding * 2.0 / 3.0).round();
    // Horizontal terminal padding keeps text clear of the rounded corners.
    debug_assert!(metrics.padding >= radius);
    radius
}

/// The space between neighbouring tab pills, before a short bar makes it yield.
pub(crate) const fn pill_gap(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

/// How far a bar chip's or control's surface sits inside its slot, before a small slot makes it
/// yield.
pub(crate) const fn chip_inset(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

/// The space a tooltip, a hint popup or a hint's detail box keeps from what it opens from, and
/// from the header when it opens upward.
pub(crate) const fn tooltip_gap(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

/// The room a popup card keeps above its terminal for its title.
pub(crate) const fn popup_title_gutter(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// How far content sits inside the edge of what holds it: a sidebar row's content and the list
/// in its card, a pane's and a heading's buttons, a keycap's legend, and the floating scrollback
/// pill's text.
pub(crate) const fn inner_inset(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// The space between a quota chip's icon and its label.
pub(crate) const fn icon_gap(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// The bundled reading icon's em and its compact gap before the percentage.
pub(crate) const fn reading_icon_reserve(metrics: CellMetrics) -> f32 {
    metrics.font_size + metrics.padding / 6.0
}

/// How far a hint keycap's hover area reaches past the keycap on each side.
pub(crate) const fn hint_hover_reach(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

/// The space a sidebar or inbox row adds to its text's height, half above the text and half
/// below, in whole pixels.
pub(crate) const fn attention_row_gap(metrics: CellMetrics) -> f32 {
    (metrics.padding / 2.0).round()
}

/// App marks span about 0.6 of the two-line block, snapped to Clawd's eleven columns.
/// 1.19 keeps rounded 1.25x line metrics below the next grid step.
/// The font cap protects text space; 253 is the last grid step below the 256px source.
pub(crate) const fn attention_app_size(metrics: CellMetrics) -> f32 {
    ((metrics.height * 1.19).min(metrics.font_size * 1.6) / 11.0)
        .round()
        .clamp(1.0, 23.0)
        * 11.0
}

/// Status is secondary to app identity, in a rounded font-sized physical box.
pub(crate) const fn attention_status_size(metrics: CellMetrics) -> f32 {
    metrics.font_size.round().clamp(1.0, 256.0)
}

/// The space between row symbols and before their text, in whole physical pixels.
pub(crate) const fn attention_symbol_gap(metrics: CellMetrics) -> f32 {
    (metrics.width / 2.0).round()
}

/// The space between a sidebar heading and its first row, in whole pixels.
pub(crate) const fn sidebar_heading_gap(metrics: CellMetrics) -> f32 {
    (metrics.padding / 3.0).round()
}

/// The margin the hints row keeps below it, and the inset that keeps a frameless pane header's
/// fill off its neighbours before a short row makes it yield.
pub(crate) const fn stack_inset(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

// The dialog. Its padding yields in a narrow dialog, so what follows takes that padding.
/// A dialog's padding where it has `width` to lay out in: the padding, or an eighth of a narrower
/// width.
pub(crate) const fn dialog_padding(metrics: CellMetrics, width: f32) -> f32 {
    metrics.padding.min(width / 8.0)
}

/// The corner radius of a dialog's selected filter chip, and of its frame where no title tab
/// fits, for a dialog padded by `padding`.
pub(crate) const fn dialog_radius(padding: f32) -> f32 {
    padding / 2.0
}

/// How far a filter chip's label starts inside the chip, for a dialog padded by `padding`.
pub(crate) const fn dialog_chip_label_inset(padding: f32) -> f32 {
    padding / 3.0
}

/// What a filter chip's label gives up of the chip's width, for a dialog padded by `padding`.
pub(crate) const fn dialog_chip_label_margins(padding: f32) -> f32 {
    padding * 2.0 / 3.0
}

// Text.
/// Shared Zoom/Restore affordances in pane headers and tab labels.
pub(crate) const ZOOM_GLYPH: &str = "□";
pub(crate) const RESTORE_GLYPH: &str = "▣";

/// The font size of the smaller label face: a hint popup's section titles, and a dialog's
/// headings, subtitle and footer.
pub(crate) const fn popup_label_font_size(metrics: CellMetrics) -> f32 {
    11.0 * metrics.scale
}

/// The height of one line of status text that wraps, in terminal rows.
pub(crate) const WRAPPED_LINE_ROWS: f32 = 1.25;

/// An attention badge's size before it is rounded to whole pixels: three quarters of the font
/// size.
pub(crate) const fn badge_size(metrics: CellMetrics) -> f32 {
    metrics.font_size * 0.75
}
