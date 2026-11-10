use super::{NATIVE_PLATFORM, NativePlatform};
use eon_workspace_protocol::v38::{
    Action as WorkspaceAction, Direction as WorkspaceDirection, InvokeIntent, PopupEntry, PopupKey,
    Side, Sidebar, Snapshot, Tab, WorkspaceAction as CommonAction,
};
use orbit_protocol::session;
use winit::{
    event::MouseButton,
    keyboard::{KeyCode, PhysicalKey},
};
use yazelix_venus::{HintSection, ShortcutGroup, ShortcutHint, ShortcutRow, WorkspaceFocus};

pub(super) fn paste_shortcut(
    physical_key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
) -> bool {
    physical_key == PhysicalKey::Code(KeyCode::Paste)
        || native_key_shortcut(physical_key, modifiers, entries)
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Paste)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeShortcutAction {
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
    Focus(WorkspaceDirection),
    Move(WorkspaceDirection),
    MovePaneSideways(Side),
    CreatePane,
    CreateColumn,
    CreateTab,
    CloseTab,
    ClosePane,
    CycleFocus,
    ReturnToTerminal,
    TraverseChrome(WorkspaceDirection),
    Copy,
    Paste,
    OpenLink,
}

#[derive(Clone, Copy)]
pub(super) enum NativeShortcutTrigger {
    Key(KeyCode, session::Modifiers),
    AltDigits,
    AltShiftDigits,
    Copy,
    Paste,
    OpenLink,
    /// Alt+Shift+Q. On macOS Command+Q quits too, through the close request (`command_q`).
    Quit,
}

impl NativeShortcutTrigger {
    pub(super) fn display_label_for(self, platform: NativePlatform) -> String {
        match self {
            Self::Key(code, modifiers) => physical_shortcut_label_for(
                platform,
                wire_modifiers(modifiers),
                &format!("{code:?}"),
            ),
            Self::AltDigits => match platform {
                NativePlatform::Linux => "Alt+1…9 / Alt+0".into(),
                NativePlatform::Macos => "Option+1…9 / Option+0".into(),
            },
            Self::AltShiftDigits => match platform {
                NativePlatform::Linux => "Alt+Shift+1…9 / Alt+Shift+0".into(),
                NativePlatform::Macos => "Option+Shift+1…9 / Option+Shift+0".into(),
            },
            Self::Copy => physical_shortcut_label_for(
                platform,
                wire_modifiers(copy_paste_modifiers(platform)),
                "KeyC",
            ),
            Self::Paste => format!(
                "{} or Paste",
                physical_shortcut_label_for(
                    platform,
                    wire_modifiers(copy_paste_modifiers(platform)),
                    "KeyV",
                )
            ),
            Self::OpenLink => physical_shortcut_label_for(
                platform,
                wire_modifiers(open_link_modifiers(platform)),
                "click",
            ),
            Self::Quit => {
                let quit = physical_shortcut_label_for(platform, wire_modifiers(ALT_SHIFT), "KeyQ");
                match platform {
                    NativePlatform::Linux => quit,
                    NativePlatform::Macos => format!("{quit} or Command+Q"),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NativeShortcut {
    group: &'static str,
    pub(super) label: &'static str,
    pub(super) trigger: NativeShortcutTrigger,
    pub(super) action: NativeShortcutAction,
}

pub(super) const ALT_SHIFT: session::Modifiers =
    session::Modifiers::ALT.union(session::Modifiers::SHIFT);
pub(super) const CTRL_ALT: session::Modifiers =
    session::Modifiers::CTRL.union(session::Modifiers::ALT);
pub(super) const CTRL_SHIFT: session::Modifiers =
    session::Modifiers::CTRL.union(session::Modifiers::SHIFT);
const CTRL_ALT_SHIFT: session::Modifiers = CTRL_ALT.union(session::Modifiers::SHIFT);
pub(super) static FIXED_SHORTCUTS: &[NativeShortcut] = &[
    NativeShortcut::key(
        "Navigate",
        "Focus next tab",
        KeyCode::Tab,
        session::Modifiers::CTRL,
        NativeShortcutAction::NextTab,
    ),
    NativeShortcut::key(
        "Navigate",
        "Focus previous tab",
        KeyCode::Tab,
        CTRL_SHIFT,
        NativeShortcutAction::PreviousTab,
    ),
    NativeShortcut::key(
        "Navigate",
        "Focus left column or previous tab",
        KeyCode::KeyH,
        session::Modifiers::ALT,
        NativeShortcutAction::Focus(WorkspaceDirection::Left),
    ),
    NativeShortcut::key(
        "Navigate",
        "Focus right column or next tab",
        KeyCode::KeyL,
        session::Modifiers::ALT,
        NativeShortcutAction::Focus(WorkspaceDirection::Right),
    ),
    NativeShortcut::key(
        "Navigate",
        "Focus pane above",
        KeyCode::KeyK,
        session::Modifiers::ALT,
        NativeShortcutAction::Focus(WorkspaceDirection::Up),
    ),
    NativeShortcut::key(
        "Navigate",
        "Focus pane below",
        KeyCode::KeyJ,
        session::Modifiers::ALT,
        NativeShortcutAction::Focus(WorkspaceDirection::Down),
    ),
    // After the HJKL focus keys, so the merged focus hint ends with 1..9.
    NativeShortcut {
        group: "Navigate",
        label: "Focus tab by position",
        trigger: NativeShortcutTrigger::AltDigits,
        action: NativeShortcutAction::FocusTabPosition,
    },
    NativeShortcut {
        group: "Navigate",
        label: "Show session by position",
        trigger: NativeShortcutTrigger::AltShiftDigits,
        action: NativeShortcutAction::ShowWorkspacePosition,
    },
    NativeShortcut::key(
        "Navigate",
        "Jump to the next agent needing attention",
        KeyCode::KeyG,
        session::Modifiers::ALT,
        NativeShortcutAction::NextAttention,
    ),
    NativeShortcut::key(
        "Navigate",
        "Jump to the previous agent needing attention",
        KeyCode::KeyG,
        ALT_SHIFT,
        NativeShortcutAction::PreviousAttention,
    ),
    NativeShortcut::key(
        "Navigate",
        "Show the attention inbox",
        KeyCode::KeyQ,
        session::Modifiers::ALT,
        NativeShortcutAction::ToggleInbox,
    ),
    NativeShortcut::key(
        "Navigate",
        "Cycle terminal, agent sidebar, tabs, and panes",
        KeyCode::F6,
        session::Modifiers::empty(),
        NativeShortcutAction::CycleFocus,
    ),
    NativeShortcut::key(
        "Navigate",
        "Return focus to the terminal",
        KeyCode::Escape,
        session::Modifiers::empty(),
        NativeShortcutAction::ReturnToTerminal,
    ),
    NativeShortcut::key(
        "Navigate",
        "Left column or previous tab while tabs are focused",
        KeyCode::ArrowLeft,
        session::Modifiers::empty(),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Left),
    ),
    NativeShortcut::key(
        "Navigate",
        "Right column or next tab while tabs are focused",
        KeyCode::ArrowRight,
        session::Modifiers::empty(),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Right),
    ),
    NativeShortcut::key(
        "Navigate",
        "Previous pane or agent row while panes or the sidebar are focused",
        KeyCode::ArrowUp,
        session::Modifiers::empty(),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Up),
    ),
    NativeShortcut::key(
        "Navigate",
        "Next pane or agent row while panes or the sidebar are focused",
        KeyCode::ArrowDown,
        session::Modifiers::empty(),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Down),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Create pane",
        KeyCode::KeyM,
        session::Modifiers::ALT,
        NativeShortcutAction::CreatePane,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Create column",
        KeyCode::KeyM,
        CTRL_ALT,
        NativeShortcutAction::CreateColumn,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Create tab",
        KeyCode::KeyT,
        ALT_SHIFT,
        NativeShortcutAction::CreateTab,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Close active tab",
        KeyCode::KeyW,
        ALT_SHIFT,
        NativeShortcutAction::CloseTab,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Close selected pane",
        KeyCode::KeyW,
        session::Modifiers::ALT,
        NativeShortcutAction::ClosePane,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Zoom selected pane or restore columns",
        KeyCode::KeyZ,
        ALT_SHIFT,
        NativeShortcutAction::ToggleZoom,
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move tab left",
        KeyCode::KeyH,
        CTRL_ALT_SHIFT,
        NativeShortcutAction::Move(WorkspaceDirection::Left),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move tab right",
        KeyCode::KeyL,
        CTRL_ALT_SHIFT,
        NativeShortcutAction::Move(WorkspaceDirection::Right),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move pane to left column",
        KeyCode::KeyH,
        CTRL_ALT,
        NativeShortcutAction::MovePaneSideways(Side::Left),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move pane to right column",
        KeyCode::KeyL,
        CTRL_ALT,
        NativeShortcutAction::MovePaneSideways(Side::Right),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move pane up",
        KeyCode::KeyK,
        CTRL_ALT,
        NativeShortcutAction::Move(WorkspaceDirection::Up),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move pane down",
        KeyCode::KeyJ,
        CTRL_ALT,
        NativeShortcutAction::Move(WorkspaceDirection::Down),
    ),
    NativeShortcut::key(
        "Host",
        "Open new Eon window",
        KeyCode::KeyN,
        ALT_SHIFT,
        NativeShortcutAction::NewWindow,
    ),
    NativeShortcut::key(
        "Host",
        "Detach: hide the window, keep terminals running",
        KeyCode::KeyD,
        ALT_SHIFT,
        NativeShortcutAction::Detach,
    ),
    NativeShortcut {
        group: "Host",
        label: "Quit: close the window, stop its terminals",
        trigger: NativeShortcutTrigger::Quit,
        action: NativeShortcutAction::Quit,
    },
    NativeShortcut::key(
        "Host",
        "Show or close shortcuts",
        KeyCode::Slash,
        session::Modifiers::ALT,
        NativeShortcutAction::ToggleViewer,
    ),
    NativeShortcut::key(
        "Host",
        "Show or hide shortcut hints",
        KeyCode::KeyB,
        ALT_SHIFT,
        NativeShortcutAction::ToggleHints,
    ),
    NativeShortcut::key(
        "Host",
        "Show or hide the agent sidebar",
        KeyCode::KeyH,
        ALT_SHIFT,
        NativeShortcutAction::ToggleSidebar,
    ),
    NativeShortcut {
        group: "Host",
        label: "Copy selected text or focused link",
        trigger: NativeShortcutTrigger::Copy,
        action: NativeShortcutAction::Copy,
    },
    NativeShortcut {
        group: "Host",
        label: "Paste",
        trigger: NativeShortcutTrigger::Paste,
        action: NativeShortcutAction::Paste,
    },
    NativeShortcut {
        group: "Host",
        label: "Open link",
        trigger: NativeShortcutTrigger::OpenLink,
        action: NativeShortcutAction::OpenLink,
    },
];

impl NativeShortcut {
    const fn key(
        group: &'static str,
        label: &'static str,
        code: KeyCode,
        modifiers: session::Modifiers,
        action: NativeShortcutAction,
    ) -> Self {
        Self {
            group,
            label,
            trigger: NativeShortcutTrigger::Key(code, modifiers),
            action,
        }
    }

