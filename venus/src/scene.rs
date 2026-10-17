use crate::{
    model::{active_popup, presented_panes},
    render::CellMetrics,
};
use eon_workspace_protocol::v10::{ClaudeQuotaState, CodexQuotaState, QuotaWindow, Snapshot};
use orbit_protocol::{
    Cell, CellStyle, CellWidth, CursorShape, Frame, Rgb, Row, Screen, StyleColor, Underline,
    session::VerticalDirection,
};
use std::{ffi::OsStr, fmt::Write, ops::Range, os::unix::ffi::OsStrExt, path::Path};
use winit::dpi::PhysicalSize;

/// Physical rectangle shared by workspace drawing, hit testing, and accessibility.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl SceneRect {
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
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
}

/// Latest bounded read-only metadata state for one visible Eon pane endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PaneMetadata {
    Connecting,
    Available {
        working_directory: String,
    },
    #[default]
    Unavailable,
}

const MAX_PANE_METADATA_FIELD_CHARS: usize = 80;

fn pane_label(id: &str, live: bool, metadata: &PaneMetadata) -> String {
    if !live {
        return format!("{id} offline");
    }
    let working_directory = match metadata {
        PaneMetadata::Connecting => return format!("{id} connecting"),
        PaneMetadata::Available { working_directory } => working_directory,
        PaneMetadata::Unavailable => return format!("{id} unavailable"),
    };
    if working_directory.trim().is_empty() {
        return id.to_owned();
    }
    let home = std::env::var_os("HOME");
    let working_directory = bounded_metadata_field(&compact_working_directory(
        working_directory,
        home.as_deref().map(Path::new),
    ));
    format!("{id}  {working_directory}")
}

fn local_working_directory(value: &str) -> &str {
    value
        .strip_prefix("file://")
        .and_then(|path| path.find('/').map(|slash| &path[slash..]))
        .unwrap_or(value)
}

fn compact_working_directory(value: &str, home: Option<&Path>) -> String {
    let value = local_working_directory(value);
    let path = Path::new(value);
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) {
        if path == home {
            return "~".to_owned();
        }
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }
    value.to_owned()
}

fn bounded_metadata_field(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect();
    let length = value.chars().count();
    if length <= MAX_PANE_METADATA_FIELD_CHARS {
        return value;
    }
    let (prefix, tail) = if let Some(tail) = value.strip_prefix("~/") {
        ("~/…/", tail)
    } else if let Some(tail) = value.strip_prefix('/') {
        ("…/", tail)
    } else {
        ("…", value.as_str())
    };
    let tail_length = MAX_PANE_METADATA_FIELD_CHARS - prefix.chars().count();
    let start = tail
        .char_indices()
        .nth(tail.chars().count().saturating_sub(tail_length))
        .map_or(0, |(index, _)| index);
    let truncated_component = start > 0 && !tail[..start].ends_with('/');
    let tail = &tail[start..];
    let tail = if truncated_component {
        tail.split_once('/').map_or(tail, |(_, tail)| tail)
    } else {
        tail
    };
    format!("{prefix}{tail}")
}

fn tab_labels(index: usize, tab_count: usize, directory: &[u8]) -> (String, String) {
    let path = Path::new(OsStr::from_bytes(directory));
    let leaf = if path == Path::new("/") {
        "/".into()
    } else {
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
    };
    let position = index + 1;
    let clean = |text: &str| {
        text.chars()
            .map(|c| if c.is_control() { '\u{fffd}' } else { c })
            .collect::<String>()
    };
    let full_path = clean(&path.as_os_str().to_string_lossy());
    (
        format!("{position}  {}", clean(&leaf)),
        format!("Tab {position} of {tab_count}  {full_path}"),
    )
}

pub(crate) fn tab_max_width(metrics: CellMetrics, viewport_width: f32) -> f32 {
    (metrics.font_size * 17.5).min((viewport_width - metrics.padding * 2.0 / 3.0).max(1.0))
}

/// One fixed action in the native Eon workspace header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceHeaderControl {
    NewTab,
    Shortcuts,
    CloseTab,
}

