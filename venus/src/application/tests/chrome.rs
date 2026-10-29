use super::*;
use crate::application::shortcuts::ALT_SHIFT;
use eon_workspace_protocol::v24::{InvokeIntent, Pane, Tab};

#[test]
fn close_prompt_asks_for_a_final_close() {
    use orbit_protocol::session::Modifiers;

    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: id.as_bytes().to_vec(),
        live: true,
        attention: None,
    };
    let tab = |id: &str, panes: &[&str]| Tab {
        id: id.into(),
        directory: b"/tmp".to_vec(),
        pending: false,
        selected_pane: panes.first().map(|pane| (*pane).into()),
        selected_popup: None,
        columns: vec![eon_workspace_protocol::v24::Column {
            panes: panes.iter().map(|id| pane(id)).collect(),
            expanded: panes
                .first()
                .map_or_else(String::new, |pane| (*pane).into()),
        }],
        popups: vec![],
        attention_rollup: None,
    };
    let snapshot = |tabs: Vec<Tab>| Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v24::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: tabs.len(),
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v24::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs,
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v24::Sidebar::Open,
    };
    let close_tab =
        |id: &str| WorkspaceAction::Workspace(CommonAction::CloseTab { tab: id.into() });
    let close_pane = |id: &str| WorkspaceAction::ClosePane { pane: id.into() };

    let one = snapshot(vec![tab("t1", &["p1"])]);
    assert_eq!(final_close_tab(&one, &close_tab("t1")), Some("t1".into()));
    assert_eq!(final_close_tab(&one, &close_pane("p1")), Some("t1".into()));
    assert_eq!(final_close_tab(&one, &close_tab("t2")), None);
    assert_eq!(
        final_close_tab(
            &one,
            &WorkspaceAction::Workspace(CommonAction::Focus(WorkspaceDirection::Left))
        ),
        None
    );
    // Runtime refuses a pane close under a popup, so it is not asked about.
    let mut covered = one.clone();
    show_test_popup(&mut covered);
    assert_eq!(final_close_tab(&covered, &close_pane("p1")), None);
    assert_eq!(
        final_close_tab(&covered, &close_tab("t1")),
        Some("t1".into())
    );
    let stacked = snapshot(vec![tab("t1", &["p1", "p2"])]);
    assert_eq!(final_close_tab(&stacked, &close_pane("p1")), None);
    let two = snapshot(vec![tab("t1", &["p1"]), tab("t2", &["p2"])]);
    assert_eq!(final_close_tab(&two, &close_tab("t1")), None);
    assert_eq!(final_close_tab(&two, &close_pane("p1")), None);
    // With another workspace, closing the shown one's last tab asks too.
    let mut shared = one.clone();
    shared
        .workspaces
        .push(eon_workspace_protocol::v24::WorkspaceInfo {
            id: "w2".into(),
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        });
    assert_eq!(
        final_close_tab(&shared, &close_tab("t1")),
        Some("t1".into())
    );
    assert_eq!(
        final_close_tab(&shared, &close_pane("p1")),
        Some("t1".into())
    );
    // An open prompt is worded from each snapshot, so one that adds a workspace repaints
    // it as closing the shown workspace.
    let mut model = WorkspaceModel::default();
    model.apply(eon_workspace_protocol::v24::Response::Snapshot(one.clone()));
    let worded = |model: &WorkspaceModel| close_prompt_workspace(model.snapshot().unwrap());
    assert_eq!(worded(&model), None);
    assert_eq!(
        model.apply(eon_workspace_protocol::v24::Response::Snapshot(shared)),
        (true, false)
    );
    assert_eq!(worded(&model).as_deref(), Some("w1"));

    // The prompt keeps its close while the named tab and pane live, even when a tab appears.
    let prompt = |action| ClosePrompt {
        tab: "t1".into(),
        action,
        scroll: 0.0,
    };
    assert!(prompt(close_tab("t1")).names_live_target(&two));
    assert!(prompt(close_pane("p1")).names_live_target(&two));
    assert!(!prompt(close_pane("p1")).names_live_target(&snapshot(vec![tab("t1", &["p2"])])));
    assert!(!prompt(close_tab("t1")).names_live_target(&snapshot(vec![tab("t2", &["p2"])])));

    assert_eq!(
        close_prompt_answer(KeyCode::Enter, Modifiers::empty()),
        Some(true)
    );
    assert_eq!(
        close_prompt_answer(KeyCode::Escape, Modifiers::empty()),
        Some(false)
    );
    assert_eq!(close_prompt_answer(KeyCode::Enter, Modifiers::SHIFT), None);
    assert_eq!(close_prompt_answer(KeyCode::KeyY, Modifiers::empty()), None);

    // Alt+Shift+D detaches unless a configured popup takes the key.
    let detach = PhysicalKey::Code(KeyCode::KeyD);
    let files = [PopupEntry {
        id: "files".into(),
        label: "Files".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v24::ALT | eon_workspace_protocol::v24::SHIFT,
            key: "KeyD".into(),
        }),
    }];
    assert_eq!(
        native_key_shortcut_for(NATIVE_PLATFORM, detach, ALT_SHIFT, &[])
            .map(|shortcut| shortcut.action),
        Some(NativeShortcutAction::Detach)
    );
    assert!(native_key_shortcut_for(NATIVE_PLATFORM, detach, ALT_SHIFT, &files).is_none());
    assert!(detach_chord(&[]).is_some());
    assert_eq!(detach_chord(&files), None);
}

