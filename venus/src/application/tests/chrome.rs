use super::*;
use crate::application::shortcuts::ALT_SHIFT;
use eon_workspace_protocol::v43::{InvokeIntent, Pane, Tab};

#[test]
fn sidebar_wheel_routes_only_the_visible_card() {
    use eon_workspace_protocol::v43::{Attention, AttentionState, Column, Sidebar};
    let mut snapshot = quit_snapshot();
    snapshot.sidebar = Sidebar::Open;
    let pane = Pane {
        id: "p1".into(),
        label: "p1".into(),
        pinned_name: None,
        tag: None,
        session: "terminal-p1".into(),
        endpoint: b"/run/p1".to_vec(),
        live: true,
        attention: Some(Attention {
            state: AttentionState::Unavailable,
            record: None,
        }),
    };
    let panes = (1..=14)
        .map(|index| {
            let mut pane = pane.clone();
            pane.id = format!("p{index}");
            pane
        })
        .collect();
    snapshot.tabs = vec![Tab {
        id: "t1".into(),
        directory: b"/work".to_vec(),
        pending: false,
        selected_pane: Some("p1".into()),
        selected_popup: None,
        columns: vec![Column {
            expanded: "p1".into(),
            panes,
        }],
        popups: vec![],
        attention_rollup: Some(AttentionState::Unavailable),
    }];
    snapshot.active_tab = "t1".into();
    let metrics = CellMetrics::for_scale(1.0);
    let scene = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(1600, 450),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
    );
    let card = scene.sidebar().unwrap();
    for point in [
        PhysicalPosition::new(f64::from(card.left + 1.0), f64::from(card.top + 1.0)),
        PhysicalPosition::new(
            f64::from(card.right() - 1.0),
            f64::from(card.bottom() - 1.0),
        ),
    ] {
        assert_eq!(
            sidebar_wheel(
                &scene,
                point,
                MouseScrollDelta::LineDelta(0.0, -1.0),
                metrics
            ),
            Some(3.0 * metrics.height)
        );
        assert_eq!(
            sidebar_wheel(
                &scene,
                point,
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -0.25)),
                metrics
            ),
            Some(0.25)
        );
        assert_eq!(
            sidebar_wheel(
                &scene,
                point,
                MouseScrollDelta::LineDelta(0.0, 1.0),
                metrics
            ),
            Some(0.0)
        );
        assert_eq!(
            sidebar_wheel(
                &scene,
                point,
                MouseScrollDelta::LineDelta(0.0, f32::NAN),
                metrics
            ),
            Some(0.0)
        );
    }
    assert_eq!(
        sidebar_wheel(
            &scene,
            PhysicalPosition::new(f64::from(card.right() + 10.0), f64::from(card.top + 10.0)),
            MouseScrollDelta::LineDelta(0.0, -1.0),
            metrics
        ),
        None
    );
    snapshot.sidebar = Sidebar::Closed;
    let hidden = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(1600, 450),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
    );
    assert_eq!(
        sidebar_wheel(
            &hidden,
            PhysicalPosition::new(f64::from(card.left), f64::from(card.top)),
            MouseScrollDelta::LineDelta(0.0, -1.0),
            metrics
        ),
        None
    );
}

#[test]
fn close_prompt_asks_for_a_final_close() {
    use orbit_protocol::session::Modifiers;

    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
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
        columns: vec![eon_workspace_protocol::v43::Column {
            panes: panes.iter().map(|id| pane(id)).collect(),
            expanded: panes
                .first()
                .map_or_else(String::new, |pane| (*pane).into()),
        }],
        popups: vec![],
        attention_rollup: None,
    };
    let snapshot = |tabs: Vec<Tab>| Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v43::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: tabs.len(),
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v43::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs,
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v43::Sidebar::Open,
        attention_queue: Vec::new(),
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
    let mut two = snapshot(vec![tab("t1", &["p1"]), tab("t2", &["p2"])]);
    two.active_tab = "t2".into();
    assert_eq!(final_close_tab(&two, &close_tab("t1")), None);
    assert_eq!(final_close_tab(&two, &close_pane("p1")), None);
    // With another workspace, closing the shown one's last tab asks too.
    let mut shared = one.clone();
    shared
        .workspaces
        .push(eon_workspace_protocol::v43::WorkspaceInfo {
            id: "s2".into(),
            persistent_id: Some("s2".into()),
            registration_error: None,
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
    model.apply(eon_workspace_protocol::v43::Response::Snapshot(one.clone()));
    let worded = |model: &WorkspaceModel| close_prompt_workspace(model.snapshot().unwrap());
    assert_eq!(worded(&model), None);
    assert_eq!(
        model.apply(eon_workspace_protocol::v43::Response::Snapshot(shared)),
        (true, false)
    );
    assert_eq!(worded(&model).as_deref(), Some("s1"));

    // The prompt keeps its close while the named tab and pane live, even when a tab appears
    // and becomes active.
    let prompt = |action| ClosePrompt {
        target: PromptTarget::CloseTab {
            tab: "t1".into(),
            action,
        },
        scroll: 0.0,
    };
    let tab_prompt = prompt(close_tab("t1"));
    assert_eq!(two.active_tab, "t2");
    assert!(matches!(
        &tab_prompt.target,
        PromptTarget::CloseTab { tab, action }
            if tab == "t1" && *action == close_tab("t1")
    ));
    assert!(tab_prompt.names_live_target(&two));
    let pane_prompt = prompt(close_pane("p1"));
    assert!(matches!(
        &pane_prompt.target,
        PromptTarget::CloseTab { tab, action }
            if tab == "t1" && *action == close_pane("p1")
    ));
    assert!(pane_prompt.names_live_target(&two));
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
            modifiers: eon_workspace_protocol::v43::ALT | eon_workspace_protocol::v43::SHIFT,
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

    // After a failed Stop the window names the Quit chord that retries it, unless a configured
    // popup takes that key.
    #[cfg(target_os = "linux")]
    assert_eq!(
        stop_incomplete_notice(&[]),
        "Quit did not finish; press Alt+Shift+Q to retry."
    );
    let agent = [PopupEntry {
        id: "agent".into(),
        label: "Agent".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v43::ALT | eon_workspace_protocol::v43::SHIFT,
            key: "KeyQ".into(),
        }),
    }];
    assert_eq!(stop_incomplete_notice(&agent), "Quit did not finish.");
}