impl WorkspaceHeaderControl {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NewTab => "New tab — Alt+Shift+T",
            Self::Shortcuts => "Keyboard shortcuts — Alt+/",
            Self::CloseTab => "Close tab — Alt+Shift+W",
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuotaProvider {
    Codex,
    Claude,
}

impl QuotaProvider {
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
    pub(crate) rect: SceneRect,
    pub(crate) logo: SceneRect,
    pub(crate) label: String,
    pub(crate) accessible_label: String,
    pub(crate) description: String,
}

/// One immutable supervising-product label in the workspace header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceVersion {
    pub(crate) rect: SceneRect,
    pub(crate) label: String,
    pub(crate) description: String,
}

/// Native workspace target at one physical point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceHit<'a> {
    Tab(&'a str),
    Quota(usize),
    Version,
    Control(WorkspaceHeaderControl),
    Drag,
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
        let height = Self::height(metrics).min((bottom - header_heights(metrics).0).max(0.0));
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

/// Bounded geometry for the one native shortcut dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutViewerScene {
    pub bounds: SceneRect,
    pub(crate) title: SceneRect,
    pub(crate) subtitle: SceneRect,
    pub content: SceneRect,
    pub groups: Vec<ShortcutGroup>,
    pub scroll: f32,
    pub max_scroll: f32,
}

impl ShortcutViewerScene {
    #[must_use]
    pub fn new(
        mut groups: Vec<ShortcutGroup>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        scroll: f32,
    ) -> Self {
        let width = size.width as f32;
        let height = size.height as f32;
        let margin = metrics.padding.min(width / 12.0).min(height / 12.0);
        let dialog_width = (metrics.font_size * 54.0).min(width - margin * 2.0);
        let bounds = SceneRect {
            left: ((width - dialog_width) / 2.0).max(0.0),
            top: margin.max(0.0),
            width: dialog_width,
            height: height - margin * 2.0,
        };
        let padding = metrics.padding.min(bounds.width / 8.0);
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
        let footer_height = (metrics.height * 1.75).min(bounds.height / 3.0);
        let content_top =
            (subtitle.bottom() + metrics.height * 0.5).min(bounds.bottom() - footer_height);
        let content = SceneRect {
            top: content_top,
            height: bounds.bottom() - footer_height - content_top,
            ..title
        };
        let heading_height = metrics.height * 1.4;
        let row_height = metrics.height * 2.25;
        let group_gap = metrics.height * 0.6;
        let content_height = groups.iter().fold(0.0, |height, group| {
            height + heading_height + row_height * group.rows.len() as f32 + group_gap
        });
        let max_scroll = (content_height - content.height).max(0.0);
        let scroll = scroll.clamp(0.0, max_scroll);
        let mut top = content.top - scroll;
        for group in &mut groups {
            group.heading = SceneRect {
                left: content.left,
                top,
                width: content.width,
                height: heading_height,
            };
            top += heading_height;
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
            bounds,
            title,
            subtitle,
            content,
            groups,
            scroll,
            max_scroll,
        }
    }
}

/// Deterministic native projection of one complete accepted Eon snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceScene {
    pub tabs: Vec<WorkspaceTab>,
    pub(crate) header: SceneRect,
    pub(crate) drag_region: SceneRect,
    pub(crate) quotas: Vec<WorkspaceQuota>,
    pub(crate) version: Option<WorkspaceVersion>,
    pub(crate) controls: [WorkspaceControl; 3],
    pub hints: Option<ShortcutHintsScene>,
    /// The selected column's headers and terminal; `other_columns` holds the rest.
    pub panes: Vec<WorkspacePane>,
    pub terminal: SceneRect,
    pub selected_column: usize,
    pub other_columns: Vec<WorkspaceColumn>,
    pub tab_viewport: SceneRect,
    pub pane_viewport: SceneRect,
    pub(crate) chrome: SceneRect,
    sheet: SceneRect,
    tab_scroll: f32,
    pane_scroll: f32,
    tab_scroll_limit: f32,
    pane_scroll_limit: f32,
    active_tab_scroll: f32,
    selected_pane_scroll: f32,
    popup_label: Option<String>,
}

/// Width of each of `count` equal columns across `width`; the last column also
/// takes the remainder, so this is the narrowest.
pub fn workspace_column_width(width: f32, count: usize) -> f32 {
    (width / count as f32).floor()
}

fn header_heights(metrics: CellMetrics) -> (f32, f32) {
    (
        (metrics.height * 1.75).round().max(1.0),
        (metrics.height * 1.5).round().max(1.0),
    )
}

fn tab_gap(metrics: CellMetrics, tab_height: f32) -> f32 {
    (metrics.padding / 3.0).min(tab_height / 4.0)
}

