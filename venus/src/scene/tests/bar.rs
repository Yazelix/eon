use super::*;

#[test]
fn eon_bar_pill_band_leaves_equal_title_cap_gap() {
    for (scale, line_height, expected) in [
        (1.0, 1.125, (10.0, 30.0)),
        (1.3, 1.125, (13.0, 38.333332)),
        (1.0, 2.0, (16.0, 48.0)),
        (1.3, 2.0, (21.0, 64.333336)),
    ] {
        let mut metrics = CellMetrics::for_scale(scale);
        metrics.height = (metrics.font_size * line_height).round();
        let header = header_heights(metrics).0;
        let cards = card_area(0.0, 1600.0, 1, header, 900.0, false, 0.0, metrics);
        let band = bar_pill_band(metrics, header, cards.top);
        let offset = if scale == 1.0 { 6.0 } else { 7.0 };
        assert!((cards.top - header - offset).abs() < 0.001);
        assert!(
            (band.start - expected.0).abs() < 0.001,
            "{scale}/{line_height}"
        );
        assert!((band.end - band.start - expected.1).abs() < 0.001);
        let shared_top = title_rule_y(
            cards.top,
            chrome_title_metrics(metrics).text_height.min(cards.height),
        );
        let cap_top = shared_top - metrics.font_size * FALLBACK_CAP_HEIGHT_EM / 2.0;
        assert!((band.start - (cap_top - band.end)).abs() < 1.0);
        assert!((band.end - header).abs() < 0.001);
        println!(
            "scale={scale}, line_height={line_height}: top={}, pill_height={}, card_top={}, shared_line={shared_top}, cap_top={cap_top}, cap_gap={}, offset={offset}",
            band.start,
            band.end - band.start,
            cards.top,
            cap_top - band.end
        );
        assert!(band.end - band.start >= metrics.height);
    }
}

#[test]
fn eon_bar_title_gap_uses_the_loaded_font_baseline_and_cap_height() {
    use crate::render::{FontSettings, FontSetup};

    for line_height in [1.125, 2.0] {
        let fonts = FontSetup::new(&FontSettings {
            line_height,
            ..FontSettings::default()
        })
        .unwrap();
        for scale in [1.0, 1.3] {
            let metrics = fonts.metrics(scale);
            assert!(metrics.title_cap_top_em.is_some());
            let header = header_heights(metrics).0;
            let cards = card_area(0.0, 1600.0, 1, header, 900.0, false, 0.0, metrics);
            let band = bar_pill_band(metrics, header, cards.top);
            let cap_top = cards.top + chrome_title_metrics(metrics).cap_top;
            assert!((cap_top - band.end - band.start).abs() <= 0.501);
            assert!((band.end - header).abs() < 0.001);
            println!(
                "loaded title: scale={scale}, line_height={line_height}, cap_offset_em={:?}, top_gap={}, pill_height={}, card_top={}, cap_top={cap_top}, cap_gap={}, offset={}",
                metrics.title_cap_top_em,
                band.start,
                band.end - band.start,
                cards.top,
                cap_top - band.end,
                cards.top - header
            );
        }
    }
}

#[test]
fn quota_without_reset_retains_percent_and_accessible_freshness() {
    for state in [CodexQuotaState::Fresh, CodexQuotaState::Stale] {
        let window = QuotaWindow {
            duration_minutes: 300,
            remaining_percent: 100,
            resets_at: None,
        };
        let text = quota_text(QuotaProvider::Claude, state, 100, Some(&window));
        let suffix = if state == CodexQuotaState::Stale {
            " old"
        } else {
            ""
        };
        assert_eq!(text.label, format!("5h 100%{suffix}"));
        assert!(text.pace.is_none());
        assert!(text.description.contains("reset time unavailable"));
        assert!(!text.description.contains("pace"));
        assert!(
            text.description
                .contains(if state == CodexQuotaState::Stale {
                    "stale"
                } else {
                    "fresh"
                })
        );
    }
}