    /// Whether the binding exists: `[sidebar] enabled = false` unbinds the sidebar toggle, and
    /// a window without a running-windows summary (eonbase) has no Quit.
    pub(super) fn bound(self, sidebar: Sidebar, running: bool) -> bool {
        match self.action {
            NativeShortcutAction::ToggleSidebar => sidebar != Sidebar::Off,
            NativeShortcutAction::Quit => running,
            _ => true,
        }
    }

    fn matches_key(
        self,
        platform: NativePlatform,
        key: PhysicalKey,
        modifiers: session::Modifiers,
    ) -> bool {
        match self.trigger {
            NativeShortcutTrigger::Key(code, expected) => {
                key == PhysicalKey::Code(code) && modifiers == expected
            }
            NativeShortcutTrigger::AltDigits => {
                modifiers == session::Modifiers::ALT && digit_key(key)
            }
            NativeShortcutTrigger::AltShiftDigits => modifiers == ALT_SHIFT && digit_key(key),
            NativeShortcutTrigger::Copy => {
                key == PhysicalKey::Code(KeyCode::KeyC)
                    && modifiers == copy_paste_modifiers(platform)
            }
            NativeShortcutTrigger::Paste => {
                key == PhysicalKey::Code(KeyCode::Paste)
                    || key == PhysicalKey::Code(KeyCode::KeyV)
                        && modifiers == copy_paste_modifiers(platform)
            }
            NativeShortcutTrigger::OpenLink => false,
            NativeShortcutTrigger::Quit => {
                key == PhysicalKey::Code(KeyCode::KeyQ) && modifiers == ALT_SHIFT
            }
        }
    }

    /// The configured popup that takes this binding's key. Eon reserves the viewer toggle,
    /// tab positions and focus keys, so no popup takes those.
    fn overriding_popup(
        self,
        platform: NativePlatform,
        entries: &[PopupEntry],
    ) -> Option<&PopupEntry> {
        let (code, modifiers) = match self.trigger {
            NativeShortcutTrigger::Key(code, modifiers)
                if !matches!(
                    self.action,
                    NativeShortcutAction::ToggleViewer | NativeShortcutAction::Focus(_)
                ) =>
            {
                (code, modifiers)
            }
            NativeShortcutTrigger::Copy => (KeyCode::KeyC, copy_paste_modifiers(platform)),
            NativeShortcutTrigger::Paste => (KeyCode::KeyV, copy_paste_modifiers(platform)),
            // On macOS Quit has two keys, and a popup on one leaves the other (`taken_keys`).
            NativeShortcutTrigger::Quit if platform == NativePlatform::Linux => {
                (KeyCode::KeyQ, ALT_SHIFT)
            }
            _ => return None,
        };
        popup_entry(entries, code, modifiers)
    }

    /// Each key of a several-key binding that a configured popup takes, as "<key> overridden by
    /// <popup>"; the binding keeps its other keys: the workspace positions (VEN-C20), and on
    /// macOS Quit's Option+Shift+Q and Command+Q (VEN-C8).
    fn taken_keys(self, platform: NativePlatform, entries: &[PopupEntry]) -> Vec<String> {
        let keys: Vec<(KeyCode, session::Modifiers)> = match self.trigger {
            NativeShortcutTrigger::AltShiftDigits => {
                DIGIT_KEYS.iter().map(|code| (*code, ALT_SHIFT)).collect()
            }
            NativeShortcutTrigger::Quit if platform == NativePlatform::Macos => vec![
                (KeyCode::KeyQ, ALT_SHIFT),
                (KeyCode::KeyQ, session::Modifiers::SUPER),
            ],
            _ => return Vec::new(),
        };
        keys.into_iter()
            .filter_map(|(code, modifiers)| {
                let entry = popup_entry(entries, code, modifiers)?;
                let key = physical_shortcut_label_for(
                    platform,
                    wire_modifiers(modifiers),
                    &format!("{code:?}"),
                );
                Some(format!("{key} overridden by {}", entry.label))
            })
            .collect()
    }

    fn matches_pointer(
        self,
        platform: NativePlatform,
        button: MouseButton,
        modifiers: session::Modifiers,
    ) -> bool {
        matches!(self.trigger, NativeShortcutTrigger::OpenLink)
            && button == MouseButton::Left
            && modifiers == open_link_modifiers(platform)
    }
}

pub(super) fn copy_paste_modifiers(platform: NativePlatform) -> session::Modifiers {
    match platform {
        NativePlatform::Linux => CTRL_SHIFT,
        NativePlatform::Macos => session::Modifiers::SUPER,
    }
}

fn open_link_modifiers(platform: NativePlatform) -> session::Modifiers {
    match platform {
        NativePlatform::Linux => session::Modifiers::CTRL,
        NativePlatform::Macos => session::Modifiers::SUPER,
    }
}

pub(super) fn command_q(
    key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
) -> bool {
    command_q_for(NATIVE_PLATFORM, key, modifiers, entries)
}

/// Command+Q on macOS, which an Eon workspace window takes as its close request (VEN-C8),
/// unless a configured popup takes the key.
pub(super) fn command_q_for(
    platform: NativePlatform,
    key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
) -> bool {
    platform == NativePlatform::Macos
        && key == PhysicalKey::Code(KeyCode::KeyQ)
        && modifiers == session::Modifiers::SUPER
        && popup_entry(entries, KeyCode::KeyQ, modifiers).is_none()
}

pub(super) fn native_key_shortcut(
    key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
) -> Option<&'static NativeShortcut> {
    native_key_shortcut_for(NATIVE_PLATFORM, key, modifiers, entries)
}

/// The fixed binding a key triggers, unless a configured popup takes that key. A popup takes any
/// one of the workspace position keys, or of Quit's keys, and leaves the others bound (VEN-C8).
pub(super) fn native_key_shortcut_for(
    platform: NativePlatform,
    key: PhysicalKey,
    modifiers: session::Modifiers,
    entries: &[PopupEntry],
) -> Option<&'static NativeShortcut> {
    FIXED_SHORTCUTS.iter().find(|shortcut| {
        let position_taken = || match key {
            PhysicalKey::Code(code) => popup_entry(entries, code, modifiers).is_some(),
            PhysicalKey::Unidentified(_) => false,
        };
        shortcut.matches_key(platform, key, modifiers)
            && shortcut.overriding_popup(platform, entries).is_none()
            && !(matches!(
                shortcut.trigger,
                NativeShortcutTrigger::AltShiftDigits | NativeShortcutTrigger::Quit
            ) && position_taken())
    })
}

/// The digit keys in the positions they name: 1..9, then 0 for the tenth.
const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

/// The zero-based position a digit key names.
fn digit_position(code: KeyCode) -> Option<usize> {
    DIGIT_KEYS.iter().position(|digit| *digit == code)
}

fn digit_key(key: PhysicalKey) -> bool {
    matches!(key, PhysicalKey::Code(code) if digit_position(code).is_some())
}

/// The detach chord's label, unless a configured popup takes its key.
pub(super) fn detach_chord(entries: &[PopupEntry]) -> Option<String> {
    FIXED_SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == NativeShortcutAction::Detach)
        .filter(|shortcut| {
            shortcut
                .overriding_popup(NATIVE_PLATFORM, entries)
                .is_none()
        })
        .map(|shortcut| shortcut.trigger.display_label_for(NATIVE_PLATFORM))
}

/// Alt+Shift+Q's label, unless a configured popup takes the key. macOS's Command+Q is a close
/// request, which detaches after a failed Stop, so it never retries one.
pub(super) fn quit_retry_chord(entries: &[PopupEntry]) -> Option<String> {
    popup_entry(entries, KeyCode::KeyQ, ALT_SHIFT)
        .is_none()
        .then(|| physical_shortcut_label_for(NATIVE_PLATFORM, wire_modifiers(ALT_SHIFT), "KeyQ"))
}

pub(super) fn native_pointer_shortcut(
    button: MouseButton,
    modifiers: session::Modifiers,
) -> Option<&'static NativeShortcut> {
    native_pointer_shortcut_for(NATIVE_PLATFORM, button, modifiers)
}

pub(super) fn native_pointer_shortcut_for(
    platform: NativePlatform,
    button: MouseButton,
    modifiers: session::Modifiers,
) -> Option<&'static NativeShortcut> {
    FIXED_SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.matches_pointer(platform, button, modifiers))
}

pub(super) fn shortcut_groups(
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
    number_keys: bool,
) -> Vec<ShortcutGroup> {
    shortcut_groups_with(NATIVE_PLATFORM, entries, sidebar, running, number_keys)
}

