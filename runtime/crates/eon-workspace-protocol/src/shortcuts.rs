//! Eon's shared shortcut descriptors. This is Rust data, not an EONW codec.

use crate::v44::{PopupEntry, PopupKey, Shortcut as ProtocolShortcut, Sidebar};

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;
const ALT_SHIFT: u8 = ALT | SHIFT;
const CTRL_ALT: u8 = CTRL | ALT;
const CTRL_SHIFT: u8 = CTRL | SHIFT;
const CTRL_ALT_SHIFT: u8 = CTRL | ALT | SHIFT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
}

impl Platform {
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Macos
    } else {
        Self::Linux
    };

    /// Lowercase wire/CLI platform name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleViewer,
    ToggleInbox,
    ToggleHints,
    ToggleSidebar,
    NextAttention,
    PreviousAttention,
    ToggleZoom,
    NewWindow,
    Detach,
    Quit,
    FocusTabPosition,
    ShowWorkspacePosition,
    NextTab,
    PreviousTab,
    Focus(Direction),
    Move(Direction),
    MovePaneSideways(Side),
    CreatePane,
    CreateColumn,
    CreateTab,
    CloseTab,
    ClosePane,
    CycleFocus,
    ReturnToTerminal,
    TraverseChrome(Direction),
    Copy,
    Paste,
    OpenLink,
}

impl Action {
    /// Stable lowercase ASCII `snake_case` action identifier (`EON-C10`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::ToggleViewer => "toggle_viewer",
            Self::ToggleInbox => "toggle_inbox",
            Self::ToggleHints => "toggle_hints",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::NextAttention => "next_attention",
            Self::PreviousAttention => "previous_attention",
            Self::ToggleZoom => "toggle_zoom",
            Self::NewWindow => "new_window",
            Self::Detach => "detach",
            Self::Quit => "quit",
            Self::FocusTabPosition => "focus_tab_position",
            Self::ShowWorkspacePosition => "show_workspace_position",
            Self::NextTab => "next_tab",
            Self::PreviousTab => "previous_tab",
            Self::Focus(Direction::Left) => "focus_left",
            Self::Focus(Direction::Right) => "focus_right",
            Self::Focus(Direction::Up) => "focus_up",
            Self::Focus(Direction::Down) => "focus_down",
            Self::Move(Direction::Left) => "move_tab_left",
            Self::Move(Direction::Right) => "move_tab_right",
            Self::Move(Direction::Up) => "move_pane_up",
            Self::Move(Direction::Down) => "move_pane_down",
            Self::MovePaneSideways(Side::Left) => "move_pane_left",
            Self::MovePaneSideways(Side::Right) => "move_pane_right",
            Self::CreatePane => "create_pane",
            Self::CreateColumn => "create_column",
            Self::CreateTab => "create_tab",
            Self::CloseTab => "close_tab",
            Self::ClosePane => "close_pane",
            Self::CycleFocus => "cycle_focus",
            Self::ReturnToTerminal => "return_to_terminal",
            Self::TraverseChrome(Direction::Left) => "traverse_chrome_left",
            Self::TraverseChrome(Direction::Right) => "traverse_chrome_right",
            Self::TraverseChrome(Direction::Up) => "traverse_chrome_up",
            Self::TraverseChrome(Direction::Down) => "traverse_chrome_down",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::OpenLink => "open_link",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: &'static str,
    pub modifiers: u8,
}

impl Chord {
    const fn new(key: &'static str, modifiers: u8) -> Self {
        Self { key, modifiers }
    }

    fn matches(self, key: &str, modifiers: u8) -> bool {
        // The dedicated native Paste key works with every modifier state.
        self.key == key && (self.key == "Paste" || self.modifiers == modifiers)
    }