#[test]
fn quota_shortfall_projects_linear_pace_with_a_basis_only() {
    // A five-hour window observed at `elapsed` share with `remaining` percent left.
    let window = |elapsed: f32, remaining: u8| QuotaWindow {
        duration_minutes: 300,
        remaining_percent: remaining,
        resets_at: Some(1000 + ((1.0 - elapsed) * 18_000.0).round() as u64),
    };
    let shortfall = |elapsed, remaining| quota_shortfall(&window(elapsed, remaining), 1000);
    for (elapsed, remaining, expected) in [
        (0.5, 60, -0.25),  // comfortably under pace: full green
        (0.5, 50, 0.0),    // exactly on pace
        (0.45, 50, 0.1),   // yellow
        (0.375, 50, 0.25), // orange
        (0.25, 50, 0.5),   // full red
        (0.1, 50, 0.8),    // the first judged instant
        (0.5, 100, -1.0),  // nothing used never runs out
        (0.05, 0, 0.95),   // used up, even in a young window
        (0.95, 0, 0.05),
    ] {
        let actual = shortfall(elapsed, remaining).expect("judged window");
        assert!(
            (actual - expected).abs() < 1e-3,
            "{elapsed} elapsed, {remaining}% left: {actual} != {expected}"
        );
    }
    assert_eq!(shortfall(0.09, 50), None, "a young window has no basis");
    let reset_less = QuotaWindow {
        resets_at: None,
        ..window(0.5, 0)
    };
    assert_eq!(quota_shortfall(&reset_less, 1000), None);
    // More used is never better; more elapsed for the same use is never worse.
    for elapsed in (10..=100).map(|step| step as f32 / 100.0) {
        let mut previous = f32::NEG_INFINITY;
        for remaining in (0..=100).rev() {
            let current = shortfall(elapsed, remaining).expect("judged window");
            assert!(current >= previous, "{elapsed} elapsed, {remaining}% left");
            previous = current;
        }
    }
    for remaining in 0..=100 {
        let mut previous = f32::INFINITY;
        for elapsed in (10..=100).map(|step| step as f32 / 100.0) {
            let current = shortfall(elapsed, remaining).expect("judged window");
            assert!(current <= previous, "{elapsed} elapsed, {remaining}% left");
            previous = current;
        }
    }
}

