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

#[test]
fn attention_inbox_binding_opens_under_every_sidebar() {
    for sidebar in [Sidebar::Open, Sidebar::Closed, Sidebar::Off] {
        assert!(
            shortcut_groups_for(NativePlatform::Linux, &[], sidebar)
                .iter()
                .flat_map(|group| &group.rows)
                .any(|row| row.shortcut == "Alt+Q" && row.action == "Show the attention inbox"),
            "Alt+Q must open the attention inbox under {sidebar:?}"
        );
    }
}

#[test]
fn attention_inbox_keeps_the_queue_filter_and_entry_across_refreshes() {
    use eon_workspace_protocol::v25::{
        Attention, AttentionState, PopupGeometry, QueueEntry, WorkspaceInfo,
    };
    use orbit_protocol::session::Modifiers;
    assert_eq!(
        attention_inbox_command(false, KeyCode::KeyQ, Modifiers::ALT, false, &[]),
        Some(AttentionInboxCommand::Toggle)
    );
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        active_tab: "t3".into(),
        workspaces: vec![
            WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 2,
                attention: None,
                recency: 1,
            },
            WorkspaceInfo {
                id: "w2".into(),
                name: Some("Review".into()),
                tab_count: 1,
                attention: None,
                recency: 0,
            },
        ],
        tabs: ["t3", "t2"]
            .into_iter()
            .map(|id| Tab {
                id: id.into(),
                directory: b"/work/site".to_vec(),
                pending: false,
                selected_pane: Some(id.replace('t', "p")),
                selected_popup: None,
                columns: vec![eon_workspace_protocol::v25::Column {
                    expanded: id.replace('t', "p"),
                    panes: vec![eon_workspace_protocol::v25::Pane {
                        id: id.replace('t', "p"),
                        label: id.replace('t', "p"),
                        pinned_name: None,
                        session: format!("session-{id}"),
                        endpoint: format!("/tmp/{id}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    }],
                }],
                popups: Vec::new(),
                attention_rollup: None,
            })
            .collect(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: Sidebar::Off,
        attention_queue: [
            ("w2", "t4", "p4", AttentionState::NeedsInput),
            ("w1", "t2", "p2", AttentionState::Error),
            ("w1", "t3", "p3", AttentionState::Finished),
        ]
        .into_iter()
        .map(|(workspace, tab, target, state)| QueueEntry {
            workspace: workspace.into(),
            tab: tab.into(),
            target: target.into(),
            attention: Attention {
                state,
                record: Some(eon_workspace_protocol::v25::AttentionRecord {
                    kind: None,
                    progress: None,
                    app: "codex".into(),
                    name: String::new(),
                    title: target.into(),
                    message: String::new(),
                    changed_at: 0,
                    stale: false,
                    others: 0,
                }),
            },
        })
        .collect(),
    };
    let mut inbox = AttentionInboxState {
        open: true,
        ..AttentionInboxState::default()
    };
    inbox.select(&snapshot, 0);
    let scene = |inbox: &AttentionInboxState, snapshot: &Snapshot| {
        DialogScene::attention_inbox(
            snapshot,
            inbox.filter,
            inbox.target(),
            PhysicalSize::new(1000, 700),
            CellMetrics::for_scale(1.0),
            inbox.scroll,
            0,
        )
    };
    let dialog = scene(&inbox, &snapshot);
    assert_eq!(
        dialog
            .attention_rows
            .iter()
            .map(|row| row.target.as_str())
            .collect::<Vec<_>>(),
        ["p4", "p2", "p3"]
    );
    assert_eq!(
        dialog
            .filters
            .iter()
            .map(|chip| chip.count)
            .collect::<Vec<_>>(),
        [3, 1, 1, 1]
    );
    for _ in 0..2 {
        inbox.set_filter(&snapshot, inbox.filter.step(false));
    }
    assert_eq!(inbox.filter, AttentionFilter::Error);
    assert_eq!(scene(&inbox, &snapshot).attention_rows[0].target, "p2");
    let mut model = WorkspaceModel::default();
    model.apply(eon_workspace_protocol::v25::Response::Snapshot(
        snapshot.clone(),
    ));
    let index = inbox.index(&snapshot).unwrap();
    snapshot.attention_queue[1].attention.record =
        Some(eon_workspace_protocol::v25::AttentionRecord {
            kind: None,
            progress: None,
            app: "codex".into(),
            name: String::new(),
            title: "Updated".into(),
            message: String::new(),
            changed_at: 1,
            stale: false,
            others: 0,
        });
    assert_eq!(
        model.apply(eon_workspace_protocol::v25::Response::Snapshot(
            snapshot.clone()
        )),
        (false, false),
        "queue-only updates leave workspace geometry alone"
    );
    let mut presentation = PresentationState::default();
    presentation.publish(presentation.candidate(None));
    assert!(inbox.refresh(model.snapshot().unwrap(), index, &mut presentation));
    assert!(
        !presentation.has_presented_geometry(),
        "queue-only refresh retires dialog hit targets and redraws"
    );
    assert_eq!(inbox.filter, AttentionFilter::Error);
    assert_eq!(inbox.target(), Some(("t2", "p2")));
    assert_ne!(dialog, scene(&inbox, &snapshot));
    assert_eq!(
        inbox.activation(&presentation),
        None,
        "Enter must not jump to a replacement before redraw"
    );
    presentation.publish(presentation.candidate(None));
    assert_eq!(
        inbox.activation(&presentation),
        Some(WorkspaceAction::FocusAttention {
            tab: "t2".into(),
            target: "p2".into()
        })
    );
    for _ in 0..2 {
        inbox.set_filter(&snapshot, inbox.filter.step(true));
    }
    inbox.move_highlight(&snapshot, false);
    assert_eq!(inbox.target(), Some(("t2", "p2")));
    assert_eq!(
        attention_inbox_command(true, KeyCode::Enter, Modifiers::empty(), false, &[]),
        Some(AttentionInboxCommand::Activate)
    );
    let index = inbox.index(&snapshot).unwrap();
    snapshot.attention_queue.remove(1);
    inbox.refresh(&snapshot, index, &mut presentation);
    assert_eq!(
        inbox.activation(&presentation),
        None,
        "the replacement is not presented yet"
    );
    presentation.publish(presentation.candidate(None));
    assert_eq!(
        inbox.activation(&presentation),
        Some(WorkspaceAction::FocusAttention {
            tab: "t3".into(),
            target: "p3".into()
        })
    );
    assert_eq!(
        inbox.target(),
        Some(("t3", "p3")),
        "the replacement keeps its old index"
    );
    snapshot.attention_queue.pop();
    inbox.reconcile(&snapshot, index);
    assert_eq!(
        inbox.target(),
        Some(("t4", "p4")),
        "else highlight the last row"
    );
    snapshot.attention_queue.clear();
    inbox.reconcile(&snapshot, 0);
    assert_eq!(inbox.target(), None);
    assert_eq!(AttentionFilter::All.step(true), AttentionFilter::Finished);
    // Every local key, its repeat and matching release are captured; no bytes are produced.
    let mut input = InputState::default();
    for code in [
        KeyCode::KeyQ,
        KeyCode::Slash,
        KeyCode::KeyV,
        KeyCode::ArrowDown,
        KeyCode::Enter,
    ] {
        let key = PhysicalKey::Code(code);
        assert!(input.consumes_shortcut(key, ElementState::Pressed, false, true));
        assert!(input.consumes_shortcut(key, ElementState::Pressed, true, false));
        assert!(input.consumes_shortcut(key, ElementState::Released, false, false));
    }
}
