use crate::{
    CpuUse, MAX_LINK_BYTES, RamUse,
    model::{active_popup, focused_terminal, presented_panes},
    render::{CellMetrics, FALLBACK_CAP_HEIGHT_EM},
};
use eon_workspace_protocol::v24::{
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
    /// Needs input, error, finished, working or background; sidebar rows also show idle and
    /// unavailable.
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
            AttentionState::Background => "waiting on background work",
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
    /// The label's judged windows, each a byte range with its pace shortfall.
    pub(crate) pace: Vec<QuotaPace>,
    pub(crate) accessible_label: String,
    pub(crate) description: String,
}

/// One fresh window's text within a quota label and how early its linear pace runs out
/// before the reset, as a share of the window; negative means it lasts with that share spare.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuotaPace {
    pub(crate) range: Range<usize>,
    pub(crate) shortfall: f32,
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
                (AttentionState::Background, _) => "Waiting on background work",
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

/// Formats `minutes` at the precision of a `window_minutes` window: days and hours for
/// day-long windows, hours and minutes for hour-long ones.
fn quota_minutes(minutes: u64, window_minutes: u32) -> String {
    if window_minutes >= 24 * 60 {
        let days = minutes / (24 * 60);
        let hours = minutes % (24 * 60) / 60;
        match (days, hours) {
            (0, hours) => format!("{hours}h"),
            (days, 0) => format!("{days}d"),
            (days, hours) => format!("{days}d{hours}h"),
        }
    } else if window_minutes >= 60 {
        let hours = minutes / 60;
        let minutes = minutes % 60;
        match (hours, minutes) {
            (0, minutes) => format!("{minutes}m"),
            (hours, 0) => format!("{hours}h"),
            (hours, minutes) => format!("{hours}h{minutes}m"),
        }
    } else {
        format!("{minutes}m")
    }
}

/// Seconds elapsed into the window at the observation, when the window has a reset.
fn quota_elapsed_seconds(window: &QuotaWindow, observed_at: u64) -> Option<u64> {
    let total_seconds = u64::from(window.duration_minutes) * 60;
    let remaining_seconds = window
        .resets_at?
        .saturating_sub(observed_at)
        .min(total_seconds);
    Some(total_seconds - remaining_seconds)
}

fn quota_window_position(window: &QuotaWindow, observed_at: u64) -> Option<String> {
    quota_elapsed_seconds(window, observed_at)
        .map(|elapsed| quota_minutes(elapsed / 60, window.duration_minutes))
}

/// How early, as a share of the window, the pace so far runs out before the reset if it
/// continues linearly; negative is the share left spare, down to -1. `None` has no basis:
/// no reset, or under a tenth of the window elapsed with quota left.
fn quota_shortfall(window: &QuotaWindow, observed_at: u64) -> Option<f32> {
    // EONW rejects zero-length windows.
    let elapsed = quota_elapsed_seconds(window, observed_at)? as f32
        / (u64::from(window.duration_minutes) * 60) as f32;
    let used = f32::from(100 - window.remaining_percent) / 100.0;
    if used >= 1.0 {
        Some(1.0 - elapsed)
    } else if elapsed < 0.1 {
        None
    } else {
        // Nothing used divides to infinity, which clamps to -1.
        Some((1.0 - elapsed / used).max(-1.0))
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

/// One provider chip's wide and compact labels with their pace, and its description.
struct QuotaText {
    wide: String,
    wide_pace: Vec<QuotaPace>,
    compact: String,
    compact_pace: Vec<QuotaPace>,
    description: String,
}

fn quota_text(
    provider: QuotaProvider,
    state: CodexQuotaState,
    observed_at: u64,
    windows: &[QuotaWindow],
) -> QuotaText {
    let suffix = if state == CodexQuotaState::Stale {
        " old"
    } else {
        ""
    };
    // Only fresh facts are judged; stale, blocked and unknown keep the neutral color.
    let shortfall = |window: &QuotaWindow| {
        (state == CodexQuotaState::Fresh)
            .then(|| quota_shortfall(window, observed_at))
            .flatten()
    };
    let label = |windows: &[QuotaWindow]| {
        let mut text = String::new();
        let mut pace = Vec::new();
        for (index, window) in windows.iter().enumerate() {
            if index > 0 {
                text.push_str(" · ");
            }
            let start = text.len();
            text.push_str(&quota_window_text(window, observed_at));
            if let Some(shortfall) = shortfall(window) {
                pace.push(QuotaPace {
                    range: start..text.len(),
                    shortfall,
                });
            }
        }
        text.push_str(suffix);
        (text, pace)
    };
    let ((wide, wide_pace), (compact, compact_pace)) = match state {
        CodexQuotaState::Fresh | CodexQuotaState::Stale => {
            let lowest = windows
                .iter()
                .min_by_key(|window| (window.remaining_percent, window.duration_minutes))
                .expect("EONW requires quota windows for fresh and stale states");
            (label(windows), label(std::slice::from_ref(lowest)))
        }
        CodexQuotaState::Blocked => (
            ("blocked".into(), Vec::new()),
            ("blocked".into(), Vec::new()),
        ),
        CodexQuotaState::Unknown => (
            ("unknown".into(), Vec::new()),
            ("unknown".into(), Vec::new()),
        ),
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
        let Some(shortfall) = shortfall(window) else {
            continue;
        };
        if window.remaining_percent == 0 {
            description.push_str(" Used up until reset.");
        } else if shortfall <= 0.0 {
            description.push_str(" At this pace it lasts until reset.");
        } else {
            // State the shortfall to the nearest unit the window counts in (hours for
            // day-long windows), and at least one.
            let unit = if window.duration_minutes >= 24 * 60 {
                60
            } else {
                1
            };
            let early = ((shortfall * window.duration_minutes as f32 / unit as f32).round() as u64)
                .max(1)
                * unit;
            let _ = write!(
                description,
                " At this pace it runs out about {} before reset.",
                quota_minutes(early, window.duration_minutes)
            );
        }
    }
    QuotaText {
        wide,
        wide_pace,
        compact,
        compact_pace,
        description,
    }
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
                .map(|(provider, window, text)| {
                    let (shown, pace) = if compact {
                        (&text.compact, &text.compact_pace)
                    } else {
                        (&text.wide, &text.wide_pace)
                    };
                    let (label, width) = header_text(HeaderText::Fact, shown);
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
                        pace: pace.clone(),
                        description: text.description.clone(),
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
mod tests;