#[test]
fn quota_pace_colors_fresh_windows_and_says_so_in_words() {
    let observed_at = 1_000_000;
    // Five hours: 2h30m in, 50% left, on pace. Seven days: 2d15h in, 50% left,
    // running out 1d18h early.
    let windows = [
        QuotaWindow {
            duration_minutes: 300,
            remaining_percent: 50,
            resets_at: Some(observed_at + 9_000),
        },
        QuotaWindow {
            duration_minutes: 10_080,
            remaining_percent: 50,
            resets_at: Some(observed_at + 378_000),
        },
    ];
    for (window, label, shortfall, detail) in [
        (
            &windows[0],
            "2h30m/5h 50%",
            0.0,
            "5h window: 50% remaining; observed 2h30m into 5h. At this pace it lasts until reset.",
        ),
        (
            &windows[1],
            "2d15h/7d 50%",
            0.25,
            "7d window: 50% remaining; observed 2d15h into 7d. At this pace it runs out about 1d18h before reset.",
        ),
    ] {
        let text = quota_text(
            QuotaProvider::Codex,
            CodexQuotaState::Fresh,
            observed_at,
            Some(window),
        );
        assert_eq!(text.label, label);
        let pace = text.pace.expect("each fresh chip judges its one window");
        assert!((pace - shortfall).abs() < 1e-3);
        assert_eq!(
            text.description,
            format!("Codex quota state: fresh. {detail}")
        );
    }

    let used_up = QuotaWindow {
        duration_minutes: 300,
        remaining_percent: 0,
        resets_at: Some(observed_at + 17_000),
    };
    let text = quota_text(
        QuotaProvider::Claude,
        CodexQuotaState::Fresh,
        observed_at,
        Some(&used_up),
    );
    assert!(text.pace.is_some());
    assert!(
        text.description
            .ends_with("observed 16m into 5h. Used up until reset.")
    );

    // A day-long window under an hour early states one hour.
    let barely = QuotaWindow {
        duration_minutes: 10_080,
        remaining_percent: 50,
        resets_at: Some(observed_at + 302_400 + 1_000),
    };
    let text = quota_text(
        QuotaProvider::Claude,
        CodexQuotaState::Fresh,
        observed_at,
        Some(&barely),
    );
    assert!(
        text.description
            .ends_with("At this pace it runs out about 1h before reset.")
    );
    // And rounds to the nearest hour: 1h50m early states 2h.
    let text = quota_text(
        QuotaProvider::Claude,
        CodexQuotaState::Fresh,
        observed_at,
        Some(&QuotaWindow {
            resets_at: Some(observed_at + 305_700),
            ..barely
        }),
    );
    assert!(
        text.description
            .ends_with("At this pace it runs out about 2h before reset."),
        "{}",
        text.description
    );

    // Stale, blocked, unknown and young windows keep neutral labels and descriptions.
    let young = QuotaWindow {
        resets_at: Some(observed_at + 17_940),
        ..windows[0].clone()
    };
    for (state, window, label) in [
        (
            CodexQuotaState::Stale,
            Some(&windows[0]),
            "2h30m/5h 50% old",
        ),
        (
            CodexQuotaState::Stale,
            Some(&windows[1]),
            "2d15h/7d 50% old",
        ),
        (CodexQuotaState::Blocked, None, "blocked"),
        (CodexQuotaState::Unknown, None, "unknown"),
        (CodexQuotaState::Fresh, Some(&young), "1m/5h 50%"),
    ] {
        let text = quota_text(QuotaProvider::Codex, state, observed_at, window);
        assert_eq!(text.label, label);
        assert!(text.pace.is_none());
        assert!(!text.description.contains("pace") && !text.description.contains("Used up"));
    }
}

