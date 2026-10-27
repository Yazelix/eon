use crate::{
    CpuUse, MAX_LINK_BYTES, RamUse,
    model::{active_popup, focused_terminal, presented_panes},
    render::{CellMetrics, FALLBACK_CAP_HEIGHT_EM},
};
use eon_workspace_protocol::v22::{
    ALT, AttentionKind, AttentionState, ClaudeQuotaState, CodexQuotaState, PopupEntry, QuotaWindow,
    SHIFT, Sidebar, Snapshot, Tab,
};
use orbit_protocol::{
    Cell, CellStyle, CellWidth, CursorShape, Frame, PlacementState, Rgb, Row, Screen, SourceRect,
    StyleColor, Underline, session::VerticalDirection,
};
use std::{
    borrow::Cow,
    ffi::OsStr,
    fmt::Write,
    ops::{Range, RangeInclusive},
    os::unix::ffi::OsStrExt,
    path::Path,
};
use winit::dpi::PhysicalSize;

/// Physical rectangle shared by workspace drawing, hit testing, and accessibility.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

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

fn painted_bottom(
    bottom: f32,
    metrics: CellMetrics,
    stroke_width: f32,
    grid_bottom: Option<f32>,
) -> f32 {
    let inset =
        (chrome_title_metrics(metrics).text_height / 2.0).min(metrics.padding - stroke_width);
    grid_safe_bottom((bottom - inset).floor(), grid_bottom, stroke_width)
}

impl WorkspaceScene {
    pub(crate) fn pane_outlines(
        &self,
        panes: &[WorkspacePane],
        terminal: SceneRect,
        fill: CardFill,
        border_width: f32,
        metrics: CellMetrics,
        stroke_width: f32,
    ) -> Vec<PaneOutline> {
        let viewport = self.pane_viewport;
        let Some(expanded) = panes.iter().position(|pane| pane.selected) else {
            return Vec::new();
        };
        let grid_bottom = self.grid_bottom(terminal, metrics);
        let mut expanded_frame = panes[expanded].frame(terminal);
        // Scrolling may leave no room for a bottom stroke inside the fixed card.
        let original_bottom = grid_safe_bottom(
            expanded_frame.bottom().min(fill.bounds.bottom()),
            grid_bottom,
            stroke_width,
        );
        let bottom = panes.get(expanded + 1).map_or(original_bottom, |next| {
            let target = title_rule_y(next.rect.top, next.rect.height) - next.rect.height;
            target
                .max(painted_bottom(
                    terminal.bottom(),
                    metrics,
                    stroke_width,
                    grid_bottom,
                ))
                .min(original_bottom)
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
                clip,
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
}

impl SceneRect {
    /// Rounded native dimensions used by Orbit resize admission.
    #[must_use]
    pub fn screen_size(self) -> PhysicalSize<u32> {
        PhysicalSize::new(
            self.width.max(0.0).round() as u32,
            self.height.max(0.0).round() as u32,
        )
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.left + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.top + self.height
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        (self.left..self.right()).contains(&x) && (self.top..self.bottom()).contains(&y)
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (left < right && top < bottom).then_some(Self {
            left,
            top,
            width: right - left,
            height: bottom - top,
        })
    }
}

/// One Eon-authored tab projected into native geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceTab {
    pub id: String,
    pub selected: bool,
    pub rect: SceneRect,
    label: String,
    accessible_label: String,
    /// The tab's attention badge while the agent sidebar is not visible.
    pub(crate) badge: Option<TabBadge>,
}

/// An EON-C31 attention badge on a tab pill's corner, on a scroll edge for hidden tabs, or as
/// a sidebar row's state glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TabBadge {
    /// Needs input, error, finished or working; sidebar rows also show idle and unavailable.
    pub(crate) state: AttentionState,
    /// Every terminal in the tab's rollup state is stale, or the row's record is.
    pub(crate) dimmed: bool,
    pub(crate) rect: SceneRect,
}

impl TabBadge {
    /// The tab's badge state, the word naming it and whether it is dimmed; idle and unavailable
    /// show no badge.
    fn of(tab: &Tab) -> Option<(AttentionState, &'static str, bool)> {
        let state = tab.attention_rollup?;
        let word = match state {
            AttentionState::NeedsInput => "needs input",
            AttentionState::Error => "error",
            AttentionState::Finished => "finished",
            AttentionState::Working => "working",
            AttentionState::Idle | AttentionState::Unavailable => return None,
        };
        let dimmed = tab
            .columns
            .iter()
            .flat_map(|column| &column.panes)
            .map(|pane| &pane.attention)
            .chain(tab.popups.iter().map(|popup| &popup.attention))
            .flatten()
            .filter(|attention| attention.state == state)
            .all(|attention| attention.record.as_ref().is_some_and(|record| record.stale));
        Some((state, word, dimmed))
    }

    /// The badge's square: as wide as the label padding at most, so it never meets the label,
    /// overhanging the pill's top-right corner into the gap after it and the inset above it.
    /// Typography too small for a six-pixel badge shows no glyph, so it gets none; the tab's
    /// accessible name still carries its state.
    fn corner(pill: SceneRect, metrics: CellMetrics, gap: f32) -> Option<SceneRect> {
        let size = metrics.padding.min(metrics.font_size * 0.75).round();
        let overhang = (size / 4.0).min(gap / 2.0).floor();
        (size >= 6.0).then_some(SceneRect {
            left: pill.right() - size + overhang,
            top: (pill.top - overhang).max(0.0),
            width: size,
            height: size,
        })
    }
}

/// What the agent sidebar's rows need besides the snapshot (VEN-C30).
#[derive(Default)]
pub struct SidebarView<'a> {
    /// The wall-clock Unix milliseconds that row ages count to.
    pub now: u64,
    /// The keyboard-focused row's target while the sidebar holds focus.
    pub focused: Option<&'a str>,
}

/// A tab's heading in the agent sidebar, such as `3  site`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SidebarHeading {
    pub(crate) tab: String,
    pub(crate) label: String,
    pub(crate) rect: SceneRect,
}

/// One terminal with attention in the agent sidebar (VEN-C30).
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarRow {
    pub tab: String,
    /// The exact pane id or popup instance that `FocusAttention` names.
    pub target: String,
    pub(crate) rect: SceneRect,
    /// The state glyph, dimmed while the record is stale.
    pub(crate) glyph: TabBadge,
    /// The Claude splat or Codex mark that stands for the reporting app.
    pub(crate) icon: Option<(QuotaProvider, SceneRect)>,
    /// Where the row's text lines start, after the glyph and any icon.
    pub(crate) text_left: f32,
    /// Claude Code's session name, else the reported title, else the app's name.
    pub(crate) title: String,
    /// The age, the source terminal, "+N" for other records and "status stale".
    pub(crate) detail: String,
    /// Where in `detail` the popup marker follows the source's catalog label, when the row's
    /// source is a popup.
    pub(crate) popup_marker: Option<usize>,
    /// A needs-input row's message line.
    pub(crate) message: Option<String>,
    pub(crate) focused: bool,
    /// The row's source terminal is the one the active tab focuses, so it takes the "you are
    /// here" outline.
    pub(crate) current: bool,
    /// Milliseconds since the record changed; unavailable rows have no age.
    age: Option<u64>,
    pub(crate) accessible_label: String,
}

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

/// The unit an age `elapsed` long counts in: its length, shown suffix and word.
fn age_unit(elapsed: u64) -> (u64, &'static str, &'static str) {
    if elapsed < HOUR_MS {
        (MINUTE_MS, "m", "minute")
    } else if elapsed < DAY_MS {
        (HOUR_MS, "h", "hour")
    } else {
        (DAY_MS, "d", "day")
    }
}

/// A row's age as shown, such as "4m", and as spoken, such as "4 minutes ago".
fn age_labels(elapsed: u64) -> (String, String) {
    if elapsed < MINUTE_MS {
        return ("now".into(), "just now".into());
    }
    let (step, short, unit) = age_unit(elapsed);
    let count = elapsed / step;
    let plural = if count == 1 { "" } else { "s" };
    (
        format!("{count}{short}"),
        format!("{count} {unit}{plural} ago"),
    )
}

/// Milliseconds until an age `elapsed` long next changes its label.
fn age_wait(elapsed: u64) -> u64 {
    let (step, _, _) = age_unit(elapsed);
    step - elapsed % step
}

impl WorkspaceTab {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn accessible_label(&self) -> &str {
        &self.accessible_label
    }
}

/// One Eon-authored pane header projected into native accordion geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspacePane {
    pub id: String,
    pub live: bool,
    pub selected: bool,
    pub rect: SceneRect,
    label: String,
}

/// A column other than the selected one: its accordion headers and terminal area.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceColumn {
    /// Position among the tab's columns, left to right.
    pub index: usize,
    pub panes: Vec<WorkspacePane>,
    pub terminal: SceneRect,
    pub(crate) chrome: SceneRect,
}

impl WorkspacePane {
    /// Painted bounds before viewport clipping; input continues to use `rect`.
    pub(crate) fn frame(&self, terminal: SceneRect) -> SceneRect {
        let top = title_rule_y(self.rect.top, self.rect.height);
        SceneRect {
            top,
            height: if self.selected {
                (terminal.bottom() - top).max(0.0)
            } else {
                // The base curve size is independent of the adjacent card's join.
                self.rect.height
            },
            ..self.rect
        }
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }
}

fn clean_label(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// The last component of a tab's launch directory, or `/`.
fn directory_leaf(directory: &[u8]) -> String {
    let path = Path::new(OsStr::from_bytes(directory));
    if path == Path::new("/") {
        "/".into()
    } else {
        clean_label(
            &path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
        )
    }
}

fn tab_labels(index: usize, tab_count: usize, directory: &[u8]) -> (String, String) {
    let position = index + 1;
    let full_path = clean_label(&OsStr::from_bytes(directory).to_string_lossy());
    (
        format!("{position}  {}", directory_leaf(directory)),
        format!("Tab {position} of {tab_count}  {full_path}"),
    )
}

/// The widest tab pill: it fits the viewport after the pills' left inset and keeps one tab gap
/// before the viewport's end.
pub(crate) fn tab_max_width(metrics: CellMetrics, viewport_width: f32) -> f32 {
    (metrics.font_size * 17.5)
        .min((viewport_width - card_gap(metrics) - metrics.padding / 3.0).max(1.0))
}

/// One action in the native Eon workspace header: the workspace label, while it opens the
/// workspace picker, or a fixed control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceHeaderControl {
    Workspaces,
    NewTab,
    Shortcuts,
    CloseTab,
}

impl WorkspaceHeaderControl {
    /// The tooltip and accessible name, which drop the chord when a configured popup takes
    /// this control's key (VEN-C20). Eon reserves Alt+/.
    fn label(self, entries: &[PopupEntry]) -> &'static str {
        let taken = |key: &str| {
            entries.iter().any(|entry| {
                entry.shortcut.bound().is_some_and(|shortcut| {
                    shortcut.modifiers == ALT | SHIFT && shortcut.key == key
                })
            })
        };
        match self {
            // The workspace label names itself from its popup entry.
            Self::Workspaces => "",
            Self::NewTab if taken("KeyT") => "New tab",
            Self::NewTab => "New tab — Alt+Shift+T",
            Self::Shortcuts => "Keyboard shortcuts — Alt+/",
            Self::CloseTab if taken("KeyW") => "Close tab",
            Self::CloseTab => "Close tab — Alt+Shift+W",
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Workspaces => "",
            Self::NewTab => "+",
            Self::Shortcuts => "?",
            Self::CloseTab => "×",
        }
    }
}

/// Geometry for one fixed Eon workspace-header action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorkspaceControl {
    pub(crate) kind: WorkspaceHeaderControl,
    pub(crate) rect: SceneRect,
    pub(crate) label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuotaProvider {
    Codex,
    Claude,
}

impl QuotaProvider {
    fn item(self) -> BarItem {
        match self {
            Self::Codex => BarItem::CodexQuota,
            Self::Claude => BarItem::ClaudeQuota,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}

/// One read-only provider fact projected into the Eon Bar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceQuota {
    pub(crate) provider: QuotaProvider,
    /// The one window, in minutes, that a per-window Claude chip shows; Codex's chip shows all.
    pub(crate) window: Option<u32>,
    pub(crate) rect: SceneRect,
    pub(crate) logo: SceneRect,
    pub(crate) label: String,
    pub(crate) accessible_label: String,
    pub(crate) description: String,
}

/// One read-only informational label in the workspace header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceLabel {
    pub(crate) rect: SceneRect,
    pub(crate) label: String,
    pub(crate) description: String,
}

/// The Eon Bar's leading workspace label (VEN-C21).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceNameLabel {
    pub(crate) rect: SceneRect,
    pub(crate) label: String,
    pub(crate) accessible_label: String,
    /// The `workspaces` popup entry's label and chord, while that entry makes the label open
    /// the workspace picker; without it the label is inert.
    pub(crate) tooltip: Option<String>,
    /// The needs-input badge while another workspace needs input.
    pub(crate) badge: Option<TabBadge>,
}

/// The shown workspace's name, or its `wN` when unnamed, with its 1-based position in window
/// order and the window's workspace count.
#[must_use]
pub fn shown_workspace(snapshot: &Snapshot) -> Option<(&str, usize, usize)> {
    let position = snapshot
        .workspaces
        .iter()
        .position(|workspace| workspace.id == snapshot.workspace)?;
    let workspace = &snapshot.workspaces[position];
    Some((
        workspace.name.as_deref().unwrap_or(&workspace.id),
        position + 1,
        snapshot.workspaces.len(),
    ))
}

/// What the Eon Bar's workspace label shows: the shown workspace, while the window holds two or
/// more workspaces or its one workspace has a name (VEN-C21).
pub(crate) fn bar_workspace(snapshot: &Snapshot) -> Option<(&str, usize, usize)> {
    shown_workspace(snapshot).filter(|&(_, _, count)| {
        count >= 2
            || snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.name.is_some())
    })
}

/// Whether a workspace other than the shown one needs input (EON-C33), which the bar label's
/// badge shows.
pub(crate) fn hidden_needs_input(snapshot: &Snapshot) -> bool {
    snapshot.workspaces.iter().any(|workspace| {
        workspace.id != snapshot.workspace
            && workspace.attention == Some(AttentionState::NeedsInput)
    })
}

/// What one header text measurement is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeaderText<'a> {
    /// The label of the tab with this ID, fitted to a tab.
    Tab(&'a str),
    /// An exact Eon Bar fact.
    Fact,
    /// Text fitted to at most this many pixels.
    Fit(f32),
}

/// One item Eon may place in the Eon Bar between the drag region and Shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarItem {
    CodexQuota,
    ClaudeQuota,
    Cpu,
    Ram,
    Clock,
    Version,
}

impl BarItem {
    /// Every item, in the order Venus shows them when Eon names none.
    pub const DEFAULT: [Self; 6] = [
        Self::CodexQuota,
        Self::ClaudeQuota,
        Self::Cpu,
        Self::Ram,
        Self::Clock,
        Self::Version,
    ];
}

/// The Eon Bar items in order, and the values Venus has for them.
pub struct EonBar<'a> {
    pub items: &'a [BarItem],
    pub version: Option<&'a str>,
    pub cpu: Option<&'a CpuUse>,
    pub ram: Option<&'a RamUse>,
    pub clock: Option<&'a LocalClock>,
    /// The `workspaces` popup entry's label and chord, while Eon configures that entry; the
    /// workspace label then opens it.
    pub workspace_picker: Option<&'a str>,
}

impl Default for EonBar<'_> {
    fn default() -> Self {
        Self {
            items: &BarItem::DEFAULT,
            version: None,
            cpu: None,
            ram: None,
            clock: None,
            workspace_picker: None,
        }
    }
}

/// One local wall-clock minute shown in the workspace header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalClock {
    label: String,
    description: String,
}

impl LocalClock {
    #[must_use]
    pub fn new(now: &jiff::Zoned) -> Self {
        let label = format!("{:02}:{:02}", now.hour(), now.minute());
        let offset = now.offset().seconds();
        let sign = if offset < 0 { '\u{2212}' } else { '+' };
        let minutes = offset.unsigned_abs() / 60;
        let description = format!(
            "Local time {label}, {}, UTC{sign}{:02}:{:02}",
            now.strftime("%A %-d %B %Y"),
            minutes / 60,
            minutes % 60,
        );
        Self { label, description }
    }
}

/// Native workspace target at one physical point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceHit<'a> {
    Tab(&'a str),
    Quota(usize),
    Version,
    Cpu,
    Ram,
    Clock,
    Control(WorkspaceHeaderControl),
    Drag,
    /// An agent sidebar row, named by its target.
    SidebarRow(&'a str),
    Pane(&'a str),
    /// Another column's terminal area, named by that column's expanded pane.
    ColumnTerminal(&'a str),
    Terminal,
}

/// Native focus region inside the one-window workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceFocus {
    #[default]
    Terminal,
    Tabs,
    Header(WorkspaceHeaderControl),
    Panes,
    /// The agent sidebar's rows; the application keeps which one.
    Sidebar,
}

/// One read-only shortcut shown by the native viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutRow {
    pub shortcut: String,
    pub action: String,
    pub rect: SceneRect,
}

/// One binding-derived footer hint, before native width fitting.
#[derive(Clone, Debug)]
pub struct ShortcutHint {
    pub modifiers: String,
    pub key: String,
    pub label: String,
    pub accessible_label: String,
    pub priority: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShortcutHintPiece {
    pub modifiers: String,
    pub key: String,
    pub label: String,
    pub accessible_label: String,
    pub rect: SceneRect,
    pub key_offset: f32,
    pub label_offset: f32,
}

/// A read-only footer whose complete hints share the renderer's shaped widths.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutHintsScene {
    pub rect: SceneRect,
    pub complete: bool,
    pub(crate) pieces: Vec<ShortcutHintPiece>,
    keep_shortcuts: bool,
}

impl ShortcutHintsScene {
    #[must_use]
    pub fn height(metrics: CellMetrics) -> f32 {
        (metrics.height * 1.5).round().max(metrics.padding)
    }

    #[must_use]
    fn reserved_height(metrics: CellMetrics) -> f32 {
        Self::height(metrics) + stack_inset(metrics) + pane_inset(metrics) / 2.0
    }

    #[must_use]
    pub fn new(
        hints: Vec<ShortcutHint>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        focus: WorkspaceFocus,
        mut measure: impl FnMut(&str) -> f32,
    ) -> Self {
        let chrome = pane_chrome_rect(
            SceneRect {
                width: size.width as f32,
                height: (size.height as f32 - stack_inset(metrics)).max(0.0),
                ..SceneRect::default()
            },
            metrics,
        );
        let bottom = chrome.bottom().min(size.height as f32);
        let height = Self::height(metrics).min((bottom - pill_row(metrics)).max(0.0));
        let rect = SceneRect {
            top: bottom - height,
            height,
            ..chrome
        };
        // The frame reaches the window's bottom edge, keeping the reserved height; hints center in it.
        let frame = SceneRect {
            height: (size.height as f32 - rect.top).max(0.0),
            ..rect
        };
        let count = hints.len();
        let mut groups: Vec<Vec<ShortcutHint>> = Vec::new();
        for hint in hints {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group[0].modifiers == hint.modifiers)
            {
                group.push(hint);
            } else {
                groups.push(vec![hint]);
            }
        }
        let mut widths = std::collections::HashMap::new();
        let mut width = |text: &str| {
            *widths
                .entry(text.to_owned())
                .or_insert_with(|| measure(text))
        };
        let pieces = loop {
            let mut left = rect.left + metrics.padding;
            let mut pieces = Vec::new();
            for (group_index, group) in groups.iter().enumerate() {
                if group_index > 0 {
                    left += metrics.padding * 2.0;
                }
                for (index, hint) in group.iter().enumerate() {
                    if index > 0 {
                        left += metrics.padding;
                    }
                    let modifiers = if index == 0 && !hint.modifiers.is_empty() {
                        hint.modifiers.clone()
                    } else {
                        String::new()
                    };
                    let key_offset = if modifiers.is_empty() {
                        0.0
                    } else {
                        width(&modifiers) + metrics.padding * 2.0
                    };
                    let label_offset = key_offset + width(&hint.key) + width(" ");
                    let piece_width = label_offset + width(&hint.label);
                    pieces.push(ShortcutHintPiece {
                        modifiers,
                        key: hint.key.clone(),
                        label: hint.label.clone(),
                        accessible_label: hint.accessible_label.clone(),
                        key_offset,
                        label_offset,
                        rect: SceneRect {
                            left,
                            top: frame.top + (frame.height - metrics.height).max(0.0) / 2.0,
                            width: piece_width,
                            height: metrics.height.min(height),
                        },
                    });
                    left += piece_width;
                }
            }
            if groups.is_empty()
                || height >= metrics.height && left <= rect.right() - metrics.padding
            {
                let spare = rect.right() - metrics.padding - left;
                let mut retained = pieces.iter_mut();
                for (index, group) in groups.iter().enumerate() {
                    let offset = if groups.len() == 1 {
                        spare / 2.0
                    } else {
                        spare * index as f32 / (groups.len() - 1) as f32
                    };
                    for piece in retained.by_ref().take(group.len()) {
                        piece.rect.left += offset;
                    }
                }
                break pieces;
            }
            let (group, index, _) = groups
                .iter()
                .enumerate()
                .flat_map(|(group, hints)| {
                    hints
                        .iter()
                        .enumerate()
                        .map(move |(index, hint)| (group, index, hint.priority))
                })
                .max_by_key(|&(group, index, priority)| (priority, group, index))
                .expect("nonempty hints");
            groups[group].remove(index);
            if groups[group].is_empty() {
                groups.remove(group);
            }
        };
        let complete = count > 0 && pieces.len() == count;
        Self {
            rect: frame,
            complete,
            pieces,
            keep_shortcuts: !complete
                || focus == WorkspaceFocus::Header(WorkspaceHeaderControl::Shortcuts),
        }
    }
}

impl ShortcutRow {
    #[must_use]
    pub fn new(shortcut: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            shortcut: shortcut.into(),
            action: action.into(),
            rect: SceneRect::default(),
        }
    }
}

/// Related shortcuts presented under one native heading.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutGroup {
    pub title: String,
    pub rows: Vec<ShortcutRow>,
    pub heading: SceneRect,
}

impl ShortcutGroup {
    #[must_use]
    pub fn new(title: impl Into<String>, rows: Vec<ShortcutRow>) -> Self {
        Self {
            title: title.into(),
            rows,
            heading: SceneRect::default(),
        }
    }
}

/// What the one native dialog shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Shortcuts,
    /// The final-close prompt: how many terminals the close stops, the detach chord unless a
    /// popup takes it, and the shown workspace's name or `wN` while other workspaces remain.
    CloseLastTab {
        terminals: usize,
        detach: Option<String>,
        workspace: Option<String>,
    },
}

impl DialogKind {
    #[must_use]
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Shortcuts => "Shortcuts",
            Self::CloseLastTab { .. } => "Close the last tab?",
        }
    }

    #[must_use]
    pub(crate) fn subtitle(&self) -> Cow<'static, str> {
        match self {
            Self::Shortcuts => "Eon surface · physical keys".into(),
            Self::CloseLastTab {
                terminals,
                workspace,
                ..
            } => format!(
                "This {} and stops {terminals} terminal{}.",
                workspace.as_ref().map_or_else(
                    || "quits the window".into(),
                    |workspace| format!("closes workspace {workspace}")
                ),
                if *terminals == 1 { "" } else { "s" }
            )
            .into(),
        }
    }

    #[must_use]
    pub(crate) fn footer(&self) -> Vec<String> {
        match self {
            Self::Shortcuts => vec!["Wheel or ↑↓ to scroll · Esc or Alt+/ to close".into()],
            Self::CloseLastTab {
                detach, workspace, ..
            } => std::iter::once(
                if workspace.is_some() {
                    "Enter Close · Esc Cancel"
                } else {
                    "Enter Quit · Esc Cancel"
                }
                .into(),
            )
            .chain(
                detach
                    .iter()
                    .map(|chord| format!("{chord} keeps it running instead")),
            )
            .collect(),
        }
    }
}