#[cfg(test)]
pub(super) fn shortcut_groups_for(
    platform: NativePlatform,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
) -> Vec<ShortcutGroup> {
    shortcut_groups_with(platform, entries, sidebar, running, true)
}

/// Opposite arrow keys the viewer lists on one row; a popup taking either key splits them.
pub(super) const PAIRED_ROWS: [(NativeShortcutAction, NativeShortcutAction, &str); 2] = [
    (
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Left),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Right),
        "Previous or next column or tab while tabs are focused",
    ),
    (
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Up),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Down),
        "Previous or next pane or agent row while one is focused",
    ),
];

/// The viewer's groups; `number_keys = false` leaves out the workspace position keys (VEN-C8).
fn shortcut_groups_with(
    platform: NativePlatform,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
    number_keys: bool,
) -> Vec<ShortcutGroup> {
    let row = |shortcut: &NativeShortcut| {
        ShortcutRow::new(
            shortcut.trigger.display_label_for(platform),
            match shortcut.overriding_popup(platform, entries) {
                Some(entry) => format!("{} (overridden by {})", shortcut.label, entry.label),
                None => match shortcut.taken_keys(platform, entries) {
                    taken if taken.is_empty() => shortcut.label.into(),
                    taken => format!("{} ({})", shortcut.label, taken.join("; ")),
                },
            },
        )
    };
    let plain = |shortcut: &NativeShortcut| {
        shortcut.overriding_popup(platform, entries).is_none()
            && shortcut.taken_keys(platform, entries).is_empty()
    };
    let mut groups = ["Navigate", "Tabs and panes", "Host"]
        .into_iter()
        .map(|title| {
            let shortcuts = FIXED_SHORTCUTS
                .iter()
                .filter(|shortcut| {
                    shortcut.group == title
                        && shortcut.bound(sidebar, running)
                        && (number_keys
                            || shortcut.action != NativeShortcutAction::ShowWorkspacePosition)
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
                    Some(other) if shortcut.action == first => rows.push(ShortcutRow::new(
                        format!(
                            "{} / {}",
                            shortcut.trigger.display_label_for(platform),
                            other.trigger.display_label_for(platform)
                        ),
                        description,
                    )),
                    // The first key's row lists this one.
                    Some(_) => {}
                    None => rows.push(row(shortcut)),
                }
            }
            ShortcutGroup::new(title, rows)
        })
        .collect::<Vec<_>>();
    // An overridden entry keeps its default chord's row; an unbound one has none.
    let rows = entries
        .iter()
        .filter_map(|entry| {
            let (shortcut, label) = match &entry.shortcut {
                PopupKey::Bound(shortcut) => (shortcut, entry.label.clone()),
                PopupKey::Overridden { default, by } => {
                    let by = entries
                        .iter()
                        .find(|other| &other.id == by)
                        .map_or(by.as_str(), |other| other.label.as_str());
                    (default, format!("{} (overridden by {by})", entry.label))
                }
                PopupKey::Unbound => return None,
            };
            Some(ShortcutRow::new(
                physical_shortcut_label_for(platform, shortcut.modifiers, &shortcut.key),
                label,
            ))
        })
        .collect::<Vec<_>>();
    if !rows.is_empty() {
        groups.insert(2, ShortcutGroup::new("Projects and tools", rows));
    }
    groups
}

/// A keycap popup's dismissal: Escape or a pointer press hides the popup until
/// the pointer leaves that keycap's hover band, as tooltips do. Neither changes
/// where the key or press goes.
#[derive(Debug, Default)]
pub(super) struct HintPopupLatch {
    shown: Option<usize>,
    dismissed: Option<usize>,
}

impl HintPopupLatch {
    /// Whether the hovered keycap's popup shows.
    pub(super) fn popup(&mut self, hovered: Option<usize>) -> bool {
        if hovered != self.dismissed {
            self.dismissed = None;
        }
        self.shown = hovered.filter(|_| self.dismissed.is_none());
        self.shown.is_some()
    }

    /// The keycap whose popup shows.
    pub(super) fn shown(&self) -> Option<usize> {
        self.shown
    }

    /// Hides the open popup; true when one was open.
    pub(super) fn dismiss(&mut self) -> bool {
        let Some(group) = self.shown.take() else {
            return false;
        };
        self.dismissed = Some(group);
        true
    }
}

pub(super) fn shortcut_hints_for(
    platform: NativePlatform,
    focus: WorkspaceFocus,
    entries: &[PopupEntry],
    sidebar: Sidebar,
    running: bool,
) -> Vec<ShortcutHint> {
    let mut hints: Vec<ShortcutHint> = Vec::new();
    // The key axis each fixed hint ended with: HJKL, 1..9 or an arrow pair.
    let mut axes = Vec::new();
    for shortcut in FIXED_SHORTCUTS {
        if !shortcut.bound(sidebar, running)
            || shortcut.overriding_popup(platform, entries).is_some()
        {
            continue;
        }
        use NativeShortcutAction as Action;
        use WorkspaceDirection::{Down, Left, Right, Up};
        // The bar's label, the popup's short label (keys without modifiers have no popup), and
        // the priority to keep the hint.
        let (label, short, priority) = match shortcut.action {
            Action::ToggleHints => ("hints", "Hints", 0),
            Action::ToggleSidebar => ("side", "Sidebar", 2),
            Action::Focus(_) => ("focus", "Focus", 1),
            Action::FocusTabPosition => ("focus", "Focus", 3),
            Action::CreatePane => ("pane", "New pane", 1),
            Action::CreateColumn => ("col", "New column", 3),
            Action::CreateTab => ("tab", "New tab", 1),
            Action::CloseTab => ("close tab", "Close tab", 3),
            Action::ClosePane => ("close pane", "Close pane", 3),
            Action::ToggleZoom => ("zoom", "Zoom pane", 3),
            Action::Move(Left | Right) => ("mv tab", "Move tab", 3),
            Action::Move(Up | Down) | Action::MovePaneSideways(_) => ("mv pane", "Move pane", 3),
            Action::NewWindow => ("window", "New window", 3),
            Action::Detach => ("detach", "Detach", 2),
            Action::ReturnToTerminal if focus != WorkspaceFocus::Terminal => {
                ("terminal", "terminal", 0)
            }
            Action::TraverseChrome(Left | Right) if focus == WorkspaceFocus::Tabs => {
                ("tabs", "tabs", 1)
            }
            Action::TraverseChrome(Up | Down)
                if matches!(focus, WorkspaceFocus::Panes | WorkspaceFocus::PaneAction(_)) =>
            {
                ("panes", "panes", 1)
            }
            Action::TraverseChrome(Up | Down) if focus == WorkspaceFocus::Sidebar => {
                ("agents", "agents", 1)
            }
            _ => continue,
        };
        let axis = match shortcut.action {
            Action::Focus(_) | Action::Move(_) | Action::MovePaneSideways(_) => 0,
            Action::FocusTabPosition => 1,
            _ => 2,
        };
        let (modifiers, key) = match shortcut.trigger {
            NativeShortcutTrigger::Key(code, modifiers) => {
                (wire_modifiers(modifiers), format!("{code:?}"))
            }
            NativeShortcutTrigger::AltDigits => (eon_workspace_protocol::v38::ALT, "1..9".into()),
            _ => continue,
        };
        let modifier_names = physical_modifiers_label_for(platform, modifiers, false);
        let modifiers = physical_modifiers_label_for(platform, modifiers, true);
        let key = physical_key_label(&key);
        let accessible_label = format!(
            "{} — {}",
            shortcut.trigger.display_label_for(platform),
            shortcut.label
        );
        if let Some(index) = hints
            .iter()
            .position(|hint| hint.modifiers == modifiers && hint.label == label)
        {
            let hint = &mut hints[index];
            if axes[index] != axis {
                // A new key axis starts a dot-separated run: HJKL · 1..9.
                hint.key.push_str(" · ");
                hint.key.push_str(key);
            } else if axis == 0 {
                // Direction letters run together sorted; for H, J, K and L that order is HJKL.
                let start = hint.key.rfind(' ').map_or(0, |space| space + 1);
                let mut run = hint.key[start..]
                    .chars()
                    .chain(key.chars())
                    .collect::<Vec<_>>();
                run.sort_unstable();
                hint.key.truncate(start);
                hint.key.extend(run);
            } else {
                hint.key.push('/');
                hint.key.push_str(key);
            }
            axes[index] = axis;
            // A merged hint says what all its keys do; the arrow pairs keep their first line.
            if let Some(description) = match label {
                "focus" => Some("Focus by direction, or a tab by position"),
                "mv pane" => Some("Move pane between columns, up or down"),
                "mv tab" => Some("Move tab left or right"),
                _ => None,
            } {
                description.clone_into(&mut hint.description);
            }
            hint.accessible_label.push_str("; ");
            hint.accessible_label.push_str(&accessible_label);
            // A merged hint drops only when its most important part would.
            hint.priority = hint.priority.min(priority);
        } else {
            axes.push(axis);
            hints.push(ShortcutHint {
                modifiers,
                modifier_names,
                key: key.into(),
                label: label.into(),
                short: short.into(),
                description: shortcut.label.into(),
                section: HintSection::Actions,
                accessible_label,
                priority,
            });
        }
    }
    hints.extend(
        entries
            .iter()
            .filter(|entry| entry.id != "project")
            .filter_map(|entry| Some((entry, entry.shortcut.bound()?)))
            .map(|(entry, shortcut)| ShortcutHint {
                modifiers: physical_modifiers_label_for(platform, shortcut.modifiers, true),
                modifier_names: physical_modifiers_label_for(platform, shortcut.modifiers, false),
                key: physical_key_label(&shortcut.key).into(),
                label: entry.label.clone(),
                short: entry.label.clone(),
                description: entry.label.clone(),
                // Custom popups follow the built-ins in the runtime's order.
                section: HintSection::Popups,
                accessible_label: format!(
                    "{} — {}",
                    physical_shortcut_label_for(platform, shortcut.modifiers, &shortcut.key),
                    entry.label
                ),
                priority: if matches!(entry.id.as_str(), "git" | "agent") {
                    2
                } else {
                    4
                },
            }),
    );
    hints
}