fn workspace_header_geometry(
    size: PhysicalSize<u32>,
    metrics: CellMetrics,
    requested_quota_width: Option<f32>,
    show_shortcuts: bool,
) -> (
    SceneRect,
    SceneRect,
    SceneRect,
    Option<SceneRect>,
    [WorkspaceControl; 3],
) {
    let width = size.width as f32;
    let tab_height = header_heights(metrics).0.min(size.height as f32);
    let identity_width =
        metrics.width + metrics.padding * 2.0 + tab_gap(metrics, tab_height) * 2.0 + 1.0;
    let minimum_tabs = (width * 0.4)
        .min(metrics.font_size * 4.0)
        .max(identity_width.min((width - metrics.width * 3.0).max(0.0)));
    let control_count = if show_shortcuts { 3.0 } else { 2.0 };
    let control_width = tab_height.min(((width - minimum_tabs) / control_count).max(0.0));
    let controls_width = control_width * control_count;
    let drag_width =
        (metrics.font_size * 4.0).min((width - controls_width - minimum_tabs).max(0.0));
    let quota_width = requested_quota_width
        .filter(|quota| {
            quota.is_finite()
                && *quota <= (width - controls_width - drag_width - minimum_tabs).max(0.0)
        })
        .unwrap_or(0.0);
    let tab_width = (width - controls_width - drag_width - quota_width).max(0.0);
    let header = SceneRect {
        width,
        height: tab_height,
        ..SceneRect::default()
    };
    let tab_viewport = SceneRect {
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
        };
        left += width;
        control
    });
    (header, tab_viewport, drag_region, quota, controls)
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

/// Insets chrome vertically only, so frames sit flush with the window's and columns' sides.
pub(crate) fn pane_chrome_rect(rect: SceneRect, metrics: CellMetrics) -> SceneRect {
    let inset = pane_inset(metrics);
    SceneRect {
        top: rect.top + inset / 2.0,
        height: (rect.height - inset).max(0.0),
        ..rect
    }
}

/// Insets a rectangle's sides by the frameless header gap, which keeps neighbouring columns'
/// header fills apart when no seam divides them.
pub(crate) fn inset_sides(rect: SceneRect, metrics: CellMetrics) -> SceneRect {
    let inset = pane_inset(metrics).min(rect.width / 4.0);
    SceneRect {
        left: rect.left + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        ..rect
    }
}

/// Widens a column's chrome past each interior seam by `radius`, so a rounded shape clipped to
/// the column keeps rounded corners only at the sheet's outer edges.
pub(crate) fn seam_extended(column: SceneRect, sheet: SceneRect, radius: f32) -> SceneRect {
    let left = if column.left > sheet.left {
        column.left - radius
    } else {
        column.left
    };
    let right = if column.right() < sheet.right() {
        column.right() + radius
    } else {
        column.right()
    };
    SceneRect {
        left,
        width: right - left,
        ..column
    }
}

impl WorkspaceScene {
    /// Maximum shaped label width inside this surface's tab viewport.
    #[must_use]
    pub(crate) fn tab_text_width(size: PhysicalSize<u32>, metrics: CellMetrics) -> f32 {
        let (_, tab_viewport, _, _, _) = workspace_header_geometry(size, metrics, None, true);
        (tab_max_width(metrics, tab_viewport.width) - metrics.padding * 2.0)
            .max(0.0)
            .floor()
    }