/// Bounded geometry for the one native dialog: the shortcut viewer or the final-close prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogScene {
    pub(crate) kind: DialogKind,
    pub bounds: SceneRect,
    pub(crate) title: SceneRect,
    pub(crate) subtitle: SceneRect,
    pub content: SceneRect,
    pub groups: Vec<ShortcutGroup>,
    pub scroll: f32,
    pub max_scroll: f32,
}

impl DialogScene {
    /// Shortcuts fill the window's height; the prompt fits its content. A group without a
    /// title has no heading.
    #[must_use]
    pub fn new(
        kind: DialogKind,
        mut groups: Vec<ShortcutGroup>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        scroll: f32,
    ) -> Self {
        let width = size.width as f32;
        let height = size.height as f32;
        let margin = metrics.padding.min(width / 12.0).min(height / 12.0);
        let dialog_width = (metrics.font_size * 54.0).min(width - margin * 2.0);
        let heading_height = metrics.height * 1.4;
        let row_height = metrics.height * 2.25;
        let group_gap = metrics.height * 0.6;
        let content_height = groups.iter().fold(0.0, |height, group| {
            let heading = if group.title.is_empty() {
                0.0
            } else {
                heading_height
            };
            height + heading + row_height * group.rows.len() as f32 + group_gap
        });
        let available = height - margin * 2.0;
        let footer_lines = kind.footer().len() as f32;
        let padding_for = |width: f32| metrics.padding.min(width / 8.0);
        let dialog_height = match kind {
            DialogKind::Shortcuts => available,
            DialogKind::CloseLastTab { .. } => (padding_for(dialog_width)
                + metrics.height * 3.0
                + content_height
                + metrics.height * (footer_lines + 0.75))
                .min(available),
        };
        let bounds = SceneRect {
            left: ((width - dialog_width) / 2.0).max(0.0),
            top: (margin + (available - dialog_height) / 2.0).max(0.0),
            width: dialog_width,
            height: dialog_height,
        };
        let padding = padding_for(bounds.width);
        let title = SceneRect {
            left: bounds.left + padding,
            top: bounds.top + padding,
            width: bounds.width - padding * 2.0,
            height: metrics.height * 1.5,
        };
        let subtitle = SceneRect {
            top: title.bottom(),
            height: metrics.height,
            ..title
        };
        let footer_height = (metrics.height * (footer_lines + 0.75)).min(bounds.height / 3.0);
        let content_top =
            (subtitle.bottom() + metrics.height * 0.5).min(bounds.bottom() - footer_height);
        let content = SceneRect {
            top: content_top,
            height: bounds.bottom() - footer_height - content_top,
            ..title
        };
        let max_scroll = (content_height - content.height).max(0.0);
        let scroll = scroll.clamp(0.0, max_scroll);
        let mut top = content.top - scroll;
        for group in &mut groups {
            let height = if group.title.is_empty() {
                0.0
            } else {
                heading_height
            };
            group.heading = SceneRect {
                left: content.left,
                top,
                width: content.width,
                height,
            };
            top += height;
            for row in &mut group.rows {
                row.rect = SceneRect {
                    left: content.left,
                    top,
                    width: content.width,
                    height: row_height,
                };
                top += row_height;
            }
            top += group_gap;
        }
        Self {
            kind,
            bounds,
            title,
            subtitle,
            content,
            groups,
            scroll,
            max_scroll,
        }
    }

    /// The final-close prompt for `tab`: its panes by header label, then its popups by
    /// entry label. `workspace` names the workspace the close removes while others remain.
    #[must_use]
    pub fn close_last_tab(
        tab: &Tab,
        entries: &[PopupEntry],
        detach: Option<String>,
        workspace: Option<String>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        scroll: f32,
    ) -> Self {
        let rows = tab
            .panes()
            .map(|pane| ShortcutRow::new("Pane", pane.label.clone()))
            .chain(tab.popups.iter().map(|popup| {
                ShortcutRow::new(
                    "Popup",
                    entries
                        .iter()
                        .find(|entry| entry.id == popup.entry)
                        .expect("EONW validates popup entries")
                        .label
                        .as_str(),
                )
            }))
            .collect::<Vec<_>>();
        Self::new(
            DialogKind::CloseLastTab {
                terminals: rows.len(),
                detach,
                workspace,
            },
            vec![ShortcutGroup::new("", rows)],
            size,
            metrics,
            scroll,
        )
    }
}

/// Deterministic native projection of one complete accepted Eon snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceScene {
    pub tabs: Vec<WorkspaceTab>,
    /// The most urgent badge of the tabs scrolled out past each tab-viewport edge.
    pub(crate) edge_badges: Vec<TabBadge>,
    /// The tab viewport less the columns its edge badges take, so no label is drawn over them.
    pub(crate) tab_label_clip: SceneRect,
    pub(crate) header: SceneRect,
    /// The shown workspace's label before the tabs, while it shows.
    pub(crate) workspace_label: Option<WorkspaceNameLabel>,
    pub(crate) drag_region: SceneRect,
    pub(crate) quotas: Vec<WorkspaceQuota>,
    pub(crate) version: Option<WorkspaceLabel>,
    pub(crate) cpu: Option<WorkspaceLabel>,
    pub(crate) ram: Option<WorkspaceLabel>,
    pub(crate) clock: Option<WorkspaceLabel>,
    pub(crate) controls: [WorkspaceControl; 3],
    pub hints: Option<ShortcutHintsScene>,
    /// The selected column's headers and terminal; `other_columns` holds the rest.
    pub panes: Vec<WorkspacePane>,
    pub terminal: SceneRect,
    pub selected_column: usize,
    pub other_columns: Vec<WorkspaceColumn>,
    pub tab_viewport: SceneRect,
    pub pane_viewport: SceneRect,
    /// Current paint only: retained rows beyond the newly requested grid.
    displayed_grid_bottoms: Vec<(SceneRect, f32)>,
    /// The selected column's card, or the popup's; `other_columns` hold their own.
    pub(crate) chrome: SceneRect,
    /// The agent sidebar's card, while it is visible.
    sidebar: Option<SceneRect>,
    /// The visible sidebar's tab headings and rows, and the list area that clips them; with no
    /// rows it shows its empty state.
    pub(crate) sidebar_headings: Vec<SidebarHeading>,
    sidebar_rows: Vec<SidebarRow>,
    pub(crate) sidebar_list: SceneRect,
    card_radius: f32,
    tab_scroll: f32,
    pane_scroll: f32,
    tab_scroll_limit: f32,
    pane_scroll_limit: f32,
    active_tab_scroll: f32,
    selected_pane_scroll: f32,
    popup_label: Option<String>,
    /// Only the selected pane is laid out while zoom is visible.
    zoomed: bool,
}

/// Width of each of `count` equal column cards in a window `width` wide, after the card gap
/// beside and between them; the last column also takes the remainder, so this is the narrowest.
pub fn workspace_column_width(width: f32, count: usize, metrics: CellMetrics) -> f32 {
    ((width - side_gap(width, count, metrics) * (count + 1) as f32).max(0.0) / count as f32).floor()
}

fn header_heights(metrics: CellMetrics) -> (f32, f32) {
    // Preserve header allocation independently of the painted bar band.
    let row = pill_row(metrics);
    (
        row + (card_gap(metrics) - tab_gap(metrics, row)).max(0.0) * 2.0,
        chrome_title_metrics(metrics).text_height,
    )
}

/// The row of tab pills and header buttons: the pills plus one tab gap above and below them.
fn pill_row(metrics: CellMetrics) -> f32 {
    (metrics.height * 1.75).round().max(1.0)
}

/// The pill row inside a header `tab_height` tall, which yields only after the header's inset.
fn tab_row(metrics: CellMetrics, tab_height: f32) -> f32 {
    pill_row(metrics).min(tab_height)
}

/// The header's height in a window `height` tall whose content above any hints ends at
/// `content_height`: the header's extra inset yields first, keeping `minimum_height` for the
/// cards, and the pill row only when the window itself is shorter.
fn header_height(
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
fn tab_gap(metrics: CellMetrics, tab_height: f32) -> f32 {
    (metrics.padding / 3.0).min(tab_row(metrics, tab_height) / 4.0)
}

/// The tab hit targets' inset: one card gap at full header height. The first tab still lines
/// up with the first card horizontally; its painted vertical band is independent.
fn tab_inset(metrics: CellMetrics, tab_height: f32) -> f32 {
    tab_gap(metrics, tab_height) + (tab_height - tab_row(metrics, tab_height)) / 2.0
}

/// The painted bar band, independent of its hover and accessibility rectangles.
fn bar_pill_band(metrics: CellMetrics, tab_height: f32, card_top: f32) -> Range<f32> {
    let nominal_top = (chrome_title_metrics(metrics).text_height / 2.0).floor();
    let end = card_top.min(header_heights(metrics).0);
    let height = (header_heights(metrics).0 - nominal_top).min(end);
    if height >= metrics.height {
        end - height..end
    } else {
        // Keep the old tiny-window band, clipped to the space left above the cards.
        let inset = tab_inset(metrics, tab_height);
        let end = (tab_height - inset).min(card_top);
        inset.min(end)..end
    }
}

/// Leave the bar's top gap before the title capitals, using the renderer's title placement.
fn bar_title_margin(metrics: CellMetrics) -> f32 {
    let title = chrome_title_metrics(metrics);
    let header = header_heights(metrics).0;
    (bar_pill_band(metrics, header, header).start - title.cap_top)
        .round()
        .max(0.0)
}

/// The even gap around and between pane cards, which is also their corner radius: 8 logical
/// pixels at default typography, always less than the label padding so no label meets a curve.
fn card_gap(metrics: CellMetrics) -> f32 {
    let radius = (metrics.padding * 2.0 / 3.0).round();
    // Horizontal terminal padding keeps text clear of the rounded corners.
    debug_assert!(metrics.padding >= radius);
    radius
}

/// The width or height a terminal needs for one padded cell.
fn minimum_terminal(metrics: CellMetrics, cell: f32) -> f32 {
    metrics.padding * 2.0 + cell
}

/// The card gap beside and between `count` columns, which yields before any column loses its last
/// padded terminal cell.
fn side_gap(width: f32, count: usize, metrics: CellMetrics) -> f32 {
    let spare = width - minimum_terminal(metrics, metrics.width) * count as f32;
    card_gap(metrics).min((spare / (count + 1) as f32).max(0.0))
}

/// The agent sidebar's width (VEN-C30).
const SIDEBAR_CELLS: f32 = 28.0;
/// The fewest cells the content area keeps beside a visible sidebar; any narrower and it hides.
const SIDEBAR_MIN_CONTENT_CELLS: f32 = 80.0;
pub(crate) const SIDEBAR_TITLE: &str = "Agents";
pub(crate) const SIDEBAR_EMPTY: &str =
    "No agent needs attention. Agents that report their status show here while they work.";

/// Shared typography for chrome titles and their descender-safe clip box.
pub(crate) struct ChromeTitleMetrics {
    pub font_size: f32,
    pub line_height: f32,
    pub text_height: f32,
    pub cap_top: f32,
}

pub(crate) fn chrome_title_metrics(metrics: CellMetrics) -> ChromeTitleMetrics {
    let text_height = metrics.height.max(metrics.font_size * 1.25);
    ChromeTitleMetrics {
        font_size: metrics.font_size,
        line_height: metrics.height,
        text_height,
        cap_top: metrics.title_cap_top_em.map_or_else(
            || (text_height - metrics.font_size * FALLBACK_CAP_HEIGHT_EM) / 2.0,
            |cap| metrics.height / 2.0 + metrics.font_size * cap,
        ),
    }
}

/// Snap the complete title midpoint once, in physical pixels.
pub(crate) fn title_rule_y(top: f32, height: f32) -> f32 {
    (top + height / 2.0).floor()
}

pub(crate) fn popup_card_rect(allocated: SceneRect, metrics: CellMetrics) -> SceneRect {
    let title_height = chrome_title_metrics(metrics)
        .text_height
        .min(allocated.height);
    // Keep one padded cell when the window cannot also fit the shared top edge.
    let top = title_rule_y(allocated.top, title_height).min(
        (allocated.bottom() - minimum_terminal(metrics, metrics.height))
            .max(allocated.top)
            .floor(),
    );
    SceneRect {
        top,
        height: (allocated.bottom() - top).max(0.0),
        ..allocated
    }
}

/// The agent sidebar's card, one card gap from the window's left edge and as tall as `cards`,
/// while `sidebar` is open and the content area to its right keeps enough cells.
fn sidebar_rect(
    sidebar: Sidebar,
    width: f32,
    cards: SceneRect,
    metrics: CellMetrics,
) -> Option<SceneRect> {
    let rect = SceneRect {
        left: card_gap(metrics),
        width: SIDEBAR_CELLS * metrics.width,
        ..cards
    };
    (sidebar == Sidebar::Open && width - rect.right() >= SIDEBAR_MIN_CONTENT_CELLS * metrics.width)
        .then(|| {
            let top = title_rule_y(
                rect.top,
                chrome_title_metrics(metrics).text_height.min(rect.height),
            );
            SceneRect {
                top,
                height: (rect.bottom() - top).max(0.0),
                ..rect
            }
        })
}

/// The agent sidebar's tab headings and rows inside its `card`, in tab and terminal order, and
/// the list area that clips them. Only tabs with an attention rollup are listed, and only their
/// terminals with attention; the keyboard-focused row scrolls into view.
fn sidebar_contents(
    snapshot: &Snapshot,
    card: SceneRect,
    metrics: CellMetrics,
    view: SidebarView<'_>,
) -> (Vec<SidebarHeading>, Vec<SidebarRow>, SceneRect) {
    let line = metrics.height;
    let padding = metrics.padding;
    // The list starts where the empty state does, below the title.
    let list_top = card.top + padding + line * 1.5;
    let list = SceneRect {
        left: card.left + padding / 2.0,
        top: list_top,
        width: (card.width - padding).max(0.0),
        height: (card.bottom() - padding - list_top).max(0.0),
    };
    let content_left = card.left + padding;
    let content_width = (card.right() - padding - content_left).max(0.0);
    let row_gap = (padding / 2.0).round();
    let glyph_size = (metrics.font_size * 0.75).round().max(1.0);
    let icon_size = metrics.font_size.round().max(1.0);
    let spacing = (metrics.width / 2.0).round();
    let mut headings = Vec::new();
    let mut rows = Vec::new();
    let mut top = list.top;
    let focused_terminal = focused_terminal(snapshot);
    for (index, tab) in snapshot.tabs.iter().enumerate() {
        if tab.attention_rollup.is_none() {
            continue;
        }
        if !headings.is_empty() {
            top += line / 2.0;
        }
        let label = tab_labels(index, snapshot.tabs.len(), &tab.directory).0;
        headings.push(SidebarHeading {
            tab: tab.id.clone(),
            label,
            rect: SceneRect {
                left: content_left,
                top,
                width: content_width,
                height: line,
            },
        });
        // A small gap sets the heading apart from its first row.
        top += line + (padding / 3.0).round();
        let leaf = directory_leaf(&tab.directory);
        // Popup rows come first, then pane rows in pane order.
        let terminals = tab
            .popups
            .iter()
            .map(|popup| {
                let label = snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.id == popup.entry)
                    .expect("EONW validates popup entries")
                    .label
                    .as_str();
                (&popup.id, &popup.attention, label, true)
            })
            .chain(
                tab.panes()
                    .map(|pane| (&pane.id, &pane.attention, pane.id.as_str(), false)),
            );
        for (target, attention, source, popup) in terminals {
            let Some(attention) = attention else {
                continue;
            };
            let record = attention.record.as_ref();
            let (icon, app) = match record.map(|record| record.app.as_str()) {
                Some("claude-code") => (Some(QuotaProvider::Claude), "Claude"),
                Some("codex") => (Some(QuotaProvider::Codex), "Codex"),
                Some(app) => (None, app),
                None => (None, ""),
            };
            let reported_title = record
                .map(|record| record.title.as_str())
                .filter(|title| !title.is_empty());
            // Only Claude Code's terminal title names its session; other programs'
            // titles are status lines, such as Codex's spinner, or shell command lines.
            let name = record
                .filter(|record| record.app == "claude-code")
                .map_or("", |record| record.name.as_str());
            // The first of the name, the reported title and the app label, never the
            // directory leaf that the tab's heading already shows.
            let title = match record {
                None => "status unavailable",
                Some(_) => [name, reported_title.unwrap_or_default(), app]
                    .into_iter()
                    .find(|text| !text.is_empty())
                    .unwrap_or("agent"),
            }
            .to_owned();
            let age = record.map(|record| view.now.saturating_sub(record.changed_at));
            let ages = age.map(age_labels);
            let others = record.map_or(0, |record| record.others);
            let stale = record.is_some_and(|record| record.stale);
            let mut detail = ages
                .as_ref()
                .map_or_else(String::new, |(age, _)| format!("{age} · "));
            detail.push_str(source);
            let popup_marker = popup.then_some(detail.len());
            if others > 0 {
                let _ = write!(detail, " · +{others}");
            }
            if stale {
                detail.push_str(" · status stale");
            }
            let message = record
                .filter(|_| attention.state == AttentionState::NeedsInput)
                .map(|record| record.message.clone())
                .filter(|message| !message.is_empty());
            let word = match (attention.state, record.and_then(|record| record.kind)) {
                (AttentionState::NeedsInput, Some(AttentionKind::Permission)) => {
                    "Needs input (permission)"
                }
                (AttentionState::NeedsInput, Some(AttentionKind::Question)) => {
                    "Needs input (question)"
                }
                (AttentionState::NeedsInput, Some(AttentionKind::Auth)) => "Needs input (sign-in)",
                (AttentionState::NeedsInput, None) => "Needs input",
                (AttentionState::Error, _) => "Error",
                (AttentionState::Finished, _) => "Finished",
                (AttentionState::Working, _) => "Working",
                (AttentionState::Idle, _) => "Idle",
                (AttentionState::Unavailable, _) => "Status unavailable",
            };
            let mut accessible_label = format!(
                "{word}: {}{}{name} in tab {}, {leaf}, {source}",
                if app.is_empty() { "agent" } else { app },
                if name.is_empty() { "" } else { " " },
                index + 1,
            );
            if popup {
                accessible_label.push_str(", popup");
            }
            let current = focused_terminal == Some(target.as_str());
            if current {
                accessible_label.push_str(", current");
            }
            if let Some(title) = reported_title {
                let _ = write!(accessible_label, ", {title}");
            }
            if let Some((_, spoken)) = &ages {
                let _ = write!(accessible_label, ", changed {spoken}");
            }
            if others > 0 {
                let plural = if others == 1 { "" } else { "s" };
                let _ = write!(accessible_label, ", {others} more record{plural}");
            }
            if stale {
                accessible_label.push_str(", status stale");
            }
            if let Some(message) = &message {
                let _ = write!(accessible_label, ": {message}");
            }
            let lines = if message.is_some() { 3.0 } else { 2.0 };
            let text_top = top + row_gap / 2.0;
            let glyph = SceneRect {
                left: content_left,
                top: text_top + (line - glyph_size) / 2.0,
                width: glyph_size,
                height: glyph_size,
            };
            let icon = icon.map(|provider| {
                (
                    provider,
                    SceneRect {
                        left: glyph.right() + spacing,
                        top: text_top + (line - icon_size) / 2.0,
                        width: icon_size,
                        height: icon_size,
                    },
                )
            });
            rows.push(SidebarRow {
                tab: tab.id.clone(),
                target: target.clone(),
                rect: SceneRect {
                    left: list.left,
                    top,
                    width: list.width,
                    height: line * lines + row_gap,
                },
                glyph: TabBadge {
                    state: attention.state,
                    dimmed: stale,
                    rect: glyph,
                },
                text_left: icon.map_or(glyph.right(), |(_, rect)| rect.right()) + spacing,
                icon,
                title,
                detail,
                popup_marker,
                message,
                focused: view.focused == Some(target.as_str()),
                current,
                age,
                accessible_label,
            });
            top += line * lines + row_gap;
        }
    }
    // Reveal the focused row: scroll just far enough to show its bottom, never past its top.
    let scroll = rows.iter().find(|row| row.focused).map_or(0.0, |row| {
        (row.rect.bottom() - list.bottom())
            .min(row.rect.top - list.top)
            .max(0.0)
    });
    for heading in &mut headings {
        heading.rect.top -= scroll;
    }
    for row in &mut rows {
        row.rect.top -= scroll;
        row.glyph.rect.top -= scroll;
        if let Some((_, rect)) = &mut row.icon {
            rect.top -= scroll;
        }
    }
    (headings, rows, list)
}

/// The area `count` cards share: inset from the content area's sides by the card gap, starting
/// below the bar and ending one gap above the window's bottom, or at the footer,
/// whose band is the bottom margin when hints show. Gaps yield before a card loses the
/// `minimum_height` its last padded terminal row needs. The content area starts at `left` and is
/// `width` wide.
#[allow(clippy::too_many_arguments)]
fn card_area(
    left: f32,
    width: f32,
    count: usize,
    tab_height: f32,
    content_height: f32,
    hints: bool,
    minimum_height: f32,
    metrics: CellMetrics,
) -> SceneRect {
    let gap = card_gap(metrics);
    let side = side_gap(width, count, metrics);
    let mut spare = (content_height - tab_height - minimum_height).max(0.0);
    let below = if hints { 0.0 } else { gap.min(spare) };
    spare -= below;
    let inset = (gap - tab_inset(metrics, tab_height)).max(0.0).min(spare);
    spare -= inset;
    let top = (tab_height + inset + bar_title_margin(metrics).min(spare)).min(content_height);
    SceneRect {
        left: left + side,
        top,
        width: (width - side * 2.0).max(0.0),
        height: (content_height - below - top).max(0.0),
    }
}

/// The header's regions. A requested lead before the tab viewport, the workspace label and the
/// gap after it, is kept while the tabs keep their minimum after it, and quota chips show only
/// once it is kept; the tab viewport starts after it.
fn workspace_header_geometry(
    size: PhysicalSize<u32>,
    tab_height: f32,
    metrics: CellMetrics,
    requested_lead: f32,
    requested_quota_width: Option<f32>,
    show_shortcuts: bool,
    entries: &[PopupEntry],
) -> (
    SceneRect,
    SceneRect,
    SceneRect,
    Option<SceneRect>,
    [WorkspaceControl; 3],
) {
    let width = size.width as f32;
    // One identity pill after its full-height left inset, plus the gap that follows it.
    let identity_width = metrics.width
        + metrics.padding * 2.0
        + card_gap(metrics)
        + tab_gap(metrics, tab_height)
        + 1.0;
    let minimum_tabs = (width * 0.4)
        .min(metrics.font_size * 4.0)
        .max(identity_width.min((width - metrics.width * 3.0).max(0.0)));
    let control_count = if show_shortcuts { 3.0 } else { 2.0 };
    let control_width =
        tab_row(metrics, tab_height).min(((width - minimum_tabs) / control_count).max(0.0));
    let controls_width = control_width * control_count;
    let drag_width =
        (metrics.font_size * 4.0).min((width - controls_width - minimum_tabs).max(0.0));
    // The room past the tabs' minimum, which the lead takes before any quota chip.
    let spare = (width - controls_width - drag_width - minimum_tabs).max(0.0);
    let lead_fits = requested_lead.is_finite() && requested_lead <= spare;
    let lead = if lead_fits { requested_lead } else { 0.0 };
    let quota_width = requested_quota_width
        .filter(|quota| lead_fits && quota.is_finite() && *quota <= spare - lead)
        .unwrap_or(0.0);
    let tab_width = (width - controls_width - drag_width - lead - quota_width).max(0.0);
    let header = SceneRect {
        width,
        height: tab_height,
        ..SceneRect::default()
    };
    let tab_viewport = SceneRect {
        left: lead,
        width: tab_width,
        height: tab_height,
        ..SceneRect::default()
    };
    let drag_region = SceneRect {
        left: tab_viewport.right(),
        width: drag_width,
        height: tab_height,
        ..SceneRect::default()
    };
    let quota = (quota_width > 0.0).then_some(SceneRect {
        left: drag_region.right(),
        width: quota_width,
        height: tab_height,
        ..SceneRect::default()
    });
    let control_kinds = [
        WorkspaceHeaderControl::NewTab,
        WorkspaceHeaderControl::Shortcuts,
        WorkspaceHeaderControl::CloseTab,
    ];
    let mut left = drag_region.right() + quota_width;
    let controls = control_kinds.map(|kind| {
        let width = if kind == WorkspaceHeaderControl::Shortcuts && !show_shortcuts {
            0.0
        } else {
            control_width
        };
        let control = WorkspaceControl {
            kind,
            rect: SceneRect {
                left,
                width,
                height: tab_height,
                ..SceneRect::default()
            },
            label: kind.label(entries),
        };
        left += width;
        control
    });
    (header, tab_viewport, drag_region, quota, controls)
}