#[test]
fn zoom_names_the_tab_panes_columns_and_selected_pane() {
    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: id.as_bytes().to_vec(),
        live: true,
        attention: None,
    };
    let column = |panes: &[&str]| eon_workspace_protocol::v24::Column {
        panes: panes.iter().map(|id| pane(id)).collect(),
        expanded: panes[0].into(),
    };
    let tab = |id: &str, selected: &str, columns| Tab {
        pending: false,
        selected_popup: None,
        popups: Vec::new(),
        id: id.into(),
        directory: b"/tmp/eon".to_vec(),
        selected_pane: Some(selected.into()),
        columns,
        attention_rollup: None,
    };
    let snapshot = |active: &str, tabs: Vec<Tab>| Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v24::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: tabs.len(),
            attention: None,
            recency: 0,
        }],
        active_tab: active.into(),
        tabs,
        geometry: eon_workspace_protocol::v24::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v24::Sidebar::Open,
    };
    let two = |selected| {
        snapshot(
            "t1",
            vec![
                tab("t1", selected, vec![column(&["p1"]), column(&["p2"])]),
                tab("t2", "p9", vec![column(&["p8"]), column(&["p9"])]),
            ],
        )
    };
    let zoom = zoom_target(&two("p2"));
    assert!(zoom.is_some());
    // Structural changes end zoom; a covering popup keeps its target.
    let mut grown = two("p2");
    grown.tabs[0].columns[1].panes.push(pane("p3"));
    assert_ne!(zoom_target(&grown), zoom);
    let mut covered = two("p2");
    covered.tabs[0].selected_popup = Some("u1".into());
    assert_eq!(zoom_target(&covered), zoom);
    // Another selected pane, column count or active tab ends it.
    assert_ne!(zoom_target(&two("p1")), zoom);
    let mut wider = two("p2");
    wider.tabs[0].columns.push(column(&["p4"]));
    assert_ne!(zoom_target(&wider), zoom);
    let mut other_tab = two("p2");
    other_tab.active_tab = "t2".into();
    assert_ne!(zoom_target(&other_tab), zoom);
    let mut stack = column(&["p1", "p2", "p3"]);
    stack.expanded = "p2".into();
    let mut single = snapshot("t1", vec![tab("t1", "p2", vec![stack])]);
    let middle = zoom_target(&single);
    assert!(middle.is_some(), "a one-column stack can zoom");
    let mut reordered = single.clone();
    reordered.tabs[0].columns[0].panes.swap(0, 2);
    assert_ne!(zoom_target(&reordered), middle);
    let mut closed = single.clone();
    closed.tabs[0].columns[0].panes.pop();
    assert_ne!(zoom_target(&closed), middle);
    let mut labels = single.clone();
    labels.tabs[0].columns[0].panes[0].label = "p1  Review · ../repo".into();
    labels.tabs[0].columns[0].panes[0].pinned_name = Some("Review".into());
    assert_eq!(zoom_target(&labels), middle);
    let mut status = single.clone();
    status.tabs[0].columns[0].panes[0].live = false;
    assert_eq!(zoom_target(&status), middle);
    for (key, direction, selected) in [
        (KeyCode::KeyJ, WorkspaceDirection::Down, "p3"),
        (KeyCode::KeyK, WorkspaceDirection::Up, "p1"),
    ] {
        assert_eq!(
            workspace_shortcut(
                key,
                session::Modifiers::ALT,
                &[],
                "t1",
                Some("p2"),
                eon_workspace_protocol::v24::Sidebar::Open
            ),
            Some(WorkspaceAction::Workspace(CommonAction::Focus(direction)))
        );
        single.tabs[0].selected_pane = Some(selected.into());
        single.tabs[0].columns[0].expanded = selected.into();
        assert_ne!(zoom_target(&single), middle);
    }
    // One pane has nothing to zoom.
    assert_eq!(
        zoom_target(&snapshot(
            "t1",
            vec![tab("t1", "p1", vec![column(&["p1"])])]
        )),
        None
    );
}