#[test]
fn zoom_names_the_tab_panes_columns_and_selected_pane() {
    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
        endpoint: id.as_bytes().to_vec(),
        live: true,
        attention: None,
    };
    let column = |panes: &[&str]| eon_workspace_protocol::v43::Column {
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v43::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: tabs.len(),
            attention: None,
            recency: 0,
        }],
        active_tab: active.into(),
        tabs,
        geometry: eon_workspace_protocol::v43::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v43::Sidebar::Open,
        attention_queue: Vec::new(),
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
    let waiting = || PendingPaneAction {
        workspace: "s1".into(),
        tab: "t1".into(),
        pane: "p2".into(),
        ticket: 7,
        accepted: false,
        action: PaneHeaderAction::Zoom,
        activate: true,
    };
    let mut pending = waiting();
    assert_eq!(pending.update(Some(&two("p1")), None), None);
    assert_eq!(
        pending.update(Some(&two("p2")), Some((8, true))),
        None,
        "another request's answer must not zoom"
    );
    assert_eq!(
        pending.update(Some(&two("p2")), Some((7, true))),
        Some(true)
    );
    assert_eq!(
        waiting().update(Some(&two("p2")), Some((7, false))),
        Some(false)
    );
    assert_eq!(
        waiting().update(Some(&two("p1")), Some((7, true))),
        Some(false),
        "an accepted answer must still select the exact pane"
    );
    assert_eq!(waiting().update(None, None), Some(false));
    let mut displaced = two("p1");
    displaced.workspace = "s2".into();
    assert_eq!(waiting().update(Some(&displaced), None), Some(false));
    displaced = two("p1");
    displaced.active_tab = "t2".into();
    assert_eq!(waiting().update(Some(&displaced), None), Some(false));
    displaced = two("p1");
    show_test_popup(&mut displaced);
    assert_eq!(waiting().update(Some(&displaced), None), Some(false));
    displaced = two("p1");
    displaced.tabs[0]
        .columns
        .retain(|column| column.expanded != "p2");
    assert_eq!(waiting().update(Some(&displaced), None), Some(false));
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
    labels.tabs[0].columns[0].panes[0].tag = Some("work".into());
    assert_eq!(zoom_target(&labels), middle);
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
                eon_workspace_protocol::v43::Sidebar::Open
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
fn scrollback_pill_leaves_a_held_selection_its_pointer() {
    let pill = Some(SceneRect {
        left: 100.0,
        top: 10.0,
        width: 50.0,
        height: 20.0,
    });
    for (pill, x, y, selecting, expected) in [
        (pill, 120.0, 20.0, false, true),
        (pill, 120.0, 20.0, true, false),
        (pill, 20.0, 20.0, false, false),
        (None, 120.0, 20.0, false, false),
    ] {
        assert_eq!(
            pill_takes_pointer(pill, PhysicalPosition::new(x, y), selecting),
            expected,
            "pointer at ({x}, {y}), selecting {selecting}"
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
fn heading_hide_sends_the_cli_actions() {
    let mut snapshot = quit_snapshot();
    snapshot.tabs = vec![
        Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: "t1".into(),
            directory: b"/work".to_vec(),
            selected_pane: None,
            attention_rollup: None,
            columns: Vec::new(),
        },
        Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: "t2".into(),
            directory: b"/work".to_vec(),
            selected_pane: None,
            attention_rollup: None,
            columns: Vec::new(),
        },
    ];
    // No popup shows, so its button has nothing to send.
    assert_eq!(heading_request(&snapshot, HeadingAction::HidePopup), None);
    // `eon popup hide u1` sends the dismissal of that instance in its owning tab.
    show_test_popup(&mut snapshot);
    assert_eq!(
        heading_request(&snapshot, HeadingAction::HidePopup),
        Some(WorkspaceAction::DismissPopup(
            eon_workspace_protocol::v43::PopupTarget {
                tab: "t1".into(),
                instance: "u1".into(),
            }
        ))
    );
    // A popup hidden in another tab is not the shown one.
    snapshot.active_tab = "t2".into();
    assert_eq!(heading_request(&snapshot, HeadingAction::HidePopup), None);

    // Hide only ever closes an open sidebar: it never toggles a closed one open.
    for (sidebar, request) in [
        (
            Sidebar::Open,
            Some(WorkspaceAction::SetSidebar { open: false }),
        ),
        (Sidebar::Closed, None),
        (Sidebar::Off, None),
    ] {
        snapshot.sidebar = sidebar;
        assert_eq!(
            heading_request(&snapshot, HeadingAction::HideSidebar),
            request
        );
    }
}

/// A one-tab snapshot whose columns hold the named panes, each column's first pane expanded.
fn stack_snapshot(columns: &[&[&str]]) -> Snapshot {
    let mut snapshot = quit_snapshot();
    snapshot.tabs = vec![Tab {
        pending: false,
        selected_popup: None,
        popups: Vec::new(),
        id: "t1".into(),
        directory: b"/work".to_vec(),
        selected_pane: Some(columns[0][0].into()),
        attention_rollup: None,
        columns: columns
            .iter()
            .map(|panes| eon_workspace_protocol::v43::Column {
                panes: panes
                    .iter()
                    .map(|id| Pane {
                        label: (*id).into(),
                        pinned_name: None,
                        tag: None,
                        id: (*id).into(),
                        session: format!("session-{id}"),
                        endpoint: format!("/tmp/{id}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    })
                    .collect(),
                expanded: panes[0].into(),
            })
            .collect(),
    }];
    snapshot
}

/// Runtime's answer to a column close naming `pane`: one idle pane terminal.
fn close_scope(pane: &str) -> CloseScope {
    use eon_workspace_protocol::v43::{CloseTerminal, TargetId, TerminalTarget};
    CloseScope {
        generation: "g7".into(),
        pane: pane.into(),
        end: CloseEnd::Column,
        session: "s1".into(),
        terminals: vec![CloseTerminal {
            target: TerminalTarget {
                workspace: "s1".into(),
                tab: "t1".into(),
                id: TargetId::Pane(pane.into()),
                terminal: "terminal-3".into(),
            },
            run: "run-a".into(),
            name: format!("{pane}  shell"),
            busy: false,
        }],
        untracked: Vec::new(),
    }
}

#[test]
fn stack_close_first_asks_runtime_with_no_consent() {
    use HeadingAction::CloseStack;
    let ask = |pane: &str| WorkspaceAction::CloseColumn {
        pane: pane.into(),
        consent: None,
    };
    // Every Stack asks the same way: one with several panes names its expanded pane, and a
    // one-pane Stack sends a column close too, never a pane close.
    let mut snapshot = stack_snapshot(&[&["p1", "p2"], &["p3"]]);
    snapshot.tabs[0].columns[0].expanded = "p2".into();
    assert_eq!(heading_request(&snapshot, CloseStack(0)), Some(ask("p2")));
    assert_eq!(heading_request(&snapshot, CloseStack(1)), Some(ask("p3")));
    assert_eq!(heading_request(&snapshot, CloseStack(2)), None);
    // So does the only Stack of the only tab, whose close would be final.
    let only = stack_snapshot(&[&["p1"]]);
    assert_eq!(heading_request(&only, CloseStack(0)), Some(ask("p1")));

    // A held press closes nothing once that column's expanded pane changed.
    let pressed = ask("p2");
    assert_eq!(
        held_heading_request(&snapshot, CloseStack(0), &pressed),
        Some(ask("p2"))
    );
    let mut moved = snapshot.clone();
    moved.tabs[0].columns[0].expanded = "p1".into();
    assert_eq!(held_heading_request(&moved, CloseStack(0), &pressed), None);

    // No Stack tab shows under a popup, so its button sends nothing there.
    show_test_popup(&mut snapshot);
    assert_eq!(heading_request(&snapshot, CloseStack(0)), None);
}

#[test]
fn held_popup_hide_never_dismisses_a_popup_shown_in_its_place() {
    let mut snapshot = quit_snapshot();
    snapshot.tabs = vec![Tab {
        pending: false,
        selected_popup: None,
        popups: Vec::new(),
        id: "t1".into(),
        directory: b"/work".to_vec(),
        selected_pane: None,
        attention_rollup: None,
        columns: Vec::new(),
    }];
    show_test_popup(&mut snapshot);
    let hide = HeadingAction::HidePopup;
    // Press Hide on Files (`u1`), and hold.
    let pressed = heading_request(&snapshot, hide).unwrap();
    assert_eq!(
        held_heading_request(&snapshot, hide, &pressed),
        Some(pressed.clone())
    );

    // A shortcut shows another popup while the button is held: the release sends nothing, so
    // the replacement, which a dismissal could stop, stays.
    let mut replacement = snapshot.tabs[0].popups[0].clone();
    replacement.id = "u2".into();
    snapshot.tabs[0].popups.push(replacement);
    snapshot.tabs[0].selected_popup = Some("u2".into());
    assert_eq!(held_heading_request(&snapshot, hide, &pressed), None);
    // Nor does it once no popup shows.
    snapshot.tabs[0].selected_popup = None;
    assert_eq!(held_heading_request(&snapshot, hide, &pressed), None);
    // Back on the pressed popup, the release hides it.
    snapshot.tabs[0].selected_popup = Some("u1".into());
    assert_eq!(
        held_heading_request(&snapshot, hide, &pressed),
        Some(pressed)
    );

    // A sidebar closed under a held Hide is not toggled open by the release.
    let hide = HeadingAction::HideSidebar;
    snapshot.sidebar = Sidebar::Open;
    let pressed = heading_request(&snapshot, hide).unwrap();
    snapshot.sidebar = Sidebar::Closed;
    assert_eq!(held_heading_request(&snapshot, hide, &pressed), None);
}

#[test]
fn sidebar_rows_take_f6_focus_step_by_arrow_and_follow_their_jump() {
    use eon_workspace_protocol::v43::{Attention, AttentionState, Column, PopupGeometry};

    // F6 visits the sidebar between the terminal and the tabs, only while it has rows.
    assert_eq!(
        cycle_focus(WorkspaceFocus::Terminal, true, true),
        WorkspaceFocus::Sidebar
    );
    assert_eq!(
        cycle_focus(WorkspaceFocus::Sidebar, true, true),
        WorkspaceFocus::Tabs
    );
    // The sidebar's Hide button is a stop inside the sidebar, so F6 leaves it as it leaves the rows.
    assert_eq!(
        cycle_focus(
            WorkspaceFocus::HeadingAction(HeadingAction::HideSidebar),
            true,
            true
        ),
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
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v43::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v43::Sidebar::Open,
        attention_queue: Vec::new(),
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
        entry: "files".into(),
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v43::PopupGeometry {
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
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v43::Sidebar::Off,
        attention_queue: Vec::new(),
    };
    assert_eq!(workspace_picker_tooltip(&snapshot.entries), None);
    assert_eq!(invoke_popup(&snapshot, WORKSPACES_ENTRY, true), None);
    snapshot.entries.push(PopupEntry {
        id: WORKSPACES_ENTRY.into(),
        label: "Sessions".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v43::ALT | eon_workspace_protocol::v43::SHIFT,
            key: "KeyS".into(),
        }),
    });
    assert_eq!(
        workspace_picker_tooltip(&snapshot.entries).as_deref(),
        Some(match NATIVE_PLATFORM {
            NativePlatform::Linux => "Sessions — Alt+Shift+S",
            NativePlatform::Macos => "Sessions — Option+Shift+S",
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
        Some("Sessions")
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
        cycle_header_focus(WorkspaceFocus::Tabs, false, false, true, false),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            true,
            false,
            true,
            false
        ),
        Some(WorkspaceFocus::Tabs)
    );
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Panes, false, false, true, false),
        None
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            false,
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
            false,
            false
        ),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    // A workspace label that opens the picker is the first header control.
    let label = WorkspaceFocus::Header(WorkspaceHeaderControl::Workspaces);
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Tabs, false, true, true, false),
        Some(label)
    );
    assert_eq!(
        cycle_header_focus(label, false, true, true, false),
        Some(WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab))
    );
    assert_eq!(
        cycle_header_focus(
            WorkspaceFocus::Header(WorkspaceHeaderControl::NewTab),
            true,
            true,
            true,
            false
        ),
        Some(label)
    );
    assert_eq!(
        cycle_header_focus(label, true, true, true, false),
        Some(WorkspaceFocus::Tabs)
    );
}