fn physical_shortcut_label_for(platform: NativePlatform, modifiers: u8, key: &str) -> String {
    let modifiers = physical_modifiers_label_for(platform, modifiers, false);
    let key = physical_key_label(key);
    if modifiers.is_empty() {
        key.into()
    } else {
        format!("{modifiers}+{key}")
    }
}

fn physical_modifiers_label_for(platform: NativePlatform, modifiers: u8, compact: bool) -> String {
    use eon_workspace_protocol::v38 as wire;

    let mut parts = Vec::with_capacity(5);
    let labels = match (platform, compact) {
        (NativePlatform::Linux, true) => ["C", "A", "S", "Super"],
        (NativePlatform::Macos, true) => ["C", "A", "S", "Cmd"],
        (NativePlatform::Linux, false) => ["Ctrl", "Alt", "Shift", "Super"],
        (NativePlatform::Macos, false) => ["Control", "Option", "Shift", "Command"],
    };
    for (bit, label) in [wire::CTRL, wire::ALT, wire::SHIFT, wire::SUPER]
        .into_iter()
        .zip(labels)
    {
        if modifiers & bit != 0 {
            parts.push(label);
        }
    }
    parts.join(if compact { "" } else { "+" })
}

fn physical_key_label(key: &str) -> &str {
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

fn wire_modifiers(modifiers: session::Modifiers) -> u8 {
    use eon_workspace_protocol::v38 as wire;

    [
        (session::Modifiers::SHIFT, wire::SHIFT),
        (session::Modifiers::CTRL, wire::CTRL),
        (session::Modifiers::ALT, wire::ALT),
        (session::Modifiers::SUPER, wire::SUPER),
    ]
    .into_iter()
    .fold(0, |bits, (native, shared)| {
        bits | if modifiers.contains(native) {
            shared
        } else {
            0
        }
    })
}

pub(super) fn workspace_shortcut(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
    entries: &[PopupEntry],
    active_tab: &str,
    selected_pane: Option<&str>,
    sidebar: Sidebar,
) -> Option<WorkspaceAction> {
    // Quit is no workspace action, so its binding does not matter here.
    let shortcut = native_key_shortcut(PhysicalKey::Code(code), modifiers, entries)
        .filter(|shortcut| shortcut.bound(sidebar, false))?;
    let action = match shortcut.action {
        NativeShortcutAction::Focus(direction) => CommonAction::Focus(direction),
        NativeShortcutAction::Move(direction) => CommonAction::Move(direction),
        NativeShortcutAction::CreatePane => CommonAction::CreatePane { focus: true },
        NativeShortcutAction::CreateColumn => return Some(WorkspaceAction::CreateColumn),
        NativeShortcutAction::MovePaneSideways(side) => {
            return Some(WorkspaceAction::MovePaneSideways(side));
        }
        NativeShortcutAction::CreateTab => CommonAction::CreateTab,
        NativeShortcutAction::CloseTab => CommonAction::CloseTab {
            tab: active_tab.into(),
        },
        NativeShortcutAction::ClosePane => {
            return selected_pane.map(|pane| WorkspaceAction::ClosePane { pane: pane.into() });
        }
        NativeShortcutAction::ToggleSidebar => return Some(WorkspaceAction::ToggleSidebar),
        NativeShortcutAction::NextAttention => return Some(WorkspaceAction::NextAttention),
        NativeShortcutAction::PreviousAttention => {
            return Some(WorkspaceAction::PreviousAttention);
        }
        _ => return None,
    };
    Some(WorkspaceAction::Workspace(action))
}

/// Whether a sideways move would need a new column at that edge. Eon owns the cap.
pub(super) fn sideways_move_opens_column(tab: &Tab, side: Side) -> bool {
    if tab.selected_popup.is_some() {
        return false;
    }
    let column = match side {
        Side::Left => tab.columns.first(),
        Side::Right => tab.columns.last(),
    };
    column.is_some_and(|column| {
        column.panes.len() > 1 && tab.selected_pane.as_ref() == Some(&column.expanded)
    })
}

/// The tab a tab-selection key selects, resolved from the accepted snapshot. `Some(None)`
/// consumes the key without an action: a missing position, or the next tab of a lone tab.
pub(super) fn tab_shortcut_target(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
    snapshot: &Snapshot,
) -> Option<Option<&Tab>> {
    let shortcut = native_key_shortcut(PhysicalKey::Code(code), modifiers, &snapshot.entries)?;
    let index = match shortcut.action {
        NativeShortcutAction::FocusTabPosition => digit_position(code)?,
        NativeShortcutAction::NextTab | NativeShortcutAction::PreviousTab => {
            let count = snapshot.tabs.len();
            let Some(active) = snapshot
                .tabs
                .iter()
                .position(|tab| tab.id == snapshot.active_tab)
            else {
                return Some(None);
            };
            let step = if shortcut.action == NativeShortcutAction::NextTab {
                1
            } else {
                count - 1
            };
            let index = (active + step) % count;
            if index == active {
                return Some(None);
            }
            index
        }
        _ => return None,
    };
    Some(snapshot.tabs.get(index))
}

/// The action a tab or workspace position key takes, resolved from the accepted snapshot: a tab
/// in the tab order, or a workspace in the window's workspace order (VEN-C8). `Some(None)`
/// consumes the key without an action: a missing position, the shown workspace's own, or the
/// next tab of a lone tab. `None` leaves the key to popups, other bindings and the terminal, as
/// `number_keys = false` does for the workspace position keys.
pub(super) fn position_key_action(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
    snapshot: &Snapshot,
    number_keys: bool,
) -> Option<Option<WorkspaceAction>> {
    if let Some(tab) = tab_shortcut_target(code, modifiers, snapshot) {
        return Some(
            tab.map(|tab| WorkspaceAction::Workspace(CommonAction::FocusId(tab.id.clone()))),
        );
    }
    let shortcut = native_key_shortcut(PhysicalKey::Code(code), modifiers, &snapshot.entries)?;
    if !number_keys || shortcut.action != NativeShortcutAction::ShowWorkspacePosition {
        return None;
    }
    let workspace = snapshot
        .workspaces
        .get(digit_position(code)?)
        .filter(|workspace| workspace.id != snapshot.active_workspace);
    Some(workspace.map(|workspace| {
        WorkspaceAction::Workspace(CommonAction::SwitchWorkspace {
            workspace: eon_workspace_protocol::v38::WorkspaceTarget::Selector(workspace.id.clone()),
        })
    }))
}

pub(super) fn popup_shortcut(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
    snapshot: &Snapshot,
    terminal_focused: bool,
) -> Option<WorkspaceAction> {
    // A popup opens only on a key no fixed binding keeps, so a reserved key keeps
    // its binding even when a snapshot binds it (VEN-C20).
    if native_key_shortcut(PhysicalKey::Code(code), modifiers, &snapshot.entries).is_some() {
        return None;
    }
    let entry = popup_entry(&snapshot.entries, code, modifiers)?;
    invoke_popup(snapshot, &entry.id, terminal_focused)
}

/// The popup entry the Eon Bar's workspace label opens (VEN-C21).
pub(super) const WORKSPACES_ENTRY: &str = "sessions";

/// The workspace label's tooltip: its popup entry's label and any chord, while Eon configures it.
pub(super) fn workspace_picker_tooltip(entries: &[PopupEntry]) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.id == WORKSPACES_ENTRY)
        .map(|entry| match entry.shortcut.bound() {
            Some(shortcut) => format!(
                "{} — {}",
                entry.label,
                physical_shortcut_label_for(NATIVE_PLATFORM, shortcut.modifiers, &shortcut.key)
            ),
            None => entry.label.clone(),
        })
}

/// Invokes the configured popup `entry` in the active tab as its key does: toggling it while
/// the terminal has focus, else focusing it.
pub(super) fn invoke_popup(
    snapshot: &Snapshot,
    entry: &str,
    terminal_focused: bool,
) -> Option<WorkspaceAction> {
    let entry = snapshot.entries.iter().find(|known| known.id == entry)?;
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)?;
    // Project names its tab's Files pair, as the runtime checks it (EON-C18).
    let instance = eon_workspace_protocol::v38::entry_popup(tab, &entry.id);
    Some(WorkspaceAction::InvokePopup {
        tab: tab.id.clone(),
        entry: entry.id.clone(),
        expected_instance: instance.map(|popup| popup.id.clone()),
        intent: if terminal_focused {
            InvokeIntent::Toggle
        } else {
            InvokeIntent::Focus
        },
    })
}

