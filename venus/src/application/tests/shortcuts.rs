use super::*;
use crate::application::shortcuts::{
    ALT_SHIFT, CTRL_ALT, CTRL_SHIFT, FIXED_SHORTCUTS, native_pointer_shortcut_for,
    shortcut_groups_for,
};
use eon_workspace_protocol::v25::{InvokeIntent, Side, Tab};

#[test]
fn new_window_shortcut_invokes_the_supplied_eon_command() {
    use std::{fs, os::unix::fs::PermissionsExt, time::SystemTime};

    let shortcut = native_key_shortcut_for(
        NativePlatform::Linux,
        PhysicalKey::Code(KeyCode::KeyN),
        ALT_SHIFT,
        &[],
    )
    .unwrap();
    assert_eq!(shortcut.action, NativeShortcutAction::NewWindow);
    assert!(
        shortcut_groups_for(
            NativePlatform::Linux,
            &[],
            eon_workspace_protocol::v25::Sidebar::Open
        )
        .iter()
        .flat_map(|group| &group.rows)
        .any(|row| row.shortcut == "Alt+Shift+N" && row.action == "Open new Eon window")
    );

    // A child shell writes each script to a fresh path. Had this process
    // opened it for writing, a child forked meanwhile by a parallel test
    // would inherit that descriptor, and exec would fail with ETXTBSY.
    let script = |name: &str, body: &str| {
        let path = std::env::temp_dir().join(format!(
            "venus-new-window-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let status = Command::new("/bin/sh")
            .args(["-c", "printf '%s' \"$1\" > \"$0\""])
            .arg(&path)
            .arg(body)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let failing = script(
        "failing",
        "#!/bin/sh\nprintf '%s|%s' \"$1\" \"$2\" >&2\nexit 7\n",
    );
    let error = run_new_window(&failing).unwrap_err();
    assert!(error.contains("window|new"), "{error}");
    let succeeding = script("succeeding", "#!/bin/sh\nexit 0\n");
    assert_eq!(run_new_window(&succeeding), Ok(()));
    fs::remove_file(failing).unwrap();
    fs::remove_file(succeeding).unwrap();
}

#[test]
fn shortcut_viewer_uses_live_rows_and_captures_its_local_sequence() {
    let toggle = PhysicalKey::Code(KeyCode::KeyB);
    assert_eq!(
        native_key_shortcut(toggle, ALT_SHIFT, &[]).map(|shortcut| shortcut.action),
        Some(NativeShortcutAction::ToggleHints)
    );
    use orbit_protocol::session::Modifiers;

    let entries = vec![PopupEntry {
        id: "project".into(),
        label: "Money ops".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v25::ALT,
            key: "KeyZ".into(),
        }),
    }];
    let groups = shortcut_groups_for(
        NativePlatform::Linux,
        &entries,
        eon_workspace_protocol::v25::Sidebar::Open,
    );
    let key_shortcut = |key, modifiers| {
        native_key_shortcut_for(NativePlatform::Linux, key, modifiers, &[]).unwrap()
    };
    let rows = groups
        .iter()
        .flat_map(|group| &group.rows)
        .map(|row| (row.shortcut.as_str(), row.action.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        groups
            .iter()
            .filter(|group| group.title != "Projects and tools")
            .map(|group| group.rows.len())
            .sum::<usize>(),
        FIXED_SHORTCUTS.len()
    );
    for shortcut in FIXED_SHORTCUTS {
        let display = shortcut.trigger.display_label_for(NativePlatform::Linux);
        assert_eq!(
            rows.iter()
                .filter(|row| **row == (display.as_str(), shortcut.label))
                .count(),
            1,
            "{}",
            display
        );
        match shortcut.trigger {
            NativeShortcutTrigger::Key(code, modifiers) => assert!(std::ptr::eq(
                key_shortcut(PhysicalKey::Code(code), modifiers),
                shortcut
            )),
            NativeShortcutTrigger::AltDigits => {
                for code in [KeyCode::Digit1, KeyCode::Digit9, KeyCode::Digit0] {
                    assert!(std::ptr::eq(
                        key_shortcut(PhysicalKey::Code(code), Modifiers::ALT),
                        shortcut
                    ));
                }
            }
            NativeShortcutTrigger::Copy => {
                assert!(std::ptr::eq(
                    key_shortcut(PhysicalKey::Code(KeyCode::KeyC), CTRL_SHIFT),
                    shortcut
                ));
            }
            NativeShortcutTrigger::Paste => {
                for (code, modifiers) in [
                    (KeyCode::KeyV, CTRL_SHIFT),
                    (KeyCode::Paste, Modifiers::empty()),
                ] {
                    assert!(std::ptr::eq(
                        key_shortcut(PhysicalKey::Code(code), modifiers),
                        shortcut
                    ));
                }
            }
            NativeShortcutTrigger::OpenLink => assert!(std::ptr::eq(
                native_pointer_shortcut_for(
                    NativePlatform::Linux,
                    MouseButton::Left,
                    Modifiers::CTRL,
                )
                .unwrap(),
                shortcut
            )),
        }
    }
    assert_eq!(
        rows.iter()
            .filter(|(shortcut, _)| *shortcut == "Alt+/")
            .count(),
        1
    );
    assert!(rows.contains(&("Ctrl+click", "Open link")));
    assert!(rows.contains(&("Alt+Z", "Money ops")));
    assert!(
        shortcut_groups_for(
            NativePlatform::Linux,
            &[],
            eon_workspace_protocol::v25::Sidebar::Open
        )
        .iter()
        .flat_map(|group| &group.rows)
        .all(|row| row.action != "Money ops")
    );

    assert_eq!(
        shortcut_viewer_command(false, KeyCode::Slash, Modifiers::ALT, false),
        Some(ShortcutViewerCommand::Toggle)
    );
    assert_eq!(
        shortcut_viewer_command(false, KeyCode::Slash, Modifiers::ALT, true),
        None
    );
    assert_eq!(
        shortcut_viewer_command(true, KeyCode::Escape, Modifiers::empty(), false),
        Some(ShortcutViewerCommand::Close)
    );
    assert_eq!(
        shortcut_viewer_command(true, KeyCode::KeyA, Modifiers::empty(), false),
        None
    );
    assert_eq!(
        workspace_shortcut(
            KeyCode::Slash,
            Modifiers::ALT,
            &[],
            "t1",
            None,
            Sidebar::Open
        ),
        None
    );

    let mut input = InputState::default();
    let slash = PhysicalKey::Code(KeyCode::Slash);
    assert!(input.consumes_shortcut(slash, ElementState::Pressed, false, true));
    assert!(input.consumes_shortcut(slash, ElementState::Pressed, true, false));
    assert!(input.consumes_shortcut(slash, ElementState::Released, false, false));
}

#[test]
fn macos_uses_command_for_standard_host_actions_only() {
    use orbit_protocol::session::Modifiers;

    let action = |key, modifiers| {
        native_key_shortcut_for(
            NativePlatform::Macos,
            PhysicalKey::Code(key),
            modifiers,
            &[],
        )
        .map(|shortcut| shortcut.action)
    };
    assert_eq!(
        action(KeyCode::KeyC, Modifiers::SUPER),
        Some(NativeShortcutAction::Copy)
    );
    assert_eq!(
        action(KeyCode::KeyV, Modifiers::SUPER),
        Some(NativeShortcutAction::Paste)
    );
    assert_eq!(action(KeyCode::KeyC, CTRL_SHIFT), None);
    assert_eq!(action(KeyCode::KeyV, CTRL_SHIFT), None);
    assert_eq!(
        action(KeyCode::KeyH, Modifiers::ALT),
        Some(NativeShortcutAction::Focus(WorkspaceDirection::Left))
    );
    assert_eq!(
        action(KeyCode::KeyH, CTRL_ALT),
        Some(NativeShortcutAction::MovePaneSideways(Side::Left))
    );
    assert_eq!(
        native_pointer_shortcut_for(NativePlatform::Macos, MouseButton::Left, Modifiers::SUPER,)
            .map(|shortcut| shortcut.action),
        Some(NativeShortcutAction::OpenLink)
    );
    assert!(
        native_pointer_shortcut_for(NativePlatform::Macos, MouseButton::Left, Modifiers::CTRL,)
            .is_none()
    );

    let rows = shortcut_groups_for(
        NativePlatform::Macos,
        &[],
        eon_workspace_protocol::v25::Sidebar::Open,
    )
    .into_iter()
    .flat_map(|group| group.rows)
    .map(|row| row.shortcut)
    .collect::<Vec<_>>();
    for label in [
        "Command+C",
        "Command+V or Paste",
        "Command+click",
        "Option+H",
        "Control+Option+H",
        "Option+/",
    ] {
        assert!(rows.iter().any(|row| row == label), "{label}");
    }
    assert_eq!(
        link_hint_for(NativePlatform::Macos, "https://example.com/", &[]),
        "https://example.com/\nCommand+click Open · Command+C Copy"
    );

    for (platform, program, arguments) in [
        (
            NativePlatform::Linux,
            "gio",
            vec!["open", "--", "https://example.com/"],
        ),
        (
            NativePlatform::Macos,
            "/usr/bin/open",
            vec!["-u", "https://example.com/"],
        ),
    ] {
        let command = link_command(platform, "https://example.com/");
        assert_eq!(command.get_program().to_str(), Some(program));
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_str().unwrap())
                .collect::<Vec<_>>(),
            arguments
        );
    }
}