/// A running window, generation g7, with three live terminals and none busy.
pub(super) fn quit_snapshot() -> Snapshot {
    use eon_workspace_protocol::v43::{BusyTerminals, RunningWindows, WindowEntry, WindowState};

    Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![],
        active_tab: "t1".into(),
        geometry: eon_workspace_protocol::v43::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs: vec![],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: Some(RunningWindows {
            this: WindowEntry {
                window: None,
                generation: "g7".into(),
                name: None,
                started_at: None,
                live_terminals: Some(3),
                state: WindowState::Live,
            },
            others: None,
            omitted: 0,
        }),
        busy: Some(BusyTerminals {
            terminals: vec![],
            omitted: 0,
        }),
        sidebar: eon_workspace_protocol::v43::Sidebar::Open,
        attention_queue: Vec::new(),
    }
}

#[test]
fn window_menu_navigation_keeps_peer_identity_and_rejects_ended_targets() {
    let control = WorkspaceHeaderControl::WindowMenu;
    assert_eq!(
        workspace_header_action(control, "t1"),
        WorkspaceHeaderAction::ToggleWindowMenu
    );
    let focus = WorkspaceFocus::Header(control);
    assert_eq!(
        cycle_header_focus(WorkspaceFocus::Tabs, false, true, true, true),
        Some(focus)
    );
    assert_eq!(
        cycle_header_focus(focus, true, true, true, true),
        Some(WorkspaceFocus::Tabs)
    );
    let mut snapshot = quit_snapshot();
    let mut peer = snapshot.running.as_ref().unwrap().this.clone();
    peer.window = Some("12345678".into());
    peer.generation = "g1-11111111111111111111111111111111".into();
    let action = WindowMenuAction::peer(&peer);
    snapshot.running.as_mut().unwrap().others = Some(vec![peer]);
    let prompt = ClosePrompt {
        target: PromptTarget::QuitOtherWindow(action.clone()),
        scroll: 0.0,
    };
    assert!(prompt.names_live_target(&snapshot));
    let actions = [
        WindowMenuAction::Detach,
        WindowMenuAction::Quit,
        action.clone(),
    ];
    assert_eq!(
        next_window_action(&actions, Some(&WindowMenuAction::Detach), true),
        Some(action.clone())
    );
    assert_eq!(
        next_window_action(&actions, Some(&action), false),
        Some(WindowMenuAction::Detach)
    );
    // A replacement under the same window ID is not the prompt's generation.
    snapshot.running.as_mut().unwrap().others.as_mut().unwrap()[0]
        .generation
        .push('2');
    assert!(!prompt.names_live_target(&snapshot));
    assert!(peer_window(&snapshot, &action).is_none());
    assert_eq!(next_window_action(&[], None, false), None);
}