fn popup_entry(
    entries: &[PopupEntry],
    code: KeyCode,
    modifiers: session::Modifiers,
) -> Option<&PopupEntry> {
    let modifiers = wire_modifiers(modifiers);
    // winit's physical KeyCode names are the canonical names in EONW v9.
    let key = format!("{code:?}");
    entries.iter().find(|entry| {
        entry
            .shortcut
            .bound()
            .is_some_and(|shortcut| shortcut.modifiers == modifiers && shortcut.key == key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v38::WorkspaceInfo;

    /// Eon's default popup catalog, copied from eon/crates/eon/src/product.rs, which this crate
    /// cannot depend on; keep the two in step.
    fn default_entries() -> [PopupEntry; 8] {
        use eon_workspace_protocol::v38::{ALT, SHIFT, Shortcut};

        [
            ("project", "Quick search", "KeyZ", ALT),
            ("git", "Git", "KeyJ", ALT | SHIFT),
            ("agent", "Agent", "KeyL", ALT | SHIFT),
            ("files", "Files", "KeyY", ALT | SHIFT),
            ("anima", "Anima", "KeyA", ALT | SHIFT),
            ("commands", "Commands", "KeyM", ALT | SHIFT),
            ("sessions", "Sessions", "KeyS", ALT | SHIFT),
            ("scrollback", "Scrollback", "KeyE", ALT | SHIFT),
        ]
        .map(|(id, label, key, modifiers)| PopupEntry {
            id: id.into(),
            label: label.into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers,
                key: key.into(),
            }),
        })
    }

    #[test]
    fn keys_popup_lists_actions_then_popups_by_short_label() {
        use eon_workspace_protocol::v38::{ALT, CTRL, Shortcut};

        // A custom popup follows the built-ins under Popups, in the runtime's order.
        let entries = default_entries().into_iter().chain([PopupEntry {
            id: "monitor".into(),
            label: "Monitor".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: CTRL | ALT,
                key: "KeyO".into(),
            }),
        }]);
        let hints = shortcut_hints_for(
            NativePlatform::Linux,
            WorkspaceFocus::Terminal,
            &entries.collect::<Vec<_>>(),
            Sidebar::Open,
            true,
        );
        let group = |names: &str| {
            hints
                .iter()
                .filter(|hint| hint.modifier_names == names)
                .map(|hint| (hint.key.as_str(), hint.short.as_str(), hint.section))
                .collect::<Vec<_>>()
        };
        use HintSection::{Actions, Popups};
        // Actions by what they act on: tab, pane, window, then the chrome toggles.
        assert_eq!(
            group("Alt+Shift"),
            [
                ("T", "New tab", Actions),
                ("W", "Close tab", Actions),
                ("Z", "Zoom pane", Actions),
                ("N", "New window", Actions),
                ("D", "Detach", Actions),
                ("B", "Hints", Actions),
                ("H", "Sidebar", Actions),
                ("J", "Git", Popups),
                ("L", "Agent", Popups),
                ("Y", "Files", Popups),
                ("A", "Anima", Popups),
                ("M", "Commands", Popups),
                ("S", "Sessions", Popups),
                ("E", "Scrollback", Popups),
            ]
        );
        assert_eq!(
            group("Alt"),
            [
                ("HJKL · 1..9", "Focus", Actions),
                ("M", "New pane", Actions),
                ("W", "Close pane", Actions),
            ]
        );
        assert_eq!(
            group("Ctrl+Alt"),
            [
                ("M", "New column", Actions),
                ("HJKL", "Move pane", Actions),
                ("O", "Monitor", Popups),
            ]
        );
        assert_eq!(group("Ctrl+Alt+Shift"), [("HL", "Move tab", Actions)]);
        // Full help keeps its sentence, which the popup shows on hover and screen readers read.
        let detach = hints.iter().find(|hint| hint.short == "Detach").unwrap();
        assert_eq!(
            detach.description,
            "Detach: hide the window, keep terminals running"
        );
        assert!(detach.accessible_label.ends_with(&detach.description));
    }

    #[test]
    fn default_hints_fit_a_1280_pixel_window() {
        use yazelix_venus::{FontSettings, FontSetup, ShortcutHintsScene};

        let entries = default_entries();
        // Eon's packaged sans face, the exact file its Nix build and dev shell name, which hosts
        // without their own fontconfig use. A host's sans-serif alias, such as Open Sans, may set
        // hints narrower; another copy of DejaVu Sans may differ.
        let font = std::env::var_os("EON_VENUS_TEST_SANS_FONT").expect(
            "EON_VENUS_TEST_SANS_FONT names Eon's packaged DejaVu Sans; run in nix develop",
        );
        let mut fonts = FontSetup::new(&FontSettings::default()).unwrap();
        fonts
            .set_header_font_file(std::path::Path::new(&font))
            .unwrap();
        // On a 13-inch 2560x1600 panel at scale 2, the bar keeps every keycap and expanded hint
        // centred, clear of the clock and the version label, while the terminal or the tabs have
        // focus.
        for focus in [WorkspaceFocus::Terminal, WorkspaceFocus::Tabs] {
            let hints =
                shortcut_hints_for(NativePlatform::Linux, focus, &entries, Sidebar::Open, true);
            for scale in [1.0, 2.0] {
                let metrics = fonts.metrics(scale);
                let size =
                    winit::dpi::PhysicalSize::new((1280.0 * scale) as u32, (800.0 * scale) as u32);
                let scene = ShortcutHintsScene::new(
                    hints.clone(),
                    size,
                    metrics,
                    Some("EON 0.1.18".len() as f32 * metrics.width),
                    Some("00:00".len() as f32 * metrics.width + metrics.padding * 2.0),
                    |text| fonts.measure_header_text(metrics, text),
                );
                assert!(scene.complete, "{focus:?} at scale {scale}");
            }
        }
    }

    #[test]
    fn direct_workspace_shortcuts_use_stable_eon_actions() {
        use orbit_protocol::session::Modifiers;

        for (key, modifiers, action) in [
            (
                KeyCode::KeyH,
                Modifiers::ALT,
                CommonAction::Focus(WorkspaceDirection::Left),
            ),
            (
                KeyCode::KeyL,
                Modifiers::ALT,
                CommonAction::Focus(WorkspaceDirection::Right),
            ),
            (
                KeyCode::KeyK,
                Modifiers::ALT,
                CommonAction::Focus(WorkspaceDirection::Up),
            ),
            (
                KeyCode::KeyJ,
                Modifiers::ALT,
                CommonAction::Focus(WorkspaceDirection::Down),
            ),
            (
                KeyCode::KeyM,
                Modifiers::ALT,
                CommonAction::CreatePane { focus: true },
            ),
            (
                KeyCode::KeyT,
                Modifiers::ALT.union(Modifiers::SHIFT),
                CommonAction::CreateTab,
            ),
            (
                KeyCode::KeyH,
                Modifiers::CTRL
                    .union(Modifiers::ALT)
                    .union(Modifiers::SHIFT),
                CommonAction::Move(WorkspaceDirection::Left),
            ),
            (
                KeyCode::KeyL,
                Modifiers::CTRL
                    .union(Modifiers::ALT)
                    .union(Modifiers::SHIFT),
                CommonAction::Move(WorkspaceDirection::Right),
            ),
            (
                KeyCode::KeyK,
                Modifiers::CTRL.union(Modifiers::ALT),
                CommonAction::Move(WorkspaceDirection::Up),
            ),
            (
                KeyCode::KeyJ,
                Modifiers::CTRL.union(Modifiers::ALT),
                CommonAction::Move(WorkspaceDirection::Down),
            ),
            (
                KeyCode::KeyW,
                Modifiers::ALT.union(Modifiers::SHIFT),
                CommonAction::CloseTab { tab: "t2".into() },
            ),
        ] {
            assert_eq!(
                workspace_shortcut(key, modifiers, &[], "t2", Some("p3"), Sidebar::Open),
                Some(WorkspaceAction::Workspace(action))
            );
        }
        assert_eq!(
            workspace_shortcut(
                KeyCode::KeyM,
                Modifiers::CTRL.union(Modifiers::ALT),
                &[],
                "t2",
                Some("p3"),
                Sidebar::Open
            ),
            Some(WorkspaceAction::CreateColumn)
        );
        for (key, side) in [(KeyCode::KeyH, Side::Left), (KeyCode::KeyL, Side::Right)] {
            assert_eq!(
                workspace_shortcut(
                    key,
                    Modifiers::CTRL.union(Modifiers::ALT),
                    &[],
                    "t2",
                    Some("p3"),
                    Sidebar::Open
                ),
                Some(WorkspaceAction::MovePaneSideways(side))
            );
        }
        assert_eq!(
            workspace_shortcut(
                KeyCode::KeyW,
                Modifiers::ALT,
                &[],
                "t2",
                Some("p3"),
                Sidebar::Open
            ),
            Some(WorkspaceAction::ClosePane { pane: "p3".into() })
        );
        assert_eq!(
            workspace_shortcut(
                KeyCode::KeyW,
                Modifiers::ALT,
                &[],
                "t2",
                None,
                Sidebar::Open
            ),
            None
        );
        for (key, modifiers) in [KeyCode::KeyT, KeyCode::KeyW]
            .into_iter()
            .flat_map(|key| {
                [
                    Modifiers::CTRL,
                    Modifiers::ALT,
                    Modifiers::CTRL.union(Modifiers::SHIFT),
                    Modifiers::CTRL
                        .union(Modifiers::ALT)
                        .union(Modifiers::SHIFT),
                ]
                .map(|modifiers| (key, modifiers))
            })
            .filter(|&(key, modifiers)| (key, modifiers) != (KeyCode::KeyW, Modifiers::ALT))
        {
            assert_eq!(
                workspace_shortcut(key, modifiers, &[], "t2", Some("p3"), Sidebar::Open),
                None
            );
        }
        // Alt+Shift+H toggles the sidebar and is unbound under `Off`.
        for (sidebar, action) in [
            (Sidebar::Open, Some(WorkspaceAction::ToggleSidebar)),
            (Sidebar::Closed, Some(WorkspaceAction::ToggleSidebar)),
            (Sidebar::Off, None),
        ] {
            assert_eq!(
                workspace_shortcut(
                    KeyCode::KeyH,
                    Modifiers::ALT.union(Modifiers::SHIFT),
                    &[],
                    "t2",
                    Some("p3"),
                    sidebar,
                ),
                action
            );
            assert_eq!(
                native_key_shortcut(PhysicalKey::Code(KeyCode::KeyQ), Modifiers::ALT, &[])
                    .map(|shortcut| shortcut.action),
                Some(NativeShortcutAction::ToggleInbox)
            );
            assert!(
                FIXED_SHORTCUTS
                    .iter()
                    .find(|shortcut| shortcut.action == NativeShortcutAction::ToggleInbox)
                    .unwrap()
                    .bound(sidebar, true)
            );
            // Alt+G and Alt+Shift+G walk the attention queue under every `sidebar`, `Off` too.
            for (modifiers, action) in [
                (Modifiers::ALT, WorkspaceAction::NextAttention),
                (
                    Modifiers::ALT.union(Modifiers::SHIFT),
                    WorkspaceAction::PreviousAttention,
                ),
            ] {
                assert_eq!(
                    workspace_shortcut(KeyCode::KeyG, modifiers, &[], "t2", None, sidebar),
                    Some(action)
                );
            }
        }
    }

    #[test]
    fn sideways_moves_open_a_column_only_for_a_stacked_outer_pane() {
        use eon_workspace_protocol::v38::{Column, Pane};

        let pane = |id: &str| Pane {
            label: id.into(),
            pinned_name: None,
            id: id.into(),
            session: format!("terminal-{id}"),
            endpoint: Vec::new(),
            live: true,
            attention: None,
        };
        let column = |ids: &[&str]| Column {
            panes: ids.iter().map(|id| pane(id)).collect(),
            expanded: ids[0].into(),
        };
        let tab = |columns, selected: &str| Tab {
            id: "t1".into(),
            directory: b"/".to_vec(),
            pending: false,
            selected_pane: Some(selected.into()),
            selected_popup: None,
            columns,
            popups: Vec::new(),
            attention_rollup: None,
        };
        let stacked = tab(vec![column(&["p1", "p2"]), column(&["p3"])], "p1");
        assert!(sideways_move_opens_column(&stacked, Side::Left));
        assert!(!sideways_move_opens_column(&stacked, Side::Right));
        let lone = tab(vec![column(&["p1"]), column(&["p2", "p3"])], "p1");
        assert!(!sideways_move_opens_column(&lone, Side::Left));
        let mut outer = tab(vec![column(&["p1"]), column(&["p2", "p3"])], "p3");
        outer.columns[1].expanded = "p3".into();
        assert!(sideways_move_opens_column(&outer, Side::Right));
        outer.selected_popup = Some("u1".into());
        assert!(!sideways_move_opens_column(&outer, Side::Right));
    }

    #[test]
    fn shortcut_hints_follow_bindings_catalog_and_native_focus() {
        let entry = PopupEntry {
            id: "agent".into(),
            label: "My agent".into(),
            shortcut: eon_workspace_protocol::v38::PopupKey::Bound(
                eon_workspace_protocol::v38::Shortcut {
                    modifiers: eon_workspace_protocol::v38::SUPER,
                    key: "KeyF".into(),
                },
            ),
        };
        let project = PopupEntry {
            id: "project".into(),
            label: "Folder chooser".into(),
            shortcut: eon_workspace_protocol::v38::PopupKey::Bound(
                eon_workspace_protocol::v38::Shortcut {
                    modifiers: eon_workspace_protocol::v38::ALT,
                    key: "KeyZ".into(),
                },
            ),
        };
        let entries = [entry.clone(), project.clone()];
        for platform in [NativePlatform::Linux, NativePlatform::Macos] {
            let terminal = shortcut_hints_for(
                platform,
                WorkspaceFocus::Terminal,
                &entries,
                Sidebar::Open,
                true,
            );
            assert!(terminal.iter().all(|hint| hint.label != project.label));
            // The viewer and hints list the sidebar toggle unless `[sidebar]` turned it off.
            for sidebar in [Sidebar::Open, Sidebar::Closed, Sidebar::Off] {
                let label = "Show or hide the agent sidebar";
                let bound = sidebar != Sidebar::Off;
                assert_eq!(
                    shortcut_groups_for(platform, &entries, sidebar, true)
                        .iter()
                        .flat_map(|group| &group.rows)
                        .any(|row| row.action == label),
                    bound
                );
                assert_eq!(
                    shortcut_hints_for(platform, WorkspaceFocus::Terminal, &entries, sidebar, true)
                        .iter()
                        .any(|hint| hint.label == "side" && hint.accessible_label.contains(label)),
                    bound
                );
                // The inbox and agent jumps stay in the viewer but leave the footer to save width.
                for label in [
                    "Show the attention inbox",
                    "Jump to the next agent needing attention",
                    "Jump to the previous agent needing attention",
                ] {
                    assert!(
                        shortcut_groups_for(platform, &entries, sidebar, true)
                            .iter()
                            .flat_map(|group| &group.rows)
                            .any(|row| row.action == label)
                    );
                    assert!(
                        shortcut_hints_for(
                            platform,
                            WorkspaceFocus::Terminal,
                            &entries,
                            sidebar,
                            true,
                        )
                        .iter()
                        .all(|hint| !hint.accessible_label.contains(label))
                    );
                }
            }
            // Full help keeps these keys; the footer leaves them out to fit a 1280-pixel window.
            for action in [
                NativeShortcutAction::Copy,
                NativeShortcutAction::Paste,
                NativeShortcutAction::OpenLink,
                NativeShortcutAction::ToggleViewer,
                NativeShortcutAction::Quit,
            ] {
                let shortcut = FIXED_SHORTCUTS
                    .iter()
                    .find(|item| item.action == action)
                    .unwrap();
                assert!(
                    terminal
                        .iter()
                        .all(|hint| !hint.accessible_label.contains(shortcut.label))
                );
                assert!(
                    shortcut_groups_for(platform, &entries, Sidebar::Open, true)
                        .iter()
                        .flat_map(|group| &group.rows)
                        .any(|row| {
                            row.action == shortcut.label
                                && row.shortcut == shortcut.trigger.display_label_for(platform)
                        })
                );
            }
            assert!(
                shortcut_groups_for(platform, &entries, Sidebar::Open, true)
                    .iter()
                    .flat_map(|group| &group.rows)
                    .any(|row| row.action == project.label
                        && row.shortcut
                            == physical_shortcut_label_for(
                                platform,
                                eon_workspace_protocol::v38::ALT,
                                "KeyZ",
                            ))
            );
            let agent = terminal
                .iter()
                .find(|hint| hint.label == entry.label)
                .unwrap();
            assert_eq!(agent.key, "F");
            assert_eq!(
                agent.modifiers,
                match platform {
                    NativePlatform::Linux => "Super",
                    NativePlatform::Macos => "Cmd",
                }
            );
            assert_eq!(
                agent.accessible_label,
                format!(
                    "{} — {}",
                    physical_shortcut_label_for(
                        platform,
                        eon_workspace_protocol::v38::SUPER,
                        "KeyF"
                    ),
                    entry.label,
                )
            );
            let hints_labeled = |label| {
                terminal
                    .iter()
                    .filter(|hint| hint.label == label)
                    .collect::<Vec<_>>()
            };
            let panes = hints_labeled("mv pane");
            assert_eq!(panes.len(), 1);
            assert_eq!(panes[0].modifiers, "CA");
            assert_eq!(panes[0].key, "HJKL");
            // One focus hint, kept as long as its most important part.
            let focus = hints_labeled("focus");
            assert_eq!(focus.len(), 1);
            assert_eq!(focus[0].modifiers, "A");
            assert_eq!(focus[0].key, "HJKL · 1..9");
            assert_eq!(focus[0].priority, 1);
            assert!(focus[0].accessible_label.contains("Focus tab by position"));
            // A popup entry on Alt+K, reserved for focus, leaves the hint whole.
            let alt_k = PopupEntry {
                id: "k".into(),
                label: "K tool".into(),
                shortcut: eon_workspace_protocol::v38::PopupKey::Bound(
                    eon_workspace_protocol::v38::Shortcut {
                        modifiers: eon_workspace_protocol::v38::ALT,
                        key: "KeyK".into(),
                    },
                ),
            };
            assert!(
                shortcut_hints_for(
                    platform,
                    WorkspaceFocus::Terminal,
                    &[alt_k],
                    Sidebar::Open,
                    true
                )
                .iter()
                .any(|hint| hint.label == "focus" && hint.key == "HJKL · 1..9")
            );
            for label in ["pane", "col", "tab", "window"] {
                assert_eq!(hints_labeled(label).len(), 1);
            }
            let mut labels = terminal.iter().map(|hint| &hint.label).collect::<Vec<_>>();
            labels.sort();
            labels.dedup();
            assert_eq!(labels.len(), terminal.len());
            assert!(terminal.iter().all(|hint| {
                !hint
                    .accessible_label
                    .contains("Return focus to the terminal")
                    && !hint.key.contains("Left")
            }));
            assert!(
                shortcut_hints_for(platform, WorkspaceFocus::Terminal, &[], Sidebar::Open, true)
                    .iter()
                    .all(|hint| hint.label != entry.label)
            );
            for focus in [
                WorkspaceFocus::Tabs,
                WorkspaceFocus::Panes,
                WorkspaceFocus::Sidebar,
            ] {
                let chrome = shortcut_hints_for(platform, focus, &[], Sidebar::Open, true);
                assert!(
                    chrome
                        .iter()
                        .any(|hint| hint.key == "Escape" && hint.label == "terminal")
                );
                assert!(
                    chrome
                        .iter()
                        .all(|hint| !matches!(hint.label.as_str(), "copy" | "paste" | "link"))
                );
                assert_eq!(
                    chrome.iter().any(|hint| hint.key == "Left/Right"),
                    focus == WorkspaceFocus::Tabs
                );
                // Up/Down walk panes or, in the sidebar, its rows.
                assert_eq!(
                    chrome
                        .iter()
                        .find(|hint| hint.key == "Up/Down")
                        .map(|hint| hint.label.as_str()),
                    match focus {
                        WorkspaceFocus::Panes => Some("panes"),
                        WorkspaceFocus::Sidebar => Some("agents"),
                        _ => None,
                    }
                );
            }
            for shortcut in FIXED_SHORTCUTS.iter().filter(|shortcut| {
                !matches!(
                    shortcut.action,
                    NativeShortcutAction::CycleFocus
                        | NativeShortcutAction::NextTab
                        | NativeShortcutAction::PreviousTab
                        | NativeShortcutAction::ReturnToTerminal
                        | NativeShortcutAction::TraverseChrome(_)
                        | NativeShortcutAction::Copy
                        | NativeShortcutAction::Paste
                        | NativeShortcutAction::OpenLink
                        | NativeShortcutAction::NextAttention
                        | NativeShortcutAction::PreviousAttention
                        | NativeShortcutAction::ShowWorkspacePosition
                        | NativeShortcutAction::ToggleViewer
                        | NativeShortcutAction::ToggleInbox
                        | NativeShortcutAction::Quit
                )
            }) {
                let chord = shortcut.trigger.display_label_for(platform);
                assert!(
                    terminal.iter().any(|hint| hint
                        .accessible_label
                        .contains(&format!("{chord} — {}", shortcut.label))),
                    "missing {chord}"
                );
            }
        }
    }

    #[test]
    fn hints_carry_full_names_and_full_help_descriptions() {
        use eon_workspace_protocol::v38::{ALT, SHIFT, Shortcut};

        let scrollback = PopupEntry {
            id: "scrollback".into(),
            label: "Scrollback".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyE".into(),
            }),
        };
        for platform in [NativePlatform::Linux, NativePlatform::Macos] {
            let hints = shortcut_hints_for(
                platform,
                WorkspaceFocus::Tabs,
                std::slice::from_ref(&scrollback),
                Sidebar::Open,
                true,
            );
            assert!(hints.iter().all(|hint| !hint.description.is_empty()));
            for hint in &hints {
                // The full names are the chord's modifiers, as full help writes them.
                assert!(
                    hint.modifiers.is_empty()
                        || hint
                            .accessible_label
                            .starts_with(&format!("{}+", hint.modifier_names)),
                    "{hint:?}"
                );
            }
            let described = |label: &str| {
                hints
                    .iter()
                    .find(|hint| hint.label == label)
                    .map(|hint| hint.description.as_str())
            };
            assert_eq!(
                described("focus"),
                Some("Focus by direction, or a tab by position")
            );
            assert_eq!(
                described("mv pane"),
                Some("Move pane between columns, up or down")
            );
            assert_eq!(described("tab"), Some("Create tab"));
            // A popup is described by its configured name, which the hint also shows.
            assert_eq!(described("Scrollback"), Some("Scrollback"));
        }
        let modifiers = |platform| {
            shortcut_hints_for(platform, WorkspaceFocus::Terminal, &[], Sidebar::Open, true)
                .iter()
                .fold(Vec::new(), |mut names, hint| {
                    if !names.contains(&hint.modifier_names) {
                        names.push(hint.modifier_names.clone());
                    }
                    names
                })
        };
        assert_eq!(
            modifiers(NativePlatform::Linux),
            ["Alt", "Ctrl+Alt", "Alt+Shift", "Ctrl+Alt+Shift"]
        );
        assert_eq!(
            modifiers(NativePlatform::Macos),
            [
                "Option",
                "Control+Option",
                "Option+Shift",
                "Control+Option+Shift"
            ]
        );
    }

    #[test]
    fn escape_or_a_press_hides_a_keycap_popup_until_the_pointer_leaves() {
        let mut latch = HintPopupLatch::default();
        assert!(!latch.dismiss(), "nothing to hide yet");
        assert!(latch.popup(Some(2)));
        assert_eq!(latch.shown(), Some(2));
        // Escape (or a press) hides it, and it stays hidden while the pointer rests there.
        assert!(latch.dismiss());
        assert_eq!(latch.shown(), None);
        assert!(!latch.popup(Some(2)));
        assert!(!latch.popup(Some(2)));
        assert!(!latch.dismiss(), "already hidden");
        // Leaving the keycap and coming back shows it again.
        assert!(!latch.popup(None));
        assert!(latch.popup(Some(2)));
        // A press hides it; moving straight to another keycap shows that one's popup.
        assert!(latch.dismiss());
        assert!(!latch.popup(Some(2)));
        assert!(latch.popup(Some(3)));
        assert!(
            latch.popup(Some(2)),
            "the dismissal ended when the pointer left"
        );
    }

    #[test]
    fn ctrl_alt_shift_group_holds_the_tab_moves() {
        use eon_workspace_protocol::v38::{ALT, CTRL, SHIFT, Shortcut};

        let tabs = |entries: &[PopupEntry]| {
            shortcut_hints_for(
                NativePlatform::Linux,
                WorkspaceFocus::Terminal,
                entries,
                Sidebar::Open,
                true,
            )
            .into_iter()
            .filter(|hint| hint.modifiers == "CAS")
            .collect::<Vec<_>>()
        };
        let moves = tabs(&[]);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].key, "HL");
        assert_eq!(moves[0].label, "mv tab");
        assert_eq!(moves[0].description, "Move tab left or right");
        assert_eq!(
            moves[0].accessible_label,
            "Ctrl+Alt+Shift+H — Move tab left; Ctrl+Alt+Shift+L — Move tab right"
        );
        // A popup configured on Ctrl+Alt+Shift+H takes that key and leaves L.
        let popup = PopupEntry {
            id: "notes".into(),
            label: "Notes".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: CTRL | ALT | SHIFT,
                key: "KeyH".into(),
            }),
        };
        let keys = tabs(&[popup])
            .into_iter()
            .map(|hint| (hint.key, hint.label))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                ("L".to_owned(), "mv tab".to_owned()),
                ("H".to_owned(), "Notes".to_owned())
            ]
        );
    }

    #[test]
    fn configured_popups_take_fixed_bindings_except_reserved_keys() {
        use eon_workspace_protocol::v38::{ALT, CTRL, SHIFT, Shortcut};

        let popup = |id: &str, modifiers, key: &str| PopupEntry {
            id: id.into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers,
                key: key.into(),
            }),
        };
        let new_window = PhysicalKey::Code(KeyCode::KeyN);
        for platform in [NativePlatform::Linux, NativePlatform::Macos] {
            let copy = copy_paste_modifiers(platform);
            let entries = [
                popup("files", ALT | SHIFT, "KeyN"),
                popup("clip", wire_modifiers(copy), "KeyC"),
                popup("hints", ALT | SHIFT, "KeyB"),
                popup("closer", ALT, "KeyW"),
                popup("git", ALT, "KeyG"),
                popup("inbox", ALT, "KeyQ"),
                popup("next", CTRL, "Tab"),
                // Takes Alt+D only; Alt+Shift+D still detaches.
                popup("dir", ALT, "KeyD"),
                // Runtime rejects these; Venus still keeps the reserved bindings.
                popup("viewer", ALT, "Slash"),
                popup("third", ALT, "Digit3"),
                popup("up", ALT, "KeyK"),
            ];
            assert_eq!(
                native_key_shortcut_for(platform, new_window, ALT_SHIFT, &[])
                    .map(|shortcut| shortcut.action),
                Some(NativeShortcutAction::NewWindow)
            );
            for (key, modifiers) in [
                (new_window, ALT_SHIFT),
                (PhysicalKey::Code(KeyCode::KeyC), copy),
                (PhysicalKey::Code(KeyCode::KeyB), ALT_SHIFT),
                (PhysicalKey::Code(KeyCode::KeyG), session::Modifiers::ALT),
                (PhysicalKey::Code(KeyCode::KeyQ), session::Modifiers::ALT),
                (PhysicalKey::Code(KeyCode::Tab), session::Modifiers::CTRL),
            ] {
                assert!(native_key_shortcut_for(platform, key, modifiers, &entries).is_none());
            }
            // The link hint stops advertising a Copy key the popup takes.
            let link = |entries: &[PopupEntry]| super::super::link_hint_for(platform, "u", entries);
            assert!(link(&[]).ends_with(" Copy"));
            assert!(!link(&entries).contains("Copy"));
            for (key, action) in [
                (KeyCode::Slash, NativeShortcutAction::ToggleViewer),
                (KeyCode::Digit3, NativeShortcutAction::FocusTabPosition),
                (
                    KeyCode::KeyK,
                    NativeShortcutAction::Focus(WorkspaceDirection::Up),
                ),
            ] {
                assert_eq!(
                    native_key_shortcut_for(
                        platform,
                        PhysicalKey::Code(key),
                        session::Modifiers::ALT,
                        &entries
                    )
                    .map(|shortcut| shortcut.action),
                    Some(action)
                );
            }

            let rows = shortcut_groups_for(platform, &entries, Sidebar::Open, true)
                .into_iter()
                .flat_map(|group| group.rows)
                .map(|row| row.action)
                .collect::<Vec<_>>();
            for action in [
                "Open new Eon window (overridden by Files)",
                "Copy selected text or focused link (overridden by Files)",
                "Show or close shortcuts",
                "Focus tab by position",
                "Focus pane above",
                "Paste",
                "Jump to the next agent needing attention (overridden by Files)",
                "Show the attention inbox (overridden by Files)",
                "Jump to the previous agent needing attention",
                "Focus next tab (overridden by Files)",
                "Focus previous tab",
            ] {
                assert!(rows.iter().any(|row| row == action), "missing {action}");
            }
            let hints = shortcut_hints_for(
                platform,
                WorkspaceFocus::Terminal,
                &entries,
                Sidebar::Open,
                true,
            );
            for taken in [
                "Show the attention inbox",
                "Open new Eon window",
                "Show or hide shortcut hints",
            ] {
                assert!(
                    hints
                        .iter()
                        .all(|hint| !hint.accessible_label.contains(taken))
                );
            }
            for kept in [
                "Focus tab by position",
                "Detach: hide the window, keep terminals running",
            ] {
                assert!(
                    hints
                        .iter()
                        .any(|hint| hint.accessible_label.contains(kept))
                );
            }
        }

        // Dispatch tries the popup first, and the fixed Alt+W no longer resolves.
        let entries = vec![popup("closer", ALT, "KeyW")];
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v38::WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: eon_workspace_protocol::v38::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: entries.clone(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/".to_vec(),
                pending: false,
                selected_pane: Some("p1".into()),
                selected_popup: None,
                columns: Vec::new(),
                popups: Vec::new(),
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v38::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        assert!(matches!(
            popup_shortcut(KeyCode::KeyW, session::Modifiers::ALT, &snapshot, true),
            Some(WorkspaceAction::InvokePopup { entry, .. }) if entry == "closer"
        ));
        assert_eq!(
            workspace_shortcut(
                KeyCode::KeyW,
                session::Modifiers::ALT,
                &entries,
                "t1",
                Some("p1"),
                Sidebar::Open
            ),
            None
        );

        // A popup entry on a focus key never dispatches; Alt+K still moves focus.
        let entries = vec![popup("up", ALT, "KeyK")];
        let snapshot = Snapshot {
            entries: entries.clone(),
            ..snapshot
        };
        assert_eq!(
            popup_shortcut(KeyCode::KeyK, session::Modifiers::ALT, &snapshot, true),
            None
        );
        assert!(matches!(
            workspace_shortcut(
                KeyCode::KeyK,
                session::Modifiers::ALT,
                &entries,
                "t1",
                Some("p1"),
                Sidebar::Open
            ),
            Some(WorkspaceAction::Workspace(CommonAction::Focus(
                WorkspaceDirection::Up
            )))
        ));
    }

    #[test]
    fn keyless_popup_entries_list_overrides_and_never_dispatch() {
        use eon_workspace_protocol::v38::{ALT, SHIFT, Shortcut};

        let chord = Shortcut {
            modifiers: ALT | SHIFT,
            key: "KeyM".into(),
        };
        let popup = |id: &str, label: &str, shortcut| PopupEntry {
            id: id.into(),
            label: label.into(),
            shortcut,
        };
        let commands = popup(
            "commands",
            "Commands",
            PopupKey::Overridden {
                default: chord.clone(),
                by: "editor".into(),
            },
        );
        let entries = [
            commands.clone(),
            popup("project", "Project", PopupKey::Unbound),
            popup("editor", "Editor", PopupKey::Bound(chord)),
        ];
        let keyless = [commands, entries[1].clone()];
        assert_eq!(
            popup_entry(&entries, KeyCode::KeyM, ALT_SHIFT).map(|entry| entry.id.as_str()),
            Some("editor")
        );
        assert!(popup_entry(&keyless, KeyCode::KeyM, ALT_SHIFT).is_none());
        for platform in [NativePlatform::Linux, NativePlatform::Macos] {
            let label = physical_shortcut_label_for(platform, ALT | SHIFT, "KeyM");
            let tools = |entries| {
                shortcut_groups_for(platform, entries, Sidebar::Open, true)
                    .into_iter()
                    .find(|group| group.title == "Projects and tools")
                    .map(|group| group.rows)
            };
            assert_eq!(
                tools(&entries),
                Some(vec![
                    ShortcutRow::new(label.clone(), "Commands (overridden by Editor)"),
                    ShortcutRow::new(label, "Editor"),
                ])
            );
            // An unbound entry has no row, so it alone leaves no group.
            assert_eq!(tools(&keyless[1..]), None);
            let hints = shortcut_hints_for(
                platform,
                WorkspaceFocus::Terminal,
                &entries,
                Sidebar::Open,
                true,
            );
            assert!(hints.iter().any(|hint| hint.label == "Editor"));
            assert!(
                hints
                    .iter()
                    .all(|hint| !matches!(hint.label.as_str(), "Commands" | "Project"))
            );
        }
    }

    fn tabs_snapshot(ids: &[&str], active: &str) -> Snapshot {
        Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: Vec::new(),
            active_tab: active.into(),
            geometry: eon_workspace_protocol::v38::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: ids
                .iter()
                .map(|id| Tab {
                    id: (*id).into(),
                    directory: b"/".to_vec(),
                    pending: false,
                    selected_pane: None,
                    selected_popup: None,
                    columns: Vec::new(),
                    popups: Vec::new(),
                    attention_rollup: None,
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: Sidebar::Open,
            attention_queue: Vec::new(),
        }
    }

    fn target_id(
        code: KeyCode,
        modifiers: session::Modifiers,
        snapshot: &Snapshot,
    ) -> Option<Option<&str>> {
        tab_shortcut_target(code, modifiers, snapshot).map(|tab| tab.map(|tab| tab.id.as_str()))
    }

    #[test]
    fn alt_digits_resolve_current_tab_positions_without_leaking_missing_targets() {
        let tabs = ["t7", "t2", "t3", "t4", "t5", "t6", "t1", "t8", "t9", "t10"];
        let snapshot = tabs_snapshot(&tabs, "t2");

        for (key, id) in [
            (KeyCode::Digit1, "t7"),
            (KeyCode::Digit9, "t9"),
            (KeyCode::Digit0, "t10"),
        ] {
            assert_eq!(
                target_id(key, session::Modifiers::ALT, &snapshot),
                Some(Some(id))
            );
        }
        // A missing position is consumed without a target.
        let short = tabs_snapshot(&tabs[..2], "t7");
        assert_eq!(
            target_id(KeyCode::Digit9, session::Modifiers::ALT, &short),
            Some(None)
        );
        assert_eq!(
            target_id(KeyCode::Digit1, session::Modifiers::empty(), &snapshot),
            None
        );
    }

    #[test]
    fn alt_shift_digits_show_workspace_list_positions_unless_a_popup_takes_the_key() {
        use eon_workspace_protocol::v38::{ALT, SHIFT, Shortcut};
        let ids = ["s3", "s2", "s1", "s4", "s5", "s6", "s7", "s8", "s9", "s10"];
        let mut snapshot = tabs_snapshot(&["t1"], "t1");
        assert_eq!(snapshot.active_workspace, "s1");
        // Recency runs against the list: the shown s1 first, then the list backwards.
        let recency = [9, 8, 0, 7, 6, 5, 4, 3, 2, 1];
        snapshot.workspaces = ids
            .iter()
            .zip(recency)
            .map(|(id, recency)| WorkspaceInfo {
                id: (*id).into(),
                persistent_id: Some((*id).into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency,
            })
            .collect();
        let switch = |id: &str| {
            Some(Some(WorkspaceAction::Workspace(
                CommonAction::SwitchWorkspace {
                    workspace: eon_workspace_protocol::v38::WorkspaceTarget::Selector(id.into()),
                },
            )))
        };
        let action = |code, modifiers, snapshot: &Snapshot| {
            position_key_action(code, modifiers, snapshot, true)
        };
        // The list's order, not recency, and Alt+Shift+0 for the tenth.
        for (key, id) in [
            (KeyCode::Digit1, "s3"),
            (KeyCode::Digit2, "s2"),
            (KeyCode::Digit0, "s10"),
        ] {
            assert_eq!(action(key, ALT_SHIFT, &snapshot), switch(id));
        }
        // `number_keys = false` leaves every position key to the terminal.
        for key in [KeyCode::Digit2, KeyCode::Digit3] {
            assert_eq!(position_key_action(key, ALT_SHIFT, &snapshot, false), None);
            assert_eq!(popup_shortcut(key, ALT_SHIFT, &snapshot, true), None);
        }
        // The shown workspace's own position and a missing one are consumed without an action.
        assert_eq!(action(KeyCode::Digit3, ALT_SHIFT, &snapshot), Some(None));
        snapshot.workspaces.truncate(2);
        assert_eq!(action(KeyCode::Digit9, ALT_SHIFT, &snapshot), Some(None));
        // Alt+digits stay tab positions, and a popup on Alt+Shift+1 takes only that key.
        assert_eq!(
            action(KeyCode::Digit1, session::Modifiers::ALT, &snapshot),
            Some(Some(WorkspaceAction::Workspace(CommonAction::FocusId(
                "t1".into()
            ))))
        );
        snapshot.entries = vec![PopupEntry {
            id: "first".into(),
            label: "First".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: "Digit1".into(),
            }),
        }];
        assert_eq!(action(KeyCode::Digit1, ALT_SHIFT, &snapshot), None);
        assert!(popup_shortcut(KeyCode::Digit1, ALT_SHIFT, &snapshot, true).is_some());
        assert_eq!(action(KeyCode::Digit2, ALT_SHIFT, &snapshot), switch("s2"));
        // The viewer names the taken key, and `--number-keys false` leaves the row out.
        let row = |entries: &[PopupEntry], number_keys| {
            shortcut_groups_with(
                NativePlatform::Linux,
                entries,
                Sidebar::Open,
                true,
                number_keys,
            )
            .iter()
            .flat_map(|group| &group.rows)
            .find(|row| row.action.starts_with("Show session by position"))
            .map(|row| row.action.clone())
        };
        assert_eq!(
            row(&snapshot.entries, true).as_deref(),
            Some("Show session by position (Alt+Shift+1 overridden by First)")
        );
        assert_eq!(row(&[], true).as_deref(), Some("Show session by position"));
        assert_eq!(row(&[], false), None);
    }

    #[test]
    fn ctrl_tab_selects_adjacent_tabs_in_order_and_wraps() {
        let snapshot = |active| tabs_snapshot(&["t3", "t1", "t2"], active);
        for (active, next, previous) in [("t3", "t1", "t2"), ("t1", "t2", "t3"), ("t2", "t3", "t1")]
        {
            assert_eq!(
                target_id(KeyCode::Tab, session::Modifiers::CTRL, &snapshot(active)),
                Some(Some(next))
            );
            assert_eq!(
                target_id(KeyCode::Tab, CTRL_SHIFT, &snapshot(active)),
                Some(Some(previous))
            );
        }
        // A lone tab consumes both keys, so nothing reaches the terminal, and selects nothing.
        let lone = tabs_snapshot(&["t1"], "t1");
        for modifiers in [session::Modifiers::CTRL, CTRL_SHIFT] {
            assert_eq!(target_id(KeyCode::Tab, modifiers, &lone), Some(None));
        }
        // Plain Tab and Shift+Tab stay chrome traversal or terminal input.
        for modifiers in [session::Modifiers::empty(), session::Modifiers::SHIFT] {
            assert_eq!(target_id(KeyCode::Tab, modifiers, &snapshot("t1")), None);
        }
    }
}