/// For each tab-viewport edge that badges are scrolled past, the most urgent of them, moved just
/// inside that edge: by state, then undimmed before dimmed. Also the viewport less the columns
/// those badges take, which clips tab labels.
fn edge_badges(tabs: &[WorkspaceTab], viewport: SceneRect, gap: f32) -> (Vec<TabBadge>, SceneRect) {
    let inset = (gap / 2.0).floor();
    let hidden = |past: &dyn Fn(SceneRect) -> bool| {
        tabs.iter()
            .filter_map(|tab| tab.badge)
            .filter(|badge| past(badge.rect))
            .min_by_key(|badge| (badge.state, badge.dimmed))
    };
    let left = hidden(&|rect| rect.left < viewport.left).map(|mut badge| {
        badge.rect.left = viewport.left + inset;
        badge
    });
    let right = hidden(&|rect| rect.right() > viewport.right()).map(|mut badge| {
        badge.rect.left = viewport.right() - inset - badge.rect.width;
        badge
    });
    let label_left = left.map_or(viewport.left, |badge| badge.rect.right());
    let label_right = right.map_or(viewport.right(), |badge| badge.rect.left);
    (
        left.into_iter().chain(right).collect(),
        SceneRect {
            left: label_left,
            width: (label_right - label_left).max(0.0),
            ..viewport
        },
    )
}

fn quota_duration(minutes: u32) -> String {
    if minutes.is_multiple_of(24 * 60) {
        format!("{}d", minutes / (24 * 60))
    } else if minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn quota_window_position(window: &QuotaWindow, observed_at: u64) -> Option<String> {
    let total_seconds = u64::from(window.duration_minutes) * 60;
    let remaining_seconds = window
        .resets_at?
        .saturating_sub(observed_at)
        .min(total_seconds);
    let elapsed_minutes = (total_seconds - remaining_seconds) / 60;
    if window.duration_minutes >= 24 * 60 {
        let days = elapsed_minutes / (24 * 60);
        let hours = elapsed_minutes % (24 * 60) / 60;
        Some(match (days, hours) {
            (0, 0) => "0h".into(),
            (0, hours) => format!("{hours}h"),
            (days, 0) => format!("{days}d"),
            (days, hours) => format!("{days}d{hours}h"),
        })
    } else if window.duration_minutes >= 60 {
        let hours = elapsed_minutes / 60;
        let minutes = elapsed_minutes % 60;
        Some(match (hours, minutes) {
            (0, minutes) => format!("{minutes}m"),
            (hours, 0) => format!("{hours}h"),
            (hours, minutes) => format!("{hours}h{minutes}m"),
        })
    } else {
        Some(format!("{elapsed_minutes}m"))
    }
}

fn quota_window_text(window: &QuotaWindow, observed_at: u64) -> String {
    let duration = quota_duration(window.duration_minutes);
    let label = match quota_window_position(window, observed_at) {
        Some(position) => format!("{position}/{duration}"),
        None => duration,
    };
    format!("{label} {}%", window.remaining_percent)
}

fn quota_text(
    provider: QuotaProvider,
    state: CodexQuotaState,
    observed_at: u64,
    windows: &[QuotaWindow],
) -> (String, String, String) {
    let suffix = if state == CodexQuotaState::Stale {
        " old"
    } else {
        ""
    };
    let wide = match state {
        CodexQuotaState::Fresh | CodexQuotaState::Stale => format!(
            "{}{suffix}",
            windows
                .iter()
                .map(|window| quota_window_text(window, observed_at))
                .collect::<Vec<_>>()
                .join(" · ")
        ),
        CodexQuotaState::Blocked => "blocked".into(),
        CodexQuotaState::Unknown => "unknown".into(),
    };
    let compact = match state {
        CodexQuotaState::Fresh | CodexQuotaState::Stale => {
            let window = windows
                .iter()
                .min_by_key(|window| (window.remaining_percent, window.duration_minutes))
                .expect("EONW requires quota windows for fresh and stale states");
            format!("{}{suffix}", quota_window_text(window, observed_at))
        }
        CodexQuotaState::Blocked => "blocked".into(),
        CodexQuotaState::Unknown => "unknown".into(),
    };
    let name = provider.name();
    let mut description = format!(
        "{name} quota {}",
        match state {
            CodexQuotaState::Fresh => "state: fresh.",
            CodexQuotaState::Stale => "state: stale.",
            CodexQuotaState::Blocked => "permission: blocked.",
            CodexQuotaState::Unknown => "permission: unknown.",
        }
    );
    for window in windows {
        let duration = quota_duration(window.duration_minutes);
        let _ = write!(
            description,
            " {duration} window: {}% remaining; ",
            window.remaining_percent
        );
        if let Some(position) = quota_window_position(window, observed_at) {
            let _ = write!(description, "observed {position} into {duration}.");
        } else {
            description.push_str("reset time unavailable.");
        }
    }
    (wide, compact, description)
}

fn stack_inset(metrics: CellMetrics) -> f32 {
    metrics.padding / 3.0
}

fn popup_title_gutter(metrics: CellMetrics) -> f32 {
    metrics.padding / 2.0
}

fn pane_inset(metrics: CellMetrics) -> f32 {
    stack_inset(metrics).min(metrics.height / 4.0)
}

/// Insets chrome vertically only, keeping a frameless header fill off its neighbours.
pub(crate) fn pane_chrome_rect(rect: SceneRect, metrics: CellMetrics) -> SceneRect {
    let inset = pane_inset(metrics);
    SceneRect {
        top: rect.top + inset / 2.0,
        height: (rect.height - inset).max(0.0),
        ..rect
    }
}

impl WorkspaceScene {
    /// Apply the bar's painted band without changing a slot's horizontal fitting.
    pub(crate) fn bar_pill_rect(&self, rect: SceneRect, metrics: CellMetrics) -> SceneRect {
        let band = bar_pill_band(metrics, self.header.height, self.pane_viewport.top);
        SceneRect {
            top: band.start,
            height: band.end - band.start,
            ..rect
        }
    }

    /// Centre existing content and control shapes on the band without resizing them.
    pub(crate) fn bar_item_rect(&self, rect: SceneRect, metrics: CellMetrics) -> SceneRect {
        let band = self.bar_pill_rect(rect, metrics);
        SceneRect {
            top: band.top + (band.height - rect.height) / 2.0,
            ..rect
        }
    }

    /// Maximum shaped label width inside this surface's tab viewport.
    #[must_use]
    pub(crate) fn tab_text_width(size: PhysicalSize<u32>, metrics: CellMetrics) -> f32 {
        // Header widths depend only on the pill row, never on the header's extra inset.
        let tab_height = header_heights(metrics).0.min(size.height as f32);
        let (_, tab_viewport, _, _, _) =
            workspace_header_geometry(size, tab_height, metrics, 0.0, None, true, &[]);
        (tab_max_width(metrics, tab_viewport.width) - metrics.padding * 2.0)
            .max(0.0)
            .floor()
    }

    /// Extra pixels around an initially requested terminal grid.
    #[must_use]
    pub fn initial_overhead(snapshot: &Snapshot, metrics: CellMetrics) -> (f32, f32) {
        let (tab_height, pane_height) = header_heights(metrics);
        // Cards sit inside the side gaps below the bar; the footer is their
        // bottom margin.
        let gap = card_gap(metrics);
        let sides = gap * 2.0;
        let above = ShortcutHintsScene::reserved_height(metrics)
            + tab_height
            + (gap - tab_inset(metrics, tab_height)).max(0.0)
            + bar_title_margin(metrics);
        if active_popup(snapshot).is_some() {
            (
                snapshot.geometry.side_margin * metrics.scale * 2.0 + sides,
                above
                    + snapshot.geometry.vertical_margin * metrics.scale * 2.0
                    + (title_rule_y(tab_height, pane_height) - tab_height)
                    + popup_title_gutter(metrics),
            )
        } else {
            let tab = snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == snapshot.active_tab)
                .expect("EONW validates the active tab");
            (
                sides,
                above + pane_height * presented_panes(tab).len() as f32,
            )
        }
    }
    #[must_use]
    pub fn from_snapshot(
        snapshot: &Snapshot,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        tab_scroll: f32,
        pane_scroll: f32,
        header_text: impl FnMut(HeaderText<'_>, &str) -> (String, f32),
    ) -> Self {
        Self::from_snapshot_with_bar(
            snapshot,
            EonBar::default(),
            size,
            metrics,
            tab_scroll,
            pane_scroll,
            header_text,
            None,
        )
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_snapshot_with_bar(
        snapshot: &Snapshot,
        bar: EonBar<'_>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        tab_scroll: f32,
        pane_scroll: f32,
        header_text: impl FnMut(HeaderText<'_>, &str) -> (String, f32),
        hints: Option<ShortcutHintsScene>,
    ) -> Self {
        Self::from_snapshot_with_zoom(
            snapshot,
            bar,
            size,
            metrics,
            tab_scroll,
            pane_scroll,
            header_text,
            hints,
            false,
            SidebarView::default(),
        )
    }

    /// Whether zoom is visible rather than refused or covered by a popup.
    #[must_use]
    pub fn zoom_visible(snapshot: &Snapshot, requested: bool) -> bool {
        requested
            && active_popup(snapshot).is_none()
            && snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == snapshot.active_tab)
                .is_some_and(|tab| tab.panes().nth(1).is_some())
    }

    /// The scene with `zoom` laying out only the selected pane, across every card, when the
    /// active tab has more than one pane and no popup covers it.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_snapshot_with_zoom(
        snapshot: &Snapshot,
        bar: EonBar<'_>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        tab_scroll: f32,
        pane_scroll: f32,
        mut header_text: impl FnMut(HeaderText<'_>, &str) -> (String, f32),
        hints: Option<ShortcutHintsScene>,
        zoom: bool,
        sidebar_view: SidebarView<'_>,
    ) -> Self {
        let width = size.width as f32;
        let height = size.height as f32;
        let pane_height = header_heights(metrics).1;
        let show_shortcuts = hints.as_ref().is_none_or(|hints| hints.keep_shortcuts);
        let content_height = hints.as_ref().map_or(height, |hints| hints.rect.top);
        let active = &snapshot.tabs[snapshot
            .tabs
            .iter()
            .position(|tab| tab.id == snapshot.active_tab)
            .expect("EONW validates the active tab")];
        // Each column needs one padded cell below its first header.
        let minimum_height = minimum_terminal(metrics, metrics.height)
            + if active.columns.is_empty() {
                0.0
            } else {
                pane_height
            };
        let tab_height = header_height(metrics, height, content_height, minimum_height);
        let gap = tab_gap(metrics, tab_height);
        let inset = tab_inset(metrics, tab_height);
        // The workspace label fits its name to a tab's widest text and keeps its position whole.
        let mut workspace_label = bar_workspace(snapshot).map(|(name, position, count)| {
            let (suffix, suffix_width) = if count >= 2 {
                header_text(HeaderText::Fact, &format!(" {position}/{count}"))
            } else {
                (String::new(), 0.0)
            };
            let fit = (Self::tab_text_width(size, metrics) - suffix_width).max(0.0);
            let (fitted, name_width) = header_text(HeaderText::Fit(fit), name);
            WorkspaceNameLabel {
                rect: SceneRect {
                    left: inset,
                    top: inset,
                    width: (name_width + suffix_width).ceil() + metrics.padding * 2.0,
                    height: (tab_height - inset * 2.0).max(0.0),
                },
                label: fitted + &suffix,
                accessible_label: format!("Workspace {name}, {position} of {count}"),
                tooltip: bar.workspace_picker.map(str::to_owned),
                badge: None,
            }
        });
        let lead = workspace_label
            .as_ref()
            .map_or(0.0, |label| label.rect.right() + gap);
        let quota_texts = bar
            .items
            .iter()
            .flat_map(|item| match item {
                BarItem::CodexQuota => snapshot
                    .codex_quota
                    .iter()
                    .map(|quota| {
                        (
                            QuotaProvider::Codex,
                            None,
                            quota_text(
                                QuotaProvider::Codex,
                                quota.state,
                                quota.observed_at,
                                &quota.windows,
                            ),
                        )
                    })
                    .collect::<Vec<_>>(),
                // Claude gets one chip per window, five-hour then seven-day.
                BarItem::ClaudeQuota => snapshot
                    .claude_quota
                    .iter()
                    .flat_map(|quota| {
                        let state = match quota.state {
                            ClaudeQuotaState::Fresh => CodexQuotaState::Fresh,
                            ClaudeQuotaState::Stale => CodexQuotaState::Stale,
                        };
                        let mut windows = quota.windows.iter().collect::<Vec<_>>();
                        windows.sort_by_key(|window| window.duration_minutes);
                        windows.into_iter().map(move |window| {
                            (
                                QuotaProvider::Claude,
                                Some(window.duration_minutes),
                                quota_text(
                                    QuotaProvider::Claude,
                                    state,
                                    quota.observed_at,
                                    std::slice::from_ref(window),
                                ),
                            )
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        let quota_padding = metrics.padding * 2.0 + metrics.font_size + metrics.padding / 2.0;
        let mut measure_quotas = |compact| {
            quota_texts
                .iter()
                .map(|(provider, window, (wide, short, description))| {
                    let (label, width) =
                        header_text(HeaderText::Fact, if compact { short } else { wide });
                    WorkspaceQuota {
                        provider: *provider,
                        window: *window,
                        rect: SceneRect {
                            width: width + quota_padding,
                            ..SceneRect::default()
                        },
                        logo: SceneRect::default(),
                        accessible_label: format!("{} {label}", provider.name()),
                        label,
                        description: description.clone(),
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut quotas = measure_quotas(false);
        let mut compact = false;
        let geometry = loop {
            let requested =
                (!quotas.is_empty()).then(|| quotas.iter().map(|quota| quota.rect.width).sum());
            let geometry = workspace_header_geometry(
                size,
                tab_height,
                metrics,
                lead,
                requested,
                show_shortcuts,
                &snapshot.entries,
            );
            if geometry.3.is_some() || quotas.is_empty() {
                break geometry;
            }
            if compact {
                // The chip closest to the tabs yields first.
                quotas.remove(0);
            } else {
                quotas = measure_quotas(true);
                compact = true;
            }
        };
        let (header, mut tab_viewport, mut drag_region, quota_rect, mut controls) = geometry;
        // Without room for the tabs' minimum after it, the label is dropped.
        if tab_viewport.left == 0.0 {
            workspace_label = None;
        }
        let mut left = quota_rect.map_or(0.0, |rect| rect.left + controls[0].rect.width);
        for quota in &mut quotas {
            quota.rect.left = left;
            quota.rect.height = tab_height;
            quota.logo = SceneRect {
                left: left + metrics.padding,
                top: (tab_height - metrics.font_size) / 2.0,
                width: metrics.font_size,
                height: metrics.font_size,
            };
            left = quota.rect.right();
        }
        let max_tab_width = tab_max_width(metrics, tab_viewport.width);
        let active_tab = snapshot
            .tabs
            .iter()
            .position(|tab| tab.id == snapshot.active_tab)
            .expect("EONW validates the active tab");
        let zoomed = Self::zoom_visible(snapshot, zoom);
        // Every card shares this area; a popup or an empty tab uses all of it as one card. The
        // area follows the tab's columns whether or not a popup covers them, so toggling a popup
        // keeps the outer card edges: each column needs one padded cell below its first header.
        // A visible sidebar narrows it to the content area at its right.
        let cards = |left: f32| {
            card_area(
                left,
                width - left,
                active.columns.len().max(1),
                tab_height,
                content_height,
                hints.is_some(),
                minimum_height,
                metrics,
            )
        };
        let sidebar = sidebar_rect(snapshot.sidebar, width, cards(0.0), metrics);
        let pane_viewport = cards(sidebar.map_or(0.0, SceneRect::right));
        let pill_band = bar_pill_band(metrics, tab_height, pane_viewport.top);
        // Unlike tab badges, it shows with the sidebar, which lists only the shown workspace.
        if let Some(label) = &mut workspace_label
            && hidden_needs_input(snapshot)
        {
            label
                .accessible_label
                .push_str(", another workspace needs input");
            let pill = SceneRect {
                top: pill_band.start,
                height: pill_band.end - pill_band.start,
                ..label.rect
            };
            label.badge = TabBadge::corner(pill, metrics, gap).map(|rect| TabBadge {
                state: AttentionState::NeedsInput,
                dimmed: false,
                rect,
            });
        }
        for quota in &mut quotas {
            quota.logo.top =
                pill_band.start + (pill_band.end - pill_band.start - quota.logo.height) / 2.0;
        }
        let (sidebar_headings, sidebar_rows, sidebar_list) = sidebar.map_or_else(
            || (Vec::new(), Vec::new(), SceneRect::default()),
            |card| sidebar_contents(snapshot, card, metrics, sidebar_view),
        );
        // Tabs follow the label's gap, or else the bar's left inset.
        let mut tab_end = if workspace_label.is_some() {
            tab_viewport.left
        } else {
            inset
        };
        let mut tabs: Vec<_> = snapshot
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let (mut label, mut accessible_label) =
                    tab_labels(index, snapshot.tabs.len(), &tab.directory);
                if zoomed && index == active_tab {
                    label.push_str(" \u{f00e}");
                    accessible_label.push_str(", zoomed");
                }
                let (label, text_width) = header_text(HeaderText::Tab(&tab.id), &label);
                let tab_width = (text_width.ceil() + metrics.padding * 2.0)
                    .clamp((metrics.font_size * 4.0).min(max_tab_width), max_tab_width);
                let left = tab_end;
                tab_end = left + tab_width + gap;
                let rect = SceneRect {
                    left,
                    top: inset,
                    width: tab_width,
                    height: (tab_height - inset * 2.0).max(0.0),
                };
                // VEN-C21 badges show while the agent sidebar is not visible.
                let badge = TabBadge::of(tab).filter(|_| sidebar.is_none()).and_then(
                    |(state, word, dimmed)| {
                        accessible_label.push_str(", ");
                        accessible_label.push_str(word);
                        if dimmed {
                            accessible_label.push_str(", status stale");
                        }
                        Some(TabBadge {
                            state,
                            dimmed,
                            rect: TabBadge::corner(
                                SceneRect {
                                    top: pill_band.start,
                                    height: pill_band.end - pill_band.start,
                                    ..rect
                                },
                                metrics,
                                gap,
                            )?,
                        })
                    },
                );
                WorkspaceTab {
                    id: tab.id.clone(),
                    selected: index == active_tab,
                    rect,
                    label,
                    accessible_label,
                    badge,
                }
            })
            .collect();
        tab_viewport.width = tab_viewport.width.min(tab_end - tab_viewport.left);
        controls[0].rect.left = tab_viewport.right();
        drag_region.left = controls[0].rect.right();
        drag_region.width = (quotas
            .first()
            .map_or(controls[1].rect.left, |quota| quota.rect.left)
            - drag_region.left)
            .max(0.0);
        // Read-only labels take spare header space without shortening tabs, in this order, and
        // stop at the first that does not fit: the clock yields first, then RAM, then CPU use,
        // then the version label. The readings also need every quota chip at full width.
        let shown = |item| bar.items.contains(&item);
        let reading = |item| !compact && shown(item);
        // CPU and RAM keep the width of `CPU 100%`, so the header never shifts as they change.
        let readings = "CPU 100%".len();
        let mut labels = [
            bar.version
                .filter(|_| shown(BarItem::Version))
                .map(|version| {
                    (
                        format!("EON {version}"),
                        format!("Running Eon version {version}"),
                        0,
                    )
                }),
            bar.cpu
                .filter(|_| reading(BarItem::Cpu))
                .map(|cpu| (cpu.label.clone(), cpu.description.clone(), readings)),
            bar.ram
                .filter(|_| reading(BarItem::Ram))
                .map(|ram| (ram.label.clone(), ram.description.clone(), readings)),
            bar.clock
                .filter(|_| reading(BarItem::Clock))
                .map(|clock| (clock.label.clone(), clock.description.clone(), 0)),
        ]
        .map(|label| {
            label.map(|(label, description, cells)| WorkspaceLabel {
                rect: SceneRect {
                    // Every label is ASCII (launch validation bounds the version), and
                    // rendering uses terminal cell spacing.
                    width: label.len().max(cells) as f32 * metrics.width + metrics.padding * 2.0,
                    height: tab_height,
                    ..SceneRect::default()
                },
                label,
                description,
            })
        });
        let mut fits = true;
        for label in &mut labels {
            let Some(width) = label.as_ref().map(|label| label.rect.width) else {
                continue;
            };
            fits &= width.is_finite()
                && width > 0.0
                && width + metrics.font_size * 4.0 <= drag_region.width;
            if fits {
                drag_region.width -= width;
            } else {
                *label = None;
            }
        }
        // Quota chips and labels sit in Eon's order between the drag region and Shortcuts.
        let [version, cpu, ram, clock] = labels;
        let mut labels = [
            (BarItem::Cpu, cpu),
            (BarItem::Ram, ram),
            (BarItem::Clock, clock),
            (BarItem::Version, version),
        ];
        let mut left = drag_region.right();
        for item in bar.items {
            if let Some(label) = labels
                .iter_mut()
                .find_map(|(kind, label)| label.as_mut().filter(|_| kind == item))
            {
                label.rect.left = left;
                left = label.rect.right();
            } else {
                for quota in quotas
                    .iter_mut()
                    .filter(|quota| quota.provider.item() == *item)
                {
                    quota.rect.left = left;
                    quota.logo.left = left + metrics.padding;
                    left = quota.rect.right();
                }
            }
        }
        let [(_, cpu), (_, ram), (_, clock), (_, version)] = labels;
        let tab_scroll_limit = (tab_end - tab_viewport.right()).max(0.0);
        let active = tabs[active_tab].rect;
        // Revealing the first tab never scrolls.
        let active_tab_scroll = if active_tab == 0 {
            0.0
        } else {
            (active.left - tab_viewport.left - (tab_viewport.width - active.width).max(0.0) / 2.0)
                .clamp(0.0, tab_scroll_limit)
        };
        let tab_scroll = if tab_scroll.is_finite() {
            tab_scroll.clamp(0.0, tab_scroll_limit)
        } else {
            0.0
        };
        for tab in &mut tabs {
            tab.rect.left -= tab_scroll;
            if let Some(badge) = &mut tab.badge {
                badge.rect.left -= tab_scroll;
            }
        }
        let (edge_badges, tab_label_clip) = edge_badges(&tabs, tab_viewport, gap);
        let active = &snapshot.tabs[active_tab];
        let popup = active_popup(snapshot);
        let card_radius = card_gap(metrics);
        if popup.is_some() || active.columns.is_empty() {
            let horizontal_inset = (snapshot.geometry.side_margin * metrics.scale).min(
                (pane_viewport.width - minimum_terminal(metrics, metrics.width)).max(0.0) / 2.0,
            );
            let edge = if popup.is_some() {
                (popup_card_rect(pane_viewport, metrics).top - pane_viewport.top).max(0.0)
            } else {
                0.0
            };
            let vertical_inset = (snapshot.geometry.vertical_margin * metrics.scale).min(
                (pane_viewport.height - edge - minimum_terminal(metrics, metrics.height)).max(0.0)
                    / 2.0,
            );
            let title_gutter = if popup.is_some() {
                // Reserve the painted edge before the optional label gutter and margins.
                edge + popup_title_gutter(metrics).min(
                    (pane_viewport.height
                        - edge
                        - vertical_inset * 2.0
                        - metrics.padding * 2.0
                        - metrics.height)
                        .max(0.0),
                )
            } else {
                0.0
            };
            return Self {
                tabs,
                edge_badges,
                tab_label_clip,
                header,
                workspace_label,
                drag_region,
                quotas,
                version,
                cpu,
                ram,
                clock,
                controls,
                hints,
                panes: Vec::new(),
                selected_column: 0,
                other_columns: Vec::new(),
                terminal: SceneRect {
                    left: pane_viewport.left + horizontal_inset,
                    top: pane_viewport.top + vertical_inset + title_gutter,
                    width: if popup.is_some() {
                        (pane_viewport.width - horizontal_inset * 2.0).max(0.0)
                    } else {
                        0.0
                    },
                    height: if popup.is_some() {
                        (pane_viewport.height - vertical_inset * 2.0 - title_gutter).max(0.0)
                    } else {
                        0.0
                    },
                },
                tab_viewport,
                pane_viewport,
                chrome: pane_viewport,
                displayed_grid_bottoms: Vec::new(),
                sidebar,
                sidebar_headings,
                sidebar_rows,
                sidebar_list,
                card_radius,
                tab_scroll,
                pane_scroll: 0.0,
                tab_scroll_limit,
                pane_scroll_limit: 0.0,
                active_tab_scroll,
                selected_pane_scroll: 0.0,
                popup_label: popup.map(|popup| {
                    snapshot
                        .entries
                        .iter()
                        .find(|entry| entry.id == popup.entry)
                        .expect("EONW validates popup entries")
                        .label
                        .clone()
                }),
                zoomed: false,
            };
        }

        let active = &snapshot.tabs[active_tab];
        let pane_height = pane_height.min(pane_viewport.height);
        let selected = active
            .selected_pane
            .as_ref()
            .expect("EONW validates the selected pane");
        let selected_column = active
            .columns
            .iter()
            .position(|column| column.expanded == *selected)
            .expect("EONW validates the selected pane");
        let count = active.columns.len();
        let content_width = width - sidebar.map_or(0.0, SceneRect::right);
        let column_gap = side_gap(content_width, count, metrics);
        let column_width = workspace_column_width(content_width, count, metrics);
        let maximum_terminal_height = (pane_viewport.height - pane_height).max(0.0);
        let minimum_terminal_height =
            minimum_terminal(metrics, metrics.height).min(maximum_terminal_height);
        let mut selected_layout = None;
        let mut other_columns = Vec::with_capacity(count - 1);
        // Zoom gives the selected pane every card's width and only one title band.
        let slots = if zoomed { 1 } else { count };
        for (slot, (index, column)) in active
            .columns
            .iter()
            .enumerate()
            .skip(if zoomed { selected_column } else { 0 })
            .take(slots)
            .enumerate()
        {
            let left = pane_viewport.left + slot as f32 * (column_width + column_gap);
            let column_width = if slot + 1 == slots {
                (pane_viewport.right() - left).max(0.0)
            } else {
                column_width
            };
            let expanded = if zoomed {
                0
            } else {
                column
                    .panes
                    .iter()
                    .position(|pane| pane.id == column.expanded)
                    .expect("EONW validates the expanded pane")
            };
            let pane_headers_height =
                if zoomed { 1 } else { column.panes.len() } as f32 * pane_height;
            let terminal_height = (pane_viewport.height - pane_headers_height)
                .clamp(minimum_terminal_height, maximum_terminal_height);
            let pane_scroll_limit =
                (pane_headers_height + terminal_height - pane_viewport.height).max(0.0);
            let expanded_scroll = (expanded as f32 * pane_height).clamp(0.0, pane_scroll_limit);
            // Only the selected column follows the user's header scroll; others reveal
            // their expanded pane.
            let column_scroll = if index != selected_column {
                expanded_scroll
            } else if pane_scroll.is_finite() {
                pane_scroll.clamp(0.0, pane_scroll_limit)
            } else {
                0.0
            };
            let panes = column
                .panes
                .iter()
                .filter(|pane| !zoomed || pane.id == *selected)
                .enumerate()
                .map(|(position, pane)| {
                    let label = pane.label.clone();
                    WorkspacePane {
                        id: pane.id.clone(),
                        live: pane.live,
                        selected: position == expanded,
                        rect: SceneRect {
                            left,
                            top: pane_viewport.top
                                + position as f32 * pane_height
                                + if position > expanded {
                                    terminal_height
                                } else {
                                    0.0
                                }
                                - column_scroll,
                            width: column_width,
                            height: pane_height,
                        },
                        label,
                    }
                })
                .collect::<Vec<_>>();
            let terminal = SceneRect {
                left,
                top: pane_viewport.top + (expanded + 1) as f32 * pane_height - column_scroll,
                width: column_width,
                height: terminal_height,
            };
            let chrome = SceneRect {
                left,
                width: column_width,
                ..pane_viewport
            };
            if index == selected_column {
                selected_layout = Some((
                    panes,
                    terminal,
                    chrome,
                    column_scroll,
                    pane_scroll_limit,
                    expanded_scroll,
                ));
            } else {
                other_columns.push(WorkspaceColumn {
                    index,
                    panes,
                    terminal,
                    chrome,
                });
            }
        }
        let (panes, terminal, chrome, pane_scroll, pane_scroll_limit, selected_pane_scroll) =
            selected_layout.expect("the selected column is laid out");

        Self {
            tabs,
            edge_badges,
            tab_label_clip,
            header,
            workspace_label,
            drag_region,
            quotas,
            version,
            cpu,
            ram,
            clock,
            controls,
            hints,
            panes,
            terminal,
            selected_column,
            other_columns,
            tab_viewport,
            pane_viewport,
            chrome,
            displayed_grid_bottoms: Vec::new(),
            sidebar,
            sidebar_headings,
            sidebar_rows,
            sidebar_list,
            card_radius,
            tab_scroll,
            pane_scroll,
            tab_scroll_limit,
            pane_scroll_limit,
            active_tab_scroll,
            selected_pane_scroll,
            popup_label: None,
            zoomed,
        }
    }

    /// The agent sidebar's card while it is visible: `snapshot.sidebar` is `Open` and the window
    /// is wide enough.
    #[must_use]
    pub fn sidebar(&self) -> Option<SceneRect> {
        self.sidebar
    }

    /// The sidebar's title layout and clipping include the area above its painted top edge.
    pub(crate) fn sidebar_area(&self) -> Option<SceneRect> {
        self.sidebar.map(|sidebar| SceneRect {
            left: sidebar.left,
            width: sidebar.width,
            ..self.pane_viewport
        })
    }

    /// Whether a visible sidebar shows its empty state: no tab has attention.
    pub(crate) fn sidebar_empty(&self) -> bool {
        self.sidebar.is_some() && self.sidebar_rows.is_empty()
    }

    /// The visible sidebar's rows in tab and terminal order.
    #[must_use]
    pub fn sidebar_rows(&self) -> &[SidebarRow] {
        &self.sidebar_rows
    }

    /// The row whose target is `target`.
    #[must_use]
    pub fn sidebar_row(&self, target: &str) -> Option<&SidebarRow> {
        self.sidebar_rows.iter().find(|row| row.target == target)
    }

    /// Milliseconds until a row's age next changes its label.
    #[must_use]
    pub fn sidebar_age_wait(&self) -> Option<u64> {
        self.sidebar_rows
            .iter()
            .filter_map(|row| row.age)
            .map(age_wait)
            .min()
    }

    /// The width the cards share: the window right of a visible sidebar.
    #[must_use]
    pub fn content_width(&self, window: f32) -> f32 {
        window - self.sidebar.map_or(0.0, SceneRect::right)
    }

    /// Corner radius of every pane and popup card.
    pub(crate) fn card_radius(&self) -> f32 {
        self.card_radius
    }

    /// The sole pane while window-local zoom is visible.
    #[must_use]
    pub fn zoomed_pane(&self) -> Option<&WorkspacePane> {
        self.panes.first().filter(|_| self.zoomed)
    }

    /// Every laid-out column's headers left to right, each flagged when it is the selected
    /// column.
    pub fn column_panes(&self) -> impl Iterator<Item = (&[WorkspacePane], bool)> {
        let selected = (!self.panes.is_empty()).then(|| {
            self.other_columns
                .iter()
                .filter(|column| column.index < self.selected_column)
                .count()
        });
        let mut others = self.other_columns.iter();
        (0..self.other_columns.len() + usize::from(selected.is_some())).map(move |index| {
            if Some(index) == selected {
                (self.panes.as_slice(), true)
            } else {
                (
                    others
                        .next()
                        .expect("other columns fill every remaining position")
                        .panes
                        .as_slice(),
                    false,
                )
            }
        })
    }

    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<WorkspaceHit<'_>> {
        for tab in &self.tabs {
            if tab
                .rect
                .intersection(self.tab_viewport)
                .is_some_and(|rect| rect.contains(x, y))
            {
                return Some(WorkspaceHit::Tab(&tab.id));
            }
        }
        // An inert workspace label is no target.
        if self.has_control(WorkspaceHeaderControl::Workspaces)
            && self
                .workspace_label
                .as_ref()
                .is_some_and(|label| label.rect.contains(x, y))
        {
            return Some(WorkspaceHit::Control(WorkspaceHeaderControl::Workspaces));
        }
        for control in self.controls {
            if control.rect.contains(x, y) {
                return Some(WorkspaceHit::Control(control.kind));
            }
        }
        if let Some(index) = self
            .quotas
            .iter()
            .position(|quota| quota.rect.contains(x, y))
        {
            return Some(WorkspaceHit::Quota(index));
        }
        if self
            .version
            .as_ref()
            .is_some_and(|version| version.rect.contains(x, y))
        {
            return Some(WorkspaceHit::Version);
        }
        if self.cpu.as_ref().is_some_and(|cpu| cpu.rect.contains(x, y)) {
            return Some(WorkspaceHit::Cpu);
        }
        if self.ram.as_ref().is_some_and(|ram| ram.rect.contains(x, y)) {
            return Some(WorkspaceHit::Ram);
        }
        if self
            .clock
            .as_ref()
            .is_some_and(|clock| clock.rect.contains(x, y))
        {
            return Some(WorkspaceHit::Clock);
        }
        if self.drag_region.contains(x, y) {
            return Some(WorkspaceHit::Drag);
        }
        if let Some(row) = self.sidebar_rows.iter().find(|row| {
            row.rect
                .intersection(self.sidebar_list)
                .is_some_and(|rect| rect.contains(x, y))
        }) {
            return Some(WorkspaceHit::SidebarRow(&row.target));
        }
        for (panes, _) in self.column_panes() {
            for pane in panes {
                if pane
                    .rect
                    .intersection(self.pane_viewport)
                    .is_some_and(|rect| rect.contains(x, y))
                {
                    return Some(WorkspaceHit::Pane(&pane.id));
                }
            }
        }
        for column in &self.other_columns {
            if column
                .terminal
                .intersection(self.pane_viewport)
                .is_some_and(|rect| rect.contains(x, y))
                && let Some(pane) = column.panes.iter().find(|pane| pane.selected)
            {
                return Some(WorkspaceHit::ColumnTerminal(&pane.id));
            }
        }
        self.visible_terminal()
            .filter(|rect| rect.contains(x, y))
            .map(|_| WorkspaceHit::Terminal)
    }

    /// Whether `kind` shows and takes input; the workspace label does only while its popup
    /// entry exists.
    #[must_use]
    pub fn has_control(&self, kind: WorkspaceHeaderControl) -> bool {
        if kind == WorkspaceHeaderControl::Workspaces {
            return self
                .workspace_label
                .as_ref()
                .is_some_and(|label| label.tooltip.is_some());
        }
        self.controls
            .iter()
            .any(|control| control.kind == kind && control.rect.width > 0.0)
    }

    pub(crate) fn with_displayed_grids(
        &self,
        metrics: CellMetrics,
        grids: impl Iterator<Item = (SceneRect, SceneRect)>,
    ) -> Cow<'_, Self> {
        let displayed_grid_bottoms = grids
            .filter_map(|(terminal, grid)| {
                // Layout clipping bounds visible retained cells; card clipping must not hide a collision.
                let grid = grid
                    .intersection(terminal)?
                    .intersection(self.pane_viewport)?;
                self.grid_bottom(terminal, metrics)
                    .is_none_or(|requested| grid.bottom() > requested)
                    .then_some((terminal, grid.bottom()))
            })
            .collect::<Vec<_>>();
        if displayed_grid_bottoms.is_empty() {
            Cow::Borrowed(self)
        } else {
            let mut paint = self.clone();
            paint.displayed_grid_bottoms = displayed_grid_bottoms;
            Cow::Owned(paint)
        }
    }

    fn grid_bottom(&self, terminal: SceneRect, metrics: CellMetrics) -> Option<f32> {
        metrics
            .surface_size(terminal.screen_size())
            .map(|size| metrics.grid_rect(terminal, size.cols, size.rows).bottom())
            .into_iter()
            .chain(
                self.displayed_grid_bottoms
                    .iter()
                    .filter(|(area, _)| *area == terminal)
                    .map(|(_, bottom)| *bottom),
            )
            .reduce(f32::max)
    }

    /// Column and sidebar paint share one bottom, clear of every visible column's grid.
    pub(crate) fn card_rect(
        &self,
        allocated: SceneRect,
        metrics: CellMetrics,
        stroke_width: f32,
    ) -> SceneRect {
        let title_height = chrome_title_metrics(metrics)
            .text_height
            .min(allocated.height);
        let top = title_rule_y(allocated.top, title_height)
            .max(allocated.top)
            .min(allocated.bottom());
        let grid_bottom = std::iter::once(self.terminal)
            .chain(self.other_columns.iter().map(|column| column.terminal))
            .filter(|terminal| terminal.intersection(self.pane_viewport).is_some())
            .filter_map(|terminal| self.grid_bottom(terminal, metrics))
            .reduce(f32::max);
        let bottom = painted_bottom(allocated.bottom(), metrics, stroke_width, grid_bottom)
            .min(allocated.bottom());
        SceneRect {
            top,
            height: (bottom - top).max(0.0),
            ..allocated
        }
    }

    /// Paint geometry is separate from terminal layout, input and accessibility bounds.
    pub(crate) fn card_fill(
        &self,
        allocated: SceneRect,
        metrics: CellMetrics,
        stroke_width: f32,
    ) -> CardFill {
        let bounds = if self.popup_label.is_some() {
            // The popup keeps its own top edge but ends where column and sidebar cards end.
            let card = popup_card_rect(allocated, metrics);
            let bottom = self
                .card_rect(allocated, metrics, stroke_width)
                .bottom()
                .max(card.top);
            SceneRect {
                height: bottom - card.top,
                ..card
            }
        } else {
            self.card_rect(allocated, metrics, stroke_width)
        };
        CardFill {
            bounds,
            radius: self.card_radius,
        }
    }

    #[must_use]
    pub fn visible_terminal(&self) -> Option<SceneRect> {
        self.terminal.intersection(self.pane_viewport)
    }

    #[must_use]
    pub fn popup_label(&self) -> Option<&str> {
        self.popup_label.as_deref()
    }

    #[must_use]
    pub fn active_tab_scroll(&self) -> f32 {
        self.active_tab_scroll
    }

    #[must_use]
    pub fn selected_pane_scroll(&self) -> f32 {
        self.selected_pane_scroll
    }

    #[must_use]
    pub fn tab_scroll_limit(&self) -> f32 {
        self.tab_scroll_limit
    }

    #[must_use]
    pub fn pane_scroll_limit(&self) -> f32 {
        self.pane_scroll_limit
    }

    #[must_use]
    pub fn tab_scroll(&self) -> f32 {
        self.tab_scroll
    }

    #[must_use]
    pub fn pane_scroll(&self) -> f32 {
        self.pane_scroll
    }
}

/// Renderer-ready color with no protocol-relative palette reference left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<Rgb> for Color {
    fn from(value: Rgb) -> Self {
        Self {
            r: value.r,
            g: value.g,
            b: value.b,
        }
    }
}

/// Resolved draw style for one canonical Orbit cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawStyle {
    pub foreground: Color,
    pub background: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub selected: bool,
    pub background_is_default: bool,
    pub protected: bool,
    pub underline: Underline,
}

impl DrawStyle {
    pub(crate) fn foreground_visible(self, blink_visible: bool) -> bool {
        !self.invisible && (blink_visible || !self.blink)
    }

    pub(crate) fn foreground_alpha(self) -> u8 {
        if self.faint { 150 } else { 255 }
    }
}

/// One exact grid cell after palette and inverse-color resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawCell {
    pub width: CellWidth,
    pub text: String,
    pub hyperlink: String,
    pub style: DrawStyle,
}

/// One deterministic row of draw cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawRow {
    pub wrapped: bool,
    pub wrap_continuation: bool,
    pub cells: Vec<DrawCell>,
}

/// A contiguous visible row span, borrowing Orbit's exact target or owning a
/// target detected in plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink<'a> {
    pub row: u16,
    pub column: u16,
    pub columns: u16,
    pub uri: Cow<'a, str>,
}

impl Hyperlink<'_> {
    #[must_use]
    pub fn rect(&self, origin: SceneRect, metrics: CellMetrics) -> SceneRect {
        SceneRect {
            left: origin.left + metrics.padding + f32::from(self.column) * metrics.width,
            top: origin.top + metrics.padding + f32::from(self.row) * metrics.height,
            width: f32::from(self.columns) * metrics.width,
            height: metrics.height,
        }
    }

    #[must_use]
    pub fn contains(&self, row: u16, column: u16) -> bool {
        self.row == row && (self.column..self.column + self.columns).contains(&column)
    }
}

fn is_uri_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte)
}