/// p1 runs a command.
pub(super) fn busy_p1() -> eon_workspace_protocol::v43::BusyTerminals {
    use eon_workspace_protocol::v43::{BusyTerminal, BusyTerminals, TargetId, TerminalTarget};

    BusyTerminals {
        terminals: vec![BusyTerminal {
            target: TerminalTarget {
                workspace: "s1".into(),
                tab: "t1".into(),
                id: TargetId::Pane("p1".into()),
                terminal: "session-p1".into(),
            },
            name: "p1".into(),
        }],
        omitted: 0,
    }
}

fn stop_g7() -> Outgoing {
    Outgoing::Stop {
        generation: "g7".into(),
    }
}

#[test]
fn quit_asks_only_while_busy() {
    let mut snapshot = quit_snapshot();
    let stop = Some(stop_g7());
    // An all-idle window quits without asking, as `eon quit` does.
    assert_eq!(quit_action(&snapshot), stop);
    assert!(!quit_asks(&snapshot));
    snapshot.busy.as_mut().unwrap().omitted = 1;
    assert!(quit_asks(&snapshot));
    snapshot.busy = Some(busy_p1());
    assert!(quit_asks(&snapshot));
    assert_eq!(quit_action(&snapshot), stop);
    // An absent busy list reads as every terminal busy.
    let busy = snapshot.busy.take();
    assert!(quit_asks(&snapshot));

    // An open prompt is worded again when only the busy list or the terminal count changes.
    let mut next = snapshot.clone();
    assert!(!quit_prompt_changed(&snapshot, &next));
    next.busy.clone_from(&busy);
    assert!(quit_prompt_changed(&snapshot, &next));
    snapshot.busy = busy;
    assert!(!quit_prompt_changed(&snapshot, &next));
    next.busy.as_mut().unwrap().terminals.clear();
    assert!(quit_prompt_changed(&snapshot, &next));
    next.busy.clone_from(&snapshot.busy);
    next.running.as_mut().unwrap().this.live_terminals = Some(2);
    assert!(quit_prompt_changed(&snapshot, &next));
    next.running.clone_from(&snapshot.running);
    next.active_tab = "t2".into();
    assert!(!quit_prompt_changed(&snapshot, &next));

    let prompt = ClosePrompt {
        target: PromptTarget::QuitWindow,
        scroll: 0.0,
    };
    assert!(prompt.names_live_target(&snapshot));
    // eonbase has no running-windows summary, so nothing quits and an open prompt closes.
    snapshot.running = None;
    assert_eq!(quit_action(&snapshot), None);
    assert!(!prompt.names_live_target(&snapshot));
}

#[test]
fn a_link_result_stays_newer_than_a_notice_shown_again_after_the_slow_notice() {
    use eon_workspace_protocol::v43::Response;

    let snapshot = quit_snapshot();
    let mut model = WorkspaceModel::default();
    model.apply(Response::Snapshot(snapshot.clone()));
    model.reject_action("refused");
    let mut links = LinkInteraction::default();
    let result = |links: &mut LinkInteraction, model: &WorkspaceModel| {
        links.show_notice(
            "Cannot open link.",
            false,
            Instant::now(),
            model.notice_serial(),
        );
    };
    let link_first = |links: &LinkInteraction, model: &WorkspaceModel| {
        links
            .notice
            .as_ref()
            .unwrap()
            .outranks(model.notice_serial())
    };

    result(&mut links, &model);
    assert!(
        link_first(&links, &model),
        "the result is newer than the refusal"
    );
    assert!(model.mark_slow());
    assert!(!link_first(&links, &model), "the slow notice is newer");
    model.apply(Response::Snapshot(snapshot.clone()));
    assert_eq!(model.notice(), Some("refused"));
    assert!(
        link_first(&links, &model),
        "the result is still newer than the refusal shown again"
    );

    assert!(model.mark_slow());
    result(&mut links, &model);
    model.apply(Response::Snapshot(snapshot.clone()));
    assert!(
        link_first(&links, &model),
        "so is a result that came while the slow notice showed"
    );
    model.reject_action("later");
    assert!(!link_first(&links, &model), "a later notice outranks it");
}

