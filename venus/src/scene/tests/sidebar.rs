use super::*;

/// One tab of two one-pane columns and the hidden popup `u1`, labelled Agent, with the sidebar
/// open.
fn sidebar_snapshot() -> Snapshot {
    use eon_workspace_protocol::v44::{
        Column, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut,
    };

    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
        endpoint: format!("/run/{id}.sock").into_bytes(),
        live: true,
        attention: None,
    };
    Snapshot {
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v44::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![PopupEntry {
            id: "agent".into(),
            label: "Agent".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT,
                key: "KeyA".into(),
            }),
        }],
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/work".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: ["p1", "p2"]
                .map(|id| Column {
                    panes: vec![pane(id)],
                    expanded: id.into(),
                })
                .into(),
            popups: vec![Popup {
                id: "u1".into(),
                entry: "agent".into(),
                session: "session-u1".into(),
                endpoint: b"/run/u1.sock".to_vec(),
                attention: None,
            }],
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
        sidebar: Sidebar::Open,
        attention_queue: Vec::new(),
    }
}

fn overflowing_sidebar_snapshot() -> Snapshot {
    let mut snapshot = sidebar_snapshot();
    snapshot.tabs[0].popups[0].attention = Some(Attention {
        state: AttentionState::Unavailable,
        record: None,
    });
    let mut pane = snapshot.tabs[0].columns[0].panes[0].clone();
    pane.attention = Some(Attention {
        state: AttentionState::Unavailable,
        record: None,
    });
    snapshot.tabs[0].columns[0].panes = (1..=20)
        .map(|index| {
            let mut pane = pane.clone();
            pane.id = format!("p{index}");
            pane
        })
        .collect();
    snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
    snapshot
}

fn scrolled_sidebar(
    snapshot: &Snapshot,
    metrics: CellMetrics,
    scroll: f32,
    focused: Option<&str>,
) -> WorkspaceScene {
    WorkspaceScene::from_snapshot_with_zoom(
        snapshot,
        EonBar::default(),
        PhysicalSize::new(
            (1600.0 * metrics.scale) as u32,
            (450.0 * metrics.scale) as u32,
        ),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * metrics.width),
        None,
        false,
        SidebarView {
            scroll,
            focused,
            ..SidebarView::default()
        },
    )
}

#[test]
fn sidebar_scroll_clamps_reveals_and_updates_pointer_targets() {
    let snapshot = overflowing_sidebar_snapshot();
    let metrics = CellMetrics::for_scale(1.0);
    let start = scrolled_sidebar(&snapshot, metrics, -100.0, None);
    assert_eq!(start.sidebar_scroll(), 0.0);
    let end = scrolled_sidebar(&snapshot, metrics, f32::MAX, None);
    assert!(end.sidebar_max_scroll() > 0.0);
    assert_eq!(end.sidebar_scroll(), end.sidebar_max_scroll());
    let pointer = (
        start.sidebar_list.left + 10.0,
        start.sidebar_list.top + 80.0,
    );
    let before = start.hit_test(pointer.0, pointer.1);
    let scrolled = scrolled_sidebar(&snapshot, metrics, 120.5, Some("p1"));
    assert_eq!(
        scrolled.sidebar_scroll(),
        120.5,
        "focus styling must not undo a wheel scroll"
    );
    let after = scrolled.hit_test(pointer.0, pointer.1);
    assert!(matches!(before, Some(WorkspaceHit::SidebarRow(_))));
    assert!(matches!(after, Some(WorkspaceHit::SidebarRow(_))));
    assert_ne!(
        before, after,
        "hover and activation follow the row now under the pointer"
    );
    let focused = scrolled_sidebar(
        &snapshot,
        metrics,
        start.sidebar_reveal_scroll("p20"),
        Some("p20"),
    );
    let row = focused.sidebar_row("p20").unwrap();
    assert!(
        row.rect.top >= focused.sidebar_list.top
            && row.rect.bottom() <= focused.sidebar_list.bottom()
    );
    let restored = scrolled_sidebar(
        &snapshot,
        metrics,
        focused.sidebar_reveal_scroll("p1"),
        Some("p1"),
    );
    assert!(restored.sidebar_row("p1").unwrap().rect.top >= restored.sidebar_list.top);
    let mut empty = snapshot;
    empty.tabs[0].attention_rollup = None;
    assert_eq!(
        scrolled_sidebar(&empty, metrics, 120.5, None).sidebar_scroll(),
        0.0
    );
}

#[test]
fn sidebar_scroll_at_start_keeps_new_attention_visible() {
    let mut snapshot = overflowing_sidebar_snapshot();
    let metrics = CellMetrics::for_scale(1.0);
    let start = scrolled_sidebar(&snapshot, metrics, 0.0, None);
    let mut popup = snapshot.tabs[0].popups[0].clone();
    popup.id = "new-attention".into();
    snapshot.tabs[0].popups.insert(0, popup);
    let updated = scrolled_sidebar(&snapshot, metrics, 0.0, None);
    assert_eq!(
        updated.sidebar_preserved_scroll(&start),
        0.0,
        "new attention above the fold must remain visible when pinned to the top"
    );
}