    pub fn display(self, platform: Platform) -> String {
        display_chord(platform, self.modifiers, self.key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(&'static str, u8),
    AltDigits,
    AltShiftDigits,
    Copy,
    Paste,
    OpenLink,
    Quit,
}

impl Trigger {
    pub fn primary_chord(self, platform: Platform) -> Chord {
        match self {
            Self::Key(key, modifiers) => Chord::new(key, modifiers),
            Self::AltDigits => Chord::new("Digit1", ALT),
            Self::AltShiftDigits => Chord::new("Digit1", ALT_SHIFT),
            Self::Copy | Self::Paste => Chord::new(
                if self == Self::Copy { "KeyC" } else { "KeyV" },
                if platform == Platform::Macos {
                    SUPER
                } else {
                    CTRL_SHIFT
                },
            ),
            Self::OpenLink => Chord::new(
                "click",
                if platform == Platform::Macos {
                    SUPER
                } else {
                    CTRL
                },
            ),
            Self::Quit => Chord::new("KeyQ", ALT_SHIFT),
        }
    }

    pub fn secondary_chord(self, platform: Platform) -> Option<Chord> {
        match self {
            Self::Paste => Some(Chord::new("Paste", 0)),
            Self::Quit if platform == Platform::Macos => Some(Chord::new("KeyQ", SUPER)),
            _ => None,
        }
    }

    pub fn chords(self, platform: Platform) -> Vec<Chord> {
        let primary = self.primary_chord(platform);
        if matches!(self, Self::AltDigits | Self::AltShiftDigits) {
            DIGIT_KEYS
                .iter()
                .map(|key| Chord::new(key, primary.modifiers))
                .collect()
        } else {
            std::iter::once(primary)
                .chain(self.secondary_chord(platform))
                .collect()
        }
    }

    pub fn matches_key(self, platform: Platform, key: &str, modifiers: u8) -> bool {
        if self == Self::OpenLink {
            return false;
        }
        let primary = self.primary_chord(platform);
        if matches!(self, Self::AltDigits | Self::AltShiftDigits) {
            modifiers == primary.modifiers && digit_position(key).is_some()
        } else {
            primary.matches(key, modifiers)
                || self
                    .secondary_chord(platform)
                    .is_some_and(|chord| chord.matches(key, modifiers))
        }
    }

    pub fn display_label_for(self, platform: Platform) -> String {
        let primary = self.primary_chord(platform);
        if matches!(self, Self::AltDigits | Self::AltShiftDigits) {
            format!(
                "{}…9 / {}",
                primary.display(platform),
                display_chord(platform, primary.modifiers, "Digit0")
            )
        } else {
            std::iter::once(primary)
                .chain(self.secondary_chord(platform))
                .map(|chord| chord.display(platform))
                .collect::<Vec<_>>()
                .join(" or ")
        }
    }
}

/// The digit keys in position order, with zero naming the tenth position.
pub const DIGIT_KEYS: [&str; 10] = [
    "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9",
    "Digit0",
];

pub fn digit_position(key: &str) -> Option<usize> {
    DIGIT_KEYS.iter().position(|digit| *digit == key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupPolicy {
    Overridable,
    /// A configured collision fails before workspace mutation.
    Reserved,
    /// A configured collision warns/unbinds; a product default collision fails.
    Focus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub group: &'static str,
    pub label: &'static str,
    pub trigger: Trigger,
    pub action: Action,
}

impl Shortcut {
    const fn key(
        group: &'static str,
        label: &'static str,
        key: &'static str,
        modifiers: u8,
        action: Action,
    ) -> Self {
        Self {
            group,
            label,
            trigger: Trigger::Key(key, modifiers),
            action,
        }
    }

    pub fn popup_policy(self) -> PopupPolicy {
        match self.action {
            Action::ToggleViewer | Action::FocusTabPosition => PopupPolicy::Reserved,
            Action::Focus(_) => PopupPolicy::Focus,
            _ => PopupPolicy::Overridable,
        }
    }

    /// Whether this shortcut can fire in a window with `sidebar` and `running` state.
    pub fn bound_for_window(self, sidebar: Sidebar, running: bool) -> bool {
        match self.action {
            Action::ToggleSidebar => sidebar != Sidebar::Off,
            Action::Quit => running,
            _ => true,
        }
    }

    /// Whether this shortcut is shown and active given `sidebar`, `running`, and `number_keys`.
    pub fn applicable(self, sidebar: Sidebar, running: bool, number_keys: bool) -> bool {
        self.bound_for_window(sidebar, running)
            && (number_keys || self.action != Action::ShowWorkspacePosition)
    }

    /// Whether `chord` can be overridden by a configured popup keybinding.
    pub fn is_overridable_chord(self, chord: Chord) -> bool {
        self.popup_policy() == PopupPolicy::Overridable && !matches!(chord.key, "click" | "Paste")
    }

    /// Configured popup overriding one specific chord of this shortcut, if any.
    pub fn chord_override(self, chord: Chord, entries: &[PopupEntry]) -> Option<&PopupEntry> {
        self.is_overridable_chord(chord)
            .then(|| popup_entry_for_chord(entries, chord.key, chord.modifiers))
            .flatten()
    }

    /// Chords of this shortcut that configured popups may override, in trigger order.
    fn overridable_chords(self, platform: Platform) -> Vec<Chord> {
        self.trigger
            .chords(platform)
            .into_iter()
            .filter(|&chord| self.is_overridable_chord(chord))
            .collect()
    }

    /// Popup overriding a shortcut with a single overridable chord.
    pub fn overriding_popup(
        self,
        platform: Platform,
        entries: &[PopupEntry],
    ) -> Option<&PopupEntry> {
        let chords = self.overridable_chords(platform);
        match chords.as_slice() {
            &[chord] => self.chord_override(chord, entries),
            _ => None,
        }
    }

    /// Individual multi-chord overridable trigger keys overridden by popups.
    pub fn taken_chords(
        self,
        platform: Platform,
        entries: &[PopupEntry],
    ) -> Vec<(Chord, &PopupEntry)> {
        let chords = self.overridable_chords(platform);
        if chords.len() <= 1 {
            return Vec::new();
        }
        chords
            .into_iter()
            .filter_map(|chord| Some((chord, self.chord_override(chord, entries)?)))
            .collect()
    }
}

/// First bound popup matching `(key, modifiers)`.
pub fn popup_entry_for_chord<'a>(
    entries: &'a [PopupEntry],
    key: &str,
    modifiers: u8,
) -> Option<&'a PopupEntry> {
    entries.iter().find(|entry| {
        entry
            .shortcut
            .bound()
            .is_some_and(|shortcut| shortcut.modifiers == modifiers && shortcut.key == key)
    })
}

/// Config spelling for `(modifiers, key)` when `v44::Shortcut` accepts it, else `None`.
pub fn config_chord_if_valid(modifiers: u8, key: &str) -> Option<String> {
    ProtocolShortcut {
        modifiers,
        key: key.to_owned(),
    }
    .validate()
    .ok()
    .map(|()| config_chord(modifiers, key))
}

pub fn shortcut(action: Action) -> &'static Shortcut {
    FIXED_SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == action)
        .expect("every action has a descriptor")
}

pub fn popup_policy(modifiers: u8, key: &str) -> PopupPolicy {
    FIXED_SHORTCUTS
        .iter()
        .find(|shortcut| {
            shortcut.popup_policy() != PopupPolicy::Overridable
                && shortcut
                    .trigger
                    .matches_key(Platform::CURRENT, key, modifiers)
        })
        .map_or(PopupPolicy::Overridable, |shortcut| shortcut.popup_policy())
}

/// Opposite chrome arrows share a viewer row until a popup overrides either key.
pub const PAIRED_ROWS: [(Action, Action, &str); 2] = [
    (
        Action::TraverseChrome(Direction::Left),
        Action::TraverseChrome(Direction::Right),
        "Previous or next column or tab while tabs are focused",
    ),
    (
        Action::TraverseChrome(Direction::Up),
        Action::TraverseChrome(Direction::Down),
        "Previous or next pane or agent row while one is focused",
    ),
];

/// Config syntax keeps named punctuation (for example Alt+Slash).
pub fn config_chord(modifiers: u8, key: &str) -> String {
    let key = key
        .strip_prefix("Key")
        .or_else(|| key.strip_prefix("Digit"))
        .unwrap_or(key);
    let modifiers = modifier_names(Platform::Linux, modifiers);
    if modifiers.is_empty() {
        key.into()
    } else {
        format!("{modifiers}+{key}")
    }
}

pub fn display_chord(platform: Platform, modifiers: u8, key: &str) -> String {
    let modifiers = modifier_names(platform, modifiers);
    let key = key_label(key);
    if modifiers.is_empty() {
        key.into()
    } else {
        format!("{modifiers}+{key}")
    }
}

pub fn modifier_names(platform: Platform, modifiers: u8) -> String {
    let names = match platform {
        Platform::Linux => ["Ctrl", "Alt", "Shift", "Super"],
        Platform::Macos => ["Control", "Option", "Shift", "Command"],
    };
    [CTRL, ALT, SHIFT, SUPER]
        .into_iter()
        .zip(names)
        .filter_map(|(bit, name)| (modifiers & bit != 0).then_some(name))
        .collect::<Vec<_>>()
        .join("+")
}

pub fn key_label(key: &str) -> &str {
    match key {
        key if key.starts_with("Key") => &key[3..],
        key if key.starts_with("Digit") => &key[5..],
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "Backquote" => "`",
        "Backslash" => "\\",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Comma" => ",",
        "Equal" => "=",
        "Minus" => "-",
        "Period" => ".",
        "Quote" => "'",
        "Semicolon" => ";",
        "Slash" => "/",
        key => key,
    }
}

pub const GROUPS: [&str; 3] = ["Navigate", "Tabs and panes", "Host"];

pub static FIXED_SHORTCUTS: &[Shortcut] = &[
    Shortcut::key("Navigate", "Focus next tab", "Tab", CTRL, Action::NextTab),
    Shortcut::key(
        "Navigate",
        "Focus previous tab",
        "Tab",
        CTRL_SHIFT,
        Action::PreviousTab,
    ),
    Shortcut::key(
        "Navigate",
        "Focus left column or previous tab",
        "KeyH",
        ALT,
        Action::Focus(Direction::Left),
    ),
    Shortcut::key(
        "Navigate",
        "Focus right column or next tab",
        "KeyL",
        ALT,
        Action::Focus(Direction::Right),
    ),
    Shortcut::key(
        "Navigate",
        "Focus pane above",
        "KeyK",
        ALT,
        Action::Focus(Direction::Up),
    ),
    Shortcut::key(
        "Navigate",
        "Focus pane below",
        "KeyJ",
        ALT,
        Action::Focus(Direction::Down),
    ),
    // After the HJKL focus keys, so the merged focus hint ends with 1..9.
    Shortcut {
        group: "Navigate",
        label: "Focus tab by position",
        trigger: Trigger::AltDigits,
        action: Action::FocusTabPosition,
    },
    Shortcut {
        group: "Navigate",
        label: "Show session by position",
        trigger: Trigger::AltShiftDigits,
        action: Action::ShowWorkspacePosition,
    },
    Shortcut::key(
        "Navigate",
        "Jump to the next agent needing attention",
        "KeyG",
        ALT,
        Action::NextAttention,
    ),
    Shortcut::key(
        "Navigate",
        "Jump to the previous agent needing attention",
        "KeyG",
        ALT_SHIFT,
        Action::PreviousAttention,
    ),
    Shortcut::key(
        "Navigate",
        "Show the attention inbox",
        "KeyQ",
        ALT,
        Action::ToggleInbox,
    ),
    Shortcut::key(
        "Navigate",
        "Cycle terminal, agent sidebar, tabs, and panes",
        "F6",
        0,
        Action::CycleFocus,
    ),
    Shortcut::key(
        "Navigate",
        "Return focus to the terminal",
        "Escape",
        0,
        Action::ReturnToTerminal,
    ),
    Shortcut::key(
        "Navigate",
        "Left column or previous tab while tabs are focused",
        "ArrowLeft",
        0,
        Action::TraverseChrome(Direction::Left),
    ),
    Shortcut::key(
        "Navigate",
        "Right column or next tab while tabs are focused",
        "ArrowRight",
        0,
        Action::TraverseChrome(Direction::Right),
    ),
    Shortcut::key(
        "Navigate",
        "Previous pane or agent row while panes or the sidebar are focused",
        "ArrowUp",
        0,
        Action::TraverseChrome(Direction::Up),
    ),
    Shortcut::key(
        "Navigate",
        "Next pane or agent row while panes or the sidebar are focused",
        "ArrowDown",
        0,
        Action::TraverseChrome(Direction::Down),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Create pane",
        "KeyM",
        ALT,
        Action::CreatePane,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Create column",
        "KeyM",
        CTRL_ALT,
        Action::CreateColumn,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Create tab",
        "KeyT",
        ALT_SHIFT,
        Action::CreateTab,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Close active tab",
        "KeyW",
        ALT_SHIFT,
        Action::CloseTab,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Close selected pane",
        "KeyW",
        ALT,
        Action::ClosePane,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Zoom selected pane or restore columns",
        "KeyZ",
        ALT_SHIFT,
        Action::ToggleZoom,
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move tab left",
        "KeyH",
        CTRL_ALT_SHIFT,
        Action::Move(Direction::Left),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move tab right",
        "KeyL",
        CTRL_ALT_SHIFT,
        Action::Move(Direction::Right),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move pane to left column",
        "KeyH",
        CTRL_ALT,
        Action::MovePaneSideways(Side::Left),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move pane to right column",
        "KeyL",
        CTRL_ALT,
        Action::MovePaneSideways(Side::Right),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move pane up",
        "KeyK",
        CTRL_ALT,
        Action::Move(Direction::Up),
    ),
    Shortcut::key(
        "Tabs and panes",
        "Move pane down",
        "KeyJ",
        CTRL_ALT,
        Action::Move(Direction::Down),
    ),
    Shortcut::key(
        "Host",
        "Open new Eon window",
        "KeyN",
        ALT_SHIFT,
        Action::NewWindow,
    ),
    Shortcut::key(
        "Host",
        "Detach: hide the window, keep terminals running",
        "KeyD",
        ALT_SHIFT,
        Action::Detach,
    ),
    Shortcut {
        group: "Host",
        label: "Quit: close the window, stop its terminals",
        trigger: Trigger::Quit,
        action: Action::Quit,
    },
    Shortcut::key(
        "Host",
        "Show or close shortcuts",
        "Slash",
        ALT,
        Action::ToggleViewer,
    ),
    Shortcut::key(
        "Host",
        "Show or hide shortcut hints",
        "KeyB",
        ALT_SHIFT,
        Action::ToggleHints,
    ),
    Shortcut::key(
        "Host",
        "Show or hide the agent sidebar",
        "KeyH",
        ALT_SHIFT,
        Action::ToggleSidebar,
    ),
    Shortcut {
        group: "Host",
        label: "Copy selected text or focused link",
        trigger: Trigger::Copy,
        action: Action::Copy,
    },
    Shortcut {
        group: "Host",
        label: "Paste",
        trigger: Trigger::Paste,
        action: Action::Paste,
    },
    Shortcut {
        group: "Host",
        label: "Open link",
        trigger: Trigger::OpenLink,
        action: Action::OpenLink,
    },
];

/// One row of the `Alt+/` shortcuts viewer (`EON-C10`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerRow {
    pub shortcut: String,
    pub label: String,
}