#[test]
fn quit_progress_follows_workspace_events() {
    use eon_workspace_protocol::v43::{Failure, Response};

    let idle = quit_snapshot();
    let mut busy = idle.clone();
    busy.busy = Some(busy_p1());
    let snapshot =
        |snapshot: &Snapshot| WorkspaceEvent::Response(Response::Snapshot(snapshot.clone()));
    let failure = || {
        WorkspaceEvent::Response(Response::Failure(Failure {
            code: "stop-incomplete".into(),
            detail: "window cleanup is incomplete; retry `eon window stop 00000007`".into(),
        }))
    };
    let effects = |refresh, step| QuitEffects { refresh, step };

    // An open quit prompt is refreshed by a busy-only or count-only change, and by nothing else.
    let mut quit = QuitProgress::Idle;
    assert_eq!(
        quit.on_workspace(Some(&busy), true, &snapshot(&idle)),
        effects(true, None)
    );
    let mut fewer = busy.clone();
    fewer.running.as_mut().unwrap().this.live_terminals = Some(2);
    assert_eq!(
        quit.on_workspace(Some(&busy), true, &snapshot(&fewer)),
        effects(true, None)
    );
    assert_eq!(
        quit.on_workspace(Some(&busy), false, &snapshot(&idle)),
        effects(false, None)
    );
    let mut moved = busy.clone();
    moved.active_tab = "t2".into();
    assert_eq!(
        quit.on_workspace(Some(&busy), true, &snapshot(&moved)),
        effects(false, None)
    );
    assert_eq!(quit, QuitProgress::Idle);

    // A busy window asks at once.
    assert_eq!(quit.begin(Some(&busy)), Some(QuitStep::Ask));
    assert_eq!(quit, QuitProgress::Idle);
    // An idle one waits, and the next snapshot decides: a command started since asks.
    assert_eq!(quit.begin(Some(&idle)), None);
    assert_eq!(quit, QuitProgress::Pending);
    assert_eq!(quit.begin(Some(&idle)), None);
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &snapshot(&busy)),
        effects(false, Some(QuitStep::Ask))
    );
    assert_eq!(quit, QuitProgress::Idle);
    // Still idle: Stop for this generation.
    quit.begin(Some(&idle));
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &snapshot(&idle)),
        effects(false, Some(QuitStep::Stop(stop_g7())))
    );

    // While that Stop is in flight, quitting does nothing and Enter is held.
    quit = QuitProgress::Stopping(5);
    assert!(!quit.may_stop());
    assert_eq!(quit.begin(Some(&idle)), None);
    assert!(quit.holds_key(KeyCode::Enter, session::Modifiers::empty()));
    assert!(!quit.holds_key(KeyCode::Escape, session::Modifiers::empty()));
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &failure()),
        effects(false, None)
    );
    let answered = |ticket| WorkspaceEvent::Answered {
        ticket,
        accepted: false,
    };
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &answered(4)),
        effects(false, None)
    );
    assert_eq!(quit, QuitProgress::Stopping(5));
    // Its failure answer allows a retry, though every refresh now fails: the pending quit
    // decides on the snapshot Venus keeps and sends another Stop.
    quit.on_workspace(Some(&idle), false, &answered(5));
    assert_eq!(quit, QuitProgress::Idle);
    assert!(quit.may_stop());
    assert!(!quit.holds_key(KeyCode::Enter, session::Modifiers::empty()));
    assert_eq!(quit.begin(Some(&idle)), None);
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &failure()),
        effects(false, Some(QuitStep::Stop(stop_g7())))
    );

    // Any other failure, such as an attention jump's no-attention answer, says nothing about
    // what runs: the quit waits for a snapshot, and a command started since still asks.
    assert_eq!(quit.begin(Some(&idle)), None);
    let no_attention = WorkspaceEvent::Response(Response::Failure(Failure {
        code: "no-attention".into(),
        detail: "no agent needs attention".into(),
    }));
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &no_attention),
        effects(false, None)
    );
    assert_eq!(quit, QuitProgress::Pending);
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &snapshot(&busy)),
        effects(false, Some(QuitStep::Ask))
    );

    // Losing the workspace drops a pending quit; a later snapshot decides nothing.
    quit.begin(Some(&idle));
    assert_eq!(
        quit.on_workspace(
            Some(&idle),
            false,
            &WorkspaceEvent::Unavailable {
                notice: "Eon is not answering.".into(),
                detail: None,
            }
        ),
        effects(false, None)
    );
    assert_eq!(quit, QuitProgress::Idle);
    assert_eq!(
        quit.on_workspace(Some(&idle), false, &snapshot(&idle)),
        effects(false, None)
    );
    // eonbase has no running window to quit.
    let mut eonbase = idle.clone();
    eonbase.running = None;
    assert_eq!(quit.begin(Some(&eonbase)), None);
    assert_eq!(quit, QuitProgress::Idle);
}