#[test]
fn shortcut_hints_drop_whole_pairs_and_rebuild_modifier_headers() {
    let hints = vec![
        ShortcutHint {
            modifiers: "Alt".into(),
            key: "M".into(),
            label: "new pane".into(),
            accessible_label: "Alt+M — New pane".into(),
            priority: 1,
        },
        ShortcutHint {
            modifiers: "Alt".into(),
            key: "/".into(),
            label: "all keys".into(),
            accessible_label: "Alt+/ — All keys".into(),
            priority: 0,
        },
        ShortcutHint {
            modifiers: "Super".into(),
            key: "F".into(),
            label: "界".repeat(80),
            accessible_label: "Super+F — Long custom tool".into(),
            priority: 4,
        },
        ShortcutHint {
            modifiers: "Ctrl+Shift".into(),
            key: "C".into(),
            label: "copy".into(),
            accessible_label: "Ctrl+Shift+C — Copy".into(),
            priority: 3,
        },
    ];
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for width in [0, 10, 120, 320, 340, 2000, 4000] {
            let scene = ShortcutHintsScene::new(
                hints.clone(),
                PhysicalSize::new((f64::from(width) * scale) as u32, 600),
                metrics,
                WorkspaceFocus::Terminal,
                |text| text.chars().count() as f32 * metrics.width,
            );
            let chrome = pane_chrome_rect(
                SceneRect {
                    width: (f64::from(width) * scale) as u32 as f32,
                    height: 600.0,
                    ..SceneRect::default()
                },
                metrics,
            );
            assert_eq!(scene.rect.left, chrome.left);
            assert_eq!(scene.rect.right(), chrome.right());
            assert_eq!(scene.rect.bottom(), 600.0);
            for piece in &scene.pieces {
                assert!(piece.key_offset >= 0.0 && piece.key_offset < piece.label_offset);
                assert!(piece.label_offset < piece.rect.width);
                assert_eq!(piece.key_offset == 0.0, piece.modifiers.is_empty());
                if !piece.modifiers.is_empty() {
                    let text_width = piece.modifiers.chars().count() as f32 * metrics.width;
                    assert!((piece.key_offset - text_width - metrics.padding * 2.0).abs() < 0.01);
                }
                assert!(piece.rect.left >= metrics.padding);
                assert!(piece.rect.right() <= scene.rect.right() - metrics.padding);
                assert!(piece.rect.bottom() <= scene.rect.bottom());
            }
            if let Some((first, last)) = scene.pieces.first().zip(scene.pieces.last()) {
                let left = first.rect.left - scene.rect.left;
                let right = scene.rect.right() - last.rect.right();
                assert!((left - right).abs() < 0.01);
                if scene
                    .pieces
                    .iter()
                    .filter(|p| !p.modifiers.is_empty())
                    .count()
                    > 1
                {
                    assert!((left - metrics.padding).abs() < 0.01);
                }
            }
            let mut group_gap: Option<f32> = None;
            for pair in scene.pieces.windows(2) {
                let gap = pair[1].rect.left - pair[0].rect.right();
                if pair[1].modifiers.is_empty() {
                    assert!((gap - metrics.padding).abs() < 0.01);
                } else {
                    assert!(gap >= metrics.padding * 2.0 - 0.01);
                    if let Some(previous) = group_gap {
                        assert!((gap - previous).abs() < 0.01);
                    }
                    group_gap = Some(gap);
                }
            }
            if width == 340 {
                assert!(!scene.complete);
                assert_eq!(scene.pieces.len(), 2);
                assert_eq!(scene.pieces[0].key, "M");
                assert_eq!(scene.pieces[0].modifiers, "Alt");
                assert_eq!(scene.pieces[1].key, "/");
                assert!(scene.pieces[1].modifiers.is_empty());
            }
            if width >= 2000 {
                assert!(scene.complete);
                assert_eq!(
                    scene
                        .pieces
                        .iter()
                        .filter(|p| !p.modifiers.is_empty())
                        .count(),
                    3
                );
            }
        }
    }
    let small_metrics = CellMetrics {
        width: 3.0,
        height: 6.0,
        font_size: 6.0,
        ..CellMetrics::for_scale(1.0)
    };
    let small = ShortcutHintsScene::new(
        hints.clone(),
        PhysicalSize::new(320, 600),
        small_metrics,
        WorkspaceFocus::Terminal,
        |text| text.chars().count() as f32 * small_metrics.width,
    );
    assert!(
        small.rect.height >= small_metrics.padding,
        "the hints row keeps at least the edge padding height"
    );
    for height in [0, 1, 10, 40] {
        let scene = ShortcutHintsScene::new(
            hints.clone(),
            PhysicalSize::new(320, height),
            CellMetrics::for_scale(1.0),
            WorkspaceFocus::Terminal,
            |text| text.chars().count() as f32 * 10.0,
        );
        assert!(scene.rect.top >= 0.0 && scene.rect.bottom() <= height as f32);
        assert!(scene.pieces.is_empty());
    }
    let scene = ShortcutHintsScene::new(
        hints,
        PhysicalSize::new(220, 600),
        CellMetrics::for_scale(1.0),
        WorkspaceFocus::Terminal,
        |text| text.chars().count() as f32 * 10.0,
    );
    assert_eq!(scene.pieces.len(), 1);
    assert_eq!(scene.pieces[0].key, "/");
    assert_eq!(scene.pieces[0].modifiers, "Alt");
    assert_eq!(scene.pieces[0].label, "all keys");
}