/// One named section of the `Alt+/` shortcuts viewer (`EON-C10`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerGroup {
    pub title: String,
    pub rows: Vec<ViewerRow>,
}

/// One trigger chord with optional config spelling and platform display label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyChord {
    pub config: Option<String>,
    pub display: String,
}

/// Popup entry that overrides one chord of a fixed shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideSource {
    pub config: Option<String>,
    pub display: String,
    pub kind: &'static str,
    pub id: String,
    pub label: String,
}

/// Effective binding for one applicable fixed action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBinding {
    pub action: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub display: Option<String>,
    pub chords: Vec<KeyChord>,
    pub source: &'static str,
    pub overridden_by: Vec<OverrideSource>,
}

/// Effective state of one enabled popup entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectivePopup {
    pub id: String,
    pub label: String,
    pub keybinding: Option<String>,
    pub display: Option<String>,
    pub default_keybinding: Option<String>,
    pub default_display: Option<String>,
    pub overridden_by: Option<String>,
    pub overridden_by_label: Option<String>,
    pub source: &'static str,
}

/// One chord protected from popup overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedChord {
    pub action: &'static str,
    pub config: String,
    pub display: String,
}

/// Reserved and focus-policy fixed chords from the shared table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedChords {
    pub strict: Vec<ReservedChord>,
    pub focus: Vec<ReservedChord>,
}