/// Accept any bounded link target without control characters for copying.
pub fn validate_copy_uri(uri: &str) -> std::result::Result<(), &'static str> {
    if uri.is_empty() || uri.len() > MAX_LINK_BYTES {
        Err("Link target must contain 1–4096 bytes.")
    } else if uri.chars().any(char::is_control) {
        Err("Link target contains control characters.")
    } else {
        Ok(())
    }
}

/// Accept only an ASCII HTTP/HTTPS URI with a host and no credentials for opening.
pub fn validate_open_uri(uri: &str) -> std::result::Result<(), &'static str> {
    validate_copy_uri(uri)?;
    let malformed =
        "Cannot open this link: expected an ASCII HTTP/HTTPS URI with a host and no credentials.";
    let (scheme, rest) = uri.split_once(':').ok_or(malformed)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err("Only HTTP and HTTPS links can be opened. Copy other targets explicitly.");
    }
    let rest = rest.strip_prefix("//").ok_or(malformed)?;
    // A deliberately narrow serialized-URI policy, not URL discovery or normalization.
    let bytes = uri.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        if !is_uri_byte(byte) {
            return Err(malformed);
        }
        if byte == b'%'
            && !bytes
                .get(i + 1..i + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
        {
            return Err("Link target contains an invalid percent escape.");
        }
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, suffix) = ipv6.split_once(']').ok_or(malformed)?;
        host.parse::<std::net::Ipv6Addr>().map_err(|_| malformed)?;
        (host, suffix)
    } else {
        let (host, port) = authority.split_at(authority.find(':').unwrap_or(authority.len()));
        if !host
            .strip_suffix('.')
            .unwrap_or(host)
            .split('.')
            .all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
            || host.len() > 253
        {
            return Err(malformed);
        }
        (host, port)
    };
    if host.is_empty()
        || (!port.is_empty()
            && !port.strip_prefix(':').is_some_and(|port| {
                !port.is_empty()
                    && port.bytes().all(|c| c.is_ascii_digit())
                    && port.parse::<u16>().is_ok()
            }))
    {
        return Err(malformed);
    }
    Ok(())
}

/// The next `http://` or `https://` at or after `from` that does not continue a word.
fn uri_start(text: &[u8], from: usize) -> Option<usize> {
    (from..text.len()).find(|&i| {
        (i == 0 || !text[i - 1].is_ascii_alphanumeric())
            && [&b"http://"[..], b"https://"].iter().any(|scheme| {
                text.get(i..i + scheme.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(scheme))
            })
    })
}

/// Drop trailing sentence punctuation and closing brackets that the URI does not open.
fn trim_uri_end(mut uri: &[u8]) -> &[u8] {
    let count = |byte| uri.iter().filter(|&&b| b == byte).count() as isize;
    let mut unopened_parens = count(b')') - count(b'(');
    let mut unopened_brackets = count(b']') - count(b'[');
    while let Some((&last, rest)) = uri.split_last() {
        match last {
            b'.' | b',' | b':' | b';' | b'!' | b'?' | b'\'' => {}
            b')' if unopened_parens > 0 => unopened_parens -= 1,
            b']' if unopened_brackets > 0 => unopened_brackets -= 1,
            _ => break,
        }
        uri = rest;
    }
    uri
}

impl DrawRow {
    fn hyperlinks(&self, row: u16) -> impl Iterator<Item = Hyperlink<'_>> {
        let mut column = 0;
        let visible_uri = |cell: &DrawCell| {
            matches!(cell.width, CellWidth::Narrow | CellWidth::Wide)
                && !cell.style.invisible
                && !cell.hyperlink.is_empty()
        };
        std::iter::from_fn(move || {
            while column < self.cells.len() {
                let cell = &self.cells[column];
                if !visible_uri(cell) {
                    column += 1;
                    continue;
                }
                let start = column;
                let uri = cell.hyperlink.as_str();
                while let Some(cell) = self.cells.get(column) {
                    if !visible_uri(cell) || cell.hyperlink != uri {
                        break;
                    }
                    column += if cell.width == CellWidth::Wide { 2 } else { 1 };
                }
                return Some(Hyperlink {
                    row,
                    column: start as u16,
                    columns: (column - start) as u16,
                    uri: Cow::Borrowed(uri),
                });
            }
            None
        })
    }

    pub(crate) fn from_protocol(row: &Row, frame: &Frame) -> Self {
        Self {
            wrapped: row.wrapped,
            wrap_continuation: row.wrap_continuation,
            cells: row
                .cells
                .iter()
                .map(|cell| DrawCell::from_protocol(cell, frame))
                .collect(),
        }
    }

    pub(crate) fn append_glyph_runs(&self, row: u16, runs: &mut Vec<GlyphRun>) {
        let mut column = 0_u16;
        while usize::from(column) < self.cells.len() {
            let cell = &self.cells[usize::from(column)];
            let span = match cell.width {
                CellWidth::Narrow => 1,
                CellWidth::Wide => 2,
                CellWidth::SpacerHead | CellWidth::SpacerTail => {
                    column += 1;
                    continue;
                }
            };
            if cell.style.foreground_visible(true) && !cell.text.trim_matches(' ').is_empty() {
                runs.push(GlyphRun {
                    column,
                    row,
                    columns: span,
                    text: cell.text.clone(),
                    style: cell.style,
                });
            }
            column = column.saturating_add(span);
        }
    }
}