#[test]
fn output_keeps_presented_geometry_until_a_structural_transition() {
    let mut presentation = PresentationState::default();
    let workspace_a = presentation.candidate(Some(7));
    presentation.publish(workspace_a);
    assert!(presentation.has_presented_geometry());

    presentation.content_changed();
    let output = presentation.candidate(Some(8));
    assert!(output.generation > workspace_a.generation);
    assert!(presentation.has_presented_geometry());
    assert_eq!(presentation.presented_revision(), Some(7));
    presentation.publish(workspace_a);
    assert_eq!(presentation.presented_revision(), Some(7));
    presentation.publish(output);
    assert_eq!(presentation.presented_revision(), Some(8));

    presentation.invalidate();
    let workspace_b = presentation.candidate(Some(7));
    assert!(!presentation.has_presented_geometry());
    presentation.publish(workspace_a);
    assert!(!presentation.has_presented_geometry());

    presentation.publish(workspace_b);
    assert!(presentation.has_presented_geometry());
    presentation.unpublish();
    assert!(!presentation.has_presented_geometry());

    presentation.publish(workspace_b);
    assert!(presentation.has_presented_geometry());
}

#[test]
fn published_accessibility_focus_waits_for_eon_queue_admission() {
    for (target, admitted, expected_focus, expected_action) in [
        (
            AccessibilityTarget::Tab("t2".into()),
            true,
            Some(WorkspaceFocus::Tabs),
            CommonAction::FocusId("t2".into()),
        ),
        (
            AccessibilityTarget::Pane("pane-3".into()),
            false,
            None,
            CommonAction::FocusId("pane-3".into()),
        ),
    ] {
        let mut queued = None;
        let focus = accessibility_workspace_focus(target, |action| {
            queued = Some(action);
            admitted
        });
        assert_eq!((focus, queued), (expected_focus, Some(expected_action)));
    }
    assert_eq!(
        accessibility_workspace_focus(AccessibilityTarget::Terminal, |_| unreachable!()),
        Some(WorkspaceFocus::Terminal)
    );
}

#[test]
fn workspace_chrome_preserves_terminal_release_pairing() {
    use ElementState::{Pressed, Released};

    for (workspace, terminal_hit, state, current, expected) in [
        (true, false, Pressed, true, false),
        (true, false, Released, false, true),
        (true, true, Pressed, true, true),
        (false, false, Pressed, true, true),
        (false, false, Pressed, false, false),
    ] {
        assert_eq!(
            button_reaches_terminal(workspace, terminal_hit, state, current),
            expected
        );
    }
}