/// Complete effective keymap projection shared by Venus (`Alt+/`) and `eon keys`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveKeymap {
    pub platform: Platform,
    pub sidebar: Sidebar,
    pub number_keys: bool,
    pub running: bool,
    pub groups: Vec<ViewerGroup>,
    pub bindings: Vec<EffectiveBinding>,
    pub popups: Vec<EffectivePopup>,
    pub reserved: ReservedChords,
}

fn popup_label<'a>(entries: &'a [PopupEntry], id: &'a str) -> &'a str {
    entries
        .iter()
        .find(|other| other.id == id)
        .map_or(id, |other| other.label.as_str())
}

/// Viewer groups matching `DialogKind::Shortcuts` (`Alt+/`).
pub fn viewer_groups(
    platform: Platform,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
    number_keys: bool,
) -> Vec<ViewerGroup> {
    let row = |shortcut: &Shortcut| {
        let label = match shortcut.overriding_popup(platform, entries) {
            Some(entry) => format!("{} (overridden by {})", shortcut.label, entry.label),
            None => {
                let taken = shortcut
                    .taken_chords(platform, entries)
                    .into_iter()
                    .map(|(chord, entry)| {
                        format!("{} overridden by {}", chord.display(platform), entry.label)
                    })
                    .collect::<Vec<_>>();
                if taken.is_empty() {
                    shortcut.label.to_owned()
                } else {
                    format!("{} ({})", shortcut.label, taken.join("; "))
                }
            }
        };
        ViewerRow {
            shortcut: shortcut.trigger.display_label_for(platform),
            label,
        }
    };
    let plain = |shortcut: &Shortcut| {
        shortcut.overriding_popup(platform, entries).is_none()
            && shortcut.taken_chords(platform, entries).is_empty()
    };
    let mut groups = GROUPS
        .into_iter()
        .map(|title| {
            let shortcuts = FIXED_SHORTCUTS
                .iter()
                .filter(|shortcut| {
                    shortcut.group == title && shortcut.applicable(sidebar, running, number_keys)
                })
                .collect::<Vec<_>>();
            let mut rows = Vec::with_capacity(shortcuts.len());
            for shortcut in &shortcuts {
                let Some(&(first, second, description)) = PAIRED_ROWS
                    .iter()
                    .find(|(first, second, _)| [*first, *second].contains(&shortcut.action))
                else {
                    rows.push(row(shortcut));
                    continue;
                };
                let partner = if shortcut.action == first {
                    second
                } else {
                    first
                };
                match shortcuts
                    .iter()
                    .find(|other| other.action == partner)
                    .filter(|other| plain(shortcut) && plain(other))
                {
                    Some(other) if shortcut.action == first => rows.push(ViewerRow {
                        shortcut: format!(
                            "{} / {}",
                            shortcut.trigger.display_label_for(platform),
                            other.trigger.display_label_for(platform)
                        ),
                        label: description.to_owned(),
                    }),
                    Some(_) => {}
                    None => rows.push(row(shortcut)),
                }
            }
            ViewerGroup {
                title: title.to_owned(),
                rows,
            }
        })
        .collect::<Vec<_>>();
    // An overridden entry keeps its default chord's row; an unbound one has none.
    let popup_rows = entries
        .iter()
        .filter_map(|entry| {
            let (shortcut, label) = match &entry.shortcut {
                PopupKey::Bound(shortcut) => (shortcut, entry.label.clone()),
                PopupKey::Overridden { default, by } => (
                    default,
                    format!(
                        "{} (overridden by {})",
                        entry.label,
                        popup_label(entries, by)
                    ),
                ),
                PopupKey::Unbound => return None,
            };
            Some(ViewerRow {
                shortcut: display_chord(platform, shortcut.modifiers, &shortcut.key),
                label,
            })
        })
        .collect::<Vec<_>>();
    if !popup_rows.is_empty() {
        groups.insert(
            2,
            ViewerGroup {
                title: "Projects and tools".to_owned(),
                rows: popup_rows,
            },
        );
    }
    groups
}