/// One Orbit vertical-preview result for an exact accepted scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScenePreview {
    TerminalOwned {
        frame_revision: u64,
        direction: VerticalDirection,
    },
    Viewport {
        frame_revision: u64,
        direction: VerticalDirection,
        edge_reached: bool,
        rows: Vec<DrawRow>,
    },
}

/// Renderer-ready cursor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawCursor {
    pub visible: bool,
    pub blinking: bool,
    pub password_input: bool,
    pub shape: CursorShape,
    pub column: u16,
    pub row: u16,
    pub at_wide_tail: bool,
    pub color: Color,
    pub explicit_color: bool,
}

impl DrawCursor {
    /// Leading visual column occupied by the cursor.
    #[must_use]
    pub fn leading_column(self) -> u16 {
        self.column.saturating_sub(u16::from(self.at_wide_tail))
    }
}

/// One complete authoritative cell positioned on the terminal grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphRun {
    pub column: u16,
    pub row: u16,
    pub columns: u16,
    pub text: String,
    pub style: DrawStyle,
}

/// One immutable, revision-tagged Venus draw scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub revision: u64,
    pub columns: u16,
    pub rows: u16,
    pub screen: Screen,
    pub title: String,
    pub working_directory: String,
    pub background: Color,
    pub foreground: Color,
    pub cursor: Option<DrawCursor>,
    pub content: Vec<DrawRow>,
    pub graphics: SceneGraphics,
}

/// A frame's image placements and the attachment whose resources they reference (VEN-C31).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneGraphics {
    pub attachment: u64,
    /// The frame's placement entries, in drawing order.
    pub placements: Vec<ScenePlacement>,
    /// Whether Orbit omitted placements past the frame's caps.
    pub placements_omitted: bool,
}

/// One image placement entry of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenePlacement {
    /// Orbit's identity for the resource in this attachment.
    pub resource: u64,
    pub state: PlacementState,
    /// The drawn part of the resource in its pixels, not yet clipped to its bounds.
    pub source: SourceRect,
    /// Where the entry lands, in cells from the grid's top-left corner, measured with the frame's
    /// cell size so the current cell size places and scales it.
    pub target: SceneRect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AccessibleText {
    pub rows: Vec<AccessibleRow>,
    pub selection: Option<AccessibleSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccessibleRow {
    pub value: String,
    pub character_lengths: Vec<u8>,
    pub column_spans: Vec<Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AccessibleSelection {
    pub anchor: AccessiblePosition,
    pub focus: AccessiblePosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AccessiblePosition {
    pub row: usize,
    pub character_index: usize,
}

impl AccessibleText {
    pub fn plain_text(&self) -> String {
        self.rows.iter().map(|row| row.value.as_str()).collect()
    }
}

impl Scene {
    /// Explicit and detected link segments in row-major order.
    pub fn hyperlinks(&self) -> impl Iterator<Item = Hyperlink<'_>> {
        let mut links = Vec::new();
        let mut row = 0;
        while row < self.content.len() {
            let line = self.logical_line(row);
            row = line.end() + 1;
            for row in line.clone() {
                links.extend(self.content[row].hyperlinks(row as u16));
            }
            links.extend(self.detected_links(line));
        }
        links.sort_by_key(|link| (link.row, link.column));
        links.into_iter()
    }

    #[must_use]
    pub fn hyperlink_at(&self, row: u16, column: u16) -> Option<Hyperlink<'_>> {
        self.content
            .get(usize::from(row))?
            .hyperlinks(row)
            .find(|link| link.contains(row, column))
            .or_else(|| {
                self.detected_links(self.logical_line(usize::from(row)))
                    .into_iter()
                    .find(|link| link.contains(row, column))
            })
    }

    /// The visible rows of the soft-wrapped line containing `row`.
    fn logical_line(&self, row: usize) -> RangeInclusive<usize> {
        let rows = &self.content;
        let mut first = row;
        while first > 0 && rows[first].wrap_continuation && rows[first - 1].wrapped {
            first -= 1;
        }
        let mut last = row;
        while rows[last].wrapped
            && rows
                .get(last + 1)
                .is_some_and(|next| next.wrap_continuation)
        {
            last += 1;
        }
        first..=last
    }

    /// Plain-text HTTP/HTTPS targets on one logical line, one segment per row.
    /// Only targets the opener accepts are detected, and an explicit link on
    /// any of a target's cells wins over the whole target.
    fn detected_links(&self, line: RangeInclusive<usize>) -> Vec<Hyperlink<'_>> {
        // One byte per narrow cell; a cell no URI can contain becomes a space.
        let mut text = Vec::new();
        let mut cells = Vec::new();
        for row in line {
            for (column, cell) in self.content[row].cells.iter().enumerate() {
                if cell.width == CellWidth::SpacerTail {
                    continue;
                }
                text.push(match cell.text.as_bytes() {
                    &[byte]
                        if cell.width == CellWidth::Narrow
                            && !cell.style.invisible
                            && is_uri_byte(byte) =>
                    {
                        byte
                    }
                    _ => b' ',
                });
                cells.push((row as u16, column as u16, !cell.hyperlink.is_empty()));
            }
        }
        let mut links = Vec::new();
        let mut next = 0;
        while let Some(start) = uri_start(&text, next) {
            let end = (start..text.len())
                .find(|&i| !is_uri_byte(text[i]))
                .unwrap_or(text.len());
            next = end;
            let span = start..start + trim_uri_end(&text[start..end]).len();
            let uri: String = text[span.clone()].iter().copied().map(char::from).collect();
            let cells = &cells[span];
            if validate_open_uri(&uri).is_err() || cells.iter().any(|&(_, _, explicit)| explicit) {
                continue;
            }
            for segment in cells.chunk_by(|a, b| a.0 == b.0) {
                let (row, column, _) = segment[0];
                links.push(Hyperlink {
                    row,
                    column,
                    columns: segment.len() as u16,
                    uri: Cow::Owned(uri.clone()),
                });
            }
        }
        links
    }

    /// Materialize a validated Orbit frame of one attachment without retaining a second wire
    /// schema.
    #[must_use]
    pub fn from_frame(frame: &Frame, attachment: u64) -> Self {
        let background = Color::from(frame.colors.background);
        let foreground = Color::from(frame.colors.foreground);
        let content = frame
            .rows
            .iter()
            .map(|row| DrawRow::from_protocol(row, frame))
            .collect();
        let cursor = frame.cursor.viewport.map(|cursor| DrawCursor {
            visible: frame.cursor.visible,
            blinking: frame.cursor.blinking,
            password_input: frame.cursor.password_input,
            shape: frame.cursor.shape,
            column: cursor.x,
            row: cursor.y,
            at_wide_tail: cursor.at_wide_tail,
            color: Color::from(frame.colors.cursor.unwrap_or(frame.colors.foreground)),
            explicit_color: frame.colors.cursor.is_some(),
        });
        let mut graphics = SceneGraphics {
            attachment,
            ..SceneGraphics::default()
        };
        if let Some(frame_graphics) = &frame.graphics {
            let cell_width = frame_graphics.cell_width as f32;
            let cell_height = frame_graphics.cell_height as f32;
            graphics.placements_omitted = frame_graphics.placements_omitted;
            graphics.placements = frame_graphics
                .placements
                .iter()
                .map(|placement| ScenePlacement {
                    resource: placement.resource,
                    state: placement.state,
                    source: placement.source,
                    target: SceneRect {
                        left: placement.col as f32 + placement.offset_x as f32 / cell_width,
                        top: placement.row as f32 + placement.offset_y as f32 / cell_height,
                        width: placement.width as f32 / cell_width,
                        height: placement.height as f32 / cell_height,
                    },
                })
                .collect();
        }

        Self {
            revision: frame.revision,
            columns: frame.dimensions.cols,
            rows: frame.dimensions.rows,
            screen: frame.screen,
            title: frame.title.clone(),
            working_directory: frame.working_directory.clone(),
            background,
            foreground,
            cursor,
            content,
            graphics,
        }
    }

    /// Build positioned text runs without changing grapheme strings.
    #[must_use]
    pub fn glyph_runs(&self) -> Vec<GlyphRun> {
        let mut runs = Vec::new();
        for (index, row) in self.content.iter().enumerate() {
            row.append_glyph_runs(
                u16::try_from(index).expect("frame row count fits u16"),
                &mut runs,
            );
        }
        runs
    }

    /// Whether native presentation needs a bounded blink wakeup.
    #[must_use]
    pub fn has_blinking_content(&self) -> bool {
        self.cursor
            .is_some_and(|cursor| cursor.visible && cursor.blinking)
            || self
                .content
                .iter()
                .flat_map(|row| &row.cells)
                .any(|cell| cell.style.blink && cell.style.foreground_visible(true))
    }

    /// Whether the authoritative frame contains selected presentation.
    #[must_use]
    pub fn has_selected_content(&self) -> bool {
        self.content
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.style.selected)
    }

    /// Text exposed to native accessibility, derived directly from this scene.
    #[must_use]
    pub fn accessible_text(&self) -> String {
        self.accessible_content().plain_text()
    }

    pub(crate) fn accessible_content(&self) -> AccessibleText {
        let mut rows = Vec::with_capacity(self.content.len());
        let mut anchor = None;
        let mut focus = None;
        for (row_index, row) in self.content.iter().enumerate() {
            let mut units = Vec::with_capacity(row.cells.len());
            let mut column = 0_usize;
            while column < row.cells.len() {
                let cell = &row.cells[column];
                let wide = cell.width == CellWidth::Wide;
                match cell.width {
                    CellWidth::Narrow | CellWidth::Wide => {
                        if cell.style.invisible || cell.text.is_empty() {
                            units.extend((0..if wide { 2 } else { 1 }).map(|offset| {
                                (
                                    " ",
                                    1,
                                    cell.style.selected,
                                    column + offset..column + offset + 1,
                                )
                            }));
                        } else {
                            let (text, length) = u8::try_from(cell.text.len())
                                .map_or(("\u{fffd}", 3), |length| (cell.text.as_str(), length));
                            units.push((
                                text,
                                length,
                                cell.style.selected,
                                column..column + usize::from(wide) + 1,
                            ));
                        }
                    }
                    CellWidth::SpacerHead => {
                        units.push((" ", 1, cell.style.selected, column..column + 1));
                    }
                    CellWidth::SpacerTail => {}
                }
                column += usize::from(wide) + 1;
            }
            while units
                .last()
                .is_some_and(|(text, _, selected, _)| *text == " " && !selected)
            {
                units.pop();
            }

            let mut value = String::new();
            let mut character_lengths = Vec::with_capacity(units.len() + 1);
            let mut column_spans = Vec::with_capacity(units.len() + 1);
            for (character_index, (text, length, selected, span)) in units.into_iter().enumerate() {
                if selected {
                    anchor.get_or_insert(AccessiblePosition {
                        row: row_index,
                        character_index,
                    });
                    focus = Some(AccessiblePosition {
                        row: row_index,
                        character_index: character_index + 1,
                    });
                }
                value.push_str(text);
                character_lengths.push(length);
                column_spans.push(span);
            }
            if row_index + 1 < self.content.len() {
                value.push('\n');
                character_lengths.push(1);
                let end = column_spans.last().map_or(0, |span| span.end);
                column_spans.push(end..end);
            }
            rows.push(AccessibleRow {
                value,
                character_lengths,
                column_spans,
            });
        }
        let selection = anchor
            .zip(focus)
            .map(|(anchor, focus)| AccessibleSelection { anchor, focus })
            .or_else(|| {
                let cursor = self.cursor?;
                let row = usize::from(cursor.row);
                let content = rows.get(row)?;
                let column = usize::from(cursor.leading_column());
                let character_index = content
                    .column_spans
                    .partition_point(|span| !span.is_empty() && span.end <= column);
                let caret = AccessiblePosition {
                    row,
                    character_index,
                };
                Some(AccessibleSelection {
                    anchor: caret,
                    focus: caret,
                })
            });
        AccessibleText { rows, selection }
    }

    /// Stable human-readable state used by focused contract checks.
    #[must_use]
    pub fn snapshot(&self) -> String {
        let mut output = format!(
            "revision={} screen={:?} size={}x{} title={:?} cwd={:?}\n",
            self.revision, self.screen, self.columns, self.rows, self.title, self.working_directory
        );
        for run in self.glyph_runs() {
            let _ = writeln!(
                output,
                "run {},{}+{} {:?} fg={:02x}{:02x}{:02x} bg={:02x}{:02x}{:02x}",
                run.column,
                run.row,
                run.columns,
                run.text,
                run.style.foreground.r,
                run.style.foreground.g,
                run.style.foreground.b,
                run.style.background.r,
                run.style.background.g,
                run.style.background.b
            );
        }
        if let Some(cursor) = self.cursor {
            let _ = writeln!(
                output,
                "cursor {},{} {:?} visible={} tail={}",
                cursor.column, cursor.row, cursor.shape, cursor.visible, cursor.at_wide_tail
            );
        }
        output
    }
}

impl DrawCell {
    fn from_protocol(cell: &Cell, frame: &Frame) -> Self {
        let mut foreground = resolve_color(
            cell.style.foreground,
            frame.colors.foreground,
            &frame.colors.palette,
        );
        let mut background = resolve_color(
            cell.style.background,
            frame.colors.background,
            &frame.colors.palette,
        );
        if cell.style.inverse ^ cell.style.selected {
            std::mem::swap(&mut foreground, &mut background);
        }
        let underline_color = match cell.style.underline_color {
            StyleColor::None => foreground,
            color => resolve_color(color, frame.colors.foreground, &frame.colors.palette),
        };
        Self {
            width: cell.width,
            text: cell.text.clone(),
            hyperlink: cell.hyperlink.clone(),
            style: resolved_style(cell.style, foreground, background, underline_color),
        }
    }
}

fn is_default_background(style: CellStyle) -> bool {
    matches!(style.background, StyleColor::None) && !style.inverse && !style.selected
}

fn resolved_style(
    style: CellStyle,
    foreground: Color,
    background: Color,
    underline_color: Color,
) -> DrawStyle {
    DrawStyle {
        foreground,
        background,
        underline_color,
        bold: style.bold,
        italic: style.italic,
        faint: style.faint,
        blink: style.blink,
        invisible: style.invisible,
        strikethrough: style.strikethrough,
        overline: style.overline,
        selected: style.selected,
        background_is_default: is_default_background(style),
        protected: style.protected,
        underline: style.underline,
    }
}