#[test]
fn structural_shortcuts_do_not_repeat_and_popup_focus_is_explicit() {
    let actions = [
        CommonAction::CreateTab,
        CommonAction::CreatePane { focus: true },
        CommonAction::Move(WorkspaceDirection::Left),
        CommonAction::CloseTab { tab: "t1".into() },
    ]
    .map(WorkspaceAction::Workspace);
    for action in actions.into_iter().chain([
        WorkspaceAction::CreateColumn,
        WorkspaceAction::ClosePane { pane: "p1".into() },
        WorkspaceAction::MovePaneSideways(Side::Right),
    ]) {
        assert!(sends_workspace_shortcut(
            &action,
            ElementState::Pressed,
            false
        ));
        assert!(!sends_workspace_shortcut(
            &action,
            ElementState::Pressed,
            true
        ));
        assert!(!sends_workspace_shortcut(
            &action,
            ElementState::Released,
            false
        ));
    }
    assert!(sends_workspace_shortcut(
        &WorkspaceAction::Workspace(CommonAction::Focus(WorkspaceDirection::Right)),
        ElementState::Pressed,
        true
    ));
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v25::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v25::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: true,
            selected_pane: None,
            selected_popup: None,
            columns: Vec::new(),
            popups: vec![],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v25::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    show_test_popup(&mut snapshot);
    for (focused, intent) in [(true, InvokeIntent::Toggle), (false, InvokeIntent::Focus)] {
        let action = popup_shortcut(
            KeyCode::KeyZ,
            orbit_protocol::session::Modifiers::ALT,
            &snapshot,
            focused,
        )
        .unwrap();
        assert_eq!(
            action,
            WorkspaceAction::InvokePopup {
                tab: "t1".into(),
                entry: "project".into(),
                expected_instance: Some("u1".into()),
                intent
            }
        );
        eon_workspace_protocol::v25::check_popup_action(&snapshot, &action).unwrap();
        assert!(!sends_workspace_shortcut(
            &action,
            ElementState::Pressed,
            true
        ));
    }
    for key in [KeyCode::Escape, KeyCode::Tab, KeyCode::Enter, KeyCode::KeyC] {
        assert!(
            popup_shortcut(
                key,
                orbit_protocol::session::Modifiers::empty(),
                &snapshot,
                true
            )
            .is_none()
        );
    }
}