fn effective_bindings(
    platform: Platform,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
    number_keys: bool,
) -> Vec<EffectiveBinding> {
    FIXED_SHORTCUTS
        .iter()
        .filter(|shortcut| shortcut.applicable(sidebar, running, number_keys))
        .map(|&shortcut| {
            let mut chords = Vec::new();
            let mut overridden_by = Vec::new();
            for chord in shortcut.trigger.chords(platform) {
                let config = config_chord_if_valid(chord.modifiers, chord.key);
                let display = chord.display(platform);
                if let Some(entry) = shortcut.chord_override(chord, entries) {
                    overridden_by.push(OverrideSource {
                        config,
                        display,
                        kind: "popup",
                        id: entry.id.clone(),
                        label: entry.label.clone(),
                    });
                } else {
                    chords.push(KeyChord { config, display });
                }
            }
            let display = if overridden_by.is_empty() {
                Some(shortcut.trigger.display_label_for(platform))
            } else if chords.is_empty() {
                None
            } else {
                Some(
                    chords
                        .iter()
                        .map(|chord| chord.display.as_str())
                        .collect::<Vec<_>>()
                        .join(" / "),
                )
            };
            EffectiveBinding {
                action: shortcut.action.id(),
                group: shortcut.group,
                label: shortcut.label,
                display,
                source: if chords.is_empty() {
                    "overridden"
                } else {
                    "default"
                },
                chords,
                overridden_by,
            }
        })
        .collect()
}