#[test]
fn a_stack_close_answer_is_the_confirm_before_its_ticket() {
    use eon_workspace_protocol::v43::{Failure, Response};

    let confirm = |pane: &str| WorkspaceEvent::Response(Response::Confirm(close_scope(pane)));
    let answered = |ticket| WorkspaceEvent::Answered {
        ticket,
        accepted: false,
    };
    let pending = |consented| {
        Some(PendingClose {
            pane: "p3".into(),
            ticket: 7,
            consented,
            answer: None,
        })
    };
    // Feeds the events in order and returns what the last one asks, with the slot left.
    let run = |mut slot: Option<PendingClose>, events: &[WorkspaceEvent]| {
        let mut ask = None;
        for event in events {
            ask = PendingClose::on_workspace(&mut slot, event);
        }
        (ask, slot)
    };

    // The `Confirm` delivered just before the request's ticket is its answer; a consented
    // request's answer means the Stack changed.
    for consented in [false, true] {
        assert_eq!(
            run(pending(consented), &[confirm("p3"), answered(7)]),
            (Some((close_scope("p3"), consented)), None)
        );
    }
    // The `Confirm` itself asks nothing: the ticket does.
    assert_eq!(run(pending(false), &[confirm("p3")]).0, None);
    // A slow notice between the request and its answer changes nothing.
    assert_eq!(
        run(
            pending(false),
            &[WorkspaceEvent::Slow, confirm("p3"), answered(7)]
        ),
        (Some((close_scope("p3"), false)), None)
    );

    // A `Confirm` naming another pane is not this request's.
    assert_eq!(
        run(pending(false), &[confirm("p4"), answered(7)]),
        (None, None)
    );
    // Nor is one that answered an older request, whose own ticket follows it: taken as the
    // answer to a consented request it would announce a change that never happened.
    let (ask, slot) = run(pending(true), &[confirm("p3"), answered(6)]);
    assert_eq!((ask, &slot), (None, &pending(true)));
    assert_eq!(run(slot, &[answered(7)]), (None, None));
    // Nor one with a snapshot between it and the ticket.
    let snapshot = WorkspaceEvent::Response(Response::Snapshot(quit_snapshot()));
    assert_eq!(
        run(
            pending(false),
            &[confirm("p3"), snapshot.clone(), answered(7)]
        ),
        (None, None)
    );

    // A request answered by a snapshot or a failure asks nothing and is over.
    let failure = WorkspaceEvent::Response(Response::Failure(Failure {
        code: "close-changed".into(),
        detail: "the column changed".into(),
    }));
    for answer in [snapshot, failure] {
        assert_eq!(
            run(
                pending(true),
                &[
                    answer,
                    WorkspaceEvent::Answered {
                        ticket: 7,
                        accepted: true
                    }
                ]
            ),
            (None, None)
        );
    }
    // Losing the workspace ends the request, and a request forgotten after local input
    // leaves no slot: either way a late answer opens nothing.
    let unavailable = WorkspaceEvent::Unavailable {
        notice: "Eon workspace unavailable".into(),
        detail: None,
    };
    assert_eq!(
        run(pending(false), &[unavailable, confirm("p3"), answered(7)]),
        (None, None)
    );
    assert_eq!(run(None, &[confirm("p3"), answered(7)]), (None, None));
}

#[test]
fn a_window_focus_change_forgets_a_pending_stack_close() {
    use eon_workspace_protocol::v43::Response;
    use winit::event::{DeviceId, TouchPhase};

    let device_id = DeviceId::dummy();
    let mouse = |state| WindowEvent::MouseInput {
        device_id,
        state,
        button: MouseButton::Left,
    };
    let commit = || WindowEvent::Ime(Ime::Commit("é".into()));
    let wheel = || WindowEvent::MouseWheel {
        device_id,
        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
        phase: TouchPhase::Moved,
    };
    let moved = || WindowEvent::CursorMoved {
        device_id,
        position: PhysicalPosition::new(1.0, 1.0),
    };
    let resized = || WindowEvent::Resized(PhysicalSize::new(800, 600));
    // What a pending Stack close asks once its answer arrives after `event`.
    let after = |event: &WindowEvent| {
        let mut slot = Some(PendingClose {
            pane: "p3".into(),
            ticket: 7,
            consented: false,
            answer: None,
        });
        PendingClose::on_window(&mut slot, event);
        let forgotten = slot.is_none();
        PendingClose::on_workspace(
            &mut slot,
            &WorkspaceEvent::Response(Response::Confirm(close_scope("p3"))),
        );
        let ask = PendingClose::on_workspace(
            &mut slot,
            &WorkspaceEvent::Answered {
                ticket: 7,
                accepted: false,
            },
        );
        (forgotten, ask)
    };

    // Local input, and the window gaining or losing focus, forget the request: its answer
    // then opens no prompt, in a window the person may have left.
    for event in [
        WindowEvent::Focused(false),
        WindowEvent::Focused(true),
        commit(),
        mouse(ElementState::Pressed),
        wheel(),
    ] {
        assert_eq!(after(&event), (true, None), "{event:?}");
    }
    // The pointer release is what sent the request, and movement and resizing are not input
    // to it: the answer still asks.
    for event in [mouse(ElementState::Released), moved(), resized()] {
        assert_eq!(
            after(&event),
            (false, Some((close_scope("p3"), false))),
            "{event:?}"
        );
    }

    // A pending pane action keeps its own rule: exactly the three input events, no focus change.
    for event in [commit(), mouse(ElementState::Pressed), wheel()] {
        assert!(local_input(&event), "{event:?}");
    }
    for event in [
        WindowEvent::Focused(false),
        WindowEvent::Focused(true),
        mouse(ElementState::Released),
        moved(),
        resized(),
    ] {
        assert!(!local_input(&event), "{event:?}");
    }
}

#[test]
fn enter_approves_only_after_the_delay_and_a_full_turn() {
    let at = Instant::now();
    let shown = Some(Shown { at, turn: 10 });
    let ms = Duration::from_millis;
    // Nothing presented yet: no wait is long enough.
    assert!(!enter_approves(at + ms(5_000), None, 100));
    // Inside the delay, however many turns passed.
    assert!(!enter_approves(at + ms(399), shown, 100));
    // After the delay, but the turn that presented the list is the only one finished since:
    // a loop stalled across the presentation ages a queued key past the delay.
    assert!(!enter_approves(at + ms(400), shown, 11));
    // After the delay and one full turn that began after the presentation.
    assert!(enter_approves(at + ms(400), shown, 12));
    // The turn counter wraps.
    let wrapped = Some(Shown { at, turn: u64::MAX });
    assert!(!enter_approves(at + ms(400), wrapped, 0));
    assert!(enter_approves(at + ms(400), wrapped, 1));
}

