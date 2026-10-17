use super::{NATIVE_PLATFORM, NativePlatform};
use eon_workspace_protocol::v10::{
    Action as WorkspaceAction, Direction as WorkspaceDirection, InvokeIntent, Snapshot,
    WorkspaceAction as CommonAction,
};
use orbit_protocol::session;
use winit::{
    event::MouseButton,
    keyboard::{Key, KeyCode, PhysicalKey},
};
use yazelix_venus::{ShortcutGroup, ShortcutHint, ShortcutRow, WorkspaceFocus};

pub(super) fn paste_shortcut(
    key: &Key,
    physical_key: PhysicalKey,
    modifiers: session::Modifiers,
) -> bool {
    matches!(key, Key::Named(winit::keyboard::NamedKey::Paste))
        || native_key_shortcut(physical_key, modifiers)
            .is_some_and(|shortcut| shortcut.action == NativeShortcutAction::Paste)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeShortcutAction {
    ToggleViewer,
    ToggleHints,
    NewWindow,
    FocusTabPosition,
    Focus(WorkspaceDirection),
    Move(WorkspaceDirection),
    CreatePane,
    CreateColumn,
    CreateTab,
    CloseTab,
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
    Copy,
    Paste,
    OpenLink,
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
pub(super) static FIXED_SHORTCUTS: &[NativeShortcut] = &[
    NativeShortcut {
        group: "Navigate",
        label: "Focus tab by position",
        trigger: NativeShortcutTrigger::AltDigits,
        action: NativeShortcutAction::FocusTabPosition,
    },
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
    NativeShortcut::key(
        "Navigate",
        "Cycle terminal, tabs, and panes",
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
        "Previous pane while panes are focused",
        KeyCode::ArrowUp,
        session::Modifiers::empty(),
        NativeShortcutAction::TraverseChrome(WorkspaceDirection::Up),
    ),
    NativeShortcut::key(
        "Navigate",
        "Next pane while panes are focused",
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
        ALT_SHIFT,
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
        "Move tab left",
        KeyCode::KeyH,
        CTRL_ALT,
        NativeShortcutAction::Move(WorkspaceDirection::Left),
    ),
    NativeShortcut::key(
        "Tabs and panes",
        "Move tab right",
        KeyCode::KeyL,
        CTRL_ALT,
        NativeShortcutAction::Move(WorkspaceDirection::Right),
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
                modifiers == session::Modifiers::ALT
                    && matches!(
                        key,
                        PhysicalKey::Code(
                            KeyCode::Digit0
                                | KeyCode::Digit1
                                | KeyCode::Digit2
                                | KeyCode::Digit3
                                | KeyCode::Digit4
                                | KeyCode::Digit5
                                | KeyCode::Digit6
                                | KeyCode::Digit7
                                | KeyCode::Digit8
                                | KeyCode::Digit9
                        )
                    )
            }
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
        }
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

pub(super) fn native_key_shortcut(
    key: PhysicalKey,
    modifiers: session::Modifiers,
) -> Option<&'static NativeShortcut> {
    native_key_shortcut_for(NATIVE_PLATFORM, key, modifiers)
}

pub(super) fn native_key_shortcut_for(
    platform: NativePlatform,
    key: PhysicalKey,
    modifiers: session::Modifiers,
) -> Option<&'static NativeShortcut> {
    FIXED_SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.matches_key(platform, key, modifiers))
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
    entries: &[eon_workspace_protocol::v10::PopupEntry],
) -> Vec<ShortcutGroup> {
    shortcut_groups_for(NATIVE_PLATFORM, entries)
}