#[test]
fn scrollback_pill_activates_only_a_complete_left_click() {
    use ElementState::{Pressed, Released};
    let mut pressed = None;
    let current = Some(PresentationIdentity {
        generation: 1,
        revision: Some(4),
    });
    let replaced = Some(PresentationIdentity {
        generation: 2,
        revision: Some(5),
    });
    for (state, target, expected) in [
        (Pressed, None, None),
        (Released, current, None),
        (Pressed, current, Some(false)),
        (Released, None, Some(false)),
        (Pressed, current, Some(false)),
        (Released, replaced, Some(false)),
        (Pressed, current, Some(false)),
        (Released, current, Some(true)),
        (Released, current, None),
    ] {
        assert_eq!(
            scrollback_button(&mut pressed, state, MouseButton::Left, target),
            expected,
            "{state:?} {target:?}"
        );
    }
}

#[test]
fn sidebar_rows_take_f6_focus_step_by_arrow_and_follow_their_jump() {
    use eon_workspace_protocol::v24::{Attention, AttentionState, Column, PopupGeometry};

    // F6 visits the sidebar between the terminal and the tabs, only while it has rows.
    assert_eq!(
        cycle_focus(WorkspaceFocus::Terminal, true, true),
        WorkspaceFocus::Sidebar
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Sidebar, true, true),
        WorkspaceFocus::Tabs
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Terminal, false, true),
        WorkspaceFocus::Tabs
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Tabs, true, true),
        WorkspaceFocus::Panes
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Tabs, true, false),
        WorkspaceFocus::Terminal
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Panes, true, true),
        WorkspaceFocus::Terminal
    );

    let pane = |id: &str, attention: bool| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: format!("/run/{id}.sock").into_bytes(),
        live: true,
        attention: attention.then_some(Attention {
            state: AttentionState::Unavailable,
            record: None,
        }),
    };
    let tab = |id: &str, panes: Vec<Pane>| Tab {
        pending: false,
        selected_popup: None,
        popups: Vec::new(),
        id: id.into(),
        directory: format!("/work/{id}").into_bytes(),
        selected_pane: Some(panes[0].id.clone()),
        attention_rollup: panes
            .iter()
            .any(|pane| pane.attention.is_some())
            .then_some(AttentionState::Unavailable),
        columns: vec![Column {
            expanded: panes[0].id.clone(),
            panes,
        }],
    };
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v24::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 2,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        tabs: vec![
            tab("t1", vec![pane("p1", false), pane("p2", true)]),
            tab("t2", vec![pane("p3", true)]),
        ],
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v24::Sidebar::Open,
    };
    let scene = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(1400, 800),
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * 10.0),
    );
    // Arrows step through the rows in order and stop at either end.
    let rows = scene.sidebar_rows();
    assert_eq!(step_sidebar_row(rows, Some("p2"), true), Some("p3"));
    assert_eq!(step_sidebar_row(rows, Some("p3"), false), Some("p2"));
    assert_eq!(step_sidebar_row(rows, Some("p3"), true), None);
    assert_eq!(step_sidebar_row(rows, Some("p2"), false), None);
    assert_eq!(step_sidebar_row(rows, None, true), None);

    // A jump focuses the terminal on any snapshot that focuses its exact target; only its
    // own answer moves focus to the tab strip, when that selected just the target's tab.
    let jump = AttentionJump {
        tab: "t2".into(),
        target: "p3".into(),
        ticket: 7,
    };
    assert_eq!(focused_terminal(&snapshot), Some("p1"));
    for answer in [false, true] {
        assert_eq!(jump_focus(&jump, &snapshot, answer), None);
    }
    snapshot.active_tab = "t2".into();
    for answer in [false, true] {
        assert_eq!(
            jump_focus(&jump, &snapshot, answer),
            Some(WorkspaceFocus::Terminal)
        );
    }
    show_test_popup(&mut snapshot);
    snapshot.tabs[1].popups.push(Popup {
        id: "u1".into(),
        entry: "project".into(),
        session: "session-u1".into(),
        endpoint: b"/run/u1.sock".to_vec(),
        attention: None,
    });
    snapshot.tabs[1].selected_popup = Some("u1".into());
    assert_eq!(focused_terminal(&snapshot), Some("u1"));
    assert_eq!(jump_focus(&jump, &snapshot, false), None);
    assert_eq!(
        jump_focus(&jump, &snapshot, true),
        Some(WorkspaceFocus::Tabs)
    );
}