#[test]
fn a_consented_stack_close_survives_resize_before_repaint() {
    let mut presentation = PresentationState::default();
    presentation.publish(presentation.candidate(Some(7)));
    assert!(stack_close_admitted(&presentation, None));

    let at = Instant::now();
    let scope = close_scope("p3");
    let prompt = ClosePrompt {
        target: PromptTarget::CloseColumn {
            scope: scope.clone(),
            changed: false,
            shown: Some(Shown { at, turn: 5 }),
        },
        scroll: 0.0,
    };
    // Resize invalidates targets, then Enter arrives before a replacement frame.
    presentation.invalidate();
    assert!(!stack_close_admitted(&presentation, None));
    assert!(prompt.accepts_enter(at + CONFIRM_DELAY, 7));
    assert!(
        stack_close_admitted(&presentation, Some(&consent_of(&scope))),
        "an accepted Stack close must survive resize before repaint"
    );
}

#[test]
fn a_stack_close_prompt_waits_for_its_own_frame_and_discards_early_enters() {
    let ms = Duration::from_millis;
    let prompt = |pane: &str, changed| ClosePrompt {
        target: PromptTarget::CloseColumn {
            scope: close_scope(pane),
            changed,
            shown: None,
        },
        scroll: 0.0,
    };
    let shown = |prompt: &ClosePrompt| match &prompt.target {
        PromptTarget::CloseColumn { shown, .. } => *shown,
        _ => None,
    };
    let list = DialogKind::CloseStack {
        stack: Some(1),
        terminals: 1,
        running: 0,
        end: CloseEnd::Column,
        session: "s1".into(),
        detach: None,
        changed: false,
    };
    let t0 = Instant::now();

    // Until a frame showing its list is presented, no Enter counts, however late.
    let mut first = prompt("p3", false);
    assert!(!first.accepts_enter(t0 + ms(5_000), 100));
    // A presented frame that drew nothing, or another dialog ahead of it, is not that frame.
    first.presented(None, t0, 5);
    first.presented(Some(&DialogKind::WindowMenu), t0, 5);
    assert_eq!(shown(&first), None);
    assert!(!first.accepts_enter(t0 + ms(5_000), 100));
    // The first frame that drew the list starts the delay, and later frames never move it.
    first.presented(Some(&list), t0, 5);
    let started = Some(Shown { at: t0, turn: 5 });
    assert_eq!(shown(&first), started);
    first.presented(Some(&list), t0 + ms(300), 9);
    assert_eq!(shown(&first), started);
    // An early Enter is refused and leaves nothing behind: the delay neither restarts nor
    // holds the key, so the same prompt accepts a later Enter on its original schedule.
    assert!(!first.accepts_enter(t0 + ms(10), 7));
    assert_eq!(shown(&first), started);
    assert!(first.accepts_enter(t0 + ms(400), 7));

    // A prompt for a second answer starts with nothing shown: an Enter queued behind the
    // first cannot approve its list, whenever it arrives, until that list's own frame.
    let mut second = prompt("p3", true);
    assert_eq!(shown(&second), None);
    assert!(!second.accepts_enter(t0 + ms(5_000), 100));
    second.presented(Some(&list), t0 + ms(500), 20);
    assert!(!second.accepts_enter(t0 + ms(899), 30));
    assert!(second.accepts_enter(t0 + ms(900), 22));

    // The tab, pane and quit prompts keep answering at once, and keep no shown time.
    for target in [
        PromptTarget::CloseTab {
            tab: "t1".into(),
            action: WorkspaceAction::Workspace(CommonAction::CloseTab { tab: "t1".into() }),
        },
        PromptTarget::CloseTab {
            tab: "t1".into(),
            action: WorkspaceAction::ClosePane { pane: "p1".into() },
        },
        PromptTarget::QuitWindow,
    ] {
        let mut prompt = ClosePrompt {
            target,
            scroll: 0.0,
        };
        assert!(prompt.accepts_enter(t0, 0));
        prompt.presented(Some(&list), t0, 0);
        assert_eq!(shown(&prompt), None);
    }
}

#[test]
fn a_stack_close_consent_repeats_exactly_what_was_listed() {
    use eon_workspace_protocol::v43::{CloseTerminal, TargetId, TerminalTarget, UntrackedRun};

    let mut scope = close_scope("p3");
    scope.end = CloseEnd::Window;
    scope.terminals.push(CloseTerminal {
        target: TerminalTarget {
            workspace: "s1".into(),
            tab: "t1".into(),
            id: TargetId::Popup("u1".into()),
            terminal: "terminal-4".into(),
        },
        run: "run-b".into(),
        name: "Files".into(),
        busy: true,
    });
    scope.untracked.push(UntrackedRun {
        terminal: "terminal-9".into(),
        run: "run-c".into(),
    });
    // The generation, the end, and every run in the answer's order: terminals, then the runs
    // no tab tracks. Run IDs only: a terminal number can be reused, a run ID cannot.
    assert_eq!(
        consent_of(&scope),
        CloseConsent {
            generation: "g7".into(),
            runs: vec!["run-a".into(), "run-b".into(), "run-c".into()],
            end: CloseEnd::Window,
        }
    );

    // The prompt lives while the answer's pane is a pane of any tab, the active one or not.
    let prompt = ClosePrompt {
        target: PromptTarget::CloseColumn {
            scope,
            changed: false,
            shown: None,
        },
        scroll: 0.0,
    };
    let mut snapshot = stack_snapshot(&[&["p1"], &["p2", "p3"]]);
    assert!(prompt.names_live_target(&snapshot));
    snapshot.active_tab = "t9".into();
    assert!(prompt.names_live_target(&snapshot));
    snapshot.tabs[0].columns.pop();
    assert!(!prompt.names_live_target(&snapshot));
}

#[test]
fn a_second_enter_reaches_nothing_while_a_stop_is_in_flight() {
    let enter = PhysicalKey::Code(KeyCode::Enter);
    let held = |quit: QuitProgress| quit.holds_key(KeyCode::Enter, session::Modifiers::empty());
    let mut input = InputState::default();
    // While a Stop is in flight, Enter's press and its release are consumed, so neither reaches
    // the terminal, and no second Stop may be sent.
    let mut quit = QuitProgress::Stopping(5);
    assert!(input.consumes_shortcut(enter, ElementState::Pressed, false, held(quit)));
    assert!(!quit.may_stop());
    // The Stop's answer may come between the press and its release; the release is still
    // consumed.
    quit = QuitProgress::Idle;
    assert!(input.consumes_shortcut(enter, ElementState::Released, false, held(quit)));
    // Without a Stop in flight, Enter reaches the terminal.
    assert!(!input.consumes_shortcut(enter, ElementState::Pressed, false, held(quit)));
    assert!(!input.consumes_shortcut(enter, ElementState::Released, false, held(quit)));
}

