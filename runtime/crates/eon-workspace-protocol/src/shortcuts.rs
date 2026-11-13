//! Eon's shared shortcut descriptors. This is Rust data, not an EONW codec.

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
                crate::v43::SHIFT,
                crate::v43::CTRL,
                crate::v43::ALT,
                crate::v43::SUPER
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
}