#[test]
fn eon_bar_controls_reuse_existing_actions_and_local_viewer() {
    // The workspace label opens the picker as its key does, from the snapshot's entry.
    assert_eq!(
        workspace_header_action(WorkspaceHeaderControl::Workspaces, "t7"),
        WorkspaceHeaderAction::PickWorkspace
    );
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v24::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: None,
            selected_popup: None,
            columns: Vec::new(),
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v24::Sidebar::Off,
    };
    assert_eq!(workspace_picker_tooltip(&snapshot.entries), None);
    assert_eq!(invoke_popup(&snapshot, WORKSPACES_ENTRY, true), None);
    snapshot.entries.push(PopupEntry {
        id: WORKSPACES_ENTRY.into(),
        label: "Workspaces".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v24::ALT | eon_workspace_protocol::v24::SHIFT,
            key: "KeyS".into(),
        }),
    });
    assert_eq!(
        workspace_picker_tooltip(&snapshot.entries).as_deref(),
        Some(match NATIVE_PLATFORM {
            NativePlatform::Linux => "Workspaces — Alt+Shift+S",
            NativePlatform::Macos => "Workspaces — Option+Shift+S",
        })
    );
    let key = popup_shortcut(KeyCode::KeyS, ALT_SHIFT, &snapshot, true);
    assert_eq!(
        key,
        Some(WorkspaceAction::InvokePopup {
            tab: "t1".into(),
            entry: WORKSPACES_ENTRY.into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        })
    );
    assert_eq!(invoke_popup(&snapshot, WORKSPACES_ENTRY, true), key);
    snapshot.entries.last_mut().unwrap().shortcut = PopupKey::Unbound;
    assert_eq!(
        workspace_picker_tooltip(&snapshot.entries).as_deref(),
        Some("Workspaces")
    );
    assert_eq!(invoke_popup(&snapshot, WORKSPACES_ENTRY, true), key);
    assert_eq!(
        workspace_header_action(WorkspaceHeaderControl::NewTab, "t7"),
        WorkspaceHeaderAction::Workspace(CommonAction::CreateTab)
    );
    assert_eq!(
        workspace_header_action(WorkspaceHeaderControl::Shortcuts, "t7"),
        WorkspaceHeaderAction::ToggleShortcuts
    );
    assert_eq!(
        workspace_header_action(WorkspaceHeaderControl::CloseTab, "t7"),
        WorkspaceHeaderAction::Workspace(CommonAction::CloseTab { tab: "t7".into() })
    );
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Tabs, false, false, true),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            true,
            false,
            true
        ),
        Some(WorkspaceFocus::Tabs)
    );
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Panes, false, false, true),
        None
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            false,
            false,
            false
        ),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::CloseTab))
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::CloseTab),
            true,
            false,
            false
        ),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    // A workspace label that opens the picker is the first header control.
    let label = WorkspaceFocus::Header(WorkspaceHeaderControl::Workspaces);
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Tabs, false, true, true),
        Some(label)
    );
    assert_eq!(
        cycle_header_focus(label, false, true, true),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            true,
            true,
            true
        ),
        Some(label)
    );
    assert_eq!(
        cycle_header_focus(label, true, true, true),
        Some(WorkspaceFocus::Tabs)
    );
}