#[test]
fn eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure() {
    use eon_workspace_protocol::v25::{Pane, PopupGeometry, Tab};

    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v25::WorkspaceInfo {
            id: "w1".into(),
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t2".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: (1..=3)
            .map(|index| Tab {
                id: format!("t{index}"),
                directory: format!("/tmp/tab-{index}").into_bytes(),
                pending: false,
                selected_pane: Some(format!("p{index}")),
                selected_popup: None,
                columns: vec![eon_workspace_protocol::v25::Column {
                    panes: vec![Pane {
                        label: format!("p{index}"),
                        pinned_name: None,
                        id: format!("p{index}"),
                        session: format!("s{index}"),
                        endpoint: format!("/tmp/p{index}.sock").into_bytes(),
                        live: true,
                        attention: None,
                    }],
                    expanded: format!("p{index}"),
                }],
                popups: Vec::new(),
                attention_rollup: None,
            })
            .collect(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v25::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let metrics = CellMetrics::for_scale(1.0);

    for width in [960, 100, 1] {
        let scene = WorkspaceScene::from_snapshot(
            &snapshot,
            PhysicalSize::new(width, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        );

        assert!(scene.tab_viewport.width > 0.0);
        assert_eq!(scene.header.height, scene.tab_viewport.height);
        if width == 100 {
            assert!(scene.tabs[1].rect.width - metrics.padding * 2.0 >= metrics.width);
        }
        assert!(scene.tab_viewport.right() <= scene.controls[0].rect.left);
        assert!(scene.controls[0].rect.right() <= scene.drag_region.left);
        assert!(scene.drag_region.right() <= scene.controls[1].rect.left);
        for pair in scene.controls.windows(2) {
            assert!(pair[0].rect.right() <= pair[1].rect.left);
        }
        assert!(scene.controls[2].rect.right() <= width as f32);
        assert_eq!(
            scene.controls.map(|control| scene.hit_test(
                control.rect.left + control.rect.width / 2.0,
                control.rect.height / 2.0
            )),
            [
                Some(WorkspaceHit::Control(WorkspaceHeaderControl::NewTab)),
                Some(WorkspaceHit::Control(WorkspaceHeaderControl::Shortcuts)),
                Some(WorkspaceHit::Control(WorkspaceHeaderControl::CloseTab)),
            ]
        );
    }

    let wide = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(960, 600),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * 10.0),
    );
    assert_eq!(wide.tab_scroll_limit(), 0.0);
    assert_eq!(
        wide.controls[0].rect.left,
        wide.tabs.last().unwrap().rect.right() + tab_gap(metrics, wide.header.height)
    );
    assert!(wide.drag_region.width > 0.0);
    assert_eq!(
        wide.hit_test(
            wide.drag_region.left + wide.drag_region.width / 2.0,
            wide.drag_region.height / 2.0
        ),
        Some(WorkspaceHit::Drag)
    );

    let narrow = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(100, 600),
        metrics,
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * 10.0),
    );
    assert!(narrow.tab_scroll_limit() > 0.0);
    assert_eq!(narrow.controls[0].rect.left, narrow.tab_viewport.right());
    assert_eq!(narrow.drag_region.width, 0.0);
    // The narrowest strip keeps the gap after its first pill, apart from New tab.
    assert_eq!(
        narrow.hit_test(
            narrow.tabs[0].rect.right() + 1.0,
            narrow.tab_viewport.height / 2.0
        ),
        None
    );
    let scrolled = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(100, 600),
        metrics,
        narrow.tab_scroll_limit(),
        0.0,
        |_, text| (text.into(), text.len() as f32 * 10.0),
    );
    assert_eq!(scrolled.controls[0].rect, narrow.controls[0].rect);
    assert_eq!(
        scrolled.tabs.last().unwrap().rect.right() + tab_gap(metrics, scrolled.header.height),
        scrolled.controls[0].rect.left
    );

    let mut snapshot = snapshot;
    snapshot.sidebar = Sidebar::Closed;
    let project = |height, metrics: CellMetrics, hints: bool| {
        let size = PhysicalSize::new(1280, height);
        WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar::default(),
            size,
            metrics,
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * metrics.width),
            hints.then(|| {
                ShortcutHintsScene::new(Vec::new(), size, metrics, WorkspaceFocus::Terminal, |_| {
                    0.0
                })
            }),
        )
    };
    // One pane, no hints: the top gap yields before the nominal 30 px pills shrink.
    for (height, expected) in [
        (102, 10.0..40.0),
        (94, 2.0..32.0),
        (60, 2.0..32.0),
        (40, 2.0..32.0),
        (30, 0.0..30.0),
        (18, 0.0..18.0),
        (17, 4.0..13.0),
        (12, 3.0..9.0),
    ] {
        let scene = project(height, metrics, false);
        let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
        assert_eq!(pill.top..pill.bottom(), expected, "window height {height}");
    }
    // The new title margin yields before the previous layout loses any space.
    for (height, offset) in [
        (102, 0.0),
        (108, 0.0),
        (110, 0.0),
        (112, 2.0),
        (114, 4.0),
        (116, 6.0),
    ] {
        let scene = project(height, metrics, false);
        assert_eq!(
            scene.pane_viewport.top - scene.header.height,
            offset,
            "height {height}"
        );
    }
    for scale in [1.0, 1.3] {
        for line_height in [1.125, 2.0] {
            let mut metrics = CellMetrics::for_scale(scale);
            metrics.height = (metrics.font_size * line_height).round();
            let ordinary = project(900, metrics, false);
            let nominal = ordinary
                .bar_pill_rect(ordinary.tabs[0].rect, metrics)
                .height;
            for hints in [false, true] {
                for height in 0..=220 {
                    let scene = project(height, metrics, hints);
                    let card_top = scene.pane_viewport.top;
                    let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
                    assert!(pill.top >= 0.0 && pill.height >= 0.0);
                    assert!(
                        pill.bottom() <= card_top + 0.001,
                        "{scale}/{line_height}, height {height}, hints {hints}"
                    );
                    if card_top >= nominal {
                        assert!((pill.height - nominal).abs() < 0.001);
                    } else if card_top >= metrics.height {
                        assert_eq!(pill.top, 0.0);
                        assert_eq!(pill.height, card_top);
                    } else {
                        let old = scene.tabs[0].rect;
                        let end = old.bottom().min(card_top);
                        assert!((pill.top - old.top.min(end)).abs() < 0.001);
                        assert!((pill.bottom() - end).abs() < 0.001);
                    }
                }
            }
        }
    }
}

