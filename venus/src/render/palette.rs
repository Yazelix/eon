//! Venus's fixed chrome colors, one constant per role, named for what the color means. Two roles
//! may share a value today and still change apart. The configured focus color and the terminal's
//! own colors come from Eon Runtime and Orbit, not from here.

use super::SceneColor;

const fn rgb(r: u8, g: u8, b: u8) -> SceneColor {
    SceneColor { r, g, b }
}

// Surfaces.
/// The opaque chrome surface: the Eon Bar, the frame around the cards, idle bar controls, and the
/// disc behind an attention badge on the bar.
pub(super) const BAR_BACKGROUND: SceneColor = rgb(10, 13, 20);
/// The terminal background before Orbit sends one: the window with no terminal, and a card or
/// strip whose terminal has no frame yet.
pub(super) const TERMINAL_FALLBACK: SceneColor = rgb(10, 13, 20);
/// The neutral outline around a card: every pane outline but the selected column's expanded
/// pane, popups, the agent sidebar, the dialog, and the sidebar row for the current pane.
pub(super) const CARD_OUTLINE: SceneColor = rgb(86, 106, 126);
/// The outline of an Eon Bar chip: quota, CPU, RAM and clock.
pub(super) const CHIP_OUTLINE: SceneColor = rgb(86, 106, 126);
/// The fill under a hovered, pressed or keyboard-selected chrome item: sidebar rows, pane header
/// titles and bar controls, and the disc behind a sidebar badge on such a row.
pub(super) const CHROME_HIGHLIGHT: SceneColor = rgb(40, 54, 70);
/// The fill under a bar control the pointer hovers.
pub(super) const CONTROL_HOVER: SceneColor = rgb(30, 40, 54);
/// The keycap behind a hint's modifier keys.
pub(super) const HINT_KEYCAP: SceneColor = rgb(30, 40, 54);
pub(super) const TOOLTIP_BACKGROUND: SceneColor = rgb(52, 72, 92);
pub(super) const DIALOG_BACKGROUND: SceneColor = rgb(20, 27, 38);
pub(super) const SCROLLBAR_TRACK: SceneColor = rgb(56, 70, 88);

// Tab pills step up from idle to hover to selected. The highlight, control hover, keycap, tooltip
// and scrollback pill fills reuse these steps, so the chrome climbs one ladder from the bar.
pub(super) const TAB_IDLE: SceneColor = rgb(30, 40, 54);
pub(super) const TAB_HOVER: SceneColor = rgb(40, 54, 70);
pub(super) const TAB_SELECTED: SceneColor = rgb(52, 72, 92);

// Text.
/// Titles and labels at full emphasis: the selected tab, the current pane, the sidebar, popup
/// and dialog titles, tooltips, and pressed or focused bar controls.
pub(super) const PRIMARY_TEXT: SceneColor = rgb(239, 244, 248);
/// Text a step below its title: other panes' titles, the current pane's ID, bar readings, hint
/// labels, idle bar controls, sidebar details, and dialog subtitles and footers.
pub(super) const SECONDARY_TEXT: SceneColor = rgb(162, 174, 190);
/// The detail line of a dimmed sidebar row.
pub(super) const TERTIARY_TEXT: SceneColor = rgb(150, 160, 176);
/// The ID in a pane title that is not the current one, a step below its secondary-text name.
pub(super) const PANE_ID: SceneColor = rgb(147, 160, 179);
/// The title of a pane whose terminal is not live.
pub(super) const DEAD_PANE_TEXT: SceneColor = rgb(232, 140, 140);
/// The labels of unselected tabs.
pub(super) const TAB_TEXT: SceneColor = rgb(198, 208, 220);
pub(super) const VERSION_TEXT: SceneColor = rgb(124, 196, 216);
pub(super) const HINT_MODIFIER_TEXT: SceneColor = rgb(162, 204, 230);
pub(super) const SHORTCUT_KEY_TEXT: SceneColor = rgb(162, 204, 230);
pub(super) const SHORTCUT_ACTION_TEXT: SceneColor = rgb(219, 226, 234);
/// The startup status text under the splash title.
pub(super) const SPLASH_TEXT: SceneColor = rgb(196, 202, 216);

/// Venus's emphasis color: chrome keyboard-focus outlines, sidebar and dialog headings, hint
/// keys, the dialog's scroll thumb, and the splash title.
pub(super) const ACCENT: SceneColor = rgb(126, 231, 185);

// Attention badge glyphs.
pub(super) const ATTENTION_NEEDS_INPUT: SceneColor = rgb(255, 196, 87);
pub(super) const ATTENTION_ERROR: SceneColor = rgb(255, 107, 107);
pub(super) const ATTENTION_FINISHED: SceneColor = rgb(96, 196, 96);
pub(super) const ATTENTION_WORKING: SceneColor = rgb(138, 180, 255);
/// The idle ring and the unavailable dot.
pub(super) const ATTENTION_IDLE: SceneColor = rgb(162, 174, 190);

// Terminal overlays.
pub(super) const SCROLLBACK_TEXT: SceneColor = rgb(162, 204, 230);
/// The scrollback pill when it sits in a pane header.
pub(super) const SCROLLBACK_PILL_HEADER: SceneColor = rgb(52, 72, 92);
/// The scrollback pill when it floats over the terminal.
pub(super) const SCROLLBACK_PILL_OVERLAY: SceneColor = rgb(40, 54, 70);
/// The outline around the hyperlink under the pointer.
pub(super) const HYPERLINK_OUTLINE: SceneColor = rgb(137, 180, 250);
pub(super) const NOTICE_TEXT: SceneColor = rgb(245, 192, 94);
pub(super) const NOTICE_BACKGROUND: SceneColor = rgb(35, 29, 18);

// Quota chip provider icons.
pub(super) const QUOTA_ICON_CODEX: SceneColor = rgb(255, 255, 255);
pub(super) const QUOTA_ICON_CLAUDE: SceneColor = rgb(217, 119, 87);