#[test]
fn a_close_request_quits_where_alt_shift_q_does() {
    let snapshot = quit_snapshot();
    assert_eq!(close_request(Some(&snapshot), false), CloseRequest::Quit);
    // An unavailable workspace cannot answer a Stop, so the close detaches.
    assert_eq!(close_request(Some(&snapshot), true), CloseRequest::Detach);
    // eonbase has no Quit, and a window without a snapshot cannot quit: both detach.
    let mut eonbase = snapshot;
    eonbase.running = None;
    assert_eq!(close_request(Some(&eonbase), false), CloseRequest::Detach);
    assert_eq!(close_request(None, false), CloseRequest::Detach);
}

#[test]
fn window_menu_pointer_waits_for_matching_release_and_hover_never_scrolls() {
    let mut menu = WindowMenuState {
        open: true,
        scroll: 75.0,
        ..WindowMenuState::default()
    };
    assert_eq!(
        menu.button(
            MouseButton::Left,
            ElementState::Pressed,
            Some(WindowMenuAction::Quit)
        ),
        None
    );
    menu.hover(WindowMenuAction::Detach);
    assert_eq!(menu.scroll, 75.0);
    assert_eq!(menu.highlighted, Some(WindowMenuAction::Detach));
    assert_eq!(
        menu.button(
            MouseButton::Left,
            ElementState::Released,
            Some(WindowMenuAction::Detach)
        ),
        None
    );
    menu.button(
        MouseButton::Left,
        ElementState::Pressed,
        Some(WindowMenuAction::Quit),
    );
    assert_eq!(
        menu.button(MouseButton::Left, ElementState::Released, None),
        None
    );
    menu.button(
        MouseButton::Left,
        ElementState::Pressed,
        Some(WindowMenuAction::Quit),
    );
    assert_eq!(
        menu.button(
            MouseButton::Left,
            ElementState::Released,
            Some(WindowMenuAction::Quit)
        ),
        Some(WindowMenuAction::Quit)
    );
    assert_eq!(
        menu.button(
            MouseButton::Left,
            ElementState::Released,
            Some(WindowMenuAction::Quit)
        ),
        None
    );
}

#[test]
fn pane_button_focus_wraps_and_skips_disabled_zoom() {
    use PaneHeaderAction::{Close, Zoom};
    use WorkspaceFocus::{PaneAction, Panes};
    // Without a Stack Close the order is the pane's own: title, Zoom while enabled, Close.
    let cycle = |focus, backwards, zoom| cycle_pane_focus(focus, backwards, true, zoom, None);
    assert_eq!(cycle(Panes, false, true), PaneAction(Zoom));
    assert_eq!(cycle(PaneAction(Zoom), false, true), PaneAction(Close));
    assert_eq!(cycle(PaneAction(Close), false, true), Panes);
    assert_eq!(cycle(Panes, true, true), PaneAction(Close));
    assert_eq!(cycle(PaneAction(Close), true, true), PaneAction(Zoom));
    assert_eq!(cycle(Panes, false, false), PaneAction(Close));
    assert_eq!(cycle(PaneAction(Close), true, false), Panes);

    // The selected Stack's Close comes after the pane's Close and before the title again.
    let stack = |index| WorkspaceFocus::HeadingAction(HeadingAction::CloseStack(index));
    let cycle = |focus, backwards| cycle_pane_focus(focus, backwards, true, true, Some(2));
    assert_eq!(cycle(PaneAction(Close), false), stack(2));
    assert_eq!(cycle(stack(2), false), Panes);
    assert_eq!(cycle(Panes, true), stack(2));
    assert_eq!(cycle(stack(2), true), PaneAction(Close));
    // Where the pane's own buttons do not fit, the title and the Stack's Close remain.
    let cycle = |focus, backwards| cycle_pane_focus(focus, backwards, false, true, Some(2));
    assert_eq!(cycle(Panes, false), stack(2));
    assert_eq!(cycle(stack(2), false), Panes);
    assert_eq!(cycle(Panes, true), stack(2));
    // With neither, Tab rests on the title.
    assert_eq!(cycle_pane_focus(Panes, false, false, true, None), Panes);
    // An unselected Stack's Close, where a screen reader can put focus, is not stranded: Tab
    // and Shift+Tab step from it as from the title.
    let cycle = |focus, backwards| cycle_pane_focus(focus, backwards, true, true, Some(2));
    assert_eq!(cycle(stack(0), false), PaneAction(Zoom));
    assert_eq!(cycle(stack(0), true), stack(2));
}

#[test]
fn stack_close_focus_stays_on_a_shown_button_and_returns_to_the_titles() {
    use HeadingAction::{CloseStack, HidePopup, HideSidebar};
    use WorkspaceFocus::{Panes, Terminal};
    let heading = WorkspaceFocus::HeadingAction;

    // A button that still shows keeps focus, whichever column is selected: the rule takes no
    // selected column, so an unselected Stack's Close is never evicted.
    for action in [CloseStack(0), CloseStack(3), HidePopup, HideSidebar] {
        assert_eq!(retained_heading_focus(action, true), heading(action));
    }
    // One that no longer shows gives focus up: a Stack's to the pane titles, a Hide
    // button's to the terminal.
    assert_eq!(retained_heading_focus(CloseStack(1), false), Panes);
    assert_eq!(retained_heading_focus(HidePopup, false), Terminal);
    assert_eq!(retained_heading_focus(HideSidebar, false), Terminal);

    // Once a consent is sent, focus never stays on a Stack's Close: the Stack that takes the
    // closed one's position is another Stack.
    assert_eq!(focus_after_stack_close(heading(CloseStack(0))), Panes);
    assert_eq!(focus_after_stack_close(heading(CloseStack(2))), Panes);
    // A close confirmed with focus elsewhere, as after a pointer click, moves nothing.
    for focus in [
        Terminal,
        Panes,
        WorkspaceFocus::PaneAction(PaneHeaderAction::Close),
        WorkspaceFocus::Tabs,
        WorkspaceFocus::Sidebar,
        heading(HideSidebar),
    ] {
        assert_eq!(focus_after_stack_close(focus), focus);
    }
}