fn resolve_color(color: StyleColor, default: Rgb, palette: &[Rgb; 256]) -> Color {
    Color::from(match color {
        StyleColor::None => default,
        StyleColor::Palette(index) => palette[usize::from(index)],
        StyleColor::Rgb(color) => color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v22::Pane;

    #[test]
    fn eon_bar_pill_band_leaves_equal_title_cap_gap() {
        for (scale, line_height, expected) in [
            (1.0, 1.125, (10.0, 30.0)),
            (1.3, 1.125, (13.0, 38.333332)),
            (1.0, 2.0, (16.0, 48.0)),
            (1.3, 2.0, (21.0, 64.333336)),
        ] {
            let mut metrics = CellMetrics::for_scale(scale);
            metrics.height = (metrics.font_size * line_height).round();
            let header = header_heights(metrics).0;
            let cards = card_area(0.0, 1600.0, 1, header, 900.0, false, 0.0, metrics);
            let band = bar_pill_band(metrics, header, cards.top);
            let offset = if scale == 1.0 { 6.0 } else { 7.0 };
            assert!((cards.top - header - offset).abs() < 0.001);
            assert!(
                (band.start - expected.0).abs() < 0.001,
                "{scale}/{line_height}"
            );
            assert!((band.end - band.start - expected.1).abs() < 0.001);
            let shared_top = title_rule_y(
                cards.top,
                chrome_title_metrics(metrics).text_height.min(cards.height),
            );
            let cap_top = shared_top - metrics.font_size * FALLBACK_CAP_HEIGHT_EM / 2.0;
            assert!((band.start - (cap_top - band.end)).abs() < 1.0);
            assert!((band.end - header).abs() < 0.001);
            println!(
                "scale={scale}, line_height={line_height}: top={}, pill_height={}, card_top={}, shared_line={shared_top}, cap_top={cap_top}, cap_gap={}, offset={offset}",
                band.start,
                band.end - band.start,
                cards.top,
                cap_top - band.end
            );
            assert!(band.end - band.start >= metrics.height);
        }
    }

    #[test]
    fn eon_bar_title_gap_uses_the_loaded_font_baseline_and_cap_height() {
        use crate::render::{FontSettings, FontSetup};

        for line_height in [1.125, 2.0] {
            let fonts = FontSetup::new(&FontSettings {
                line_height,
                ..FontSettings::default()
            })
            .unwrap();
            for scale in [1.0, 1.3] {
                let metrics = fonts.metrics(scale);
                assert!(metrics.title_cap_top_em.is_some());
                let header = header_heights(metrics).0;
                let cards = card_area(0.0, 1600.0, 1, header, 900.0, false, 0.0, metrics);
                let band = bar_pill_band(metrics, header, cards.top);
                let cap_top = cards.top + chrome_title_metrics(metrics).cap_top;
                assert!((cap_top - band.end - band.start).abs() <= 0.501);
                assert!((band.end - header).abs() < 0.001);
                println!(
                    "loaded title: scale={scale}, line_height={line_height}, cap_offset_em={:?}, top_gap={}, pill_height={}, card_top={}, cap_top={cap_top}, cap_gap={}, offset={}",
                    metrics.title_cap_top_em,
                    band.start,
                    band.end - band.start,
                    cards.top,
                    cap_top - band.end,
                    cards.top - header
                );
            }
        }
    }

    #[test]
    fn compact_pane_headers_share_title_height_and_full_row_targets() {
        use eon_workspace_protocol::v22::{Column, PopupGeometry};

        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp/eon".to_vec(),
                pending: false,
                selected_pane: Some("p2".into()),
                selected_popup: None,
                columns: vec![Column {
                    panes: (1..=4)
                        .map(|n| Pane {
                            label: format!("p{n}"),
                            pinned_name: None,
                            id: format!("p{n}"),
                            session: format!("s{n}"),
                            endpoint: format!("/tmp/p{n}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        })
                        .collect(),
                    expanded: "p2".into(),
                }],
                popups: Vec::new(),
                attention_rollup: None,
            }],
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
        };
        for (scale, line_height) in [
            (1.0, 1.125),
            (1.25, 1.125),
            (1.3, 1.125),
            (1.5, 1.125),
            (2.0, 1.125),
            (1.0, 2.0),
            (1.3, 2.0),
        ] {
            let mut metrics = CellMetrics::for_scale(scale);
            metrics.height = (metrics.font_size * line_height).round();
            let scene = WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new(1600, 900),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
            );
            if scale == 1.0 && line_height == 1.125 {
                assert_eq!(scene.header.height, 40.0);
                assert_eq!(scene.tabs[0].rect.top, 8.0);
                assert_eq!(scene.tabs[0].rect.height, 24.0);
                assert_eq!(
                    scene.pane_viewport,
                    SceneRect {
                        left: 296.0,
                        top: 46.0,
                        width: 1296.0,
                        height: 846.0
                    }
                );
                assert_eq!(
                    scene.terminal,
                    SceneRect {
                        left: 296.0,
                        top: 86.0,
                        width: 1296.0,
                        height: 766.0
                    }
                );
            }
            if scale <= 1.3 {
                use eon_workspace_protocol::v22::{CodexQuota, CodexQuotaState};
                let mut snapshot = snapshot.clone();
                snapshot.codex_quota = Some(CodexQuota {
                    state: CodexQuotaState::Unknown,
                    observed_at: 0,
                    windows: Vec::new(),
                });
                let cpu = CpuUse {
                    label: "CPU 12%".into(),
                    description: "CPU".into(),
                };
                let ram = RamUse {
                    label: "RAM 43%".into(),
                    description: "RAM".into(),
                };
                let clock = LocalClock {
                    label: "15:43".into(),
                    description: "Clock".into(),
                };
                let widgets = WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    EonBar {
                        version: Some("0.1.1"),
                        cpu: Some(&cpu),
                        ram: Some(&ram),
                        clock: Some(&clock),
                        ..EonBar::default()
                    },
                    PhysicalSize::new(1600, 900),
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * metrics.width),
                    None,
                );
                assert_eq!(widgets.header, scene.header);
                assert_eq!(widgets.pane_viewport, scene.pane_viewport);
                assert_eq!(widgets.terminal, scene.terminal);
                assert_eq!(widgets.tabs[0].rect, scene.tabs[0].rect);
                let pill = widgets.bar_pill_rect(widgets.tabs[0].rect, metrics);
                let centre = pill.top + pill.height / 2.0;
                for rect in widgets.quotas.iter().map(|quota| quota.rect).chain([
                    widgets.cpu.as_ref().unwrap().rect,
                    widgets.ram.as_ref().unwrap().rect,
                    widgets.clock.as_ref().unwrap().rect,
                ]) {
                    let painted = widgets.bar_pill_rect(rect, metrics);
                    assert_eq!((painted.top, painted.height), (pill.top, pill.height));
                    assert_eq!(rect.top, 0.0);
                    assert_eq!(rect.height, widgets.header.height);
                }
                for rect in widgets
                    .controls
                    .iter()
                    .map(|control| control.rect)
                    .chain([widgets.version.as_ref().unwrap().rect])
                {
                    let content = widgets.bar_item_rect(rect, metrics);
                    assert!((content.top + content.height / 2.0 - centre).abs() < 0.001);
                    assert_eq!(content.height, rect.height);
                }
                let logo = widgets.quotas[0].logo;
                assert!((logo.top + logo.height / 2.0 - centre).abs() < 0.001);
            }
            for pane in &scene.panes {
                assert_eq!(pane.rect.height, chrome_title_metrics(metrics).text_height);
                assert!(pane.rect.height >= metrics.height);
                for x in [pane.rect.left + 0.5, pane.rect.right() - 0.5] {
                    for y in [pane.rect.top + 0.5, pane.rect.bottom() - 0.5] {
                        assert_eq!(scene.hit_test(x, y), Some(WorkspaceHit::Pane(&pane.id)));
                    }
                }
            }
            for pane in &scene.panes {
                let frame = pane.frame(scene.terminal);
                assert_eq!(frame.top, title_rule_y(pane.rect.top, pane.rect.height));
                if pane.selected {
                    assert_eq!(frame.bottom(), scene.terminal.bottom());
                } else {
                    assert_eq!(frame.height, pane.rect.height);
                }
            }
            assert_eq!(scene.panes[1].rect.bottom(), scene.terminal.top);
            assert_eq!(scene.terminal.bottom(), scene.panes[2].rect.top);
            assert_eq!(
                scene
                    .card_rect(
                        scene.chrome,
                        metrics,
                        crate::render::physical_outline_width(2, metrics.scale)
                    )
                    .top,
                (scene.pane_viewport.top + chrome_title_metrics(metrics).text_height / 2.0).floor(),
                "painted pane top at scale {scale}"
            );
        }
        // Resizing can collapse every header to the same Y; direction still follows pane order.
        for size in [PhysicalSize::new(1280, 1), PhysicalSize::new(1, 720)] {
            let metrics = CellMetrics::for_scale(1.3);
            let scene =
                WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
                    (text.into(), text.len() as f32 * metrics.width)
                });
            let outlines = scene.pane_outlines(
                &scene.panes,
                scene.terminal,
                scene.card_fill(
                    scene.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale),
                ),
                3.0,
                metrics,
                3.0,
            );
            assert!(
                outlines
                    .iter()
                    .all(|outline| outline.bounds.top.is_finite()
                        && outline.bounds.height.is_finite())
            );
        }
        let metrics = CellMetrics::for_scale(1.0);
        let size = PhysicalSize::new(1280, 720);
        for (count, before, after) in [(1, 32, 33), (3, 29, 30), (4, 28, 29)] {
            let mut snapshot = snapshot.clone();
            snapshot.tabs[0].columns[0].panes.truncate(count);
            snapshot.tabs[0].selected_pane = Some("p1".into());
            snapshot.tabs[0].columns[0].expanded = "p1".into();
            let hints = ShortcutHintsScene::new(
                Vec::new(),
                size,
                metrics,
                WorkspaceFocus::Terminal,
                |_| 0.0,
            );
            let scene = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
                Some(hints),
            );
            let rows = |header: f32| {
                ((scene.chrome.height - count as f32 * header - metrics.padding * 2.0)
                    / metrics.height)
                    .floor() as u16
            };
            assert_eq!(rows((metrics.height * 1.5).round()), before);
            let actual_rows =
                ((scene.terminal.height - metrics.padding * 2.0) / metrics.height).floor() as u16;
            assert_eq!(actual_rows, after);
            assert_eq!(rows(scene.panes[0].rect.height), after);
            println!(
                "1280x720, {count} panes: {before} -> {after} rows, allocated height {}",
                scene.chrome.height
            );
            let painted = scene.card_rect(
                scene.chrome,
                metrics,
                crate::render::physical_outline_width(2, metrics.scale),
            );
            assert_eq!(
                painted.top,
                (scene.panes[0].rect.top + scene.panes[0].rect.height / 2.0).floor()
            );
            assert_eq!(
                painted.bottom(),
                (scene.chrome.bottom() - chrome_title_metrics(metrics).text_height / 2.0).floor()
            );
            assert_eq!(painted.top - scene.chrome.top, 10.0);
        }
    }

    #[test]
    fn quota_without_reset_retains_percent_and_accessible_freshness() {
        for state in [CodexQuotaState::Fresh, CodexQuotaState::Stale] {
            let window = QuotaWindow {
                duration_minutes: 300,
                remaining_percent: 100,
                resets_at: None,
            };
            let (wide, compact, description) =
                quota_text(QuotaProvider::Claude, state, 100, &[window]);
            let suffix = if state == CodexQuotaState::Stale {
                " old"
            } else {
                ""
            };
            assert_eq!(wide, format!("5h 100%{suffix}"));
            assert_eq!(compact, format!("5h 100%{suffix}"));
            assert!(description.contains("reset time unavailable"));
            assert!(description.contains(if state == CodexQuotaState::Stale {
                "stale"
            } else {
                "fresh"
            }));
        }
    }

    #[test]
    fn agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state() {
        use eon_workspace_protocol::v22::{
            AttentionState, Column, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut,
        };

        let pane = |id: &str| Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: format!("/run/{id}.sock").into_bytes(),
            live: true,
            attention: None,
        };
        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
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
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: ALT,
                    key: "KeyA".into(),
                }),
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/work".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: ["p1", "p2"]
                    .map(|id| Column {
                        panes: vec![pane(id)],
                        expanded: id.into(),
                    })
                    .into(),
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "agent".into(),
                    session: "session-u1".into(),
                    endpoint: b"/run/u1.sock".to_vec(),
                    attention: None,
                }],
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
        };
        let metrics = CellMetrics::for_scale(1.0);
        let project = |snapshot: &Snapshot, width| {
            WorkspaceScene::from_snapshot(
                snapshot,
                PhysicalSize::new(width, 600),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
            )
        };
        let gap = card_gap(metrics);

        // A visible sidebar is 28 cells wide, one card gap from the window's left edge and as
        // tall as the cards, which it pushes right; the columns share the content area.
        let open = project(&snapshot, 1400);
        let sidebar = open.sidebar().unwrap();
        assert_eq!((sidebar.left, sidebar.width), (gap, 280.0));
        assert_eq!(
            (sidebar.top, sidebar.bottom()),
            (
                open.card_rect(
                    open.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale)
                )
                .top,
                open.pane_viewport.bottom()
            )
        );
        assert_eq!(open.content_width(1400.0), 1400.0 - sidebar.right());
        assert_eq!(
            (open.pane_viewport.left, open.pane_viewport.right()),
            (sidebar.right() + gap, 1400.0 - gap)
        );
        assert_eq!(
            open.chrome.width,
            workspace_column_width(open.content_width(1400.0), 2, metrics)
        );
        assert!(open.sidebar_empty());

        // Closed, the cards take the window back; the first tab pill never moves.
        snapshot.sidebar = Sidebar::Closed;
        let closed = project(&snapshot, 1400);
        assert_eq!(closed.sidebar(), None);
        assert_eq!(closed.content_width(1400.0), 1400.0);
        assert_eq!(closed.pane_viewport.left, gap);
        assert_eq!(closed.tabs[0].rect, open.tabs[0].rect);
        snapshot.sidebar = Sidebar::Off;
        assert_eq!(project(&snapshot, 1400).pane_viewport, closed.pane_viewport);

        // A popup's card is the content area, so it never covers the sidebar.
        snapshot.sidebar = Sidebar::Open;
        snapshot.tabs[0].selected_popup = Some("u1".into());
        let popup = project(&snapshot, 1400);
        assert_eq!(popup.sidebar(), Some(sidebar));
        assert_eq!(popup.chrome, open.pane_viewport);
        assert!(popup.terminal.left > sidebar.right());
        snapshot.tabs[0].selected_popup = None;

        // The sidebar hides once the content area would keep fewer than 80 cells, leaving the
        // layout it would have closed.
        let narrowest = (gap + 280.0 + 800.0) as u32;
        assert!(project(&snapshot, narrowest).sidebar().is_some());
        let hidden = project(&snapshot, narrowest - 1);
        assert_eq!(hidden.sidebar(), None);
        snapshot.sidebar = Sidebar::Closed;
        assert_eq!(
            hidden.pane_viewport,
            project(&snapshot, narrowest - 1).pane_viewport
        );

        // Any attention replaces the empty state.
        snapshot.sidebar = Sidebar::Open;
        snapshot.tabs[0].popups[0].attention = Some(eon_workspace_protocol::v22::Attention {
            state: AttentionState::Unavailable,
            record: None,
        });
        snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
        assert!(!project(&snapshot, 1400).sidebar_empty());
    }

    #[test]
    fn agent_sidebar_rows_show_attention_in_tab_order_and_reveal_the_focused_row() {
        use eon_workspace_protocol::v22::{
            Attention, AttentionRecord, Column, Popup, PopupEntry, PopupGeometry, PopupKey,
            Shortcut,
        };

        const NOW: u64 = 1_800_000_000_000;
        let record = |app: &str, title: &str, age: u64| AttentionRecord {
            kind: None,
            progress: None,
            app: app.into(),
            name: String::new(),
            title: title.into(),
            message: String::new(),
            changed_at: NOW - age,
            stale: false,
            others: 0,
        };
        let pane = |id: &str, attention: Option<Attention>| Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: format!("/run/{id}.sock").into_bytes(),
            live: true,
            attention,
        };
        let tab = |id: &str, directory: &str, panes: Vec<Pane>, popups, rollup| Tab {
            id: id.into(),
            directory: directory.as_bytes().to_vec(),
            pending: false,
            selected_pane: Some(panes[0].id.clone()),
            selected_popup: None,
            columns: panes
                .into_iter()
                .map(|pane| Column {
                    expanded: pane.id.clone(),
                    panes: vec![pane],
                })
                .collect(),
            popups,
            attention_rollup: rollup,
        };
        let needs_input = Attention {
            state: AttentionState::NeedsInput,
            record: Some(AttentionRecord {
                kind: Some(AttentionKind::Permission),
                name: "eon-9a".into(),
                message: "Allow Bash: cargo test?".into(),
                others: 2,
                ..record("claude-code", "Review PR", 4 * MINUTE_MS)
            }),
        };
        // Other apps' terminal titles, such as Codex's spinner title or a shell's command
        // line, never lead a row.
        let working = Attention {
            state: AttentionState::Working,
            record: Some(AttentionRecord {
                stale: true,
                name: "⠼ w".into(),
                ..record("codex", "Fix tests", 30_000)
            }),
        };
        let idle = Attention {
            state: AttentionState::Idle,
            record: Some(AttentionRecord {
                name: "~/w> ./i".into(),
                ..record("aider", "", 2 * HOUR_MS)
            }),
        };
        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 3,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "agent".into(),
                label: "Agent popup".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: ALT,
                    key: "KeyA".into(),
                }),
            }],
            tabs: vec![
                tab("t1", "/work", vec![pane("p1", None)], Vec::new(), None),
                tab(
                    "t2",
                    "/home/u/site",
                    vec![pane("p2", Some(needs_input)), pane("p3", None)],
                    vec![Popup {
                        id: "u2".into(),
                        entry: "agent".into(),
                        session: "session-u2".into(),
                        endpoint: b"/run/u2.sock".to_vec(),
                        attention: Some(Attention {
                            state: AttentionState::Unavailable,
                            record: None,
                        }),
                    }],
                    Some(AttentionState::NeedsInput),
                ),
                tab(
                    "t3",
                    "/srv/api",
                    vec![pane("p4", Some(working)), pane("p5", Some(idle))],
                    Vec::new(),
                    Some(AttentionState::Working),
                ),
            ],
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Open,
        };
        let metrics = CellMetrics::for_scale(1.0);
        let project = |snapshot: &Snapshot, height, focused| {
            WorkspaceScene::from_snapshot_with_zoom(
                snapshot,
                EonBar::default(),
                PhysicalSize::new(1400, height),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
                None,
                false,
                SidebarView { now: NOW, focused },
            )
        };

        // Only tabs with attention are listed, each under its heading, with one row per
        // terminal with attention, in tab order: a popup's row first, then the panes' rows in
        // pane order.
        let scene = project(&snapshot, 800, None);
        assert!(!scene.sidebar_empty());
        assert_eq!(
            scene
                .sidebar_headings
                .iter()
                .map(|heading| (heading.tab.as_str(), heading.label.as_str()))
                .collect::<Vec<_>>(),
            [("t2", "2  site"), ("t3", "3  api")]
        );
        #[allow(clippy::type_complexity)]
        fn summary(
            row: &SidebarRow,
        ) -> (
            &str,
            &str,
            AttentionState,
            bool,
            Option<QuotaProvider>,
            &str,
            &str,
            Option<&str>,
        ) {
            (
                row.tab.as_str(),
                row.target.as_str(),
                row.glyph.state,
                row.glyph.dimmed,
                row.icon.map(|(provider, _)| provider),
                row.title.as_str(),
                row.detail.as_str(),
                row.message.as_deref(),
            )
        }
        assert_eq!(
            scene.sidebar_rows().iter().map(summary).collect::<Vec<_>>(),
            [
                (
                    "t2",
                    "u2",
                    AttentionState::Unavailable,
                    false,
                    None,
                    "status unavailable",
                    "Agent popup",
                    None,
                ),
                (
                    "t2",
                    "p2",
                    AttentionState::NeedsInput,
                    false,
                    Some(QuotaProvider::Claude),
                    "eon-9a",
                    "4m · p2 · +2",
                    Some("Allow Bash: cargo test?"),
                ),
                (
                    "t3",
                    "p4",
                    AttentionState::Working,
                    true,
                    Some(QuotaProvider::Codex),
                    "Fix tests",
                    "now · p4 · status stale",
                    None,
                ),
                // A non-Claude terminal title is ignored; without a title, the app shows
                // verbatim, never the tab's directory leaf.
                (
                    "t3",
                    "p5",
                    AttentionState::Idle,
                    false,
                    None,
                    "aider",
                    "2h · p5",
                    None,
                ),
            ]
        );
        assert_eq!(
            scene
                .sidebar_rows()
                .iter()
                .map(|row| row.accessible_label.as_str())
                .collect::<Vec<_>>(),
            [
                "Status unavailable: agent in tab 2, site, Agent popup, popup",
                "Needs input (permission): Claude eon-9a in tab 2, site, p2, Review PR, \
                 changed 4 minutes ago, 2 more records: Allow Bash: cargo test?",
                "Working: Codex in tab 3, api, p4, Fix tests, changed just now, status stale",
                "Idle: aider in tab 3, api, p5, changed 2 hours ago",
            ]
        );

        // Rows stack inside the card below the title, the needs-input row one line taller;
        // the icon sits after the glyph and the text after both.
        let card = scene.sidebar().unwrap();
        let rows = scene.sidebar_rows();
        assert!(scene.sidebar_list.top > card.top && scene.sidebar_list.bottom() < card.bottom());
        assert_eq!(scene.sidebar_headings[0].rect.top, scene.sidebar_list.top);
        // Text clips to a box taller than the line pitch, which cut the descender of the
        // title's g, and the title's box ends above the list.
        assert!(chrome_title_metrics(metrics).text_height > metrics.height);
        assert!(
            card.top + metrics.padding + chrome_title_metrics(metrics).text_height
                <= scene.sidebar_list.top
        );
        for pair in rows.windows(2) {
            assert!(pair[0].rect.bottom() <= pair[1].rect.top);
        }
        assert_eq!(rows[1].rect.height - rows[3].rect.height, metrics.height);
        let (_, icon) = rows[1].icon.unwrap();
        assert!(rows[1].glyph.rect.right() < icon.left && icon.right() < rows[1].text_left);
        assert!(rows[0].text_left < rows[1].text_left);
        assert!(
            rows.iter()
                .all(|row| row.rect.left >= card.left && row.rect.right() <= card.right())
        );

        // Rows are hit targets; the youngest age changes its label first.
        let center = |rect: SceneRect| (rect.left + rect.width / 2.0, rect.top + rect.height / 2.0);
        let (x, y) = center(rows[2].rect);
        assert_eq!(scene.hit_test(x, y), Some(WorkspaceHit::SidebarRow("p4")));
        assert_eq!(
            scene.sidebar_row("u2").map(|row| row.tab.as_str()),
            Some("t2")
        );
        assert_eq!(scene.sidebar_age_wait(), Some(30_000));

        // In a short window the list clips, and the focused row scrolls just into view.
        let short = project(&snapshot, 260, None);
        let list = short.sidebar_list;
        assert!(short.sidebar_rows()[3].rect.bottom() > list.bottom());
        assert!(short.sidebar_rows().iter().all(|row| !row.focused));
        let revealed = project(&snapshot, 260, Some("p5"));
        let focused = &revealed.sidebar_rows()[3];
        assert!(focused.focused);
        assert_eq!(focused.rect.bottom(), list.bottom());
        let (x, y) = center(focused.rect);
        assert_eq!(
            revealed.hit_test(x, y),
            Some(WorkspaceHit::SidebarRow("p5"))
        );
        let offset = short.sidebar_rows()[3].rect.top - focused.rect.top;
        assert!(offset > 0.0);
        assert_eq!(
            revealed.sidebar_headings[0].rect.top,
            short.sidebar_headings[0].rect.top - offset
        );
        // The part of a row scrolled above the list is no hit target.
        let cut = revealed
            .sidebar_rows()
            .iter()
            .find(|row| row.rect.top < list.top - 1.0 && row.rect.bottom() > list.top)
            .unwrap();
        let (x, _) = center(cut.rect);
        assert_ne!(
            revealed.hit_test(x, list.top - 1.0),
            Some(WorkspaceHit::SidebarRow(cut.target.as_str()))
        );

        // A popup's row carries the marker after the catalog label whether or not the popup is
        // shown, and no pane's row does; headings never carry it.
        let marked = |scene: &WorkspaceScene| {
            (
                scene
                    .sidebar_headings
                    .iter()
                    .map(|heading| heading.label.clone())
                    .collect::<Vec<_>>(),
                scene
                    .sidebar_rows()
                    .iter()
                    .filter(|row| row.popup_marker.is_some())
                    .map(|row| {
                        let (before, after) = row.detail.split_at(row.popup_marker.unwrap());
                        (row.target.clone(), before.to_owned(), after.to_owned())
                    })
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            marked(&scene),
            (
                vec!["2  site".into(), "3  api".into()],
                vec![("u2".into(), "Agent popup".into(), String::new())]
            )
        );
        let mut shown = snapshot.clone();
        shown.tabs[1].selected_popup = Some("u2".into());
        shown.tabs[1].popups[0].attention = Some(Attention {
            state: AttentionState::Working,
            record: Some(AttentionRecord {
                others: 1,
                ..record("claude-code", "", 4 * MINUTE_MS)
            }),
        });
        let scene = project(&shown, 800, None);
        assert_eq!(
            marked(&scene),
            (
                vec!["2  site".into(), "3  api".into()],
                vec![("u2".into(), "4m · Agent popup".into(), " · +1".into())]
            )
        );
        assert_eq!(
            scene.sidebar_row("u2").unwrap().accessible_label,
            "Working: Claude in tab 2, site, Agent popup, popup, changed 4 minutes ago, \
             1 more record"
        );

        // Only the row of the active tab's focused terminal, its shown popup or else its
        // selected pane, is current. A popup shown in another tab marks nothing, and the
        // sidebar's own focus stays a separate state.
        let current = |scene: &WorkspaceScene| {
            scene
                .sidebar_rows()
                .iter()
                .filter(|row| row.current)
                .map(|row| row.target.clone())
                .collect::<Vec<_>>()
        };
        assert!(current(&project(&snapshot, 800, None)).is_empty());
        assert!(current(&scene).is_empty());
        let mut pane_focused = snapshot.clone();
        pane_focused.active_tab = "t2".into();
        let scene = project(&pane_focused, 800, Some("p4"));
        assert_eq!(current(&scene), ["p2"]);
        let (p2, p4) = (
            scene.sidebar_row("p2").unwrap(),
            scene.sidebar_row("p4").unwrap(),
        );
        assert!(!p2.focused && p4.focused && !p4.current);
        assert_eq!(
            p2.accessible_label,
            "Needs input (permission): Claude eon-9a in tab 2, site, p2, current, Review PR, \
             changed 4 minutes ago, 2 more records: Allow Bash: cargo test?"
        );
        shown.active_tab = "t2".into();
        let scene = project(&shown, 800, None);
        assert_eq!(current(&scene), ["u2"]);
        assert_eq!(
            scene.sidebar_row("u2").unwrap().accessible_label,
            "Working: Claude in tab 2, site, Agent popup, popup, current, \
             changed 4 minutes ago, 1 more record"
        );
    }

    #[test]
    fn agent_sidebar_ages_count_whole_minutes_hours_and_days() {
        assert_eq!(age_labels(0), ("now".into(), "just now".into()));
        assert_eq!(age_labels(MINUTE_MS - 1).0, "now");
        assert_eq!(age_labels(MINUTE_MS), ("1m".into(), "1 minute ago".into()));
        assert_eq!(age_labels(HOUR_MS - 1).0, "59m");
        assert_eq!(age_labels(HOUR_MS).0, "1h");
        assert_eq!(
            age_labels(3 * DAY_MS + 1),
            ("3d".into(), "3 days ago".into())
        );
        // Each label holds until the next whole unit.
        assert_eq!(age_wait(0), MINUTE_MS);
        assert_eq!(age_wait(MINUTE_MS + 1), MINUTE_MS - 1);
        assert_eq!(age_wait(HOUR_MS + MINUTE_MS), HOUR_MS - MINUTE_MS);
        assert_eq!(age_wait(DAY_MS), DAY_MS);
    }

    #[test]
    fn popup_outer_chrome_reuses_the_pane_stack_rectangle() {
        use eon_workspace_protocol::v22::{
            ALT, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Tab,
        };

        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
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
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: ALT,
                    key: "KeyA".into(),
                }),
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/work".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: vec![eon_workspace_protocol::v22::Column {
                    panes: vec![Pane {
                        label: "p1".into(),
                        pinned_name: None,
                        id: "p1".into(),
                        session: "session-1".into(),
                        endpoint: b"/run/pane.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                    expanded: "p1".into(),
                }],
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "agent".into(),
                    session: "session-2".into(),
                    endpoint: b"/run/popup.sock".to_vec(),
                    attention: None,
                }],
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v22::Sidebar::Open,
        };
        // Even the smallest windows keep terminal padding; only the gaps and empty grid yield.
        for scale in [1.0, 1.25, 1.3, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for count in 1..=4 {
                let mut snapshot = snapshot.clone();
                snapshot.tabs[0].columns = (1..=count)
                    .map(|index| {
                        let mut column = snapshot.tabs[0].columns[0].clone();
                        column.panes[0].id = format!("p{index}");
                        column.expanded = format!("p{index}");
                        column
                    })
                    .collect();
                for size in [
                    PhysicalSize::new(1, 1),
                    PhysicalSize::new(120, 600),
                    PhysicalSize::new(800, 105),
                    PhysicalSize::new(320, 180),
                    PhysicalSize::new(2000, 900),
                ] {
                    for selected_popup in [None, Some("u1".into())] {
                        snapshot.tabs[0].selected_popup = selected_popup;
                        for scroll in [0.0, 247.5, f32::MAX] {
                            let hints = ShortcutHintsScene::new(
                                Vec::new(),
                                size,
                                metrics,
                                WorkspaceFocus::Terminal,
                                |_| 0.0,
                            );
                            let scene = WorkspaceScene::from_snapshot_with_bar(
                                &snapshot,
                                EonBar::default(),
                                size,
                                metrics,
                                0.0,
                                scroll,
                                |_, text| (text.into(), text.len() as f32 * metrics.width),
                                Some(hints),
                            );
                            for (allocated, terminal) in
                                std::iter::once((scene.chrome, scene.terminal)).chain(
                                    scene
                                        .other_columns
                                        .iter()
                                        .map(|column| (column.chrome, column.terminal)),
                                )
                            {
                                let fill = scene.card_fill(
                                    allocated,
                                    metrics,
                                    crate::render::physical_outline_width(2, metrics.scale),
                                );
                                assert!(
                                    metrics.padding >= fill.radius,
                                    "padding below radius at scale {scale}, size {size:?}, columns {count}"
                                );
                                assert!(
                                    terminal.left + metrics.padding
                                        >= fill.bounds.left + fill.radius
                                );
                                assert!(
                                    terminal.right() - metrics.padding
                                        <= fill.bounds.right() - fill.radius
                                );
                            }
                        }
                    }
                }
            }
        }
        let project = |snapshot: &Snapshot| {
            WorkspaceScene::from_snapshot(
                snapshot,
                PhysicalSize::new(960, 600),
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
            )
        };
        let stack = project(&snapshot);
        snapshot.tabs[0].selected_popup = Some("u1".into());
        let popup = project(&snapshot);

        assert_eq!(popup.chrome, stack.chrome);
        let painted = stack.card_rect(stack.chrome, CellMetrics::for_scale(1.0), 2.0);
        assert_eq!(
            painted.top,
            stack.panes[0].rect.top + stack.panes[0].rect.height / 2.0
        );
        // The popup ends on the edge the column card and the sidebar card share.
        let popup_card = popup
            .card_fill(popup.chrome, CellMetrics::for_scale(1.0), 2.0)
            .bounds;
        assert_eq!(popup_card.bottom(), painted.bottom());
        assert_eq!(
            popup.card_rect(popup.chrome, CellMetrics::for_scale(1.0), 2.0),
            painted
        );
        for scale in [1.0, 1.3] {
            let metrics = CellMetrics::for_scale(scale);
            for count in 1..=4 {
                let mut snapshot = snapshot.clone();
                snapshot.tabs[0].columns = (1..=count)
                    .map(|index| {
                        let mut column = snapshot.tabs[0].columns[0].clone();
                        column.panes[0].id = format!("p{index}");
                        column.expanded = format!("p{index}");
                        column
                    })
                    .collect();
                let project = |snapshot: &Snapshot| {
                    WorkspaceScene::from_snapshot(
                        snapshot,
                        PhysicalSize::new(2200, 900),
                        metrics,
                        0.0,
                        0.0,
                        |_, text| (text.into(), text.len() as f32 * metrics.width),
                    )
                };
                snapshot.tabs[0].selected_popup = None;
                let panes = project(&snapshot);
                snapshot.tabs[0].selected_popup = Some("u1".into());
                let popup = project(&snapshot);
                let painted = panes.card_rect(
                    panes.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale),
                );
                assert_eq!(popup.pane_viewport, panes.pane_viewport);
                assert_eq!(
                    popup
                        .card_rect(
                            popup.chrome,
                            metrics,
                            crate::render::physical_outline_width(2, metrics.scale)
                        )
                        .top,
                    painted.top
                );
                assert_eq!(popup.sidebar().unwrap().top, painted.top);
                let stroke = crate::render::physical_outline_width(2, metrics.scale);
                let popup_bottom = popup
                    .card_fill(popup.chrome, metrics, stroke)
                    .bounds
                    .bottom();
                assert_eq!(
                    popup_bottom,
                    painted.bottom(),
                    "popup bottom at scale {scale}, {count} columns"
                );
                assert_eq!(
                    popup_bottom,
                    popup
                        .card_rect(popup.sidebar_area().unwrap(), metrics, stroke)
                        .bottom(),
                    "popup and sidebar bottoms at scale {scale}, {count} columns"
                );
                for column in &panes.other_columns {
                    assert_eq!(
                        panes
                            .card_rect(
                                column.chrome,
                                metrics,
                                crate::render::physical_outline_width(2, metrics.scale)
                            )
                            .top,
                        painted.top
                    );
                }
                assert!(
                    (popup.terminal.top
                        - painted.top
                        - snapshot.geometry.vertical_margin * metrics.scale
                        - popup_title_gutter(metrics))
                    .abs()
                        < 0.001,
                    "popup retains its label gutter at scale {scale}, {count} columns"
                );
            }
        }
        // Short windows must not let the popup's top border cross its terminal cells.
        for scale in [1.0, 1.3] {
            let base = CellMetrics::for_scale(scale);
            let metrics = CellMetrics {
                font_size: 32.0 * base.scale,
                height: 36.0 * base.scale,
                ..base
            };
            for count in [1, 2, 3, 4, 0] {
                let mut snapshot = snapshot.clone();
                let column = snapshot.tabs[0].columns[0].clone();
                snapshot.tabs[0].columns = (0..count)
                    .map(|index| {
                        let id = format!("p{}", index + 1);
                        let mut column = column.clone();
                        column.panes[0].id.clone_from(&id);
                        column.panes[0].attention = Some(eon_workspace_protocol::v22::Attention {
                            state: AttentionState::Unavailable,
                            record: None,
                        });
                        column.expanded = id;
                        column
                    })
                    .collect();
                snapshot.tabs[0].selected_pane = (count > 0).then(|| "p1".into());
                snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
                for height in [130, 1, 50, 74, 105, 140, 150, 160, 220, 900] {
                    let project = |snapshot: &Snapshot| {
                        WorkspaceScene::from_snapshot_with_zoom(
                            snapshot,
                            EonBar::default(),
                            PhysicalSize::new(5000, height),
                            metrics,
                            0.0,
                            0.0,
                            |_, text| (text.into(), text.len() as f32 * metrics.width),
                            None,
                            false,
                            SidebarView {
                                focused: Some("p1"),
                                ..SidebarView::default()
                            },
                        )
                    };
                    snapshot.tabs[0].selected_popup = None;
                    let panes = project(&snapshot);
                    let pane_card = panes.card_rect(
                        panes.chrome,
                        metrics,
                        crate::render::physical_outline_width(2, metrics.scale),
                    );
                    for (headers, _) in panes.column_panes() {
                        let first = &headers[0];
                        assert_eq!(
                            title_rule_y(first.rect.top, first.rect.height),
                            pane_card.top
                        );
                    }
                    if count > 0 {
                        assert!(panes.terminal.top >= pane_card.top);
                    }
                    let sidebar = panes.sidebar().unwrap();
                    assert_eq!(sidebar.top, pane_card.top);
                    for row in panes.sidebar_rows() {
                        assert!(row.rect.top >= sidebar.top);
                        if let Some(visible) = row.rect.intersection(panes.sidebar_list) {
                            assert!(visible.top >= sidebar.top);
                            assert!(visible.bottom() <= sidebar.bottom());
                        }
                    }
                    snapshot.tabs[0].selected_popup = Some("u1".into());
                    let popup = project(&snapshot);
                    let popup_card = popup_card_rect(popup.chrome, metrics);
                    assert!(
                        popup.terminal.top >= popup_card.top,
                        "popup terminal {:?} crosses card {:?}: scale {scale}, {count} columns, height {height}",
                        popup.terminal,
                        popup_card
                    );
                    assert!(
                        popup.terminal.height + 0.001
                            >= minimum_terminal(metrics, metrics.height).min(popup.chrome.height)
                    );
                    assert!(popup.terminal.bottom() <= popup.chrome.bottom() + 0.001);
                    if popup.chrome.height
                        >= minimum_terminal(metrics, metrics.height) + pane_card.top
                            - popup.chrome.top
                    {
                        assert_eq!(popup_card.top, pane_card.top);
                    }
                    let stroke = crate::render::physical_outline_width(2, metrics.scale);
                    let popup_bottom = popup
                        .card_fill(popup.chrome, metrics, stroke)
                        .bounds
                        .bottom();
                    assert_eq!(
                        popup_bottom,
                        pane_card.bottom(),
                        "popup bottom: scale {scale}, {count} columns, height {height}"
                    );
                    assert_eq!(
                        popup_bottom,
                        popup
                            .card_rect(popup.sidebar_area().unwrap(), metrics, stroke)
                            .bottom(),
                        "popup and sidebar bottoms: scale {scale}, {count} columns, height {height}"
                    );
                }
            }
        }
        let metrics = CellMetrics::for_scale(1.0);
        let size = PhysicalSize::new(1280, 720);
        for count in [1, 3, 4] {
            let mut snapshot = snapshot.clone();
            let pane = snapshot.tabs[0].columns[0].panes[0].clone();
            snapshot.tabs[0].columns[0].panes = (1..=count)
                .map(|index| Pane {
                    id: format!("p{index}"),
                    ..pane.clone()
                })
                .collect();
            let hints = ShortcutHintsScene::new(
                Vec::new(),
                size,
                metrics,
                WorkspaceFocus::Terminal,
                |_| 0.0,
            );
            let popup = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
                Some(hints),
            );
            let old_height = popup.chrome.height
                - 2.0 * snapshot.geometry.vertical_margin * metrics.scale
                - popup_title_gutter(metrics);
            let moved_edge = popup
                .card_rect(
                    popup.chrome,
                    metrics,
                    crate::render::physical_outline_width(2, metrics.scale),
                )
                .top
                - popup.chrome.top;
            assert_eq!(popup.terminal.height, old_height - moved_edge);
            assert_eq!(
                popup.terminal.bottom(),
                popup.chrome.bottom() - snapshot.geometry.vertical_margin * metrics.scale
            );
            let rows =
                |height: f32| ((height - 2.0 * metrics.padding) / metrics.height).floor() as u16;
            assert_eq!((rows(old_height), rows(popup.terminal.height)), (33, 32));
            println!(
                "1280x720, popup over {count} panes: {} -> {} rows, terminal height {old_height} -> {}",
                rows(old_height),
                rows(popup.terminal.height),
                popup.terminal.height
            );
        }
        let size = PhysicalSize::new(960, 600);
        let metrics = CellMetrics::for_scale(1.0);
        let gap = card_gap(metrics);
        // The card keeps one gap from the window's sides.
        assert_eq!(
            (stack.chrome.left, stack.chrome.right()),
            (gap, 960.0 - gap)
        );
        assert_ne!(popup.terminal, popup.chrome);
        // Tab hit rectangles keep their original inset; the card area adds the title margin.
        let small = CellMetrics {
            width: 5.0,
            height: 9.0,
            font_size: 8.0,
            ..metrics
        };
        for (metrics, title_margin) in [(metrics, 6.0), (small, 3.0)] {
            let scene =
                WorkspaceScene::from_snapshot(&snapshot, size, metrics, 0.0, 0.0, |_, text| {
                    (text.into(), text.len() as f32 * 10.0)
                });
            let gap = card_gap(metrics);
            let pill = scene.tabs[0].rect;
            assert_eq!((pill.left, pill.top), (gap, gap), "{metrics:?}");
            assert_eq!(
                scene.chrome.top - pill.bottom(),
                gap + title_margin,
                "{metrics:?}"
            );
        }
        // Toggling the popup keeps the shared card area, even where gaps yield on narrow or
        // short windows with several columns.
        let mut columns = snapshot.clone();
        for index in 2..=3 {
            columns.tabs[0]
                .columns
                .push(eon_workspace_protocol::v22::Column {
                    panes: vec![Pane {
                        label: format!("p{index}"),
                        pinned_name: None,
                        id: format!("p{index}"),
                        session: format!("session-{}", index + 1),
                        endpoint: format!("/run/pane-{index}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    }],
                    expanded: format!("p{index}"),
                });
        }
        for size in [PhysicalSize::new(120, 600), PhysicalSize::new(800, 105)] {
            let areas = [None, Some("u1".into())].map(|selected_popup| {
                columns.tabs[0].selected_popup = selected_popup;
                WorkspaceScene::from_snapshot(&columns, size, metrics, 0.0, 0.0, |_, text| {
                    (text.into(), text.len() as f32 * 10.0)
                })
                .pane_viewport
            });
            assert_eq!(areas[0], areas[1], "{size:?}");
        }
        for selected_popup in [None, Some("u1".into())] {
            snapshot.tabs[0].selected_popup = selected_popup;
            let baseline = project(&snapshot);
            let hint = ShortcutHint {
                modifiers: "Alt".into(),
                key: "/".into(),
                label: "all keys".into(),
                accessible_label: "Alt+/ — Show shortcuts".into(),
                priority: 0,
            };
            for focus in [
                WorkspaceFocus::Terminal,
                WorkspaceFocus::Header(WorkspaceHeaderControl::Shortcuts),
            ] {
                let footer =
                    ShortcutHintsScene::new(vec![hint.clone()], size, metrics, focus, |text| {
                        text.len() as f32 * 10.0
                    });
                let footer_rect = footer.rect;
                let with_hints = WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    EonBar::default(),
                    size,
                    metrics,
                    0.0,
                    0.0,
                    |_, text| (text.into(), text.len() as f32 * 10.0),
                    Some(footer),
                );
                // The footer's band replaces the bottom gap as the cards' bottom margin.
                assert_eq!(
                    baseline.terminal.height - with_hints.terminal.height,
                    footer_rect.height - gap
                );
                assert_eq!(baseline.pane_viewport.bottom(), size.height as f32 - gap);
                assert_eq!(with_hints.pane_viewport.bottom(), footer_rect.top);
                assert_eq!(footer_rect.bottom(), size.height as f32);
                assert_eq!(with_hints.chrome.left, gap);
                assert_eq!(with_hints.hit_test(20.0, footer_rect.top + 5.0), None);
                assert_eq!(
                    with_hints.has_control(WorkspaceHeaderControl::Shortcuts),
                    focus != WorkspaceFocus::Terminal
                );
                assert!(with_hints.has_control(WorkspaceHeaderControl::CloseTab));
            }
        }
    }

    #[test]
    fn shortcut_hints_drop_whole_pairs_and_rebuild_modifier_headers() {
        let hints = vec![
            ShortcutHint {
                modifiers: "Alt".into(),
                key: "M".into(),
                label: "new pane".into(),
                accessible_label: "Alt+M — New pane".into(),
                priority: 1,
            },
            ShortcutHint {
                modifiers: "Alt".into(),
                key: "/".into(),
                label: "all keys".into(),
                accessible_label: "Alt+/ — All keys".into(),
                priority: 0,
            },
            ShortcutHint {
                modifiers: "Super".into(),
                key: "F".into(),
                label: "界".repeat(80),
                accessible_label: "Super+F — Long custom tool".into(),
                priority: 4,
            },
            ShortcutHint {
                modifiers: "Ctrl+Shift".into(),
                key: "C".into(),
                label: "copy".into(),
                accessible_label: "Ctrl+Shift+C — Copy".into(),
                priority: 3,
            },
        ];
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for width in [0, 10, 120, 320, 340, 2000, 4000] {
                let scene = ShortcutHintsScene::new(
                    hints.clone(),
                    PhysicalSize::new((f64::from(width) * scale) as u32, 600),
                    metrics,
                    WorkspaceFocus::Terminal,
                    |text| text.chars().count() as f32 * metrics.width,
                );
                let chrome = pane_chrome_rect(
                    SceneRect {
                        width: (f64::from(width) * scale) as u32 as f32,
                        height: 600.0,
                        ..SceneRect::default()
                    },
                    metrics,
                );
                assert_eq!(scene.rect.left, chrome.left);
                assert_eq!(scene.rect.right(), chrome.right());
                assert_eq!(scene.rect.bottom(), 600.0);
                for piece in &scene.pieces {
                    assert!(piece.key_offset >= 0.0 && piece.key_offset < piece.label_offset);
                    assert!(piece.label_offset < piece.rect.width);
                    assert_eq!(piece.key_offset == 0.0, piece.modifiers.is_empty());
                    if !piece.modifiers.is_empty() {
                        let text_width = piece.modifiers.chars().count() as f32 * metrics.width;
                        assert!(
                            (piece.key_offset - text_width - metrics.padding * 2.0).abs() < 0.01
                        );
                    }
                    assert!(piece.rect.left >= metrics.padding);
                    assert!(piece.rect.right() <= scene.rect.right() - metrics.padding);
                    assert!(piece.rect.bottom() <= scene.rect.bottom());
                }
                if let Some((first, last)) = scene.pieces.first().zip(scene.pieces.last()) {
                    let left = first.rect.left - scene.rect.left;
                    let right = scene.rect.right() - last.rect.right();
                    assert!((left - right).abs() < 0.01);
                    if scene
                        .pieces
                        .iter()
                        .filter(|p| !p.modifiers.is_empty())
                        .count()
                        > 1
                    {
                        assert!((left - metrics.padding).abs() < 0.01);
                    }
                }
                let mut group_gap: Option<f32> = None;
                for pair in scene.pieces.windows(2) {
                    let gap = pair[1].rect.left - pair[0].rect.right();
                    if pair[1].modifiers.is_empty() {
                        assert!((gap - metrics.padding).abs() < 0.01);
                    } else {
                        assert!(gap >= metrics.padding * 2.0 - 0.01);
                        if let Some(previous) = group_gap {
                            assert!((gap - previous).abs() < 0.01);
                        }
                        group_gap = Some(gap);
                    }
                }
                if width == 340 {
                    assert!(!scene.complete);
                    assert_eq!(scene.pieces.len(), 2);
                    assert_eq!(scene.pieces[0].key, "M");
                    assert_eq!(scene.pieces[0].modifiers, "Alt");
                    assert_eq!(scene.pieces[1].key, "/");
                    assert!(scene.pieces[1].modifiers.is_empty());
                }
                if width >= 2000 {
                    assert!(scene.complete);
                    assert_eq!(
                        scene
                            .pieces
                            .iter()
                            .filter(|p| !p.modifiers.is_empty())
                            .count(),
                        3
                    );
                }
            }
        }
        let small_metrics = CellMetrics {
            width: 3.0,
            height: 6.0,
            font_size: 6.0,
            ..CellMetrics::for_scale(1.0)
        };
        let small = ShortcutHintsScene::new(
            hints.clone(),
            PhysicalSize::new(320, 600),
            small_metrics,
            WorkspaceFocus::Terminal,
            |text| text.chars().count() as f32 * small_metrics.width,
        );
        assert!(
            small.rect.height >= small_metrics.padding,
            "the hints row keeps at least the edge padding height"
        );
        for height in [0, 1, 10, 40] {
            let scene = ShortcutHintsScene::new(
                hints.clone(),
                PhysicalSize::new(320, height),
                CellMetrics::for_scale(1.0),
                WorkspaceFocus::Terminal,
                |text| text.chars().count() as f32 * 10.0,
            );
            assert!(scene.rect.top >= 0.0 && scene.rect.bottom() <= height as f32);
            assert!(scene.pieces.is_empty());
        }
        let scene = ShortcutHintsScene::new(
            hints,
            PhysicalSize::new(220, 600),
            CellMetrics::for_scale(1.0),
            WorkspaceFocus::Terminal,
            |text| text.chars().count() as f32 * 10.0,
        );
        assert_eq!(scene.pieces.len(), 1);
        assert_eq!(scene.pieces[0].key, "/");
        assert_eq!(scene.pieces[0].modifiers, "Alt");
        assert_eq!(scene.pieces[0].label, "all keys");
    }

    #[test]
    fn eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure() {
        use eon_workspace_protocol::v22::{Pane, PopupGeometry, Tab};

        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t2".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: (1..=3)
                .map(|index| Tab {
                    id: format!("t{index}"),
                    directory: format!("/tmp/tab-{index}").into_bytes(),
                    pending: false,
                    selected_pane: Some(format!("p{index}")),
                    selected_popup: None,
                    columns: vec![eon_workspace_protocol::v22::Column {
                        panes: vec![Pane {
                            label: format!("p{index}"),
                            pinned_name: None,
                            id: format!("p{index}"),
                            session: format!("s{index}"),
                            endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        }],
                        expanded: format!("p{index}"),
                    }],
                    popups: Vec::new(),
                    attention_rollup: None,
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v22::Sidebar::Open,
        };
        let metrics = CellMetrics::for_scale(1.0);

        for width in [960, 100, 1] {
            let scene = WorkspaceScene::from_snapshot(
                &snapshot,
                PhysicalSize::new(width, 600),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
            );

            assert!(scene.tab_viewport.width > 0.0);
            assert_eq!(scene.header.height, scene.tab_viewport.height);
            if width == 100 {
                assert!(scene.tabs[1].rect.width - metrics.padding * 2.0 >= metrics.width);
            }
            assert!(scene.tab_viewport.right() <= scene.controls[0].rect.left);
            assert!(scene.controls[0].rect.right() <= scene.drag_region.left);
            assert!(scene.drag_region.right() <= scene.controls[1].rect.left);
            for pair in scene.controls.windows(2) {
                assert!(pair[0].rect.right() <= pair[1].rect.left);
            }
            assert!(scene.controls[2].rect.right() <= width as f32);
            assert_eq!(
                scene.controls.map(|control| scene.hit_test(
                    control.rect.left + control.rect.width / 2.0,
                    control.rect.height / 2.0
                )),
                [
                    Some(WorkspaceHit::Control(WorkspaceHeaderControl::NewTab)),
                    Some(WorkspaceHit::Control(WorkspaceHeaderControl::Shortcuts)),
                    Some(WorkspaceHit::Control(WorkspaceHeaderControl::CloseTab)),
                ]
            );
        }

        let wide = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(960, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        assert_eq!(wide.tab_scroll_limit(), 0.0);
        assert_eq!(
            wide.controls[0].rect.left,
            wide.tabs.last().unwrap().rect.right() + tab_gap(metrics, wide.header.height)
        );
        assert!(wide.drag_region.width > 0.0);
        assert_eq!(
            wide.hit_test(
                wide.drag_region.left + wide.drag_region.width / 2.0,
                wide.drag_region.height / 2.0
            ),
            Some(WorkspaceHit::Drag)
        );

        let narrow = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(100, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        assert!(narrow.tab_scroll_limit() > 0.0);
        assert_eq!(narrow.controls[0].rect.left, narrow.tab_viewport.right());
        assert_eq!(narrow.drag_region.width, 0.0);
        // The narrowest strip keeps the gap after its first pill, apart from New tab.
        assert_eq!(
            narrow.hit_test(
                narrow.tabs[0].rect.right() + 1.0,
                narrow.tab_viewport.height / 2.0
            ),
            None
        );
        let scrolled = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(100, 600),
            metrics,
            narrow.tab_scroll_limit(),
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );
        assert_eq!(scrolled.controls[0].rect, narrow.controls[0].rect);
        assert_eq!(
            scrolled.tabs.last().unwrap().rect.right() + tab_gap(metrics, scrolled.header.height),
            scrolled.controls[0].rect.left
        );

        let mut snapshot = snapshot;
        snapshot.sidebar = Sidebar::Closed;
        let project = |height, metrics: CellMetrics, hints: bool| {
            let size = PhysicalSize::new(1280, height);
            WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar::default(),
                size,
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * metrics.width),
                hints.then(|| {
                    ShortcutHintsScene::new(
                        Vec::new(),
                        size,
                        metrics,
                        WorkspaceFocus::Terminal,
                        |_| 0.0,
                    )
                }),
            )
        };
        // One pane, no hints: the top gap yields before the nominal 30 px pills shrink.
        for (height, expected) in [
            (102, 10.0..40.0),
            (94, 2.0..32.0),
            (60, 2.0..32.0),
            (40, 2.0..32.0),
            (30, 0.0..30.0),
            (18, 0.0..18.0),
            (17, 4.0..13.0),
            (12, 3.0..9.0),
        ] {
            let scene = project(height, metrics, false);
            let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
            assert_eq!(pill.top..pill.bottom(), expected, "window height {height}");
        }
        // The new title margin yields before the previous layout loses any space.
        for (height, offset) in [
            (102, 0.0),
            (108, 0.0),
            (110, 0.0),
            (112, 2.0),
            (114, 4.0),
            (116, 6.0),
        ] {
            let scene = project(height, metrics, false);
            assert_eq!(
                scene.pane_viewport.top - scene.header.height,
                offset,
                "height {height}"
            );
        }
        for scale in [1.0, 1.3] {
            for line_height in [1.125, 2.0] {
                let mut metrics = CellMetrics::for_scale(scale);
                metrics.height = (metrics.font_size * line_height).round();
                let ordinary = project(900, metrics, false);
                let nominal = ordinary
                    .bar_pill_rect(ordinary.tabs[0].rect, metrics)
                    .height;
                for hints in [false, true] {
                    for height in 0..=220 {
                        let scene = project(height, metrics, hints);
                        let card_top = scene.pane_viewport.top;
                        let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
                        assert!(pill.top >= 0.0 && pill.height >= 0.0);
                        assert!(
                            pill.bottom() <= card_top + 0.001,
                            "{scale}/{line_height}, height {height}, hints {hints}"
                        );
                        if card_top >= nominal {
                            assert!((pill.height - nominal).abs() < 0.001);
                        } else if card_top >= metrics.height {
                            assert_eq!(pill.top, 0.0);
                            assert_eq!(pill.height, card_top);
                        } else {
                            let old = scene.tabs[0].rect;
                            let end = old.bottom().min(card_top);
                            assert!((pill.top - old.top.min(end)).abs() < 0.001);
                            assert!((pill.bottom() - end).abs() < 0.001);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn attention_badges_mark_tab_corners_without_moving_tabs() {
        use eon_workspace_protocol::v22::{
            Attention, AttentionRecord, AttentionState as State, Column, Pane, PopupGeometry,
        };

        let attention = |state, stale| Attention {
            state,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: "codex".into(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1,
                stale,
                others: 0,
            }),
        };
        // Each tab's panes and their attention; the rollup is the most urgent of them.
        let tabs: [&[Option<(State, bool)>]; 8] = [
            &[Some((State::NeedsInput, false))],
            &[Some((State::Error, false))],
            &[Some((State::Finished, true))],
            &[Some((State::Working, false)), Some((State::Idle, false))],
            &[Some((State::Idle, false))],
            &[None],
            &[
                Some((State::NeedsInput, true)),
                Some((State::NeedsInput, false)),
            ],
            &[Some((State::Working, true))],
        ];
        let mut snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: tabs
                .iter()
                .enumerate()
                .map(|(index, panes)| Tab {
                    id: format!("t{}", index + 1),
                    directory: format!("/tmp/tab-{}", index + 1).into_bytes(),
                    pending: false,
                    selected_pane: Some(format!("p{index}-0")),
                    selected_popup: None,
                    columns: vec![Column {
                        panes: panes
                            .iter()
                            .enumerate()
                            .map(|(pane, state)| Pane {
                                label: format!("p{index}-{pane}"),
                                pinned_name: None,
                                id: format!("p{index}-{pane}"),
                                session: format!("s{index}-{pane}"),
                                endpoint: format!("/tmp/p{index}-{pane}.sock").into_bytes(),
                                live: true,
                                attention: state.map(|(state, stale)| attention(state, stale)),
                            })
                            .collect(),
                        expanded: format!("p{index}-0"),
                    }],
                    popups: Vec::new(),
                    attention_rollup: panes.iter().flatten().map(|(state, _)| *state).min(),
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v22::Sidebar::Closed,
        };
        let metrics = CellMetrics::for_scale(1.0);
        let project = |snapshot: &Snapshot, width, scroll| {
            WorkspaceScene::from_snapshot(
                snapshot,
                PhysicalSize::new(width, 600),
                metrics,
                scroll,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
            )
        };
        let scene = project(&snapshot, 1600, 0.0);
        let badges = scene
            .tabs
            .iter()
            .map(|tab| tab.badge.map(|badge| (badge.state, badge.dimmed)))
            .collect::<Vec<_>>();
        assert_eq!(
            badges,
            [
                Some((State::NeedsInput, false)),
                Some((State::Error, false)),
                Some((State::Finished, true)),
                Some((State::Working, false)),
                None,
                None,
                Some((State::NeedsInput, false)),
                Some((State::Working, true)),
            ],
            "the active tab is badged, idle and empty tabs are not, and a badge dims only when \
             every terminal in its state is stale"
        );
        assert_eq!(
            scene.tabs[0].accessible_label(),
            "Tab 1 of 8  /tmp/tab-1, needs input"
        );
        assert_eq!(
            scene.tabs[2].accessible_label(),
            "Tab 3 of 8  /tmp/tab-3, finished, status stale"
        );
        assert_eq!(scene.tabs[4].accessible_label(), "Tab 5 of 8  /tmp/tab-5");
        assert!(scene.edge_badges.is_empty(), "every tab fits");
        // Typography too small for a visible glyph draws no badge but keeps the state.
        let tiny = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(400, 150),
            CellMetrics::for_scale(0.25),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 2.5),
        );
        assert!(tiny.tabs.iter().all(|tab| tab.badge.is_none()));
        assert_eq!(
            tiny.tabs[0].accessible_label(),
            "Tab 1 of 8  /tmp/tab-1, needs input"
        );
        // While the sidebar shows, the tabs carry no badge or state; hidden for width, they do.
        let mut open = snapshot.clone();
        open.sidebar = eon_workspace_protocol::v22::Sidebar::Open;
        let beside = project(&open, 1600, 0.0);
        assert!(beside.sidebar().is_some());
        assert!(beside.tabs.iter().all(|tab| tab.badge.is_none()));
        assert_eq!(beside.tabs[0].accessible_label(), "Tab 1 of 8  /tmp/tab-1");
        let narrow = project(&open, 600, 0.0);
        assert!(narrow.sidebar().is_none());
        assert_eq!(
            narrow.tabs[0].badge.map(|badge| badge.state),
            Some(State::NeedsInput)
        );

        let mut plain = snapshot.clone();
        for tab in &mut plain.tabs {
            tab.attention_rollup = None;
            for pane in &mut tab.columns[0].panes {
                pane.attention = None;
            }
        }
        for (width, scroll) in [(1600, 0.0), (400, 0.0), (400, f32::MAX)] {
            let badged = project(&snapshot, width, scroll);
            let unbadged = project(&plain, width, scroll);
            for (tab, plain) in badged.tabs.iter().zip(&unbadged.tabs) {
                assert_eq!((tab.rect, tab.label()), (plain.rect, plain.label()));
                if let Some(badge) = tab.badge {
                    // The badge stays clear of the label's text box and inside the header.
                    assert!(badge.rect.left >= tab.rect.right() - metrics.padding);
                    assert!(badge.rect.top >= 0.0);
                    assert!(badge.rect.bottom() <= badged.header.height);
                }
            }
            assert_eq!(badged.tab_viewport, unbadged.tab_viewport);
        }

        // Overflow: each scroll edge shows the most urgent badge scrolled past it.
        let edges = |scene: &WorkspaceScene| {
            scene
                .edge_badges
                .iter()
                .map(|badge| {
                    assert!(badge.rect.left >= scene.tab_viewport.left);
                    assert!(badge.rect.right() <= scene.tab_viewport.right());
                    // Labels are clipped clear of edge badges, which they would otherwise cover.
                    assert!(badge.rect.intersection(scene.tab_label_clip).is_none());
                    (
                        badge.rect.left < scene.tab_viewport.width / 2.0,
                        badge.state,
                        badge.dimmed,
                    )
                })
                .collect::<Vec<_>>()
        };
        let start = project(&snapshot, 400, 0.0);
        assert!(start.tab_scroll_limit() > 0.0);
        assert_eq!(edges(&start), [(false, State::NeedsInput, false)]);
        let end = project(&snapshot, 400, f32::MAX);
        assert_eq!(edges(&end), [(true, State::NeedsInput, false)]);
        snapshot.tabs[0].columns[0].panes[0].attention = None;
        snapshot.tabs[0].attention_rollup = None;
        assert_eq!(
            edges(&project(&snapshot, 400, f32::MAX)),
            [(true, State::Error, false)]
        );
    }

    #[test]
    fn tab_directory_labels_use_current_position_and_bounded_path_context() {
        assert_eq!(
            tab_labels(0, 5, b"/home/alice"),
            ("1  alice".into(), "Tab 1 of 5  /home/alice".into())
        );
        for directory in [b"/".as_slice(), b"///"] {
            assert_eq!(
                tab_labels(1, 5, directory),
                (
                    "2  /".into(),
                    format!("Tab 2 of 5  {}", String::from_utf8_lossy(directory))
                )
            );
        }
        assert_eq!(
            tab_labels(2, 5, b"/srv/nova"),
            ("3  nova".into(), "Tab 3 of 5  /srv/nova".into())
        );
        assert_eq!(
            tab_labels(3, 5, b"/tmp/eon-\xff"),
            ("4  eon-�".into(), "Tab 4 of 5  /tmp/eon-�".into())
        );
        let long = format!("/tmp/{}", "eon".repeat(100));
        let labels = tab_labels(4, 5, long.as_bytes());
        assert_eq!(labels.0, format!("5  {}", "eon".repeat(100)));
        assert_eq!(labels.1, format!("Tab 5 of 5  {long}"));
    }

    fn draw_style(selected: bool) -> DrawStyle {
        DrawStyle {
            foreground: Color::default(),
            background: Color::default(),
            underline_color: Color::default(),
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected,
            background_is_default: !selected,
            protected: false,
            underline: Underline::None,
        }
    }

    fn draw_cell(text: &str, width: CellWidth, selected: bool) -> DrawCell {
        DrawCell {
            width,
            text: text.into(),
            hyperlink: String::new(),
            style: draw_style(selected),
        }
    }

    #[test]
    fn background_opacity_provenance_distinguishes_default_explicit_selection_and_inverse() {
        let mut style = CellStyle {
            foreground: StyleColor::None,
            background: StyleColor::None,
            underline_color: StyleColor::None,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected: false,
            protected: false,
            underline: Underline::None,
        };

        assert!(is_default_background(style));
        style.background = StyleColor::Rgb(Rgb::BLACK);
        assert!(!is_default_background(style));
        style.background = StyleColor::None;
        style.selected = true;
        assert!(!is_default_background(style));
        style.selected = false;
        style.inverse = true;
        assert!(!is_default_background(style));
    }

    #[test]
    fn hyperlinks_follow_visible_heads_and_wide_cells() {
        let mut row = DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: vec![
                draw_cell("A", CellWidth::Narrow, false),
                draw_cell("界", CellWidth::Wide, false),
                draw_cell("", CellWidth::SpacerTail, false),
                draw_cell("hidden", CellWidth::Narrow, false),
                draw_cell("B", CellWidth::Narrow, false),
                draw_cell("", CellWidth::SpacerHead, false),
            ],
        };
        for cell in &mut row.cells {
            cell.hyperlink = "https://example.com/exact?x=%26&y=2".into();
        }
        row.cells[2].hyperlink = "https://wrong-tail.invalid".into();
        row.cells[3].style.invisible = true;
        let links = row.hyperlinks(2).collect::<Vec<_>>();
        assert_eq!(links.len(), 2);
        assert_eq!((links[0].row, links[0].column, links[0].columns), (2, 0, 3));
        assert_eq!(links[0].uri, row.cells[0].hyperlink);
        assert!(links[0].contains(2, 2));
        assert!(!links[0].contains(2, 3));
        assert_eq!((links[1].column, links[1].columns), (4, 1));
    }

    /// Rows of narrow cells padded to `columns`; `true` marks a row that soft-wraps.
    fn text_scene(columns: usize, rows: &[(&str, bool)]) -> Scene {
        let content = rows
            .iter()
            .enumerate()
            .map(|(index, (text, wrapped))| DrawRow {
                wrapped: *wrapped,
                wrap_continuation: index > 0 && rows[index - 1].1,
                cells: format!("{text:columns$}")
                    .chars()
                    .map(|c| draw_cell(&c.to_string(), CellWidth::Narrow, false))
                    .collect(),
            })
            .collect::<Vec<_>>();
        Scene {
            revision: 1,
            columns: columns as u16,
            rows: content.len() as u16,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content,
        }
    }

    fn link_spans(scene: &Scene) -> Vec<(u16, u16, u16, String)> {
        scene
            .hyperlinks()
            .map(|link| (link.row, link.column, link.columns, link.uri.into_owned()))
            .collect()
    }

    #[test]
    fn plain_text_links_trim_punctuation_and_unopened_brackets() {
        for (text, expected) in [
            ("see https://example.com/a.", Some("https://example.com/a")),
            ("(https://example.com/x)", Some("https://example.com/x")),
            (
                "https://e.org/Rust_(game),",
                Some("https://e.org/Rust_(game)"),
            ),
            ("[HTTP://localhost:5173/]", Some("HTTP://localhost:5173/")),
            ("'http://[::1]:3000/'", Some("http://[::1]:3000/")),
            ("xhttps://example.com", None),
            ("ftp://e.org file:///etc localhost:3000", None),
            ("https://user@example.com/", None),
            ("https://", None),
        ] {
            let scene = text_scene(40, &[(text, false)]);
            let links = link_spans(&scene);
            let Some(uri) = expected else {
                assert!(links.is_empty(), "{text:?}: {links:?}");
                continue;
            };
            let column = text.find(uri).unwrap() as u16;
            assert_eq!(
                links,
                [(0, column, uri.len() as u16, uri.into())],
                "{text:?}"
            );
            let link = scene.hyperlink_at(0, column + 1).unwrap();
            assert_eq!(link.uri, uri);
            assert!(scene.hyperlink_at(0, column + uri.len() as u16).is_none());
        }
    }

    #[test]
    fn plain_text_links_join_soft_wrapped_rows_only() {
        let uri = "https://example.com/p";
        let wrapped = text_scene(14, &[("go https://exa", true), ("mple.com/p ok", false)]);
        assert_eq!(
            link_spans(&wrapped),
            [(0, 3, 11, uri.into()), (1, 0, 10, uri.into())]
        );
        assert_eq!(wrapped.hyperlink_at(1, 9).unwrap().uri, uri);
        assert!(wrapped.hyperlink_at(1, 10).is_none());

        let hard = text_scene(14, &[("go https://exa", false), ("mple.com/p ok", false)]);
        assert_eq!(link_spans(&hard), [(0, 3, 11, "https://exa".into())]);
    }

    #[test]
    fn explicit_links_and_non_uri_cells_bound_plain_text_links() {
        let mut scene = text_scene(40, &[("a https://e.org/x b https://f.org/y", false)]);
        scene.content[0].cells[16].hyperlink = "https://explicit.invalid/".into();
        assert_eq!(
            link_spans(&scene),
            [
                (0, 16, 1, "https://explicit.invalid/".into()),
                (0, 20, 15, "https://f.org/y".into()),
            ]
        );
        assert!(scene.hyperlink_at(0, 3).is_none());

        let row = &mut scene.content[0];
        row.cells[15] = draw_cell("界", CellWidth::Wide, false);
        row.cells[16] = draw_cell("", CellWidth::SpacerTail, false);
        row.cells[33].style.invisible = true;
        assert_eq!(
            link_spans(&scene),
            [
                (0, 2, 13, "https://e.org".into()),
                (0, 20, 13, "https://f.org".into()),
            ]
        );
    }

    #[test]
    fn glyph_runs_preserve_authoritative_cell_starts() {
        let scene = Scene {
            revision: 1,
            columns: 10,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![
                    draw_cell("A", CellWidth::Narrow, false),
                    draw_cell("e\u{301}", CellWidth::Narrow, false),
                    draw_cell("क्ष", CellWidth::Narrow, false),
                    draw_cell("界", CellWidth::Wide, false),
                    draw_cell("", CellWidth::SpacerTail, false),
                    draw_cell("👩‍💻", CellWidth::Wide, false),
                    draw_cell("", CellWidth::SpacerTail, false),
                    draw_cell("א", CellWidth::Narrow, false),
                    draw_cell("𐐀", CellWidth::Narrow, false),
                    draw_cell("│", CellWidth::Narrow, false),
                ],
            }],
        };

        let actual = scene
            .glyph_runs()
            .into_iter()
            .map(|run| (run.column, run.columns, run.text))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [
                (0, 1, "A"),
                (1, 1, "e\u{301}"),
                (2, 1, "क्ष"),
                (3, 2, "界"),
                (5, 2, "👩‍💻"),
                (7, 1, "א"),
                (8, 1, "𐐀"),
                (9, 1, "│"),
            ]
            .map(|(column, columns, text)| (column, columns, text.to_owned()))
        );
    }

    #[test]
    fn accessibility_projection_keeps_authoritative_selected_cells() {
        let mut scene = Scene {
            revision: 3,
            columns: 4,
            rows: 2,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: Some(DrawCursor {
                visible: true,
                blinking: false,
                password_input: false,
                shape: CursorShape::Block,
                column: 0,
                row: 0,
                at_wide_tail: false,
                color: Color::default(),
                explicit_color: false,
            }),
            content: vec![
                DrawRow {
                    wrapped: false,
                    wrap_continuation: false,
                    cells: vec![
                        draw_cell("a", CellWidth::Narrow, false),
                        draw_cell("e\u{301}", CellWidth::Narrow, true),
                        draw_cell("", CellWidth::Narrow, true),
                        draw_cell("", CellWidth::Narrow, false),
                    ],
                },
                DrawRow {
                    wrapped: false,
                    wrap_continuation: false,
                    cells: vec![
                        draw_cell("界", CellWidth::Wide, true),
                        draw_cell("", CellWidth::SpacerTail, true),
                        draw_cell("", CellWidth::Narrow, false),
                        draw_cell("", CellWidth::Narrow, false),
                    ],
                },
            ],
        };

        let content = scene.accessible_content();
        assert_eq!(content.plain_text(), "ae\u{301} \n界");
        assert_eq!(
            content.selection,
            Some(AccessibleSelection {
                anchor: AccessiblePosition {
                    row: 0,
                    character_index: 1,
                },
                focus: AccessiblePosition {
                    row: 1,
                    character_index: 1,
                },
            })
        );
        assert_eq!(content.rows[0].character_lengths, [1, 3, 1, 1]);
        assert_eq!(content.rows[0].column_spans, [0..1, 1..2, 2..3, 3..3]);
        assert_eq!(content.rows[1].column_spans.len(), 1);
        assert_eq!(content.rows[1].column_spans[0], 0..2);

        for cell in scene.content.iter_mut().flat_map(|row| &mut row.cells) {
            cell.style.selected = false;
        }
        for (row, column, at_wide_tail, character_index) in [
            (0, 0, false, 0),
            (0, 1, false, 1),
            (0, 2, false, 2),
            (0, 3, false, 2),
            (1, 0, false, 0),
            (1, 1, true, 0),
            (1, 2, false, 1),
            (1, 3, false, 1),
        ] {
            let cursor = scene.cursor.as_mut().unwrap();
            cursor.row = row;
            cursor.column = column;
            cursor.at_wide_tail = at_wide_tail;
            let caret = AccessiblePosition {
                row: usize::from(row),
                character_index,
            };
            assert_eq!(
                scene.accessible_content().selection,
                Some(AccessibleSelection {
                    anchor: caret,
                    focus: caret,
                }),
                "cursor at {row},{column}"
            );
        }
        scene.content[1].cells.clear();
        let caret = AccessiblePosition {
            row: 1,
            character_index: 0,
        };
        assert_eq!(
            scene.accessible_content().selection,
            Some(AccessibleSelection {
                anchor: caret,
                focus: caret,
            })
        );
        scene.cursor = None;
        assert_eq!(scene.accessible_content().selection, None);
    }

    #[test]
    fn accessibility_projection_keeps_cells_atomic_and_marks_oversized_text() {
        let scene = |cells: Vec<DrawCell>| Scene {
            revision: 4,
            columns: u16::try_from(cells.len()).unwrap(),
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            graphics: SceneGraphics::default(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                cells,
            }],
        };
        let selected_first_cell = Some(AccessibleSelection {
            anchor: AccessiblePosition {
                row: 0,
                character_index: 0,
            },
            focus: AccessiblePosition {
                row: 0,
                character_index: 1,
            },
        });

        let content = scene(vec![
            draw_cell("e\u{301}", CellWidth::Narrow, true),
            draw_cell("👩‍💻", CellWidth::Wide, false),
            draw_cell("", CellWidth::SpacerTail, false),
        ])
        .accessible_content();
        assert_eq!(content.rows[0].character_lengths, [3, 11]);
        assert_eq!(content.selection, selected_first_cell);

        let representable = format!("e{}", "\u{301}".repeat(127));
        let content =
            scene(vec![draw_cell(&representable, CellWidth::Narrow, true)]).accessible_content();
        assert_eq!(content.plain_text(), representable);
        assert_eq!(content.rows[0].character_lengths, [255]);

        let oversized = format!("e{}", "\u{301}".repeat(128));
        let scene = scene(vec![draw_cell(&oversized, CellWidth::Narrow, true)]);
        assert_eq!(scene.glyph_runs()[0].text, oversized);
        let content = scene.accessible_content();
        assert_eq!(content.plain_text(), "\u{fffd}");
        assert_eq!(content.rows[0].character_lengths, [3]);
        assert_eq!(content.selection, selected_first_cell);
    }

    #[test]
    fn shortcut_viewer_clamps_scroll_and_keeps_fixed_dismissal_space() {
        let groups = vec![ShortcutGroup::new(
            "Projects and tools",
            (0..32)
                .map(|index| ShortcutRow::new("Alt+Z", format!("Entry {index}")))
                .collect(),
        )];
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            for (width, height) in [(960, 600), (320, 240), (320, 180)] {
                let size = PhysicalSize::new(
                    (f64::from(width) * scale) as u32,
                    (f64::from(height) * scale) as u32,
                );
                let start =
                    DialogScene::new(DialogKind::Shortcuts, groups.clone(), size, metrics, -100.0);
                let end = DialogScene::new(
                    DialogKind::Shortcuts,
                    groups.clone(),
                    size,
                    metrics,
                    f32::MAX,
                );

                assert_eq!(start.scroll, 0.0);
                assert!(end.max_scroll > 0.0);
                assert_eq!(end.scroll, end.max_scroll);
                assert!(start.bounds.width <= size.width as f32);
                assert!(start.bounds.height <= size.height as f32);
                assert!(start.title.bottom() <= start.subtitle.top);
                assert!(
                    start.subtitle.bottom() + metrics.height * 0.25 <= start.groups[0].heading.top,
                    "header text overlaps content at {size:?}, scale {scale}"
                );
                assert!(start.groups[0].heading.bottom() <= start.groups[0].rows[0].rect.top);
                if height >= 240 {
                    assert!(start.groups[0].rows[0].rect.bottom() <= start.content.bottom());
                }
                assert_eq!(start.title, end.title);
                assert_eq!(start.subtitle, end.subtitle);
                assert_eq!(start.content, end.content);
                assert!(start.content.bottom() < start.bounds.bottom());
                assert_eq!(end.groups[0].rows.len(), 32);
            }
        }

        let metrics = CellMetrics::for_scale(1.0);
        let tiny = DialogScene::new(
            DialogKind::Shortcuts,
            Vec::new(),
            PhysicalSize::new(1, 1),
            metrics,
            0.0,
        );
        assert!(tiny.bounds.right() <= 1.0 && tiny.bounds.bottom() <= 1.0);
        assert!(
            tiny.content.right() <= tiny.bounds.right()
                && tiny.content.bottom() <= tiny.bounds.bottom()
        );
        assert!(tiny.content.height >= 0.0);

        // The close prompt fits its rows: header labels, then popup entry labels.
        let pane = |id: &str| Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: id.as_bytes().to_vec(),
            live: true,
            attention: None,
        };
        let tab = Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v22::Column {
                panes: vec![pane("p1"), pane("p2")],
                expanded: "p1".into(),
            }],
            popups: vec![eon_workspace_protocol::v22::Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "popup-session".into(),
                endpoint: b"/tmp/popup.sock".to_vec(),
                attention: None,
            }],
            attention_rollup: None,
        };
        let entries = [PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: eon_workspace_protocol::v22::PopupKey::Bound(
                eon_workspace_protocol::v22::Shortcut {
                    modifiers: ALT | SHIFT,
                    key: "KeyL".into(),
                },
            ),
        }];
        let size = PhysicalSize::new(960, 600);
        let prompt =
            |detach| DialogScene::close_last_tab(&tab, &entries, detach, None, size, metrics, 0.0);
        let asked = prompt(Some("Alt+Shift+D".into()));
        let panes = tab.panes().count();
        assert_eq!(
            asked.kind,
            DialogKind::CloseLastTab {
                terminals: panes + 1,
                detach: Some("Alt+Shift+D".into()),
                workspace: None,
            }
        );
        let rows = &asked.groups[0].rows;
        assert_eq!(rows.len(), panes + 1);
        assert!(rows[..panes].iter().all(|row| row.shortcut == "Pane"));
        assert_eq!(rows[0].action, tab.panes().next().unwrap().label);
        assert_eq!(
            (rows[panes].shortcut.as_str(), rows[panes].action.as_str()),
            ("Popup", "Agent")
        );
        assert_eq!(asked.groups[0].heading.height, 0.0);
        assert_eq!(asked.max_scroll, 0.0);
        assert!(rows[panes].rect.bottom() <= asked.content.bottom());
        assert!(asked.bounds.height < size.height as f32 / 2.0);
        assert_eq!(asked.kind.footer().len(), 2);
        // Without the detach chord the prompt drops its hint line and shrinks.
        let taken = prompt(None);
        assert_eq!(taken.kind.footer(), ["Enter Quit · Esc Cancel"]);
        assert!(taken.bounds.height < asked.bounds.height);
        // While other workspaces remain, the close removes the shown one instead.
        let workspace = DialogScene::close_last_tab(
            &tab,
            &entries,
            Some("Alt+Shift+D".into()),
            Some("notes".into()),
            size,
            metrics,
            0.0,
        );
        assert_eq!(workspace.kind.title(), "Close the last tab?");
        assert_eq!(
            workspace.kind.subtitle(),
            "This closes workspace notes and stops 3 terminals."
        );
        assert_eq!(
            workspace.kind.footer(),
            [
                "Enter Close · Esc Cancel",
                "Alt+Shift+D keeps it running instead"
            ]
        );
        // A list taller than the window scrolls within the prompt.
        let crowded = DialogScene::close_last_tab(
            &tab,
            &entries,
            None,
            None,
            PhysicalSize::new(320, 180),
            metrics,
            f32::MAX,
        );
        assert!(crowded.max_scroll > 0.0);
        assert_eq!(crowded.scroll, crowded.max_scroll);
        assert!(crowded.bounds.bottom() <= 180.0);
    }
}