#[test]
fn sidebar_scroll_preserves_visible_rows_across_refresh() {
    let mut snapshot = overflowing_sidebar_snapshot();
    let metrics = CellMetrics::for_scale(1.0);
    let before = scrolled_sidebar(&snapshot, metrics, 220.5, None);
    let anchor = before
        .sidebar_rows()
        .iter()
        .find(|row| row.rect.intersection(before.sidebar_list).is_some())
        .unwrap()
        .clone();
    let mut inserted = snapshot.tabs[0].columns[0].panes[0].clone();
    inserted.id = "inserted".into();
    snapshot.tabs[0].columns[0].panes.insert(0, inserted);
    let next = scrolled_sidebar(&snapshot, metrics, before.sidebar_scroll(), None);
    let stable = scrolled_sidebar(
        &snapshot,
        metrics,
        next.sidebar_preserved_scroll(&before),
        None,
    );
    assert_eq!(
        stable.sidebar_row(&anchor.target).unwrap().rect.top,
        anchor.rect.top
    );
    snapshot.tabs[0].columns[0].panes.remove(0);
    let next = scrolled_sidebar(&snapshot, metrics, stable.sidebar_scroll(), None);
    let restored = scrolled_sidebar(
        &snapshot,
        metrics,
        next.sidebar_preserved_scroll(&stable),
        None,
    );
    assert_eq!(
        restored.sidebar_row(&anchor.target).unwrap().rect.top,
        anchor.rect.top
    );
    snapshot.tabs[0].columns[0]
        .panes
        .retain(|pane| pane.id != anchor.target);
    let next = scrolled_sidebar(&snapshot, metrics, restored.sidebar_scroll(), None);
    let removed = scrolled_sidebar(
        &snapshot,
        metrics,
        next.sidebar_preserved_scroll(&restored),
        None,
    );
    let neighbour = restored
        .sidebar_rows()
        .iter()
        .find(|row| row.rect.top > anchor.rect.top)
        .unwrap();
    assert_eq!(
        removed.sidebar_row(&neighbour.target).unwrap().rect.top,
        neighbour.rect.top
    );
}

#[test]
fn sidebar_thumb_gutter_clears_rows_and_markers_at_fractional_scales() {
    let snapshot = overflowing_sidebar_snapshot();
    for scale in [1.0, 1.125, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let scene = scrolled_sidebar(&snapshot, metrics, 120.5, None);
        for border in [1.0_f32, 2.0, 3.0] {
            let stroke = (border * metrics.scale).round().max(1.0);
            let track = scene.sidebar_scroll_track(metrics, stroke).unwrap();
            let card = scene.sidebar().unwrap();
            for row in scene.sidebar_rows() {
                assert!(
                    track.left > row.rect.right() + 1.0,
                    "clear keyboard outlines at {scale}"
                );
                assert!(
                    row.popup_marker
                        .is_none_or(|marker| marker.right() < track.left)
                );
            }
            assert!(track.right() < card.right() - stroke);
            assert!(
                track.top >= scene.sidebar_list.top
                    && track.bottom() <= scene.sidebar_list.bottom()
            );
            let mut fitting = snapshot.clone();
            fitting.tabs[0].columns[0].panes.truncate(1);
            assert!(
                scrolled_sidebar(&fitting, metrics, 0.0, None)
                    .sidebar_scroll_track(metrics, stroke)
                    .is_none()
            );
        }
    }
}