#[test]
fn attention_badges_mark_tab_corners_without_moving_tabs() {
    use eon_workspace_protocol::v25::{
        Attention, AttentionRecord, AttentionState as State, Column, Pane, PopupGeometry,
    };

    let attention = |state, stale| Attention {
        state,
        record: Some(AttentionRecord {
            kind: None,
            progress: None,
            app: "codex".into(),
            name: String::new(),
            title: String::new(),
            message: String::new(),
            changed_at: 1,
            stale,
            others: 0,
        }),
    };
    // Each tab's panes and their attention; the rollup is the most urgent of them.
    let tabs: [&[Option<(State, bool)>]; 8] = [
        &[Some((State::NeedsInput, false))],
        &[Some((State::Error, false))],
        &[Some((State::Finished, true))],
        &[Some((State::Working, false)), Some((State::Idle, false))],
        &[Some((State::Idle, false))],
        &[None],
        &[
            Some((State::NeedsInput, true)),
            Some((State::NeedsInput, false)),
        ],
        &[Some((State::Working, true))],
    ];
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
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: tabs
            .iter()
            .enumerate()
            .map(|(index, panes)| Tab {
                id: format!("t{}", index + 1),
                directory: format!("/tmp/tab-{}", index + 1).into_bytes(),
                pending: false,
                selected_pane: Some(format!("p{index}-0")),
                selected_popup: None,
                columns: vec![Column {
                    panes: panes
                        .iter()
                        .enumerate()
                        .map(|(pane, state)| Pane {
                            label: format!("p{index}-{pane}"),
                            pinned_name: None,
                            id: format!("p{index}-{pane}"),
                            session: format!("s{index}-{pane}"),
                            endpoint: format!("/tmp/p{index}-{pane}.sock").into_bytes(),
                            live: true,
                            attention: state.map(|(state, stale)| attention(state, stale)),
                        })
                        .collect(),
                    expanded: format!("p{index}-0"),
                }],
                popups: Vec::new(),
                attention_rollup: panes.iter().flatten().map(|(state, _)| *state).min(),
            })
            .collect(),
        codex_quota: None,
        claude_quota: None,
        sidebar: eon_workspace_protocol::v25::Sidebar::Closed,
        attention_queue: Vec::new(),
    };
    let metrics = CellMetrics::for_scale(1.0);
    let project = |snapshot: &Snapshot, width, scroll| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(width, 600),
            metrics,
            scroll,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    let scene = project(&snapshot, 1600, 0.0);
    let badges = scene
        .tabs
        .iter()
        .map(|tab| tab.badge.map(|badge| (badge.state, badge.dimmed)))
        .collect::<Vec<_>>();
    assert_eq!(
        badges,
        [
            Some((State::NeedsInput, false)),
            Some((State::Error, false)),
            Some((State::Finished, true)),
            Some((State::Working, false)),
            None,
            None,
            Some((State::NeedsInput, false)),
            Some((State::Working, true)),
        ],
        "the active tab is badged, idle and empty tabs are not, and a badge dims only when \
         every terminal in its state is stale"
    );
    assert_eq!(
        scene.tabs[0].accessible_label(),
        "Tab 1 of 8  /tmp/tab-1, needs input"
    );
    assert_eq!(
        scene.tabs[2].accessible_label(),
        "Tab 3 of 8  /tmp/tab-3, finished, status stale"
    );
    assert_eq!(scene.tabs[4].accessible_label(), "Tab 5 of 8  /tmp/tab-5");
    assert!(scene.edge_badges.is_empty(), "every tab fits");
    // Typography too small for a visible glyph draws no badge but keeps the state.
    let tiny = WorkspaceScene::from_snapshot(
        &snapshot,
        PhysicalSize::new(400, 150),
        CellMetrics::for_scale(0.25),
        0.0,
        0.0,
        |_, text| (text.into(), text.len() as f32 * 2.5),
    );
    assert!(tiny.tabs.iter().all(|tab| tab.badge.is_none()));
    assert_eq!(
        tiny.tabs[0].accessible_label(),
        "Tab 1 of 8  /tmp/tab-1, needs input"
    );
    // Background work badges a tab and names its state.
    let mut waiting = snapshot.clone();
    waiting.tabs[4].columns[0].panes[0].attention = Some(attention(State::Background, false));
    waiting.tabs[4].attention_rollup = Some(State::Background);
    let scene = project(&waiting, 1600, 0.0);
    assert_eq!(
        scene.tabs[4].badge.map(|badge| (badge.state, badge.dimmed)),
        Some((State::Background, false))
    );
    assert_eq!(
        scene.tabs[4].accessible_label(),
        "Tab 5 of 8  /tmp/tab-5, waiting on background work"
    );
    // While the sidebar shows, the tabs carry no badge or state; hidden for width, they do.
    let mut open = snapshot.clone();
    open.sidebar = eon_workspace_protocol::v25::Sidebar::Open;
    let beside = project(&open, 1600, 0.0);
    assert!(beside.sidebar().is_some());
    assert!(beside.tabs.iter().all(|tab| tab.badge.is_none()));
    assert_eq!(beside.tabs[0].accessible_label(), "Tab 1 of 8  /tmp/tab-1");
    let narrow = project(&open, 600, 0.0);
    assert!(narrow.sidebar().is_none());
    assert_eq!(
        narrow.tabs[0].badge.map(|badge| badge.state),
        Some(State::NeedsInput)
    );

    let mut plain = snapshot.clone();
    for tab in &mut plain.tabs {
        tab.attention_rollup = None;
        for pane in &mut tab.columns[0].panes {
            pane.attention = None;
        }
    }
    for (width, scroll) in [(1600, 0.0), (400, 0.0), (400, f32::MAX)] {
        let badged = project(&snapshot, width, scroll);
        let unbadged = project(&plain, width, scroll);
        for (tab, plain) in badged.tabs.iter().zip(&unbadged.tabs) {
            assert_eq!((tab.rect, tab.label()), (plain.rect, plain.label()));
            if let Some(badge) = tab.badge {
                // The badge stays clear of the label's text box and inside the header.
                assert!(badge.rect.left >= tab.rect.right() - metrics.padding);
                assert!(badge.rect.top >= 0.0);
                assert!(badge.rect.bottom() <= badged.header.height);
            }
        }
        assert_eq!(badged.tab_viewport, unbadged.tab_viewport);
    }

    // Overflow: each scroll edge shows the most urgent badge scrolled past it.
    let edges = |scene: &WorkspaceScene| {
        scene
            .edge_badges
            .iter()
            .map(|badge| {
                assert!(badge.rect.left >= scene.tab_viewport.left);
                assert!(badge.rect.right() <= scene.tab_viewport.right());
                // Labels are clipped clear of edge badges, which they would otherwise cover.
                assert!(badge.rect.intersection(scene.tab_label_clip).is_none());
                (
                    badge.rect.left < scene.tab_viewport.width / 2.0,
                    badge.state,
                    badge.dimmed,
                )
            })
            .collect::<Vec<_>>()
    };
    let start = project(&snapshot, 400, 0.0);
    assert!(start.tab_scroll_limit() > 0.0);
    assert_eq!(edges(&start), [(false, State::NeedsInput, false)]);
    let end = project(&snapshot, 400, f32::MAX);
    assert_eq!(edges(&end), [(true, State::NeedsInput, false)]);
    snapshot.tabs[0].columns[0].panes[0].attention = None;
    snapshot.tabs[0].attention_rollup = None;
    assert_eq!(
        edges(&project(&snapshot, 400, f32::MAX)),
        [(true, State::Error, false)]
    );
}

#[test]
fn tab_directory_labels_use_current_position_and_bounded_path_context() {
    assert_eq!(
        tab_labels(0, 5, b"/home/alice"),
        ("1  alice".into(), "Tab 1 of 5  /home/alice".into())
    );
    for directory in [b"/".as_slice(), b"///"] {
        assert_eq!(
            tab_labels(1, 5, directory),
            (
                "2  /".into(),
                format!("Tab 2 of 5  {}", String::from_utf8_lossy(directory))
            )
        );
    }
    assert_eq!(
        tab_labels(2, 5, b"/srv/nova"),
        ("3  nova".into(), "Tab 3 of 5  /srv/nova".into())
    );
    assert_eq!(
        tab_labels(3, 5, b"/tmp/eon-\xff"),
        ("4  eon-�".into(), "Tab 4 of 5  /tmp/eon-�".into())
    );
    let long = format!("/tmp/{}", "eon".repeat(100));
    let labels = tab_labels(4, 5, long.as_bytes());
    assert_eq!(labels.0, format!("5  {}", "eon".repeat(100)));
    assert_eq!(labels.1, format!("Tab 5 of 5  {long}"));
}