fn effective_popups(platform: Platform, entries: &[PopupEntry]) -> Vec<EffectivePopup> {
    entries
        .iter()
        .map(|entry| match &entry.shortcut {
            PopupKey::Bound(shortcut) => EffectivePopup {
                id: entry.id.clone(),
                label: entry.label.clone(),
                keybinding: Some(config_chord(shortcut.modifiers, &shortcut.key)),
                display: Some(display_chord(platform, shortcut.modifiers, &shortcut.key)),
                default_keybinding: None,
                default_display: None,
                overridden_by: None,
                overridden_by_label: None,
                source: "bound",
            },
            PopupKey::Unbound => EffectivePopup {
                id: entry.id.clone(),
                label: entry.label.clone(),
                keybinding: None,
                display: None,
                default_keybinding: None,
                default_display: None,
                overridden_by: None,
                overridden_by_label: None,
                source: "unbound",
            },
            PopupKey::Overridden { default, by } => EffectivePopup {
                id: entry.id.clone(),
                label: entry.label.clone(),
                keybinding: None,
                display: None,
                default_keybinding: Some(config_chord(default.modifiers, &default.key)),
                default_display: Some(display_chord(platform, default.modifiers, &default.key)),
                overridden_by: Some(by.clone()),
                overridden_by_label: Some(popup_label(entries, by).to_owned()),
                source: "overridden",
            },
        })
        .collect()
}

fn reserved_chords(platform: Platform) -> ReservedChords {
    let mut strict = Vec::new();
    let mut focus = Vec::new();
    for &shortcut in FIXED_SHORTCUTS {
        let destination = match shortcut.popup_policy() {
            PopupPolicy::Overridable => continue,
            PopupPolicy::Reserved => &mut strict,
            PopupPolicy::Focus => &mut focus,
        };
        for chord in shortcut.trigger.chords(platform) {
            destination.push(ReservedChord {
                action: shortcut.action.id(),
                config: config_chord_if_valid(chord.modifiers, chord.key)
                    .expect("reserved and focus shortcuts use configurable chords"),
                display: chord.display(platform),
            });
        }
    }
    ReservedChords { strict, focus }
}