pub(super) fn shortcut_groups_for(
    platform: NativePlatform,
    entries: &[eon_workspace_protocol::v10::PopupEntry],
) -> Vec<ShortcutGroup> {
    let mut groups = ["Navigate", "Tabs and panes", "Host"]
        .into_iter()
        .map(|title| {
            ShortcutGroup::new(
                title,
                FIXED_SHORTCUTS
                    .iter()
                    .filter(|shortcut| shortcut.group == title)
                    .map(|shortcut| {
                        ShortcutRow::new(
                            shortcut.trigger.display_label_for(platform),
                            shortcut.label,
                        )
                    })
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    if !entries.is_empty() {
        groups.insert(
            2,
            ShortcutGroup::new(
                "Projects and tools",
                entries
                    .iter()
                    .map(|entry| {
                        ShortcutRow::new(
                            physical_shortcut_label_for(
                                platform,
                                entry.shortcut.modifiers,
                                &entry.shortcut.key,
                            ),
                            entry.label.clone(),
                        )
                    })
                    .collect(),
            ),
        );
    }
    groups
}

pub(super) fn shortcut_hints_for(
    platform: NativePlatform,
    focus: WorkspaceFocus,
    entries: &[eon_workspace_protocol::v10::PopupEntry],
) -> Vec<ShortcutHint> {
    let mut hints: Vec<ShortcutHint> = Vec::new();
    for shortcut in FIXED_SHORTCUTS {
        use NativeShortcutAction as Action;
        use WorkspaceDirection::{Down, Left, Right, Up};
        let (label, priority) = match shortcut.action {
            Action::ToggleViewer => ("keys", 0),
            Action::ToggleHints => ("hints", 0),
            Action::Focus(Left | Right) => ("tabs", 1),
            Action::Focus(Up | Down) => ("panes", 1),
            Action::CreatePane => ("new pane", 1),
            Action::CreateColumn => ("new column", 3),
            Action::CreateTab => ("new tab", 1),
            Action::FocusTabPosition => ("tab", 3),
            Action::CloseTab => ("close tab", 3),
            Action::Move(_) => ("move", 3),
            Action::NewWindow => ("new window", 3),
            Action::ReturnToTerminal if focus != WorkspaceFocus::Terminal => ("terminal", 0),
            Action::TraverseChrome(Left | Right) if focus == WorkspaceFocus::Tabs => ("tabs", 1),
            Action::TraverseChrome(Up | Down) if focus == WorkspaceFocus::Panes => ("panes", 1),
            _ => continue,
        };
        let (modifiers, key) = match shortcut.trigger {
            NativeShortcutTrigger::Key(code, modifiers) => {
                (wire_modifiers(modifiers), format!("{code:?}"))
            }
            NativeShortcutTrigger::AltDigits => (eon_workspace_protocol::v10::ALT, "1..9".into()),
            _ => continue,
        };
        let modifiers = physical_modifiers_label_for(platform, modifiers, true);
        let key = physical_key_label(&key);
        let accessible_label = format!(
            "{} — {}",
            shortcut.trigger.display_label_for(platform),
            shortcut.label
        );
        if let Some(hint) = hints
            .iter_mut()
            .find(|hint| hint.modifiers == modifiers && hint.label == label)
        {
            hint.key
                .push_str(if matches!(shortcut.action, Action::Move(Up)) {
                    " · "
                } else {
                    "/"
                });
            hint.key.push_str(key);
            hint.accessible_label.push_str("; ");
            hint.accessible_label.push_str(&accessible_label);
        } else {
            hints.push(ShortcutHint {
                modifiers,
                key: key.into(),
                label: label.into(),
                accessible_label,
                priority,
            });
        }
    }
    hints.extend(
        entries
            .iter()
            .filter(|entry| entry.id != "project")
            .map(|entry| ShortcutHint {
                modifiers: physical_modifiers_label_for(platform, entry.shortcut.modifiers, true),
                key: physical_key_label(&entry.shortcut.key).into(),
                label: entry.label.clone(),
                accessible_label: format!(
                    "{} — {}",
                    physical_shortcut_label_for(
                        platform,
                        entry.shortcut.modifiers,
                        &entry.shortcut.key
                    ),
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
    use eon_workspace_protocol::v10 as wire;

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
    parts.join(if compact { "-" } else { "+" })
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
    use eon_workspace_protocol::v10 as wire;

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
    active_tab: &str,
) -> Option<WorkspaceAction> {
    let action = match native_key_shortcut(PhysicalKey::Code(code), modifiers)?.action {
        NativeShortcutAction::Focus(direction) => CommonAction::Focus(direction),
        NativeShortcutAction::Move(direction) => CommonAction::Move(direction),
        NativeShortcutAction::CreatePane => CommonAction::CreatePane,
        NativeShortcutAction::CreateColumn => return Some(WorkspaceAction::CreateColumn),
        NativeShortcutAction::CreateTab => CommonAction::CreateTab,
        NativeShortcutAction::CloseTab => CommonAction::CloseTab {
            tab: active_tab.into(),
        },
        _ => return None,
    };
    Some(WorkspaceAction::Workspace(action))
}

pub(super) fn tab_shortcut_index(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
) -> Option<usize> {
    if native_key_shortcut(PhysicalKey::Code(code), modifiers)
        .is_none_or(|shortcut| shortcut.action != NativeShortcutAction::FocusTabPosition)
    {
        return None;
    }
    match code {
        KeyCode::Digit1 => Some(0),
        KeyCode::Digit2 => Some(1),
        KeyCode::Digit3 => Some(2),
        KeyCode::Digit4 => Some(3),
        KeyCode::Digit5 => Some(4),
        KeyCode::Digit6 => Some(5),
        KeyCode::Digit7 => Some(6),
        KeyCode::Digit8 => Some(7),
        KeyCode::Digit9 => Some(8),
        KeyCode::Digit0 => Some(9),
        _ => None,
    }
}

pub(super) fn popup_shortcut(
    code: KeyCode,
    modifiers: orbit_protocol::session::Modifiers,
    snapshot: &Snapshot,
    terminal_focused: bool,
) -> Option<WorkspaceAction> {
    let normalized = wire_modifiers(modifiers);
    // winit's physical KeyCode names are the canonical names in EONW v9.
    let key = format!("{code:?}");
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.shortcut.modifiers == normalized && entry.shortcut.key == key)?;
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.active_tab)?;
    let instance = tab.popups.iter().find(|popup| popup.entry == entry.id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v10::PopupEntry;

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
            (KeyCode::KeyM, Modifiers::ALT, CommonAction::CreatePane),
            (
                KeyCode::KeyT,
                Modifiers::ALT.union(Modifiers::SHIFT),
                CommonAction::CreateTab,
            ),
            (
                KeyCode::KeyH,
                Modifiers::CTRL.union(Modifiers::ALT),
                CommonAction::Move(WorkspaceDirection::Left),
            ),
            (
                KeyCode::KeyL,
                Modifiers::CTRL.union(Modifiers::ALT),
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
                workspace_shortcut(key, modifiers, "t2"),
                Some(WorkspaceAction::Workspace(action))
            );
        }
        assert_eq!(
            workspace_shortcut(KeyCode::KeyM, Modifiers::ALT.union(Modifiers::SHIFT), "t2"),
            Some(WorkspaceAction::CreateColumn)
        );
        for key in [KeyCode::KeyT, KeyCode::KeyW] {
            for modifiers in [
                Modifiers::CTRL,
                Modifiers::ALT,
                Modifiers::CTRL.union(Modifiers::SHIFT),
                Modifiers::CTRL
                    .union(Modifiers::ALT)
                    .union(Modifiers::SHIFT),
            ] {
                assert_eq!(workspace_shortcut(key, modifiers, "t2"), None);
            }
        }
        assert_eq!(
            workspace_shortcut(KeyCode::KeyH, Modifiers::ALT.union(Modifiers::SHIFT), "t2"),
            None
        );
    }

    #[test]
    fn shortcut_hints_follow_bindings_catalog_and_native_focus() {
        let entry = PopupEntry {
            id: "agent".into(),
            label: "My agent".into(),
            shortcut: eon_workspace_protocol::v10::Shortcut {
                modifiers: eon_workspace_protocol::v10::SUPER,
                key: "KeyF".into(),
            },
        };
        let project = PopupEntry {
            id: "project".into(),
            label: "Folder chooser".into(),
            shortcut: eon_workspace_protocol::v10::Shortcut {
                modifiers: eon_workspace_protocol::v10::ALT,
                key: "KeyZ".into(),
            },
        };
        let entries = [entry.clone(), project.clone()];
        for platform in [NativePlatform::Linux, NativePlatform::Macos] {
            let terminal = shortcut_hints_for(platform, WorkspaceFocus::Terminal, &entries);
            assert!(terminal.iter().all(|hint| hint.label != project.label));
            for action in [
                NativeShortcutAction::Copy,
                NativeShortcutAction::Paste,
                NativeShortcutAction::OpenLink,
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
                    shortcut_groups_for(platform, &entries)
                        .iter()
                        .flat_map(|group| &group.rows)
                        .any(|row| {
                            row.action == shortcut.label
                                && row.shortcut == shortcut.trigger.display_label_for(platform)
                        })
                );
            }
            assert!(
                shortcut_groups_for(platform, &entries)
                    .iter()
                    .flat_map(|group| &group.rows)
                    .any(|row| row.action == project.label
                        && row.shortcut
                            == physical_shortcut_label_for(
                                platform,
                                project.shortcut.modifiers,
                                &project.shortcut.key,
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
                        entry.shortcut.modifiers,
                        &entry.shortcut.key
                    ),
                    entry.label,
                )
            );
            let moves: Vec<_> = terminal
                .iter()
                .filter(|hint| hint.label == "move")
                .collect();
            assert_eq!(moves.len(), 1);
            assert_eq!(moves[0].modifiers, "C-A");
            assert_eq!(moves[0].key, "H/L · K/J");
            assert!(
                terminal
                    .iter()
                    .any(|hint| hint.key == "H/L" && hint.label == "tabs")
            );
            assert!(terminal.iter().all(|hint| {
                !hint
                    .accessible_label
                    .contains("Return focus to the terminal")
                    && !hint.key.contains("Left")
            }));
            assert!(
                shortcut_hints_for(platform, WorkspaceFocus::Terminal, &[])
                    .iter()
                    .all(|hint| hint.label != entry.label)
            );
            for focus in [WorkspaceFocus::Tabs, WorkspaceFocus::Panes] {
                let chrome = shortcut_hints_for(platform, focus, &[]);
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
                assert_eq!(
                    chrome.iter().any(|hint| hint.key == "Up/Down"),
                    focus == WorkspaceFocus::Panes
                );
            }
            for shortcut in FIXED_SHORTCUTS.iter().filter(|shortcut| {
                !matches!(
                    shortcut.action,
                    NativeShortcutAction::CycleFocus
                        | NativeShortcutAction::ReturnToTerminal
                        | NativeShortcutAction::TraverseChrome(_)
                        | NativeShortcutAction::Copy
                        | NativeShortcutAction::Paste
                        | NativeShortcutAction::OpenLink
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
    fn alt_digits_resolve_current_tab_positions_without_leaking_missing_targets() {
        use orbit_protocol::session::Modifiers;

        let tabs = ["t7", "t2", "t3", "t4", "t5", "t6", "t1", "t8", "t9", "t10"];

        for (key, expected_index, id) in [
            (KeyCode::Digit1, 0, "t7"),
            (KeyCode::Digit9, 8, "t9"),
            (KeyCode::Digit0, 9, "t10"),
        ] {
            let index = tab_shortcut_index(key, Modifiers::ALT);
            assert_eq!(index, Some(expected_index));
            assert_eq!(index.and_then(|index| tabs.get(index)), Some(&id));
        }
        let missing = tab_shortcut_index(KeyCode::Digit9, Modifiers::ALT);
        assert!(missing.is_some());
        assert_eq!(missing.and_then(|index| tabs[..2].get(index)), None);
        assert_eq!(
            tab_shortcut_index(KeyCode::Digit1, Modifiers::empty()),
            None
        );
    }
}
