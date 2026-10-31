use super::*;

#[test]
fn agent_sidebar_pushes_columns_hides_for_width_and_shows_its_empty_state() {
    use eon_workspace_protocol::v26::{
        AttentionState, Column, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut,
    };

    let pane = |id: &str| Pane {
        label: id.into(),
        pinned_name: None,
        id: id.into(),
        session: format!("session-{id}"),
        endpoint: format!("/run/{id}.sock").into_bytes(),
        live: true,
        attention: None,
    };
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
            id: "w1".into(),
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
        sidebar: Sidebar::Open,
        attention_queue: Vec::new(),
    };
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
    snapshot.tabs[0].popups[0].attention = Some(eon_workspace_protocol::v26::Attention {
        state: AttentionState::Unavailable,
        record: None,
    });
    snapshot.tabs[0].attention_rollup = Some(AttentionState::Unavailable);
    assert!(!project(&snapshot, 1400).sidebar_empty());
}

#[test]
fn agent_sidebar_rows_show_attention_in_tab_order_and_reveal_the_focused_row() {
    use eon_workspace_protocol::v26::{
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
        id: id.into(),
        session: format!("session-{id}"),
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
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v26::WorkspaceInfo {
            id: "w1".into(),
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
        sidebar: Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let metrics = CellMetrics::for_scale(1.0);
    let project = |snapshot: &Snapshot, height, focused| {
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
            SidebarView { now: NOW, focused },
        )
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
                "Agent popup",
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

    // A popup's row carries the marker after the catalog label whether or not the popup is
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
                .map(|row| {
                    let (before, after) = row.detail.split_at(row.popup_marker.unwrap());
                    (row.target.clone(), before.to_owned(), after.to_owned())
                })
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        marked(&scene),
        (
            vec!["2  site".into(), "3  api".into()],
            vec![("u2".into(), "Agent popup".into(), String::new())]
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
            vec![("u2".into(), "4m · Agent popup".into(), " · +1".into())]
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