#[test]
fn heading_hide_targets_sit_in_their_title_rows_and_yield_when_narrow() {
    let mut snapshot = sidebar_snapshot();
    snapshot.tabs[0].selected_popup = Some("u1".into());
    let metrics = CellMetrics::for_scale(1.0);
    let project = |snapshot: &Snapshot, width| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(width, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    let (popup, sidebar) = (HeadingAction::HidePopup, HeadingAction::HideSidebar);

    // Each button is a square of the title height at the right of its heading's title row,
    // half a padding from the card's edge, and is what a pointer there hits.
    let scene = project(&snapshot, 1400);
    let button = chrome_title_metrics(metrics).text_height;
    for (action, right) in [
        (popup, scene.chrome.right()),
        (sidebar, scene.sidebar().unwrap().right()),
    ] {
        let target = scene.heading_action(action).unwrap();
        assert_eq!(
            (target.top, target.width, target.height),
            (scene.pane_viewport.top, button, button),
            "{action:?}"
        );
        assert_eq!(target.right(), right - metrics.padding / 2.0, "{action:?}");
        for (x, y) in [
            (target.left, target.top),
            (target.right() - 0.5, target.bottom() - 0.5),
        ] {
            assert_eq!(
                scene.hit_test(x, y),
                Some(WorkspaceHit::HeadingAction(action)),
                "{action:?}"
            );
        }
        // It covers neither the popup's terminal nor the sidebar's rows.
        assert_eq!(target.intersection(scene.terminal), None, "{action:?}");
        assert_eq!(target.intersection(scene.sidebar_list), None, "{action:?}");
    }
    assert_eq!(scene.heading_action_label(popup), "Hide popup Agent");
    assert_eq!(scene.heading_action_label(sidebar), "Hide agent sidebar");

    // Without a shown popup only the sidebar has a button, and where the popup's was hits none.
    let shown = scene.heading_action(popup).unwrap();
    snapshot.tabs[0].selected_popup = None;
    let columns = project(&snapshot, 1400);
    assert_eq!(columns.heading_action(popup), None);
    assert_eq!(
        columns.heading_action(sidebar),
        scene.heading_action(sidebar)
    );
    assert_ne!(
        columns.hit_test(shown.left, shown.top),
        Some(WorkspaceHit::HeadingAction(popup))
    );

    // A sidebar that hides for width, or is closed, has no button.
    let narrowest = (card_gap(metrics) + 280.0 + 800.0) as u32;
    assert!(
        project(&snapshot, narrowest)
            .heading_action(sidebar)
            .is_some()
    );
    assert_eq!(
        project(&snapshot, narrowest - 1).heading_action(sidebar),
        None
    );
    snapshot.sidebar = Sidebar::Closed;
    assert_eq!(project(&snapshot, 1400).heading_action(sidebar), None);

    // A popup shows its button only with four paddings of title row left beside it, so a
    // narrow popup shows none.
    snapshot.tabs[0].selected_popup = Some("u1".into());
    let mut shown = [false; 2];
    for width in (40..=400).step_by(10) {
        let scene = project(&snapshot, width);
        let target = scene.heading_action(popup);
        shown[usize::from(target.is_some())] = true;
        if let Some(target) = target {
            assert!(
                target.left - scene.chrome.left >= metrics.padding * 3.5,
                "{width}: {target:?} in {:?}",
                scene.chrome
            );
        }
    }
    assert_eq!(
        shown, [true; 2],
        "narrow popups yield and roomy ones do not"
    );
}

#[test]
fn agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state() {
    use eon_workspace_protocol::v44::AttentionState;

    let mut snapshot = sidebar_snapshot();
    let metrics = CellMetrics::for_scale(1.0);
    let project = |snapshot: &Snapshot, width| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(width, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    let gap = card_gap(metrics);

    // A visible sidebar is 28 cells wide, one card gap from the window's left edge and as
    // tall as the cards, which it pushes right; the columns share the content area.
    let open = project(&snapshot, 1400);
    let sidebar = open.sidebar().unwrap();
    assert_eq!((sidebar.left, sidebar.width), (gap, 280.0));
    assert_eq!(
        (sidebar.top, sidebar.bottom()),
        (
            open.card_rect(
                open.chrome,
                metrics,
                crate::render::physical_outline_width(2, metrics.scale)
            )
            .top,
            open.pane_viewport.bottom()
        )
    );
    assert_eq!(open.content_width(1400.0), 1400.0 - sidebar.right());
    assert_eq!(
        (open.pane_viewport.left, open.pane_viewport.right()),
        (sidebar.right() + gap, 1400.0 - gap)
    );
    assert_eq!(
        open.chrome.width,
        workspace_column_width(open.content_width(1400.0), 2, metrics)
    );
    assert!(open.sidebar_empty());

    // Closed, the cards take the window back; the first tab pill never moves.
    snapshot.sidebar = Sidebar::Closed;
    let closed = project(&snapshot, 1400);
    assert_eq!(closed.sidebar(), None);
    assert_eq!(closed.content_width(1400.0), 1400.0);
    assert_eq!(closed.pane_viewport.left, gap);
    assert_eq!(closed.tabs[0].rect, open.tabs[0].rect);
    snapshot.sidebar = Sidebar::Off;
    assert_eq!(project(&snapshot, 1400).pane_viewport, closed.pane_viewport);

    // A popup's card is the content area, so it never covers the sidebar.
    snapshot.sidebar = Sidebar::Open;
    snapshot.tabs[0].selected_popup = Some("u1".into());
    let popup = project(&snapshot, 1400);
    assert_eq!(popup.sidebar(), Some(sidebar));
    assert_eq!(popup.chrome, open.pane_viewport);
    assert!(popup.terminal.left > sidebar.right());
    snapshot.tabs[0].selected_popup = None;

    // The sidebar hides once the content area would keep fewer than 80 cells, leaving the
    // layout it would have closed.
    let narrowest = (gap + 280.0 + 800.0) as u32;
    assert!(project(&snapshot, narrowest).sidebar().is_some());
    let hidden = project(&snapshot, narrowest - 1);
    assert_eq!(hidden.sidebar(), None);
    snapshot.sidebar = Sidebar::Closed;
    assert_eq!(
        hidden.pane_viewport,
        project(&snapshot, narrowest - 1).pane_viewport
    );

    // Any attention replaces the empty state.
    snapshot.sidebar = Sidebar::Open;
    snapshot.tabs[0].popups[0].attention = Some(eon_workspace_protocol::v44::Attention {
        state: AttentionState::Unavailable,
        record: None,
    });
    snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
    assert!(!project(&snapshot, 1400).sidebar_empty());
}

#[test]
fn agent_sidebar_rows_show_attention_in_tab_order_and_reveal_the_focused_row() {
    use eon_workspace_protocol::v44::{
        Attention, AttentionRecord, Column, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut,
    };

    const NOW: u64 = 1_800_000_000_000;
    let record = |app: &str, title: &str, age: u64| AttentionRecord {
        kind: None,
        progress: None,
        app: app.into(),
        name: String::new(),
        title: title.into(),
        message: String::new(),
        changed_at: NOW - age,
        stale: false,
        others: 0,
    };
    let pane = |id: &str, attention: Option<Attention>| Pane {
        label: id.into(),
        pinned_name: None,
        tag: None,
        id: id.into(),
        session: format!("terminal-{id}"),
        endpoint: format!("/run/{id}.sock").into_bytes(),
        live: true,
        attention,
    };
    let tab = |id: &str, directory: &str, panes: Vec<Pane>, popups, rollup| Tab {
        id: id.into(),
        directory: directory.as_bytes().to_vec(),
        pending: false,
        selected_pane: Some(panes[0].id.clone()),
        selected_popup: None,
        columns: panes
            .into_iter()
            .map(|pane| Column {
                expanded: pane.id.clone(),
                panes: vec![pane],
            })
            .collect(),
        popups,
        attention_rollup: rollup,
    };
    let needs_input = Attention {
        state: AttentionState::NeedsInput,
        record: Some(AttentionRecord {
            kind: Some(AttentionKind::Permission),
            name: "eon-9a".into(),
            message: "Allow Bash: cargo test?".into(),
            others: 2,
            ..record("claude-code", "Review PR", 4 * MINUTE_MS)
        }),
    };
    // Other apps' terminal titles, such as Codex's spinner title or a shell's command
    // line, never lead a row.
    let working = Attention {
        state: AttentionState::Working,
        record: Some(AttentionRecord {
            stale: true,
            name: "⠼ w".into(),
            ..record("codex", "Fix tests", 30_000)
        }),
    };
    let idle = Attention {
        state: AttentionState::Idle,
        record: Some(AttentionRecord {
            name: "~/w> ./i".into(),
            ..record("aider", "", 2 * HOUR_MS)
        }),
    };
    let snapshot = Snapshot {
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v44::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 3,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![PopupEntry {
            id: "agent".into(),
            label: "Agent popup".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT,
                key: "KeyA".into(),
            }),
        }],
        tabs: vec![
            tab("t1", "/work", vec![pane("p1", None)], Vec::new(), None),
            tab(
                "t2",
                "/home/u/site",
                vec![pane("p2", Some(needs_input)), pane("p3", None)],
                vec![Popup {
                    id: "u2".into(),
                    entry: "agent".into(),
                    session: "session-u2".into(),
                    endpoint: b"/run/u2.sock".to_vec(),
                    attention: Some(Attention {
                        state: AttentionState::Unavailable,
                        record: None,
                    }),
                }],
                Some(AttentionState::NeedsInput),
            ),
            tab(
                "t3",
                "/srv/api",
                vec![pane("p4", Some(working)), pane("p5", Some(idle))],
                Vec::new(),
                Some(AttentionState::Working),
            ),
        ],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let metrics = CellMetrics::for_scale(1.0);
    let project = |snapshot: &Snapshot, height, focused| {
        let build = |scroll| {
            WorkspaceScene::from_snapshot_with_zoom(
                snapshot,
                EonBar::default(),
                PhysicalSize::new(1400, height),
                metrics,
                0.0,
                0.0,
                |_, text| (text.into(), text.len() as f32 * 10.0),
                None,
                false,
                SidebarView {
                    now: NOW,
                    focused,
                    scroll,
                },
            )
        };
        let scene = build(0.0);
        build(focused.map_or(0.0, |target| scene.sidebar_reveal_scroll(target)))
    };

    // Only tabs with attention are listed, each under its heading, with one row per
    // terminal with attention, in tab order: a popup's row first, then the panes' rows in
    // pane order.
    let scene = project(&snapshot, 800, None);
    assert!(!scene.sidebar_empty());
    assert_eq!(
        scene
            .sidebar_headings
            .iter()
            .map(|heading| (heading.tab.as_str(), heading.label.as_str()))
            .collect::<Vec<_>>(),
        [("t2", "2  site"), ("t3", "3  api")]
    );
    #[allow(clippy::type_complexity)]
    fn summary(
        row: &SidebarRow,
    ) -> (
        &str,
        &str,
        AttentionState,
        bool,
        Option<QuotaProvider>,
        &str,
        &str,
        Option<&str>,
    ) {
        (
            row.tab.as_str(),
            row.target.as_str(),
            row.glyph.state,
            row.glyph.dimmed,
            row.icon.map(|(provider, _)| provider),
            row.title.as_str(),
            row.detail.as_str(),
            row.message.as_deref(),
        )
    }
    assert_eq!(
        scene.sidebar_rows().iter().map(summary).collect::<Vec<_>>(),
        [
            (
                "t2",
                "u2",
                AttentionState::Unavailable,
                false,
                None,
                "status unavailable",
                "Agent popup\u{200e}",
                None,
            ),
            (
                "t2",
                "p2",
                AttentionState::NeedsInput,
                false,
                Some(QuotaProvider::Claude),
                "eon-9a",
                "4m · p2 · +2",
                Some("Allow Bash: cargo test?"),
            ),
            (
                "t3",
                "p4",
                AttentionState::Working,
                true,
                Some(QuotaProvider::Codex),
                "Fix tests",
                "now · p4 · status stale",
                None,
            ),
            // A non-Claude terminal title is ignored; without a title, the app shows
            // verbatim, never the tab's directory leaf.
            (
                "t3",
                "p5",
                AttentionState::Idle,
                false,
                None,
                "aider",
                "2h · p5",
                None,
            ),
        ]
    );
    assert_eq!(
        scene
            .sidebar_rows()
            .iter()
            .map(|row| row.accessible_label.as_str())
            .collect::<Vec<_>>(),
        [
            "Status unavailable: agent in tab 2, site, Agent popup, popup",
            "Needs input (permission): Claude eon-9a in tab 2, site, p2, Review PR, \
             changed 4 minutes ago, 2 more records: Allow Bash: cargo test?",
            "Working: Codex in tab 3, api, p4, Fix tests, changed just now, status stale",
            "Idle: aider in tab 3, api, p5, changed 2 hours ago",
        ]
    );

    // Pane pins name both attention surfaces without changing the reporting app or popup.
    let mut renamed = snapshot.clone();
    for (app, name, pin, expected, provider) in [
        ("codex", "", None, "Codex", QuotaProvider::Codex),
        (
            "codex",
            "",
            Some("events ciwn.8"),
            "events ciwn.8",
            QuotaProvider::Codex,
        ),
        ("codex", "", None, "Codex", QuotaProvider::Codex),
        (
            "claude-code",
            "eon-overseer",
            None,
            "eon-overseer",
            QuotaProvider::Claude,
        ),
        (
            "claude-code",
            "eon-overseer",
            Some("person's choice"),
            "person's choice",
            QuotaProvider::Claude,
        ),
    ] {
        let pane = &mut renamed.tabs[1].columns[0].panes[0];
        pane.pinned_name = pin.map(str::to_owned);
        let attention = pane.attention.as_mut().unwrap();
        let record = attention.record.as_mut().unwrap();
        record.app = app.into();
        record.name = name.into();
        record.title.clear();
        renamed.attention_queue = vec![eon_workspace_protocol::v44::QueueEntry {
            workspace: renamed.workspace.clone(),
            tab: "t2".into(),
            target: "p2".into(),
            attention: attention.clone(),
        }];
        let sidebar = project(&renamed, 800, None);
        let inbox = DialogScene::attention_inbox(
            &renamed,
            AttentionFilter::All,
            None,
            PhysicalSize::new(1400, 800),
            metrics,
            0.0,
            NOW,
        );
        for row in [sidebar.sidebar_row("p2").unwrap(), &inbox.attention_rows[0]] {
            assert_eq!(row.title, expected);
            assert_eq!(row.icon.map(|(provider, _)| provider), Some(provider));
            if pin.is_some() {
                assert_eq!(
                    row.accessible_label,
                    match provider {
                        QuotaProvider::Google => unreachable!("Google has no sidebar icon"),
                        QuotaProvider::Codex =>
                            "Needs input (permission): Codex, events ciwn.8 \
                            in tab 2, site, p2, changed 4 minutes ago, 2 more records: \
                            Allow Bash: cargo test?",
                        QuotaProvider::Claude =>
                            "Needs input (permission): Claude eon-overseer, \
                            person's choice in tab 2, site, p2, changed 4 minutes ago, \
                            2 more records: Allow Bash: cargo test?",
                    }
                );
            }
        }
        assert_eq!(
            sidebar.sidebar_row("u2").unwrap().title,
            "status unavailable"
        );
    }
    let mut other_session = renamed.clone();
    other_session
        .workspaces
        .push(eon_workspace_protocol::v44::WorkspaceInfo {
            id: "s2".into(),
            persistent_id: Some("0123456789abcdef0123456789abcdef0000000000000002".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: Some(AttentionState::NeedsInput),
            recency: 0,
        });
    other_session.attention_queue[0].workspace = "s2".into();
    assert_eq!(
        DialogScene::attention_inbox(
            &other_session,
            AttentionFilter::All,
            None,
            PhysicalSize::new(1400, 800),
            metrics,
            0.0,
            NOW,
        )
        .attention_rows[0]
            .title,
        "eon-overseer"
    );
    renamed.tabs[1].popups[0].attention = renamed.tabs[1].columns[0].panes[0].attention.clone();
    renamed.attention_queue[0].target = "u2".into();
    assert_eq!(
        project(&renamed, 800, None)
            .sidebar_row("u2")
            .unwrap()
            .title,
        "eon-overseer"
    );
    let inbox = DialogScene::attention_inbox(
        &renamed,
        AttentionFilter::All,
        None,
        PhysicalSize::new(1400, 800),
        metrics,
        0.0,
        NOW,
    );
    let row = &inbox.attention_rows[0];
    assert_eq!(row.title, "eon-overseer");
    let marker = row.popup_marker.unwrap();
    assert_eq!(marker.right(), row.rect.right() - metrics.padding / 2.0);
    assert!(row.text_right < marker.left);
    assert!(row.accessible_label.contains(", popup"));
    renamed.tabs[1].columns[0].panes[0].attention = Some(Attention {
        state: AttentionState::Unavailable,
        record: None,
    });
    assert_eq!(
        project(&renamed, 800, None)
            .sidebar_row("p2")
            .unwrap()
            .title,
        "person's choice"
    );

    // A row waiting on background work shows its own glyph and word.
    let mut waiting = snapshot.clone();
    if let Some(attention) = &mut waiting.tabs[2].columns[0].panes[0].attention {
        attention.state = AttentionState::Background;
    }
    let waiting = project(&waiting, 800, None);
    let row = &waiting.sidebar_rows()[2];
    assert_eq!(row.glyph.state, AttentionState::Background);
    assert_eq!(
        row.accessible_label,
        "Waiting on background work: Codex in tab 3, api, p4, Fix tests, changed just now, \
         status stale"
    );

    // Rows stack inside the card below the title, the needs-input row one line taller;
    // the icon sits after the glyph and the text after both.
    let card = scene.sidebar().unwrap();
    let rows = scene.sidebar_rows();
    assert!(scene.sidebar_list.top > card.top && scene.sidebar_list.bottom() < card.bottom());
    assert_eq!(scene.sidebar_headings[0].rect.top, scene.sidebar_list.top);
    // Text clips to a box taller than the line pitch, which cut the descender of the
    // title's g, and the title's box ends above the list.
    assert!(chrome_title_metrics(metrics).text_height > metrics.height);
    assert!(
        card.top + metrics.padding + chrome_title_metrics(metrics).text_height
            <= scene.sidebar_list.top
    );
    for pair in rows.windows(2) {
        assert!(pair[0].rect.bottom() <= pair[1].rect.top);
    }
    assert_eq!(rows[1].rect.height - rows[3].rect.height, metrics.height);
    let (_, icon) = rows[1].icon.unwrap();
    assert!(rows[1].glyph.rect.right() < icon.left && icon.right() < rows[1].text_left);
    assert!(rows[0].text_left < rows[1].text_left);
    assert!(
        rows.iter()
            .all(|row| row.rect.left >= card.left && row.rect.right() <= card.right())
    );

    // Rows are hit targets; the youngest age changes its label first.
    let center = |rect: SceneRect| (rect.left + rect.width / 2.0, rect.top + rect.height / 2.0);
    let (x, y) = center(rows[2].rect);
    assert_eq!(scene.hit_test(x, y), Some(WorkspaceHit::SidebarRow("p4")));
    assert_eq!(
        scene.sidebar_row("u2").map(|row| row.tab.as_str()),
        Some("t2")
    );
    assert_eq!(scene.sidebar_age_wait(), Some(30_000));

    // In a short window the list clips, and the focused row scrolls just into view.
    let short = project(&snapshot, 260, None);
    let list = short.sidebar_list;
    assert!(short.sidebar_rows()[3].rect.bottom() > list.bottom());
    assert!(short.sidebar_rows().iter().all(|row| !row.focused));
    let revealed = project(&snapshot, 260, Some("p5"));
    let focused = &revealed.sidebar_rows()[3];
    assert!(focused.focused);
    assert_eq!(focused.rect.bottom(), list.bottom());
    let (x, y) = center(focused.rect);
    assert_eq!(
        revealed.hit_test(x, y),
        Some(WorkspaceHit::SidebarRow("p5"))
    );
    let offset = short.sidebar_rows()[3].rect.top - focused.rect.top;
    assert!(offset > 0.0);
    assert_eq!(
        short.sidebar_row("u2").unwrap().popup_marker.unwrap().top
            - revealed
                .sidebar_row("u2")
                .unwrap()
                .popup_marker
                .unwrap()
                .top,
        offset
    );
    assert_eq!(
        revealed.sidebar_headings[0].rect.top,
        short.sidebar_headings[0].rect.top - offset
    );
    // The part of a row scrolled above the list is no hit target.
    let cut = revealed
        .sidebar_rows()
        .iter()
        .find(|row| row.rect.top < list.top - 1.0 && row.rect.bottom() > list.top)
        .unwrap();
    let (x, _) = center(cut.rect);
    assert_ne!(
        revealed.hit_test(x, list.top - 1.0),
        Some(WorkspaceHit::SidebarRow(cut.target.as_str()))
    );

    // A popup's row carries the right-edge marker whether or not the popup is
    // shown, and no pane's row does; headings never carry it.
    let marked = |scene: &WorkspaceScene| {
        (
            scene
                .sidebar_headings
                .iter()
                .map(|heading| heading.label.clone())
                .collect::<Vec<_>>(),
            scene
                .sidebar_rows()
                .iter()
                .filter(|row| row.popup_marker.is_some())
                .map(|row| (row.target.clone(), row.detail.clone()))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        marked(&scene),
        (
            vec!["2  site".into(), "3  api".into()],
            vec![("u2".into(), "Agent popup\u{200e}".into())]
        )
    );
    let mut shown = snapshot.clone();
    shown.tabs[1].selected_popup = Some("u2".into());
    shown.tabs[1].popups[0].attention = Some(Attention {
        state: AttentionState::Working,
        record: Some(AttentionRecord {
            others: 1,
            ..record("claude-code", "", 4 * MINUTE_MS)
        }),
    });
    let scene = project(&shown, 800, None);
    assert_eq!(
        marked(&scene),
        (
            vec!["2  site".into(), "3  api".into()],
            vec![("u2".into(), "4m · Agent popup\u{200e} · +1".into())]
        )
    );
    assert_eq!(
        scene.sidebar_row("u2").unwrap().accessible_label,
        "Working: Claude in tab 2, site, Agent popup, popup, changed 4 minutes ago, \
         1 more record"
    );

    // Only the row of the active tab's focused terminal, its shown popup or else its
    // selected pane, is current. A popup shown in another tab marks nothing, and the
    // sidebar's own focus stays a separate state.
    let current = |scene: &WorkspaceScene| {
        scene
            .sidebar_rows()
            .iter()
            .filter(|row| row.current)
            .map(|row| row.target.clone())
            .collect::<Vec<_>>()
    };
    assert!(current(&project(&snapshot, 800, None)).is_empty());
    assert!(current(&scene).is_empty());
    let mut pane_focused = snapshot.clone();
    pane_focused.active_tab = "t2".into();
    let scene = project(&pane_focused, 800, Some("p4"));
    assert_eq!(current(&scene), ["p2"]);
    let (p2, p4) = (
        scene.sidebar_row("p2").unwrap(),
        scene.sidebar_row("p4").unwrap(),
    );
    assert!(!p2.focused && p4.focused && !p4.current);
    assert_eq!(
        p2.accessible_label,
        "Needs input (permission): Claude eon-9a in tab 2, site, p2, current, Review PR, \
         changed 4 minutes ago, 2 more records: Allow Bash: cargo test?"
    );
    shown.active_tab = "t2".into();
    let scene = project(&shown, 800, None);
    assert_eq!(current(&scene), ["u2"]);
    assert_eq!(
        scene.sidebar_row("u2").unwrap().accessible_label,
        "Working: Claude in tab 2, site, Agent popup, popup, current, \
         changed 4 minutes ago, 1 more record"
    );
}

#[test]
fn agent_sidebar_ages_count_whole_minutes_hours_and_days() {
    assert_eq!(age_labels(0), ("now".into(), "just now".into()));
    assert_eq!(age_labels(MINUTE_MS - 1).0, "now");
    assert_eq!(age_labels(MINUTE_MS), ("1m".into(), "1 minute ago".into()));
    assert_eq!(age_labels(HOUR_MS - 1).0, "59m");
    assert_eq!(age_labels(HOUR_MS).0, "1h");
    assert_eq!(
        age_labels(3 * DAY_MS + 1),
        ("3d".into(), "3 days ago".into())
    );
    // Each label holds until the next whole unit.
    assert_eq!(age_wait(0), MINUTE_MS);
    assert_eq!(age_wait(MINUTE_MS + 1), MINUTE_MS - 1);
    assert_eq!(age_wait(HOUR_MS + MINUTE_MS), HOUR_MS - MINUTE_MS);
    assert_eq!(age_wait(DAY_MS), DAY_MS);
}

#[test]
fn popup_marker_reserves_the_right_edge_and_tracks_row_height_and_scroll() {
    use eon_workspace_protocol::v44::{Attention, AttentionRecord, AttentionState};

    for scale in [0.75, 1.0, 1.25, 1.5, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for message in ["", "A long question that occupies the third line"] {
            for width in [0.0, metrics.font_size, 28.0 * metrics.width] {
                let attention = Attention {
                    state: AttentionState::NeedsInput,
                    record: Some(AttentionRecord {
                        kind: None,
                        progress: None,
                        app: "codex".into(),
                        name: String::new(),
                        title: "Long popup label".repeat(20),
                        message: message.into(),
                        changed_at: 1,
                        stale: true,
                        others: 2,
                    }),
                };
                let mut row = attention_row(
                    &attention,
                    None,
                    "t1",
                    "u1",
                    "חלון",
                    true,
                    None,
                    None,
                    false,
                    false,
                    60_001,
                    metrics,
                    SceneRect {
                        left: 10.0,
                        top: 30.0,
                        width,
                        height: 800.0,
                    },
                );
                let glyph = row.glyph.rect;
                assert_eq!(glyph.width, metrics.font_size.round().max(1.0));
                assert_eq!(glyph.height, glyph.width);
                let lines = if message.is_empty() { 2.0 } else { 3.0 };
                assert_eq!(
                    row.rect.height,
                    metrics.height * lines + attention_row_gap(metrics)
                );
                assert!(glyph.top >= row.rect.top);
                assert!(
                    glyph.bottom()
                        <= row.rect.top + attention_row_gap(metrics) / 2.0 + metrics.height
                );
                let marker = row.popup_marker.unwrap();
                assert_eq!(
                    marker.width,
                    (metrics.font_size * 1.25)
                        .round()
                        .max(1.0)
                        .min((row.rect.width - metrics.padding / 2.0).max(0.0))
                );
                assert!(marker.left >= row.rect.left && marker.right() <= row.rect.right());
                assert!(marker.top >= row.rect.top && marker.bottom() <= row.rect.bottom());
                assert_eq!(
                    marker.top + marker.height / 2.0,
                    row.rect.top + row.rect.height / 2.0
                );
                assert_eq!(marker.width, marker.height);
                assert!(row.text_right < marker.left);
                if width == 28.0 * metrics.width {
                    assert!(marker.width > metrics.font_size);
                    assert_eq!(marker.right(), row.rect.right() - metrics.padding / 2.0);
                    assert!(row.text_right > row.text_left);
                } else {
                    assert!(row.text_right <= row.text_left);
                }
                assert_eq!(row.detail, "1m · חלון\u{200e} · +2 · status stale");
                assert!(row.glyph.dimmed && row.accessible_label.contains(", popup"));
                let top = row.rect.top;
                row.shift(-13.0);
                assert_eq!(row.rect.top, top - 13.0);
                assert_eq!(row.popup_marker.unwrap().top, marker.top - 13.0);
            }
        }
    }
}

#[test]
fn popup_detail_keeps_metadata_ltr_after_an_rtl_catalog_label() {
    use eon_workspace_protocol::v44::{Attention, AttentionRecord, AttentionState};
    use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};

    let metrics = CellMetrics::for_scale(1.0);
    let row = attention_row(
        &Attention {
            state: AttentionState::Working,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: "codex".into(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1,
                stale: true,
                others: 2,
            }),
        },
        None,
        "t1",
        "u1",
        "שלום",
        true,
        None,
        None,
        false,
        false,
        60_001,
        metrics,
        SceneRect {
            width: 1000.0,
            height: 100.0,
            ..SceneRect::default()
        },
    );
    let mut fonts = FontSystem::new();
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(metrics.font_size, metrics.height));
    buffer.set_size(Some(1000.0), Some(metrics.height));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        &row.detail,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    let line = buffer.layout_runs().next().unwrap();
    let x = |offset| {
        line.glyphs
            .iter()
            .find(|glyph| glyph.start == offset)
            .unwrap()
            .x
    };
    let plus = row.detail.find("+2").unwrap();
    let source = row.detail.find("שלום").unwrap();
    let source_end = line
        .glyphs
        .iter()
        .filter(|glyph| (source..source + "שלום".len()).contains(&glyph.start))
        .map(|glyph| glyph.x + glyph.w)
        .fold(0.0, f32::max);
    assert!(source_end < x(plus), "metadata must follow the RTL label");
    assert!(x(plus) < x(plus + 1), "+2 must read left to right");
    assert!(x(plus + 1) < x(row.detail.find("status stale").unwrap()));
}
