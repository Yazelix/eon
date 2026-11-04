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
            scope: None,
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
        scope: None,
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
            scope: None,
        },
        QuotaWindow {
            duration_minutes: 10_080,
            remaining_percent: 50,
            resets_at: Some(observed_at + 378_000),
            scope: None,
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
        scope: None,
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
        scope: None,
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
            label: "+pane".into(),
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
    use eon_workspace_protocol::v32::{Pane, PopupGeometry, Tab};

    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v32::WorkspaceInfo {
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
                columns: vec![eon_workspace_protocol::v32::Column {
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
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
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
    use eon_workspace_protocol::v32::{
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
        workspaces: vec![eon_workspace_protocol::v32::WorkspaceInfo {
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
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Closed,
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
    open.sidebar = eon_workspace_protocol::v32::Sidebar::Open;
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

#[test]
fn cpu_and_ram_chips_colour_under_load_and_say_so() {
    use crate::{CpuReadings, CpuTimes, Load, LoadLevels, RamUse};
    use eon_workspace_protocol::v32::{Pane, PopupGeometry, Tab};
    use std::time::{Duration, Instant};

    let levels = LoadLevels::default();
    let ram = |percent: u64, levels| {
        RamUse::parse(&format!(
            "MemTotal: 100000 kB\nMemAvailable: {} kB\n",
            (100 - percent) * 1000
        ))
        .unwrap()
        .rated(levels)
    };
    for (percent, load, words) in [
        (79, Load::Normal, ""),
        (80, Load::High, ": under pressure"),
        (89, Load::High, ": under pressure"),
        (90, Load::Critical, ": under heavy pressure"),
    ] {
        let shown = ram(percent, levels);
        assert_eq!(shown.load, load, "{percent}%");
        assert_eq!(
            shown.description,
            format!("RAM use {percent}% of 0.1 GiB{words}")
        );
    }
    let configured = LoadLevels {
        ram_warning_percent: 50,
        ram_critical_percent: 50,
        ..levels
    };
    assert_eq!(ram(49, configured).load, Load::Normal);
    assert_eq!(ram(50, configured).load, Load::Critical);

    // One reading every 2 seconds, each interval `busy` percent busy.
    let start = Instant::now();
    let times = |busy: u64, idle: u64| {
        CpuTimes::parse(&format!(
            "cpu {busy} 0 0 {idle} 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0"
        ))
    };
    let gap = Duration::from_secs(3);
    let run = |readings: &mut CpuReadings, levels, from: u64, busy: &[u64]| {
        let (mut busy_ticks, mut idle_ticks) = (0, 0);
        busy.iter()
            .enumerate()
            .map(|(step, percent)| {
                busy_ticks += percent;
                idle_ticks += 100 - percent;
                readings.push(
                    start + Duration::from_secs(from + 2 * step as u64),
                    times(busy_ticks, idle_ticks),
                    gap,
                    levels,
                )
            })
            .collect::<Vec<_>>()
    };
    let average = |percent: u64, window: u64| {
        format!("; {percent}% on average over the last {window} seconds")
    };
    // A burst colours the chip at once, and hover gives the quieter average beside it.
    let mut readings = CpuReadings::default();
    let mut burst = vec![10; 16];
    burst[15] = 100;
    let shown = run(&mut readings, levels, 0, &burst);
    let busy = shown[15].as_ref().unwrap();
    assert_eq!((busy.label.as_str(), busy.load), ("CPU 100%", Load::High));
    assert_eq!(
        busy.description,
        format!(
            "CPU use 100% across 1 logical CPU, over the last 2 seconds: under high load{}",
            average(16, 30)
        )
    );
    // The level is inclusive and follows the shown use, whatever the average.
    let mut readings = CpuReadings::default();
    let mut at_level = vec![10; 17];
    at_level[16] = 90;
    let shown = run(&mut readings, levels, 0, &at_level).remove(16).unwrap();
    assert_eq!(shown.load, Load::High);
    assert!(
        shown
            .description
            .ends_with(&format!(": under high load{}", average(15, 30)))
    );
    let mut readings = CpuReadings::default();
    let mut below = vec![95; 17];
    below[16] = 89;
    let shown = run(&mut readings, levels, 0, &below).remove(16).unwrap();
    assert_eq!(shown.load, Load::Normal);
    assert_eq!(
        shown.description,
        format!(
            "CPU use 89% across 1 logical CPU, over the last 2 seconds{}",
            average(95, 30)
        )
    );
    // The average shows only once the readings span the window.
    let mut readings = CpuReadings::default();
    let shown = run(&mut readings, levels, 0, &[95; 16]);
    assert!(shown[0].is_none());
    assert_eq!(shown[14].as_ref().unwrap().load, Load::High);
    assert_eq!(
        shown[14].as_ref().unwrap().description,
        "CPU use 95% across 1 logical CPU, over the last 2 seconds: under high load"
    );
    assert!(
        shown[15]
            .as_ref()
            .unwrap()
            .description
            .ends_with(&average(95, 30))
    );
    // A stalled or failed reading starts the window over.
    let resumed = run(&mut readings, levels, 40, &[95; 16]);
    assert!(resumed[0].is_none());
    assert!(
        !resumed[14]
            .as_ref()
            .unwrap()
            .description
            .contains("average")
    );
    assert!(
        resumed[15]
            .as_ref()
            .unwrap()
            .description
            .ends_with(&average(95, 30))
    );
    assert!(
        readings
            .push(start + Duration::from_secs(72), None, gap, levels)
            .is_none()
    );
    // Configured levels and windows apply.
    let quick = LoadLevels {
        cpu_warning_percent: 50,
        cpu_average: Duration::from_secs(4),
        ..levels
    };
    let mut readings = CpuReadings::default();
    let shown = run(&mut readings, quick, 0, &[60, 40, 60]);
    assert_eq!(shown[1].as_ref().unwrap().load, Load::Normal);
    let quick_shown = shown[2].as_ref().unwrap();
    assert_eq!(quick_shown.load, Load::High);
    assert!(quick_shown.description.ends_with(&average(50, 4)));
    // The average covers exactly the window, even when it is odd or the readings are uneven:
    // an idle interval then a busy one is 67% over the last 3 of 4 seconds, and 50% over the last
    // 4 of 4.5 seconds when the idle interval took 2.5 of them.
    for (window, busy_at, average) in [(3, [2.0, 4.0], 67), (4, [2.5, 4.5], 50)] {
        let levels = LoadLevels {
            cpu_warning_percent: 1,
            cpu_average: Duration::from_secs(window),
            ..levels
        };
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let mut readings = CpuReadings::default();
        readings.push(at(0.0), times(0, 0), gap, levels);
        let idle = (busy_at[0] * 100.0) as u64;
        readings.push(at(busy_at[0]), times(0, idle), gap, levels);
        let busy = ((busy_at[1] - busy_at[0]) * 100.0) as u64;
        let shown = readings
            .push(at(busy_at[1]), times(busy, idle), gap, levels)
            .unwrap();
        assert!(
            shown.description.ends_with(&format!(
                "; {average}% on average over the last {window} seconds"
            )),
            "{}",
            shown.description
        );
    }

    // The scene carries each reading's load to its chip, and the words to hover and accessibility.
    let pressed = ram(91, levels);
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v32::WorkspaceInfo {
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
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let scene = WorkspaceScene::from_snapshot_with_bar(
        &snapshot,
        EonBar {
            items: &[crate::BarItem::Cpu, crate::BarItem::Ram],
            cpu: Some(busy),
            ram: Some(&pressed),
            ..EonBar::default()
        },
        PhysicalSize::new(3000, 600),
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
        None,
    );
    let (cpu, ram) = (scene.cpu.unwrap(), scene.ram.unwrap());
    assert_eq!((cpu.load, ram.load), (Load::High, Load::Critical));
    assert_eq!(cpu.description, busy.description);
    assert_eq!(
        ram.description,
        "RAM use 91% of 0.1 GiB: under heavy pressure"
    );
}

#[test]
fn a_failed_cpu_reading_restarts_the_average_window() {
    use crate::{CpuReadings, CpuTimes, LoadLevels};
    use std::time::{Duration, Instant};

    let levels = LoadLevels::default();
    let start = Instant::now();
    let gap = Duration::from_secs(3);
    let (mut busy, mut idle) = (0, 0);
    let mut readings = CpuReadings::default();
    // Two seconds apart, each interval 95% busy; `None` is a failed reading.
    let mut push = |step: u64, read: bool| {
        busy += 95;
        idle += 5;
        let times = CpuTimes::parse(&format!(
            "cpu {busy} 0 0 {idle} 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0"
        ))
        .filter(|_| read);
        readings.push(start + Duration::from_secs(2 * step), times, gap, levels)
    };
    let average = "; 95% on average over the last 30 seconds";
    for step in 0..16 {
        push(step, true);
    }
    assert!(push(16, true).unwrap().description.ends_with(average));
    assert!(push(17, false).is_none());
    // The first reading after the failure has nothing to compare with.
    assert!(push(18, true).is_none());
    // Its use shows again from the next reading, without an average until a full 30 seconds of
    // new readings has accumulated.
    for step in 19..33 {
        let description = push(step, true).unwrap().description;
        assert!(
            !description.contains("average"),
            "step {step}: {description}"
        );
    }
    assert!(push(33, true).unwrap().description.ends_with(average));
}

#[test]
fn disk_chip_shows_filesystem_use_and_colours_only_when_filling_up() {
    use crate::{BarItem, DiskUse, Load, LoadLevels};
    use eon_workspace_protocol::v32::{Pane, PopupGeometry, Tab};

    let levels = LoadLevels::default();
    // A 1000 GiB filesystem with no root reserve, `percent` used.
    let disk = |percent: u64, levels| {
        let gib = 1 << 18;
        DiskUse::of(
            1000 * gib,
            (100 - percent) * 10 * gib,
            (100 - percent) * 10 * gib,
            4096,
            "/",
        )
        .unwrap()
        .rated(levels)
    };
    for (percent, load, words) in [
        (84, Load::Normal, ""),
        (85, Load::High, ": filling up"),
        (94, Load::High, ": filling up"),
        (95, Load::Critical, ": nearly full"),
    ] {
        let shown = disk(percent, levels);
        assert_eq!(shown.label, format!("DISK {percent}%"));
        assert_eq!(shown.load, load, "{percent}%");
        assert_eq!(
            shown.description,
            format!(
                "Disk use {percent}% of 1000.0 GiB on /, {}.0 GiB free{words}",
                (100 - percent) * 10
            )
        );
    }
    let configured = LoadLevels {
        disk_warning_percent: 50,
        disk_critical_percent: 70,
        ..levels
    };
    assert_eq!(disk(49, configured).load, Load::Normal);
    assert_eq!(disk(50, configured).load, Load::High);
    assert_eq!(disk(70, configured).load, Load::Critical);

    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: vec![eon_workspace_protocol::v32::WorkspaceInfo {
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
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let metrics = CellMetrics::for_scale(1.0);
    let project = |items: &[BarItem], disk| {
        WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar {
                items,
                disk,
                ..EonBar::default()
            },
            PhysicalSize::new(3000, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            None,
        )
    };
    let full = disk(95, levels);
    let scene = project(&[BarItem::Disk], Some(&full));
    let shown = scene.disk.as_ref().unwrap();
    assert_eq!(shown.label, "DISK 95%");
    assert_eq!(shown.load, Load::Critical);
    assert_eq!(shown.description, full.description);
    // Sized for `DISK 100%`, so the bar never shifts as the use changes.
    assert_eq!(
        shown.rect.width,
        "DISK 100%".len() as f32 * metrics.width + metrics.padding * 2.0
    );
    assert_eq!(shown.rect.right(), scene.controls[1].rect.left);
    assert_eq!(
        scene.hit_test(
            shown.rect.left + shown.rect.width / 2.0,
            shown.rect.height / 2.0
        ),
        Some(crate::WorkspaceHit::Disk)
    );
    // A missing reading, or a disk left out of the bar, shows no chip.
    let empty = project(&[BarItem::Disk], None);
    assert!(empty.disk.is_none());
    assert_eq!(empty.drag_region.right(), empty.controls[1].rect.left);
    assert!(project(&[BarItem::Ram], Some(&full)).disk.is_none());
}

#[test]
fn eon_icon_leads_the_bar_and_yields_first() {
    use eon_workspace_protocol::v32::{Pane, PopupGeometry, Tab};
    let tab = |id: String| Tab {
        id: id.clone(),
        directory: b"/tmp".to_vec(),
        pending: false,
        selected_pane: Some(id.clone()),
        selected_popup: None,
        columns: vec![eon_workspace_protocol::v32::Column {
            panes: vec![Pane {
                label: id.clone(),
                pinned_name: None,
                id: id.clone(),
                session: id.clone(),
                endpoint: b"/tmp/p.sock".to_vec(),
                live: true,
                attention: None,
            }],
            expanded: id,
        }],
        popups: Vec::new(),
        attention_rollup: None,
    };
    let used = crate::RamUse::parse("MemTotal: 100000 kB\nMemAvailable: 50000 kB\n")
        .unwrap()
        .rated(crate::LoadLevels::default());
    let mut states = [false; 2];
    for scale in [1.0, 1.25, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for (workspaces, tab_count) in [(1, 1), (2, 1), (1, 12), (2, 12)] {
            let snapshot = Snapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: (1..=workspaces)
                    .map(|n| eon_workspace_protocol::v32::WorkspaceInfo {
                        id: format!("w{n}"),
                        name: None,
                        tab_count,
                        attention: None,
                        recency: 0,
                    })
                    .collect(),
                active_tab: "t1".into(),
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: Vec::new(),
                tabs: (1..=tab_count).map(|n| tab(format!("t{n}"))).collect(),
                codex_quota: None,
                claude_quota: None,
                codex_quota_failure: None,
                claude_quota_failure: None,
                running: None,
                busy: None,
                sidebar: eon_workspace_protocol::v32::Sidebar::Open,
                attention_queue: Vec::new(),
            };
            for (version, ram) in [
                (Some("9.8.7"), None),
                (None, None),
                (Some("9.8.7"), Some(&used)),
            ] {
                // The icon hides first and stays hidden: once it shows as the window widens, it
                // shows at every wider width.
                let mut shown_from = None;
                for logical_width in (100..=1600).step_by(10) {
                    let size = PhysicalSize::new((logical_width as f64 * scale) as u32, 600);
                    let project = |icon, tab_scroll| {
                        WorkspaceScene::from_snapshot_with_bar(
                            &snapshot,
                            EonBar {
                                version,
                                icon,
                                ram,
                                ..EonBar::default()
                            },
                            size,
                            metrics,
                            tab_scroll,
                            0.0,
                            |_, text| {
                                (
                                    text.to_owned(),
                                    text.chars().count() as f32 * 10.0 * scale as f32,
                                )
                            },
                            None,
                        )
                    };
                    let plain = project(false, 0.0);
                    let iconic = project(true, 0.0);
                    assert!(plain.eon_icon.is_none());
                    if tab_count == 12 && logical_width == 600 {
                        assert!(plain.tab_scroll_limit() > 0.5, "{scale}: 12 tabs overflow");
                    }
                    // The icon yields before anything else: without room it leaves today's bar,
                    // whatever the tab scroll.
                    let Some(icon) = iconic.eon_icon else {
                        assert_eq!(iconic, plain);
                        assert_eq!(shown_from, None, "{scale} {logical_width}: icon came back");
                        let limit = plain.tab_scroll_limit();
                        for tab_scroll in [limit / 2.0, limit - 1.0, limit, 1.0e6] {
                            assert_eq!(
                                project(true, tab_scroll),
                                project(false, tab_scroll),
                                "{scale} {logical_width} {tab_scroll}"
                            );
                        }
                        states[1] = true;
                        continue;
                    };
                    shown_from.get_or_insert(logical_width);
                    states[0] = true;
                    // It costs nothing: the whole strip fits unscrolled with it, and every chip
                    // and reading that has data shows, at any tab scroll.
                    assert!(iconic.tab_scroll_limit() <= 0.5, "{scale} {logical_width}");
                    assert_eq!(iconic.version.is_some(), version.is_some());
                    assert_eq!(iconic.ram.is_some(), ram.is_some());
                    assert_eq!(iconic.version, plain.version);
                    assert_eq!(iconic.ram, plain.ram);
                    for tab_scroll in [5.0, 1.0e6] {
                        let scrolled = project(true, tab_scroll);
                        assert_eq!(scrolled.eon_icon, iconic.eon_icon);
                        assert_eq!(scrolled.tabs, iconic.tabs);
                    }
                    // A square two logical pixels shorter than the tab pills, in whole pixels,
                    // whose centre is one bar edge from the window's left and top, with the same
                    // gap before and after it.
                    let height = iconic.header.height;
                    let gap = tab_gap(metrics, height);
                    let edge = bar_edge(metrics, height, iconic.pane_viewport.top);
                    let pill = iconic.bar_pill_rect(iconic.tabs[0].rect, metrics);
                    let side = (pill.height - 2.0 * metrics.scale).round();
                    assert_eq!((icon.width, icon.height), (side, side), "{scale}");
                    assert!((icon.top + side / 2.0 - (pill.top + pill.height / 2.0)).abs() <= 0.5);
                    assert_eq!(icon.left, (edge - side / 2.0).round());
                    assert_eq!(icon.top, icon.left);
                    assert_eq!(icon.left.fract(), 0.0);
                    assert!(icon.bottom() <= height);
                    let next = iconic
                        .workspace_label
                        .as_ref()
                        .map_or(iconic.tabs[0].rect.left, |label| label.rect.left);
                    assert_eq!(next - icon.right(), icon.left, "{scale} {logical_width}");
                    if let Some(label) = &iconic.workspace_label {
                        assert_eq!(iconic.tabs[0].rect.left, label.rect.right() + gap);
                    }
                    assert_eq!(iconic.tab_viewport.left, iconic.tabs[0].rect.left);
                    // Every tab moves by the same amount without narrowing.
                    let shift = icon.right() + icon.left - tab_inset(metrics, height);
                    for (iconic, plain) in iconic.tabs.iter().zip(&plain.tabs) {
                        assert_eq!(iconic.rect.left, plain.rect.left + shift);
                        assert_eq!(iconic.rect.width, plain.rect.width);
                    }
                    assert_eq!(iconic.version, plain.version);
                    // Close keeps the right end; nothing but the icon is drawn before the tabs.
                    assert_eq!(iconic.controls[2].rect.right(), size.width as f32);
                    assert!(iconic.tab_viewport.left >= icon.right() + icon.left);
                    // It is no hit target, and the tabs keep theirs.
                    let centre = (icon.left + icon.width / 2.0, icon.top + icon.height / 2.0);
                    assert_eq!(iconic.hit_test(centre.0, centre.1), None);
                    for tab in iconic.tabs.iter().filter(|tab| {
                        tab.rect.right() <= iconic.tab_viewport.right() && tab.rect.width > 1.0
                    }) {
                        assert_eq!(
                            iconic.hit_test(tab.rect.left + tab.rect.width / 2.0, height / 2.0),
                            Some(WorkspaceHit::Tab(&tab.id))
                        );
                    }
                }
            }
        }
    }
    assert_eq!(states, [true; 2], "both a shown and a yielded icon");
}

#[test]
fn live_items_never_show_or_hide_the_icon_or_move_the_tabs() {
    use eon_workspace_protocol::v32::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let levels = crate::LoadLevels::default();
    let ram = |available: u32| {
        crate::RamUse::parse(&format!(
            "MemTotal: 100000 kB\nMemAvailable: {available} kB\n"
        ))
        .unwrap()
        .rated(levels)
    };
    let (ram_low, ram_high) = (ram(99_000), ram(0));
    let gib = 1 << 18;
    let disk = DiskUse::of(1000 * gib, 0, 0, 4096, "/")
        .unwrap()
        .rated(levels);
    let clock = LocalClock {
        label: "23:58".into(),
        description: "Clock".into(),
    };
    let observed_at = 1_000_000;
    // A window `remaining` percent full, `elapsed` minutes in when it has a reset.
    let window = |minutes: u32, remaining, elapsed: Option<u32>, scope: Option<&str>| QuotaWindow {
        duration_minutes: minutes,
        remaining_percent: remaining,
        resets_at: elapsed.map(|elapsed| observed_at + u64::from(minutes - elapsed) * 60),
        scope: scope.map(Into::into),
    };
    let narrow = |minutes| window(minutes, 5, None, None);
    let wide = |minutes| window(minutes, 100, Some(minutes - 1), None);
    // Each provider keeps its windows; what they show runs from narrowest to widest.
    let codex = [
        (CodexQuotaState::Fresh, vec![narrow(300), narrow(10_080)]),
        (CodexQuotaState::Fresh, vec![wide(300), wide(10_080)]),
        (CodexQuotaState::Stale, vec![wide(300), wide(10_080)]),
        // The five-hour window reset before the next read.
        (CodexQuotaState::Fresh, vec![wide(10_080)]),
    ]
    .map(|(state, windows)| CodexQuota {
        state,
        observed_at,
        windows,
    });
    let claude = [
        (
            ClaudeQuotaState::Fresh,
            vec![
                narrow(300),
                narrow(10_080),
                window(10_080, 5, None, Some("Fable")),
            ],
        ),
        (
            ClaudeQuotaState::Stale,
            vec![
                wide(300),
                wide(10_080),
                window(10_080, 100, Some(10_079), Some("Fable")),
            ],
        ),
        // Unscoped windows that reset before the next read.
        (
            ClaudeQuotaState::Fresh,
            vec![
                wide(10_080),
                window(10_080, 100, Some(10_079), Some("Fable")),
            ],
        ),
        (
            ClaudeQuotaState::Fresh,
            vec![wide(300), window(10_080, 100, Some(10_079), Some("Fable"))],
        ),
        (
            ClaudeQuotaState::Fresh,
            vec![window(10_080, 100, Some(10_079), Some("Fable"))],
        ),
        // The scoped window reset before the next read, alone or with an unscoped one.
        (ClaudeQuotaState::Fresh, vec![wide(300), wide(10_080)]),
        (ClaudeQuotaState::Stale, vec![wide(300)]),
    ]
    .map(|(state, windows)| ClaudeQuota {
        state,
        observed_at,
        windows,
    });
    let (codex_read, claude_read) = (codex[1].clone(), claude[1].clone());
    let cpu = |percent: u64| crate::CpuUse {
        percent,
        label: format!("CPU {percent}%"),
        description: "CPU".into(),
        load: Load::Normal,
    };
    let (cpu_low, cpu_high) = (cpu(1), cpu(100));
    let readings = [
        (None, None, None, None),
        (Some(&cpu_low), Some(&ram_low), Some(&disk), Some(&clock)),
        (Some(&cpu_high), Some(&ram_high), Some(&disk), Some(&clock)),
    ];
    let mut states = [false; 2];
    for scale in [1.0, 1.25, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        // Every chip reserved at its widest needs a wide window for the icon.
        for logical_width in (100..=3000).step_by(25) {
            let size = PhysicalSize::new((logical_width as f64 * scale) as u32, 600);
            let mut seen = None;
            for codex in &codex {
                for claude in &claude {
                    // What Venus remembers from the read that reported every window.
                    let mut read = crate::QuotaWindows::default();
                    read.observe(Some(&codex_read), Some(&claude_read));
                    read.observe(Some(codex), Some(claude));
                    for (cpu, ram, disk, clock) in readings {
                        let mut snapshot = snapshot.clone();
                        snapshot.codex_quota = Some(codex.clone());
                        snapshot.claude_quota = Some(claude.clone());
                        let scene = WorkspaceScene::from_snapshot_with_bar(
                            &snapshot,
                            EonBar {
                                version: Some("9.8.7"),
                                icon: true,
                                cpu,
                                ram,
                                disk,
                                clock,
                                quota_windows: &read,
                                ..EonBar::default()
                            },
                            size,
                            metrics,
                            0.0,
                            0.0,
                            |_, text| {
                                (
                                    text.to_owned(),
                                    text.chars().count() as f32 * 10.0 * scale as f32,
                                )
                            },
                            None,
                        );
                        let shown = (
                            scene.eon_icon,
                            scene
                                .tabs
                                .iter()
                                .map(|tab| tab.rect.left)
                                .collect::<Vec<_>>(),
                        );
                        states[usize::from(shown.0.is_some())] = true;
                        match &seen {
                            None => seen = Some(shown),
                            Some(seen) => assert_eq!(
                                &shown, seen,
                                "{scale} {logical_width}: {codex:?} {claude:?} {ram:?}"
                            ),
                        }
                    }
                }
            }
        }
    }
    assert_eq!(states, [true; 2], "both a shown and a yielded icon");
}

#[test]
fn quota_facts_and_windows_appearing_may_hide_the_icon_once() {
    use eon_workspace_protocol::v32::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let observed_at = 1_000_000;
    let window = |minutes: u32, scope: Option<&str>| QuotaWindow {
        duration_minutes: minutes,
        remaining_percent: 50,
        resets_at: None,
        scope: scope.map(Into::into),
    };
    let codex = CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at,
        windows: vec![window(300, None), window(10_080, None)],
    };
    let claude = |scoped: bool| ClaudeQuota {
        state: ClaudeQuotaState::Fresh,
        observed_at,
        windows: [window(300, None), window(10_080, None)]
            .into_iter()
            .chain(scoped.then(|| window(10_080, Some("Fable"))))
            .collect(),
    };
    let weekly = |quota: &CodexQuota| CodexQuota {
        windows: vec![window(10_080, None)],
        ..quota.clone()
    };
    let blocked = CodexQuota {
        state: CodexQuotaState::Blocked,
        observed_at,
        windows: Vec::new(),
    };
    // From no fact to a Codex fact; from Claude's two windows to a third, scoped one; a read
    // that finds a second Codex window or ends a block.
    let changes = [
        (
            (None, Some(claude(false))),
            (Some(codex.clone()), Some(claude(false))),
        ),
        ((None, None), (None, Some(claude(false)))),
        (
            (Some(codex.clone()), Some(claude(false))),
            (Some(codex.clone()), Some(claude(true))),
        ),
        (
            (Some(weekly(&codex)), Some(claude(true))),
            (Some(codex.clone()), Some(claude(true))),
        ),
        (
            (Some(blocked), Some(claude(true))),
            (Some(codex), Some(claude(true))),
        ),
    ];
    for scale in [1.0, 1.25, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for (index, (before, after)) in changes.iter().enumerate() {
            let mut toggled = false;
            for logical_width in (100..=3000).step_by(25) {
                let size = PhysicalSize::new((logical_width as f64 * scale) as u32, 600);
                let project = |(codex, claude): &(Option<CodexQuota>, Option<ClaudeQuota>)| {
                    let mut snapshot = snapshot.clone();
                    snapshot.codex_quota = codex.clone();
                    snapshot.claude_quota = claude.clone();
                    WorkspaceScene::from_snapshot_with_bar(
                        &snapshot,
                        EonBar {
                            version: Some("9.8.7"),
                            icon: true,
                            ..EonBar::default()
                        },
                        size,
                        metrics,
                        0.0,
                        0.0,
                        |_, text| {
                            (
                                text.to_owned(),
                                text.chars().count() as f32 * 10.0 * scale as f32,
                            )
                        },
                        None,
                    )
                    .eon_icon
                    .is_some()
                };
                let (shown_before, shown_after) = (project(before), project(after));
                // More reserved chips never bring the icon back; near the threshold they hide it.
                assert!(
                    shown_before || !shown_after,
                    "{scale} {logical_width} change {index}"
                );
                toggled |= shown_before != shown_after;
            }
            assert!(
                toggled,
                "{scale}: change {index} hides the icon at some width"
            );
        }
    }
}

#[test]
fn a_full_bar_with_room_shows_the_icon() {
    use eon_workspace_protocol::v32::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab, WorkspaceInfo,
    };
    // Lucca's maximized 1861-pixel bar of 2026-11-04, which left about 340 pixels empty but
    // showed no icon: workspace 1 of 2, Codex's weekly window, Claude's weekly and Fable
    // windows, every reading and the version.
    let tab = |index: usize, directory: &str| Tab {
        id: format!("t{index}"),
        directory: directory.as_bytes().to_vec(),
        pending: false,
        selected_pane: Some(format!("p{index}")),
        selected_popup: None,
        columns: vec![eon_workspace_protocol::v32::Column {
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
    };
    let workspace = |id: &str, name: &str, recency| WorkspaceInfo {
        id: id.into(),
        name: Some(name.into()),
        tab_count: 1,
        attention: None,
        recency,
    };
    let observed_at = 1_000_000;
    let weekly = |scope: Option<&str>| QuotaWindow {
        duration_minutes: 10_080,
        remaining_percent: 60,
        resets_at: Some(observed_at + 4 * 24 * 60 * 60),
        scope: scope.map(Into::into),
    };
    let codex = CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at,
        windows: vec![weekly(None)],
    };
    let claude = ClaudeQuota {
        state: ClaudeQuotaState::Fresh,
        observed_at,
        windows: vec![weekly(None), weekly(Some("Fable"))],
    };
    let mut read = crate::QuotaWindows::default();
    read.observe(Some(&codex), Some(&claude));
    let levels = crate::LoadLevels::default();
    let ram = crate::RamUse::parse("MemTotal: 100000 kB\nMemAvailable: 50000 kB\n")
        .unwrap()
        .rated(levels);
    let disk = DiskUse::of(1000 << 18, 0, 0, 4096, "/")
        .unwrap()
        .rated(levels);
    let cpu = crate::CpuUse {
        percent: 12,
        label: "CPU 12%".into(),
        description: "CPU".into(),
        load: Load::Normal,
    };
    let clock = LocalClock {
        label: "09:20".into(),
        description: "Clock".into(),
    };
    for tabs in [
        vec![tab(1, "/home/lucca/pjs/eon")],
        vec![
            tab(1, "/home/lucca/pjs/eon"),
            tab(2, "/home/lucca/pjs/nova"),
            tab(3, "/home/lucca/pjs/starcompass"),
        ],
    ] {
        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![workspace("w1", "main", 0), workspace("w2", "evo", 1)],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs,
            codex_quota: Some(codex.clone()),
            claude_quota: Some(claude.clone()),
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v32::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let scene = WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar {
                version: Some("0.1.15"),
                icon: true,
                cpu: Some(&cpu),
                ram: Some(&ram),
                disk: Some(&disk),
                clock: Some(&clock),
                quota_windows: &read,
                ..EonBar::default()
            },
            PhysicalSize::new(1861, 1000),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            // About the bar's 16-pixel sans-serif: the live bar then leaves about the room his did.
            |_, text| (text.to_owned(), text.chars().count() as f32 * 9.0),
            None,
        );
        assert_eq!(scene.quotas.len(), 3);
        assert!(scene.eon_icon.is_some(), "{} tabs", snapshot.tabs.len());
    }
}

#[test]
fn shortcuts_version_and_close_end_the_bar_one_edge_from_the_corner() {
    use eon_workspace_protocol::v32::{Pane, PopupGeometry, Tab};
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let used = crate::RamUse::parse("MemTotal: 100000 kB\nMemAvailable: 50000 kB\n")
        .unwrap()
        .rated(crate::LoadLevels::default());
    for scale in [1.0, 1.25, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let width = 1280.0 * scale as f32;
        for (version, ram) in [Some("9.8.7"), None]
            .into_iter()
            .flat_map(|version| [(version, None), (version, Some(&used))])
        {
            let scene = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar {
                    version,
                    ram,
                    icon: true,
                    ..EonBar::default()
                },
                PhysicalSize::new(width as u32, (800.0 * scale) as u32),
                metrics,
                0.0,
                0.0,
                |_, text| {
                    (
                        text.to_owned(),
                        text.chars().count() as f32 * 10.0 * scale as f32,
                    )
                },
                None,
            );
            let [_, shortcuts, close] = scene.controls;
            let height = scene.header.height;
            let edge = bar_edge(metrics, height, scene.pane_viewport.top);
            // Close's glyph centre is as far from the window's right edge as from its top, its
            // target reaches the edge, and its button is as wide as the others.
            assert_eq!(close.rect.right(), width);
            assert!((width - close.centre - edge).abs() < 0.01, "{scale}");
            assert!(close.centre - close.rect.left <= close.rect.right() - close.centre);
            assert_eq!(
                close.centre - close.rect.left,
                shortcuts.rect.right() - shortcuts.centre
            );
            // Shortcuts, then the version label, then Close; Shortcuts keeps its whole target.
            assert_eq!(
                shortcuts.centre,
                shortcuts.rect.right() - scene.controls[0].rect.width / 2.0
            );
            // After a chip the round button keeps the chips' own spacing: two outline insets
            // between the chip's outline and the circle, with the target reaching back over it.
            if let Some(ram) = &scene.ram {
                let chip_inset = (metrics.padding / 3.0).min(height / 4.0);
                assert_eq!(ram.rect.right(), shortcuts.rect.left);
                let circle = scene.round_button_rect(&shortcuts, metrics);
                let gap = circle.left - (ram.rect.right() - chip_inset);
                assert!((gap - 2.0 * chip_inset).abs() <= 0.5, "{scale}: {gap}");
                assert_eq!(
                    scene.hit_test(ram.rect.left + ram.rect.width / 2.0, height / 2.0),
                    Some(WorkspaceHit::Ram)
                );
            } else {
                assert_eq!(shortcuts.rect.width, scene.controls[0].rect.width);
            }
            assert!(shortcuts.rect.width >= metrics.height.round());
            match &scene.version {
                Some(label) => {
                    assert_eq!(label.label, "EON 9.8.7");
                    assert_eq!(label.rect.left, shortcuts.rect.right());
                    assert_eq!(label.rect.right(), close.rect.left);
                }
                None => assert_eq!(shortcuts.rect.right(), close.rect.left),
            }
            let middle = height / 2.0;
            for (x, hit) in [
                (shortcuts.rect.left + 0.5, WorkspaceHeaderControl::Shortcuts),
                (shortcuts.centre, WorkspaceHeaderControl::Shortcuts),
                (
                    shortcuts.rect.right() - 0.5,
                    WorkspaceHeaderControl::Shortcuts,
                ),
                (close.rect.left + 0.5, WorkspaceHeaderControl::CloseTab),
                (close.centre, WorkspaceHeaderControl::CloseTab),
                (width - 0.5, WorkspaceHeaderControl::CloseTab),
            ] {
                assert_eq!(scene.hit_test(x, middle), Some(WorkspaceHit::Control(hit)));
            }
            if let Some(label) = &scene.version {
                let centre = label.rect.left + label.rect.width / 2.0;
                assert_eq!(scene.hit_test(centre, middle), Some(WorkspaceHit::Version));
            }
            // The Eon icon and the round Shortcuts button are as tall as the tab pills, centred
            // on them, and the button's circle stays inside its target.
            let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
            let pill_centre = pill.top + pill.height / 2.0;
            let circle = scene.round_button_rect(&shortcuts, metrics);
            assert_eq!(
                (circle.width, circle.height),
                (pill.height.round(), pill.height.round())
            );
            assert!((circle.left + circle.width / 2.0 - shortcuts.centre).abs() <= 0.5);
            assert!((circle.top + circle.height / 2.0 - pill_centre).abs() <= 0.5);
            assert!(circle.left >= shortcuts.rect.left && circle.right() <= shortcuts.rect.right());
            assert_eq!(
                scene.hit_test(circle.left + 0.5, pill_centre),
                Some(WorkspaceHit::Control(WorkspaceHeaderControl::Shortcuts))
            );
            // The Eon icon mirrors Close's glyph about the bar's middle.
            let icon = scene.eon_icon.unwrap();
            assert_eq!(
                icon.height,
                (pill.height - 2.0 * metrics.scale).round(),
                "{scale}"
            );
            let icon_centre = icon.left + icon.width / 2.0;
            assert!((icon_centre - edge).abs() <= 0.5, "{scale}");
            assert!(
                (icon.top + icon.height / 2.0 - edge).abs() <= 0.5,
                "{scale}"
            );
        }
    }
}

#[test]
fn quota_windows_keep_their_room_until_the_next_read() {
    use eon_workspace_protocol::v32::{ClaudeQuota, ClaudeQuotaState, CodexQuota};
    let window = |minutes: u32, scope: Option<&str>| QuotaWindow {
        duration_minutes: minutes,
        remaining_percent: 50,
        resets_at: None,
        scope: scope.map(Into::into),
    };
    let claude = |observed_at, windows| ClaudeQuota {
        state: ClaudeQuotaState::Fresh,
        observed_at,
        windows,
    };
    let codex = |observed_at, windows| CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at,
        windows,
    };
    let both = vec![(None, 300), (Some("Fable".to_owned()), 10_080)];
    let mut read = crate::QuotaWindows::default();
    read.observe(
        Some(&codex(1, vec![window(300, None), window(300, None)])),
        Some(&claude(
            1,
            vec![window(300, None), window(10_080, Some("Fable"))],
        )),
    );
    assert_eq!(read.claude.windows, both);
    assert_eq!(read.codex.windows, [(None, 300), (None, 300)]);
    // The runtime drops a window at its reset; the same read keeps its room.
    read.observe(
        Some(&codex(1, vec![window(300, None)])),
        Some(&claude(1, vec![window(10_080, Some("Fable"))])),
    );
    assert_eq!(read.claude.windows, both);
    assert_eq!(read.codex.windows, [(None, 300), (None, 300)]);
    // A new read decides the set again.
    read.observe(None, Some(&claude(2, vec![window(300, None)])));
    assert_eq!(read.claude.windows, [(None, 300)]);
    read.observe(None, Some(&claude(3, vec![window(10_080, Some("Fable"))])));
    assert_eq!(read.claude.windows, [(Some("Fable".to_owned()), 10_080)]);
    // The quota going away forgets them.
    read.observe(None, None);
    assert_eq!(read, crate::QuotaWindows::default());
}

#[test]
fn reserved_chips_take_the_widest_text() {
    use eon_workspace_protocol::v32::{CodexQuota, Pane, PopupGeometry, Tab};
    let snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let observed_at = 1_000_000;
    // Full windows 4h10m and 6d10h in: more zeros than the latest minute's 4h59m and 6d23h.
    let window = |minutes: u32, left: u32| QuotaWindow {
        duration_minutes: minutes,
        remaining_percent: 100,
        resets_at: Some(observed_at + u64::from(left) * 60),
        scope: None,
    };
    // A font whose zero is wider than its other digits.
    let zero = |text: &str| {
        text.chars()
            .map(|c| if c == '0' { 13.0 } else { 10.0 })
            .sum::<f32>()
    };
    // A font whose `unknown` is wider than its `blocked`.
    let letters = |text: &str| {
        text.chars()
            .map(|c| if "nuw".contains(c) { 12.0 } else { 10.0 })
            .sum::<f32>()
    };
    let codex = |state, windows| {
        let mut snapshot = snapshot.clone();
        snapshot.codex_quota = Some(CodexQuota {
            state,
            observed_at,
            windows,
        });
        snapshot
    };
    let metrics = CellMetrics::for_scale(1.0);
    let scene = |snapshot: &Snapshot, logical_width, measure: &dyn Fn(&str) -> f32| {
        WorkspaceScene::from_snapshot_with_bar(
            snapshot,
            EonBar {
                items: &[BarItem::CodexQuota5h, BarItem::CodexQuota7d],
                icon: true,
                ..EonBar::default()
            },
            PhysicalSize::new(logical_width, 600),
            metrics,
            0.0,
            0.0,
            |_, text| (text.to_owned(), measure(text)),
            None,
        )
    };
    let even = |text: &str| text.chars().count() as f32 * 10.0;
    // Both supported window lengths retain their widest text.
    for (snapshot, measure) in [
        (
            codex(
                CodexQuotaState::Stale,
                vec![window(300, 50), window(10_080, 14 * 60)],
            ),
            &zero as &dyn Fn(&str) -> f32,
        ),
        (codex(CodexQuotaState::Stale, vec![window(300, 61)]), &even),
    ] {
        let mut seen = [false; 2];
        for logical_width in 300..=1600 {
            let scene = scene(&snapshot, logical_width, measure);
            seen[usize::from(scene.eon_icon.is_some())] = true;
            assert!(
                scene.eon_icon.is_none() || (scene.bar_complete && scene.tab_scroll_limit() <= 0.5),
                "{logical_width}: the icon shows beside a chip that does not fit"
            );
        }
        assert_eq!(seen, [true; 2]);
    }
    // Blocked and unknown keep one room, whichever word shows.
    let (blocked, unknown) = (
        codex(CodexQuotaState::Blocked, Vec::new()),
        codex(CodexQuotaState::Unknown, Vec::new()),
    );
    let mut toggled = [false; 2];
    for logical_width in 300..=1000 {
        let shown = [&blocked, &unknown].map(|snapshot| {
            let scene = scene(snapshot, logical_width, &letters);
            assert!(
                scene.eon_icon.is_none() || (scene.bar_complete && scene.tab_scroll_limit() <= 0.5),
                "{logical_width}: the icon shows beside a chip that does not fit"
            );
            scene.eon_icon.is_some()
        });
        assert_eq!(shown[0], shown[1], "{logical_width}");
        toggled[usize::from(shown[0])] = true;
    }
    assert_eq!(toggled, [true; 2]);
}

#[test]
fn quota_window_widgets_filter_order_and_reserve_only_selected_windows() {
    use eon_workspace_protocol::v32::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let mut snapshot = Snapshot {
        workspace: "w1".into(),
        active_workspace: "w1".into(),
        workspaces: Vec::new(),
        active_tab: "t1".into(),
        geometry: PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v32::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/tmp/p1.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: Vec::new(),
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: eon_workspace_protocol::v32::Sidebar::Open,
        attention_queue: Vec::new(),
    };
    let window = |duration_minutes, scope: Option<&str>| QuotaWindow {
        duration_minutes,
        remaining_percent: 50,
        resets_at: None,
        scope: scope.map(Into::into),
    };
    snapshot.claude_quota = Some(ClaudeQuota {
        state: ClaudeQuotaState::Fresh,
        observed_at: 100,
        windows: vec![
            window(300, None),
            window(10080, None),
            window(10080, Some("Fable")),
        ],
    });
    snapshot.codex_quota = Some(CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at: 100,
        windows: vec![window(300, None), window(10080, None)],
    });
    let mut remembered = crate::QuotaWindows::default();
    remembered.observe(
        snapshot.codex_quota.as_ref(),
        snapshot.claude_quota.as_ref(),
    );
    let cpu = crate::CpuUse {
        percent: 12,
        label: "CPU 12%".into(),
        description: "CPU".into(),
        load: Load::Normal,
    };
    let render =
        |snapshot: &Snapshot, items: &[BarItem], width, remembered: &crate::QuotaWindows| {
            WorkspaceScene::from_snapshot_with_bar(
                snapshot,
                EonBar {
                    items,
                    icon: true,
                    cpu: Some(&cpu),
                    quota_windows: remembered,
                    ..EonBar::default()
                },
                PhysicalSize::new(width, 600),
                CellMetrics::for_scale(1.0),
                0.0,
                0.0,
                |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
                None,
            )
        };
    let project =
        |snapshot: &Snapshot, items: &[BarItem], width| render(snapshot, items, width, &remembered);
    let selected = [BarItem::ClaudeQuota7d, BarItem::Cpu, BarItem::ClaudeQuota5h];
    let scene = project(&snapshot, &selected, 3000);
    assert_eq!(
        scene
            .quotas
            .iter()
            .map(|q| q.label.as_str())
            .collect::<Vec<_>>(),
        ["7d 50%", "5h 50%"]
    );
    assert!(scene.quotas[0].rect.right() <= scene.cpu.as_ref().unwrap().rect.left);
    assert!(scene.cpu.as_ref().unwrap().rect.right() <= scene.quotas[1].rect.left);
    assert!(
        scene
            .quotas
            .iter()
            .all(|q| !q.accessible_label.contains("Fable") && !q.description.contains("Fable"))
    );
    assert_eq!(snapshot.claude_quota.as_ref().unwrap().windows.len(), 3);
    let all = project(
        &snapshot,
        &[
            BarItem::ClaudeQuota7d,
            BarItem::ClaudeQuota5h,
            BarItem::ClaudeQuotaScoped,
        ],
        3000,
    );
    assert_eq!(
        all.quotas
            .iter()
            .map(|q| q.label.as_str())
            .collect::<Vec<_>>(),
        ["7d 50%", "5h 50%", "Fable 7d 50%"]
    );
    let scoped = project(&snapshot, &[BarItem::ClaudeQuotaScoped], 3000);
    assert_eq!(scoped.quotas.len(), 1);
    assert_eq!(
        scoped.quotas[0].window,
        Some(QuotaWindowIdentity::ClaudeScoped(0))
    );
    let codex = project(
        &snapshot,
        &[BarItem::CodexQuota7d, BarItem::CodexQuota5h],
        3000,
    );
    assert_eq!(
        codex
            .quotas
            .iter()
            .map(|q| q.label.as_str())
            .collect::<Vec<_>>(),
        ["7d 50%", "5h 50%"]
    );
    // An unselected scoped window must consume no reserved room, even after its reset.
    let mut without_scoped = snapshot.clone();
    without_scoped
        .claude_quota
        .as_mut()
        .unwrap()
        .windows
        .retain(|w| w.scope.is_none());
    for width in (500..1800).step_by(10) {
        assert_eq!(
            project(&snapshot, &selected, width).eon_icon.is_some(),
            render(
                &without_scoped,
                &selected,
                width,
                &crate::QuotaWindows::default()
            )
            .eon_icon
            .is_some(),
            "{width}"
        );
    }
}