/// Complete effective keymap derived from the shared table, popups, and window flags.
pub fn effective_keymap(
    platform: Platform,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
    number_keys: bool,
) -> EffectiveKeymap {
    EffectiveKeymap {
        platform,
        sidebar,
        number_keys,
        running,
        groups: viewer_groups(platform, entries, sidebar, running, number_keys),
        bindings: effective_bindings(platform, entries, sidebar, running, number_keys),
        popups: effective_popups(platform, entries),
        reserved: reserved_chords(platform),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_and_native_alternatives_preserve_their_distinct_boundaries() {
        for key in DIGIT_KEYS.into_iter().chain(["Slash"]) {
            assert_eq!(popup_policy(ALT, key), PopupPolicy::Reserved);
            assert_eq!(popup_policy(ALT | SHIFT, key), PopupPolicy::Overridable);
        }
        for key in ["KeyH", "KeyJ", "KeyK", "KeyL"] {
            assert_eq!(popup_policy(ALT, key), PopupPolicy::Focus);
            assert_eq!(popup_policy(ALT | SHIFT, key), PopupPolicy::Overridable);
        }
        for platform in [Platform::Linux, Platform::Macos] {
            for modifiers in 0..16 {
                assert!(Trigger::Paste.matches_key(platform, "Paste", modifiers));
                assert!(!Trigger::OpenLink.matches_key(platform, "click", modifiers));
            }
            assert_eq!(Trigger::AltDigits.chords(platform).len(), 10);
            assert_eq!(
                Trigger::AltDigits.display_label_for(platform),
                if platform == Platform::Linux {
                    "Alt+1…9 / Alt+0"
                } else {
                    "Option+1…9 / Option+0"
                }
            );
        }
        assert!(Trigger::Quit.matches_key(Platform::Macos, "KeyQ", SUPER));
        assert!(!Trigger::Quit.matches_key(Platform::Linux, "KeyQ", SUPER));
        assert_eq!(
            [SHIFT, CTRL, ALT, SUPER],
            [
                crate::v44::SHIFT,
                crate::v44::CTRL,
                crate::v44::ALT,
                crate::v44::SUPER
            ]
        );
    }

    #[test]
    fn config_and_display_keep_their_audiences() {
        assert_eq!(config_chord(ALT, "Slash"), "Alt+Slash");
        assert_eq!(display_chord(Platform::Linux, ALT, "Slash"), "Alt+/");
        assert_eq!(
            display_chord(Platform::Macos, ALT | SHIFT, "KeyT"),
            "Option+Shift+T"
        );
        assert_eq!(
            config_chord(CTRL | ALT | SHIFT | SUPER, "Digit0"),
            "Ctrl+Alt+Shift+Super+0"
        );
        assert_eq!(display_chord(Platform::Macos, 0, "ArrowLeft"), "Left");
    }

    #[test]
    fn action_ids_are_unique_snake_case_and_cover_fixed_shortcuts() {
        let ids: Vec<&str> = FIXED_SHORTCUTS
            .iter()
            .map(|shortcut| shortcut.action.id())
            .collect();
        let expected = [
            "next_tab",
            "previous_tab",
            "focus_left",
            "focus_right",
            "focus_up",
            "focus_down",
            "focus_tab_position",
            "show_workspace_position",
            "next_attention",
            "previous_attention",
            "toggle_inbox",
            "cycle_focus",
            "return_to_terminal",
            "traverse_chrome_left",
            "traverse_chrome_right",
            "traverse_chrome_up",
            "traverse_chrome_down",
            "create_pane",
            "create_column",
            "create_tab",
            "close_tab",
            "close_pane",
            "toggle_zoom",
            "move_tab_left",
            "move_tab_right",
            "move_pane_left",
            "move_pane_right",
            "move_pane_up",
            "move_pane_down",
            "new_window",
            "detach",
            "quit",
            "toggle_viewer",
            "toggle_hints",
            "toggle_sidebar",
            "copy",
            "paste",
            "open_link",
        ];
        assert_eq!(ids, expected);
        let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len());
        for id in ids {
            assert!(!id.is_empty());
            assert!(
                id.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "non-snake_case action id: {id}"
            );
        }
    }

    #[test]
    fn effective_keymap_and_viewer_groups_project_overrides_and_flags() {
        let entries = vec![
            PopupEntry {
                id: "pane-maker".into(),
                label: "Pane Override".into(),
                shortcut: PopupKey::Bound(ProtocolShortcut {
                    modifiers: ALT,
                    key: "KeyM".into(),
                }),
            },
            PopupEntry {
                id: "slot-one".into(),
                label: "Slot One".into(),
                shortcut: PopupKey::Bound(ProtocolShortcut {
                    modifiers: ALT_SHIFT,
                    key: "Digit1".into(),
                }),
            },
            PopupEntry {
                id: "git".into(),
                label: "Git".into(),
                shortcut: PopupKey::Overridden {
                    default: ProtocolShortcut {
                        modifiers: ALT_SHIFT,
                        key: "KeyJ".into(),
                    },
                    by: "pane-maker".into(),
                },
            },
            PopupEntry {
                id: "silent".into(),
                label: "Silent Tool".into(),
                shortcut: PopupKey::Unbound,
            },
        ];
        let linux = effective_keymap(Platform::Linux, &entries, Sidebar::Open, true, true);
        assert_eq!(
            linux
                .groups
                .iter()
                .map(|g| g.title.as_str())
                .collect::<Vec<_>>(),
            ["Navigate", "Tabs and panes", "Projects and tools", "Host"]
        );
        // Paired chrome rows are present in viewer groups, while bindings keeps each action.
        let nav_rows = &linux.groups[0].rows;
        assert!(nav_rows.iter().any(|r| r.shortcut == "Left / Right"));
        assert!(nav_rows.iter().any(|r| r.shortcut == "Up / Down"));

        let create_pane = linux
            .bindings
            .iter()
            .find(|b| b.action == "create_pane")
            .unwrap();
        assert_eq!(create_pane.source, "overridden");
        assert_eq!(create_pane.display, None);
        assert!(create_pane.chords.is_empty());
        assert_eq!(create_pane.overridden_by.len(), 1);
        assert_eq!(create_pane.overridden_by[0].id, "pane-maker");
        assert_eq!(create_pane.overridden_by[0].kind, "popup");
        assert!(linux.groups[1].rows.iter().any(
            |r| r.shortcut == "Alt+M" && r.label == "Create pane (overridden by Pane Override)"
        ));

        let show_ws = linux
            .bindings
            .iter()
            .find(|b| b.action == "show_workspace_position")
            .unwrap();
        assert_eq!(show_ws.source, "default");
        assert_eq!(show_ws.chords.len(), 9);
        assert_eq!(show_ws.overridden_by.len(), 1);
        assert_eq!(
            show_ws.overridden_by[0].config.as_deref(),
            Some("Alt+Shift+1")
        );
        assert!(
            !show_ws
                .display
                .as_deref()
                .unwrap()
                .contains("Alt+Shift+1 /")
        );
        assert!(
            nav_rows.iter().any(|r| r.label == "Show session by position (Alt+Shift+1 overridden by Slot One)")
        );

        // Non-configurable triggers report config = None.
        let open_link = linux
            .bindings
            .iter()
            .find(|b| b.action == "open_link")
            .unwrap();
        assert_eq!(open_link.display.as_deref(), Some("Ctrl+click"));
        assert_eq!(open_link.chords[0].config, None);
        let cycle_focus = linux
            .bindings
            .iter()
            .find(|b| b.action == "cycle_focus")
            .unwrap();
        assert_eq!(cycle_focus.chords[0].config, None);
        let paste = linux.bindings.iter().find(|b| b.action == "paste").unwrap();
        assert_eq!(paste.chords[0].config.as_deref(), Some("Ctrl+Shift+V"));
        assert_eq!(paste.chords[1].config, None);

        // Popups include unbound and overridden state, while viewer group omits unbound.
        assert_eq!(linux.popups.len(), 4);
        assert_eq!(linux.popups[2].source, "overridden");
        assert_eq!(
            linux.popups[2].default_keybinding.as_deref(),
            Some("Alt+Shift+J")
        );
        assert_eq!(
            linux.popups[2].overridden_by_label.as_deref(),
            Some("Pane Override")
        );
        assert_eq!(linux.popups[3].source, "unbound");
        assert_eq!(linux.groups[2].rows.len(), 3);
        assert!(
            linux.groups[2]
                .rows
                .iter()
                .any(|r| r.label == "Git (overridden by Pane Override)")
        );

        // Reserved and focus lists.
        assert_eq!(linux.reserved.strict.len(), 11);
        assert!(
            linux
                .reserved
                .strict
                .iter()
                .any(|c| c.action == "toggle_viewer"
                    && c.config == "Alt+Slash"
                    && c.display == "Alt+/")
        );
        assert_eq!(linux.reserved.focus.len(), 4);
        assert!(
            linux
                .reserved
                .focus
                .iter()
                .all(|c| c.action.starts_with("focus_"))
        );

        // Conditional flags and macOS Quit partial/full overrides.
        let mac_entries = [
            PopupEntry {
                id: "cmd-q".into(),
                label: "Cmd Q".into(),
                shortcut: PopupKey::Bound(ProtocolShortcut {
                    modifiers: SUPER,
                    key: "KeyQ".into(),
                }),
            },
            PopupEntry {
                id: "silent".into(),
                label: "Silent".into(),
                shortcut: PopupKey::Unbound,
            },
        ];
        let filtered = effective_keymap(
            Platform::Macos,
            &mac_entries[1..],
            Sidebar::Off,
            false,
            false,
        );
        assert_eq!(filtered.groups.len(), 3);
        assert!(
            !filtered
                .bindings
                .iter()
                .any(|b| b.action == "toggle_sidebar")
        );
        assert!(!filtered.bindings.iter().any(|b| b.action == "quit"));
        assert!(
            !filtered
                .bindings
                .iter()
                .any(|b| b.action == "show_workspace_position")
        );

        let mac_partial = effective_keymap(
            Platform::Macos,
            &mac_entries[..1],
            Sidebar::Open,
            true,
            true,
        );
        let mac_quit = mac_partial
            .bindings
            .iter()
            .find(|b| b.action == "quit")
            .unwrap();
        assert_eq!(mac_quit.source, "default");
        assert_eq!(mac_quit.display.as_deref(), Some("Option+Shift+Q"));
        assert_eq!(mac_quit.overridden_by.len(), 1);
        assert_eq!(mac_quit.overridden_by[0].config.as_deref(), Some("Super+Q"));
        assert_eq!(mac_quit.overridden_by[0].display, "Command+Q");

        let mac_both = [
            mac_entries[0].clone(),
            PopupEntry {
                id: "opt-shift-q".into(),
                label: "Opt Shift Q".into(),
                shortcut: PopupKey::Bound(ProtocolShortcut {
                    modifiers: ALT_SHIFT,
                    key: "KeyQ".into(),
                }),
            },
        ];
        let mac_full = effective_keymap(Platform::Macos, &mac_both, Sidebar::Open, true, true);
        let mac_quit_full = mac_full
            .bindings
            .iter()
            .find(|b| b.action == "quit")
            .unwrap();
        assert_eq!(mac_quit_full.source, "overridden");
        assert_eq!(mac_quit_full.display, None);
        assert_eq!(mac_quit_full.overridden_by.len(), 2);
    }

    #[test]
    fn viewer_groups_and_effective_bindings_agree_on_every_override() {
        for platform in [Platform::Linux, Platform::Macos] {
            for shortcut in FIXED_SHORTCUTS {
                for chord in shortcut.trigger.chords(platform) {
                    let entries = [PopupEntry {
                        id: "probe".into(),
                        label: "Probe Override".into(),
                        shortcut: PopupKey::Bound(ProtocolShortcut {
                            modifiers: chord.modifiers,
                            key: chord.key.into(),
                        }),
                    }];
                    let keymap = effective_keymap(platform, &entries, Sidebar::Open, true, true);
                    let overridden_bindings: Vec<_> = keymap
                        .bindings
                        .iter()
                        .filter(|binding| !binding.overridden_by.is_empty())
                        .collect();
                    let overridden_rows: Vec<_> = keymap
                        .groups
                        .iter()
                        .filter(|group| group.title != "Projects and tools")
                        .flat_map(|group| group.rows.iter())
                        .filter(|row| row.label.contains("overridden by Probe Override"))
                        .collect();
                    assert_eq!(
                        overridden_rows.len(),
                        overridden_bindings.len(),
                        "platform={platform:?} action={} chord={chord:?}\nrows={overridden_rows:?}\nbindings={overridden_bindings:?}",
                        shortcut.action.id()
                    );
                    for binding in overridden_bindings {
                        assert!(
                            binding
                                .overridden_by
                                .iter()
                                .all(|source| source.label == "Probe Override")
                        );
                        assert!(
                            overridden_rows.iter().any(|row| {
                                row.label.starts_with(&format!("{} (", binding.label))
                            }),
                            "missing viewer override row for {} on {platform:?}",
                            binding.action
                        );
                    }
                }
            }
        }
    }
}