    /// Extra pixels around an initially requested terminal grid.
    #[must_use]
    pub fn initial_overhead(snapshot: &Snapshot, metrics: CellMetrics) -> (f32, f32) {
        let (tab_height, pane_height) = header_heights(metrics);
        if active_popup(snapshot).is_some() {
            (
                snapshot.geometry.side_margin * metrics.scale * 2.0,
                ShortcutHintsScene::reserved_height(metrics)
                    + tab_height
                    + snapshot.geometry.vertical_margin * metrics.scale * 2.0
                    + popup_title_gutter(metrics),
            )
        } else {
            let tab = snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == snapshot.active_tab)
                .expect("EONW validates the active tab");
            (
                0.0,
                ShortcutHintsScene::reserved_height(metrics)
                    + tab_height
                    + pane_height * presented_panes(tab).len() as f32
                    + if presented_panes(tab).is_empty() {
                        0.0
                    } else {
                        stack_inset(metrics)
                    },
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
        header_text: impl FnMut(Option<&str>, &str) -> (String, f32),
    ) -> Self {
        Self::from_snapshot_with_metadata(
            snapshot,
            None,
            size,
            metrics,
            tab_scroll,
            pane_scroll,
            |_| None,
            header_text,
            None,
        )
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_snapshot_with_metadata<'a>(
        snapshot: &Snapshot,
        eon_version: Option<&str>,
        size: PhysicalSize<u32>,
        metrics: CellMetrics,
        tab_scroll: f32,
        pane_scroll: f32,
        metadata: impl Fn(&[u8]) -> Option<&'a PaneMetadata>,
        mut header_text: impl FnMut(Option<&str>, &str) -> (String, f32),
        hints: Option<ShortcutHintsScene>,
    ) -> Self {
        let width = size.width as f32;
        let height = size.height as f32;
        let (tab_height, pane_height) = header_heights(metrics);
        let tab_height = tab_height.min(height);
        let show_shortcuts = hints.as_ref().is_none_or(|hints| hints.keep_shortcuts);
        let content_height = hints.as_ref().map_or(height, |hints| hints.rect.top);
        let gap = tab_gap(metrics, tab_height);
        let quota_texts = snapshot
            .codex_quota
            .as_ref()
            .map(|quota| {
                (
                    QuotaProvider::Codex,
                    quota_text(
                        QuotaProvider::Codex,
                        quota.state,
                        quota.observed_at,
                        &quota.windows,
                    ),
                )
            })
            .into_iter()
            .chain(snapshot.claude_quota.as_ref().map(|quota| {
                (
                    QuotaProvider::Claude,
                    quota_text(
                        QuotaProvider::Claude,
                        match quota.state {
                            ClaudeQuotaState::Fresh => CodexQuotaState::Fresh,
                            ClaudeQuotaState::Stale => CodexQuotaState::Stale,
                        },
                        quota.observed_at,
                        &quota.windows,
                    ),
                )
            }))
            .collect::<Vec<_>>();
        let quota_padding = metrics.padding * 2.0 + metrics.font_size + metrics.padding / 2.0;
        let mut measure_quotas = |compact| {
            quota_texts
                .iter()
                .map(|(provider, (wide, short, description))| {
                    let (label, width) = header_text(None, if compact { short } else { wide });
                    WorkspaceQuota {
                        provider: *provider,
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
            let geometry = workspace_header_geometry(size, metrics, requested, show_shortcuts);
            if geometry.3.is_some() || quotas.is_empty() {
                break geometry;
            }
            if compact {
                quotas.pop();
            } else {
                quotas = measure_quotas(true);
                compact = true;
            }
        };
        let (header, mut tab_viewport, mut drag_region, quota_rect, mut controls) = geometry;
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
        let pane_viewport = SceneRect {
            left: 0.0,
            top: tab_height,
            width,
            height: (content_height - tab_height).max(0.0),
        };
        let active_tab = snapshot
            .tabs
            .iter()
            .position(|tab| tab.id == snapshot.active_tab)
            .expect("EONW validates the active tab");
        let mut tab_end = gap;
        let mut tabs: Vec<_> = snapshot
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let (label, accessible_label) =
                    tab_labels(index, snapshot.tabs.len(), &tab.directory);
                let (label, text_width) = header_text(Some(&tab.id), &label);
                let tab_width = (text_width.ceil() + metrics.padding * 2.0)
                    .clamp((metrics.font_size * 4.0).min(max_tab_width), max_tab_width);
                let left = tab_end;
                tab_end += tab_width + gap;
                WorkspaceTab {
                    id: tab.id.clone(),
                    selected: index == active_tab,
                    rect: SceneRect {
                        left,
                        top: gap,
                        width: tab_width,
                        height: (tab_height - gap * 2.0).max(0.0),
                    },
                    label,
                    accessible_label,
                }
            })
            .collect();
        tab_viewport.width = tab_viewport.width.min(tab_end);
        controls[0].rect.left = tab_viewport.right();
        drag_region.left = controls[0].rect.right();
        drag_region.width = (quotas
            .first()
            .map_or(controls[1].rect.left, |quota| quota.rect.left)
            - drag_region.left)
            .max(0.0);
        let version = eon_version.and_then(|version| {
            let label = format!("EON {version}");
            // Launch validation bounds identity to ASCII; rendering uses terminal cell spacing.
            let width = label.len() as f32 * metrics.width + metrics.padding * 2.0;
            // Informational identity uses spare header space without shortening tabs.
            if !width.is_finite()
                || width <= 0.0
                || width + metrics.font_size * 4.0 > drag_region.width
            {
                return None;
            }
            let right = controls[1].rect.left;
            drag_region.width -= width;
            for quota in &mut quotas {
                quota.rect.left -= width;
                quota.logo.left -= width;
            }
            Some(WorkspaceVersion {
                rect: SceneRect {
                    left: right - width,
                    width,
                    height: tab_height,
                    top: 0.0,
                },
                label,
                description: format!("Running Eon version {version}"),
            })
        });
        let tab_scroll_limit = (tab_end - tab_viewport.width).max(0.0);
        let active = tabs[active_tab].rect;
        let active_tab_scroll = (active.left - (tab_viewport.width - active.width).max(0.0) / 2.0)
            .clamp(0.0, tab_scroll_limit);
        let tab_scroll = if tab_scroll.is_finite() {
            tab_scroll.clamp(0.0, tab_scroll_limit)
        } else {
            0.0
        };
        for tab in &mut tabs {
            tab.rect.left -= tab_scroll;
        }
        let active = &snapshot.tabs[active_tab];
        let popup = active_popup(snapshot);
        let stack_viewport = SceneRect {
            height: (pane_viewport.height - stack_inset(metrics)).max(0.0),
            ..pane_viewport
        };
        let chrome = pane_chrome_rect(stack_viewport, metrics);
        let sheet = SceneRect {
            top: chrome.top,
            height: (height - chrome.top).max(0.0),
            ..pane_viewport
        };
        if popup.is_some() || active.columns.is_empty() {
            let horizontal_inset = (snapshot.geometry.side_margin * metrics.scale)
                .min((pane_viewport.width - metrics.padding * 2.0 - metrics.width).max(0.0) / 2.0);
            let vertical_inset = (snapshot.geometry.vertical_margin * metrics.scale).min(
                (pane_viewport.height - metrics.padding * 2.0 - metrics.height).max(0.0) / 2.0,
            );
            let title_gutter = if popup.is_some() {
                popup_title_gutter(metrics).min(
                    (pane_viewport.height
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
                header,
                drag_region,
                quotas,
                version,
                controls,
                hints,
                panes: Vec::new(),
                selected_column: 0,
                other_columns: Vec::new(),
                terminal: SceneRect {
                    left: horizontal_inset,
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
                chrome,
                sheet,
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
            };
        }

        let pane_viewport = stack_viewport;
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
        let column_width = workspace_column_width(width, count);
        let maximum_terminal_height = (pane_viewport.height - pane_height).max(0.0);
        let minimum_terminal_height =
            (metrics.padding * 2.0 + metrics.height).min(maximum_terminal_height);
        let mut selected_layout = None;
        let mut other_columns = Vec::with_capacity(count - 1);
        for (index, column) in active.columns.iter().enumerate() {
            let left = index as f32 * column_width;
            let column_width = if index + 1 == count {
                width - left
            } else {
                column_width
            };
            let expanded = column
                .panes
                .iter()
                .position(|pane| pane.id == column.expanded)
                .expect("EONW validates the expanded pane");
            let pane_headers_height = column.panes.len() as f32 * pane_height;
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
                .enumerate()
                .map(|(position, pane)| {
                    let label = metadata(&pane.endpoint).map_or_else(
                        || pane_label(&pane.id, pane.live, &PaneMetadata::Unavailable),
                        |metadata| pane_label(&pane.id, pane.live, metadata),
                    );
                    WorkspacePane {
                        id: pane.id.clone(),
                        live: pane.live,
                        selected: position == expanded,
                        rect: SceneRect {
                            left,
                            top: tab_height
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
                top: tab_height + (expanded + 1) as f32 * pane_height - column_scroll,
                width: column_width,
                height: terminal_height,
            };
            let chrome = pane_chrome_rect(
                SceneRect {
                    left,
                    width: column_width,
                    ..stack_viewport
                },
                metrics,
            );
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
            header,
            drag_region,
            quotas,
            version,
            controls,
            hints,
            panes,
            terminal,
            selected_column,
            other_columns,
            tab_viewport,
            pane_viewport,
            chrome,
            sheet,
            tab_scroll,
            pane_scroll,
            tab_scroll_limit,
            pane_scroll_limit,
            active_tab_scroll,
            selected_pane_scroll,
            popup_label: None,
        }
    }

    /// The workspace sheet: the full width, from the stack chrome's top to the window's bottom
    /// edge, through the footer when hints show.
    pub(crate) fn sheet(&self) -> SceneRect {
        self.sheet
    }

    /// Every column's headers left to right, each flagged when it is the selected column.
    pub fn column_panes(&self) -> impl Iterator<Item = (&[WorkspacePane], bool)> {
        let selected = (!self.panes.is_empty()).then_some(self.selected_column);
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
        if self.drag_region.contains(x, y) {
            return Some(WorkspaceHit::Drag);
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

    #[must_use]
    pub fn has_control(&self, kind: WorkspaceHeaderControl) -> bool {
        self.controls
            .iter()
            .any(|control| control.kind == kind && control.rect.width > 0.0)
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
    pub kitty_virtual_placeholder: bool,
    pub cells: Vec<DrawCell>,
}

/// A contiguous visible row span, borrowing Orbit's exact target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hyperlink<'a> {
    pub row: u16,
    pub column: u16,
    pub columns: u16,
    pub uri: &'a str,
}

impl Hyperlink<'_> {
    #[must_use]
    pub fn rect(self, origin: SceneRect, metrics: CellMetrics) -> SceneRect {
        SceneRect {
            left: origin.left + metrics.padding + f32::from(self.column) * metrics.width,
            top: origin.top + metrics.padding + f32::from(self.row) * metrics.height,
            width: f32::from(self.columns) * metrics.width,
            height: metrics.height,
        }
    }

    #[must_use]
    pub fn contains(self, row: u16, column: u16) -> bool {
        self.row == row && (self.column..self.column + self.columns).contains(&column)
    }
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
                    uri,
                });
            }
            None
        })
    }

    pub(crate) fn from_protocol(row: &Row, frame: &Frame) -> Self {
        Self {
            wrapped: row.wrapped,
            wrap_continuation: row.wrap_continuation,
            kitty_virtual_placeholder: row.kitty_virtual_placeholder,
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub fn hyperlinks(&self) -> impl Iterator<Item = Hyperlink<'_>> {
        self.content
            .iter()
            .enumerate()
            .flat_map(|(row, content)| content.hyperlinks(row as u16))
    }

    #[must_use]
    pub fn hyperlink_at(&self, row: u16, column: u16) -> Option<Hyperlink<'_>> {
        self.content
            .get(usize::from(row))?
            .hyperlinks(row)
            .find(|link| link.contains(row, column))
    }

    /// Materialize a validated Orbit frame without retaining a second wire schema.
    #[must_use]
    pub fn from_frame(frame: &Frame) -> Self {
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
    fn popup_outer_chrome_reuses_the_pane_stack_rectangle() {
        use eon_workspace_protocol::v10::{
            ALT, Pane, Popup, PopupEntry, PopupGeometry, Shortcut, Tab,
        };

        let mut snapshot = Snapshot {
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "agent".into(),
                label: "Agent".into(),
                shortcut: Shortcut {
                    modifiers: ALT,
                    key: "KeyA".into(),
                },
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/work".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: vec![eon_workspace_protocol::v10::Column {
                    panes: vec![Pane {
                        id: "p1".into(),
                        session: "session-1".into(),
                        endpoint: b"/run/pane.sock".to_vec(),
                        live: true,
                    }],
                    expanded: "p1".into(),
                }],
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "agent".into(),
                    session: "session-2".into(),
                    endpoint: b"/run/popup.sock".to_vec(),
                }],
            }],
            codex_quota: None,
            claude_quota: None,
        };
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
        // The sheet's sides sit on the window's side edges.
        assert_eq!((stack.chrome.left, stack.chrome.right()), (0.0, 960.0));
        assert_ne!(popup.terminal, popup.chrome);
        let size = PhysicalSize::new(960, 600);
        let metrics = CellMetrics::for_scale(1.0);
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
                let with_hints = WorkspaceScene::from_snapshot_with_metadata(
                    &snapshot,
                    None,
                    size,
                    metrics,
                    0.0,
                    0.0,
                    |_| None,
                    |_, text| (text.into(), text.len() as f32 * 10.0),
                    Some(footer),
                );
                assert_eq!(
                    baseline.terminal.height - with_hints.terminal.height,
                    footer_rect.height + size.height as f32 - footer_rect.bottom()
                );
                assert_eq!(
                    baseline.pane_viewport.bottom() - with_hints.pane_viewport.bottom(),
                    footer_rect.height + size.height as f32 - footer_rect.bottom()
                );
                assert_eq!(footer_rect.left, with_hints.chrome.left);
                assert_eq!(footer_rect.right(), with_hints.chrome.right());
                // The footer and the hidden-footer sheet both end on the window's bottom edge.
                assert_eq!(footer_rect.bottom(), size.height as f32);
                assert_eq!(baseline.sheet().bottom(), size.height as f32);
                assert_eq!(with_hints.sheet().bottom(), size.height as f32);
                assert!(with_hints.pane_viewport.bottom() <= footer_rect.top);
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
            "lower corners must fit below the separator"
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
        use eon_workspace_protocol::v10::{Pane, PopupGeometry, Tab};

        let snapshot = Snapshot {
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
                    columns: vec![eon_workspace_protocol::v10::Column {
                        panes: vec![Pane {
                            id: format!("p{index}"),
                            session: format!("s{index}"),
                            endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                            live: true,
                        }],
                        expanded: format!("p{index}"),
                    }],
                    popups: Vec::new(),
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
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

    #[test]
    fn pane_metadata_label_is_bounded_safe_and_has_honest_fallbacks() {
        let metadata = PaneMetadata::Available {
            working_directory: format!("file:///tmp/{}", "eon".repeat(30)),
        };
        let label = pane_label("p1", true, &metadata);

        assert_eq!(label, format!("p1  …/{}", "eon".repeat(26)));
        assert!(!label.contains("file://"));
        assert_eq!(label.chars().count(), 84);
        assert_eq!(
            compact_working_directory("file:///tmp/eon", Some(Path::new(""))),
            "/tmp/eon"
        );
        let home = std::env::var("HOME").expect("HOME is required by Venus");
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: format!("file://localhost{home}/pjs/yazelix-dir/eon"),
                },
            ),
            "p1  ~/pjs/yazelix-dir/eon"
        );
        let long_home_label = pane_label(
            "p1",
            true,
            &PaneMetadata::Available {
                working_directory: format!("{home}/{}/eon-desktop", "parent/".repeat(20)),
            },
        );
        assert!(long_home_label.starts_with("p1  ~/…/"));
        assert!(long_home_label.ends_with("/eon-desktop"));
        assert!(!long_home_label.contains(&home));
        assert!(long_home_label.chars().count() <= 84);
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: "file://server/share".into(),
                },
            ),
            "p1  /share"
        );
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: "file:///".into(),
                },
            ),
            "p1  /"
        );
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: format!("file://host{home}"),
                },
            ),
            "p1  ~"
        );
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: "file:///tmp/a\nb".into(),
                },
            ),
            "p1  /tmp/a�b"
        );
        assert_eq!(
            pane_label("p1", true, &PaneMetadata::Connecting),
            "p1 connecting"
        );
        assert_eq!(
            pane_label("p1", true, &PaneMetadata::Unavailable),
            "p1 unavailable"
        );
        assert_eq!(
            pane_label(
                "p1",
                true,
                &PaneMetadata::Available {
                    working_directory: String::new(),
                },
            ),
            "p1"
        );
        assert_eq!(pane_label("p1", false, &metadata), "p1 offline");
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
            kitty_virtual_placeholder: false,
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

    #[test]
    fn glyph_runs_preserve_authoritative_cell_starts() {
        let scene = Scene {
            revision: 1,
            columns: 10,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
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
                    kitty_virtual_placeholder: false,
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
                    kitty_virtual_placeholder: false,
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
            background: Color::default(),
            foreground: Color::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
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
                let start = ShortcutViewerScene::new(groups.clone(), size, metrics, -100.0);
                let end = ShortcutViewerScene::new(groups.clone(), size, metrics, f32::MAX);

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
        let tiny = ShortcutViewerScene::new(Vec::new(), PhysicalSize::new(1, 1), metrics, 0.0);
        assert!(tiny.bounds.right() <= 1.0 && tiny.bounds.bottom() <= 1.0);
        assert!(
            tiny.content.right() <= tiny.bounds.right()
                && tiny.content.bottom() <= tiny.bounds.bottom()
        );
        assert!(tiny.content.height >= 0.0);
    }
}
