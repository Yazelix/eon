use super::*;

#[test]
fn eon_icon_opens_window_controls_without_hints_or_sidebar() {
    use eon_workspace_protocol::v42::{RunningWindows, WindowEntry, WindowState};
    let mut snapshot = one_pane_snapshot();
    snapshot.sidebar = eon_workspace_protocol::v42::Sidebar::Off;
    snapshot.running = Some(RunningWindows {
        this: WindowEntry {
            window: None,
            generation: "g1-0123456789abcdef0123456789abcdef".into(),
            name: Some("project".into()),
            started_at: Some(1),
            live_terminals: Some(2),
            state: WindowState::Live,
        },
        others: Some(vec![]),
        omitted: 0,
    });
    snapshot.tabs = (1..=32)
        .map(|n| {
            let mut tab = snapshot.tabs[0].clone();
            tab.id = format!("t{n}");
            tab
        })
        .collect();
    let scene = WorkspaceScene::from_snapshot_with_bar(
        &snapshot,
        EonBar {
            icon: true,
            items: &[],
            ..EonBar::default()
        },
        PhysicalSize::new(1200, 720),
        CellMetrics::for_scale(1.0),
        0.0,
        0.0,
        |_, text| (text.to_owned(), text.len() as f32 * 10.0),
        None,
    );
    assert!(scene.hints.is_none() && scene.sidebar().is_none());
    assert!(scene.tab_scroll_limit() > 0.5);
    let icon = scene.eon_icon.unwrap();
    assert!(
        matches!(
            scene.hit_test(icon.left + icon.width / 2.0, icon.top + icon.height / 2.0),
            Some(WorkspaceHit::Control(WorkspaceHeaderControl::WindowMenu))
        ),
        "the icon must be an actionable window control"
    );
}

#[test]
fn window_menu_keeps_long_names_and_descriptions_inside_a_narrow_dialog() {
    use eon_workspace_protocol::v42::{RunningWindows, WindowEntry, WindowState};
    let window = |id: &str| WindowEntry {
        window: Some(id.into()),
        generation: format!("g1-{id}"),
        // The longest name a window takes, in wide glyphs.
        name: Some("W".repeat(64)),
        started_at: Some(1),
        live_terminals: Some(2),
        state: WindowState::Live,
    };
    let running = RunningWindows {
        this: window("01234567"),
        others: Some(vec![window("89abcdef")]),
        omitted: 0,
    };
    let metrics = CellMetrics::for_scale(1.0);
    let measure = crate::CellText(metrics);
    let menu = DialogScene::window_menu(
        &running,
        &[],
        None,
        PhysicalSize::new(320, 500),
        metrics,
        0.0,
        &measure,
    );
    assert!(menu.bounds.right() <= 320.0);
    assert!(menu.content.right() <= menu.bounds.right());
    for group in &menu.groups {
        assert!(group.heading.right() <= menu.content.right() + 0.01);
    }
    for row in menu.menu_rows() {
        assert!(
            row.rect.left >= menu.content.left && row.rect.right() <= menu.content.right() + 0.01
        );
        // Each description has the height of its lines wrapped in the width it draws in.
        let width = row.rect.right() - (row.rect.left + row.label_width + menu.label_gap);
        let lines = measure.lines(&row.action, width);
        assert!(
            row.rect.height >= metrics.height * (1.0 + 1.25 * (lines - 1) as f32),
            "{} needs {lines} lines in {width}",
            row.action
        );
    }
}

#[test]
fn window_menu_projects_summary_and_clips_actions_to_its_scroll_area() {
    use eon_workspace_protocol::v42::{PopupKey, RunningWindows, WindowEntry, WindowState};
    let window = |id: &str, count| WindowEntry {
        window: Some(id.into()),
        generation: format!("g1-{id}"),
        name: Some(format!("project-{id}")),
        started_at: Some(1),
        live_terminals: count,
        state: WindowState::Live,
    };
    let mut running = RunningWindows {
        this: window("01234567", Some(2)),
        others: Some(vec![]),
        omitted: 0,
    };
    let project = |running: &RunningWindows, scroll| {
        DialogScene::window_menu(
            running,
            &[],
            Some(&WindowMenuAction::Detach),
            PhysicalSize::new(800, 500),
            CellMetrics::for_scale(1.0),
            scroll,
            &crate::CellText(CellMetrics::for_scale(1.0)),
        )
    };
    let empty = project(&running, 0.0);
    assert!(
        empty.groups[0]
            .title
            .contains("project-01234567 (01234567) · 2 terminals")
    );
    assert_eq!(empty.menu_rows().count(), 2);
    assert_eq!(empty.groups[1].rows[0].action, "No other windows");
    let updates = DialogScene::window_menu(
        &running,
        &[PopupEntry {
            id: "commands".into(),
            label: "Actions".into(),
            shortcut: PopupKey::Unbound,
        }],
        Some(&WindowMenuAction::Updates),
        PhysicalSize::new(800, 500),
        CellMetrics::for_scale(1.0),
        0.0,
        &crate::CellText(CellMetrics::for_scale(1.0)),
    );
    let row = updates.menu_rows().last().unwrap();
    assert_eq!(row.menu_action, Some(WindowMenuAction::Updates));
    assert_eq!(row.shortcut, "Updates");
    assert_eq!(row.action, "Search update in Actions");
    assert!(row.focused);
    assert_eq!(
        updates.menu_target_at(row.rect.left + 1.0, row.rect.top + row.rect.height / 2.0),
        Some(&WindowMenuAction::Updates)
    );
    running.others = Some(
        (0..32)
            .map(|i| window(&format!("{i:08x}"), Some(i)))
            .collect(),
    );
    running.omitted = 2;
    let full = project(&running, 0.0);
    assert_eq!(full.menu_rows().count(), 34);
    assert!(full.max_scroll > 0.0);
    for row in full.menu_rows() {
        let x = row.rect.left + 1.0;
        let y = row.rect.top + row.rect.height / 2.0;
        assert_eq!(
            full.menu_target_at(x, y),
            full.content
                .contains(x, y)
                .then_some(row.menu_action.as_ref())
                .flatten()
        );
    }
    let end = project(&running, f32::MAX);
    assert_eq!(end.scroll, end.max_scroll);
    assert!(
        end.groups[1]
            .rows
            .last()
            .unwrap()
            .action
            .contains("2 more windows")
    );
    // Peers disappearing clamps a previously larger scroll before keyboard reveal.
    running.others.as_mut().unwrap().truncate(8);
    let shrunk = project(&running, end.scroll);
    assert!(shrunk.scroll < end.scroll);
    let revealed = project(
        &running,
        shrunk.menu_action_scroll(&WindowMenuAction::Detach),
    );
    let first = revealed.menu_rows().next().unwrap();
    assert_eq!(first.rect.intersection(revealed.content), Some(first.rect));
    let mut unknown = window("deadbeef", None);
    unknown.state = WindowState::Unreachable;
    assert!(window_summary(&unknown).ends_with("terminal count unknown · not answering"));
    unknown.window = None;
    unknown.generation = "g1-0123456789abcdef0123456789abcdef".into();
    assert_eq!(
        WindowMenuAction::peer(&unknown),
        WindowMenuAction::QuitOther {
            id: "0123456789abcdef0123456789abcdef".into(),
            generation: unknown.generation,
        }
    );
}

#[test]
fn peer_quit_keeps_id_and_count_apart_from_a_long_name() {
    use eon_workspace_protocol::v42::{WindowEntry, WindowState};
    let peer = WindowEntry {
        window: Some("01234567".into()),
        generation: "g1-11111111111111111111111111111111".into(),
        name: Some("n".repeat(64)),
        started_at: None,
        live_terminals: Some(65535),
        state: WindowState::Live,
    };
    let metrics = CellMetrics::for_scale(1.0);
    let scene = DialogScene::quit_other_window(
        &peer,
        PhysicalSize::new(320, 500),
        metrics,
        0.0,
        &crate::CellText(metrics),
    );
    assert_eq!(scene.kind.subtitle(), "n".repeat(64));
    assert_eq!(scene.groups[0].rows[0].shortcut, "ID");
    assert_eq!(scene.groups[0].rows[0].action, "01234567");
    assert_eq!(scene.groups[0].rows[1].shortcut, "Terminals");
    assert_eq!(scene.groups[0].rows[1].action, "65535");
    for row in &scene.groups[0].rows {
        assert_eq!(row.rect.intersection(scene.content), Some(row.rect));
    }
    assert!(scene.subtitle.bottom() <= scene.content.top);
}

#[test]
fn eon_bar_pill_band_keeps_title_tabs_one_card_gap_below() {
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
        assert!(
            (band.start - expected.0).abs() < 0.001,
            "{scale}/{line_height}"
        );
        assert!((band.end - band.start - expected.1).abs() < 0.001);
        assert!((band.end - header).abs() < 0.001);
        // A title tab's cap rises above the card area's top title row.
        let tab_top = cards.top - title_tab_rise(metrics);
        assert!(
            (tab_top - band.end - card_gap(metrics)).abs() < 0.001,
            "{scale}/{line_height}: tab top {tab_top}, pills end {}",
            band.end
        );
        assert!(band.end - band.start >= metrics.height);
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
        let text = quota_text(QuotaProvider::Claude, state, 100, Some(&window), None);
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
        (0.45, 50, 0.1),   // orange
        (0.375, 50, 0.25), // full red
        (0.25, 50, 0.5),   // still full red
        (0.1, 50, 0.8),
        (0.09, 50, 0.82), // no elapsed-time gate
        (0.005, 99, 0.5), // 1% used after only 90 seconds
        (0.01, 99, 0.0),  // 1% used, exactly on pace
        (0.5, 99, -1.0),  // little used, comfortably under pace
        (0.0, 99, 1.0),   // use at the window start is still judged
        (0.05, 0, 0.95),  // used up, even in a young window
        (0.95, 0, 0.05),
    ] {
        let actual = shortfall(elapsed, remaining).expect("judged window");
        assert!(
            (actual - expected).abs() < 1e-3,
            "{elapsed} elapsed, {remaining}% left: {actual} != {expected}"
        );
    }
    for elapsed in [0.0, 0.005, 0.1, 0.5, 1.0] {
        assert_eq!(shortfall(elapsed, 100), None, "nothing used has no basis");
    }
    let reset_less = QuotaWindow {
        resets_at: None,
        ..window(0.5, 0)
    };
    assert_eq!(quota_shortfall(&reset_less, 1000), None);
    let future_start = QuotaWindow {
        resets_at: Some(1000 + 18_060),
        ..window(0.0, 99)
    };
    assert_eq!(quota_shortfall(&future_start, 1000), Some(1.0));
    // More used is never better; more elapsed for the same use is never worse.
    for elapsed in (0..=100).map(|step| step as f32 / 100.0) {
        let mut previous = f32::NEG_INFINITY;
        for remaining in (0..=99).rev() {
            let current = shortfall(elapsed, remaining).expect("judged window");
            assert!(current >= previous, "{elapsed} elapsed, {remaining}% left");
            previous = current;
        }
    }
    for remaining in 0..=99 {
        let mut previous = f32::INFINITY;
        for elapsed in (0..=100).map(|step| step as f32 / 100.0) {
            let current = shortfall(elapsed, remaining).expect("judged window");
            assert!(current <= previous, "{elapsed} elapsed, {remaining}% left");
            previous = current;
        }
    }
}

#[test]
fn quota_tooltip_lists_what_the_chip_does_not_show() {
    use CodexQuotaState::{Blocked, Fresh, Stale, Unknown};
    use QuotaProvider::{Claude, Codex};

    // Wednesday 4 November 2026, 12:00 at UTC−03:00.
    let local = |offset| {
        jiff::civil::date(2026, 11, 4)
            .at(12, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::fixed(offset))
            .unwrap()
    };
    let now = local(jiff::tz::offset(-3));
    let observed_at = now.timestamp().as_second() as u64;
    let quota = |duration_minutes, remaining_percent, resets_in: Option<u64>| QuotaWindow {
        duration_minutes,
        remaining_percent,
        resets_at: resets_in.map(|seconds| observed_at + seconds),
        scope: None,
    };
    let fable = QuotaWindow {
        scope: Some("Fable".into()),
        ..quota(10_080, 60, Some(100_800))
    };
    for (provider, state, window, tooltip) in [
        // Today gives the time alone.
        (
            Codex,
            Fresh,
            Some(quota(300, 50, Some(9_000))),
            "Codex 5h window\nResets at 14:30\nLasts until reset at this pace",
        ),
        // Within the next six days, the weekday.
        (
            Codex,
            Fresh,
            Some(quota(10_080, 50, Some(378_000))),
            "Codex 7d window\nResets Sun 21:00\nRuns out about 1d18h before reset",
        ),
        (
            Claude,
            Fresh,
            Some(quota(300, 0, Some(17_000))),
            "Claude 5h window\nResets at 16:43\nUsed up until reset",
        ),
        (
            Claude,
            Fresh,
            Some(fable),
            "Claude Fable-only 7d window\nResets Thu 16:00\nLasts until reset at this pace",
        ),
        // Under 1% used has no pace, even with a reset.
        (
            Codex,
            Fresh,
            Some(quota(300, 100, Some(17_940))),
            "Codex 5h window\nResets at 16:59",
        ),
        (
            Codex,
            Fresh,
            Some(quota(300, 99, Some(17_940))),
            "Codex 5h window\nResets at 16:59\nRuns out about 3h20m before reset",
        ),
        (
            Codex,
            Fresh,
            Some(quota(300, 100, Some(1_800))),
            "Codex 5h window\nResets at 12:30",
        ),
        (
            Codex,
            Fresh,
            Some(quota(300, 99, Some(18_000))),
            "Codex 5h window\nResets at 17:00\nRuns out about 5h before reset",
        ),
        (
            Codex,
            Stale,
            Some(quota(300, 99, Some(17_940))),
            "Codex 5h window\nResets at 16:59\nData is stale",
        ),
        // After that, the date.
        (
            Codex,
            Fresh,
            Some(quota(10_080, 50, Some(601_200))),
            "Codex 7d window\nResets 11 Nov 11:00\nRuns out about 6d22h before reset",
        ),
        (
            Claude,
            Fresh,
            Some(quota(10_080, 85, Some(586_800))),
            "Claude 7d window\nResets 11 Nov 07:00\nRuns out about 5d15h before reset",
        ),
        (
            Codex,
            Fresh,
            Some(quota(300, 50, None)),
            "Codex 5h window\nReset time unavailable",
        ),
        (
            Codex,
            Stale,
            Some(quota(10_080, 50, Some(378_000))),
            "Codex 7d window\nResets Sun 21:00\nData is stale",
        ),
        (Codex, Blocked, None, "Codex quota\nPermission blocked"),
        (Codex, Unknown, None, "Codex quota\nPermission unknown"),
    ] {
        let text = quota_text(provider, state, observed_at, window.as_ref(), Some(&now));
        assert_eq!(text.tooltip, tooltip, "{}", text.label);
        assert!(!text.tooltip.contains('%'), "{tooltip}");
    }

    // A stale reading names the same reset however old it is, and says when that has passed.
    let reset = QuotaWindow {
        resets_at: Some(observed_at + 1_800),
        ..quota(300, 50, None)
    };
    for age in [0, 7_200, 20_000] {
        let text = quota_text(Codex, Stale, observed_at - age, Some(&reset), Some(&now));
        assert_eq!(
            text.tooltip, "Codex 5h window\nResets at 12:30\nData is stale",
            "{age}"
        );
    }
    let later = now.checked_add(jiff::Span::new().hours(1)).unwrap();
    let text = quota_text(Codex, Stale, observed_at, Some(&reset), Some(&later));
    assert_eq!(
        text.tooltip,
        "Codex 5h window\nHas reset since this reading\nData is stale"
    );
    // The reset follows the local time zone, and without one hover leaves it out.
    let text = quota_text(
        Codex,
        Fresh,
        observed_at,
        Some(&reset),
        Some(&local(jiff::tz::Offset::UTC)),
    );
    assert!(
        text.tooltip.contains("\nResets at 15:30\n"),
        "{}",
        text.tooltip
    );
    let text = quota_text(Codex, Fresh, observed_at, Some(&reset), None);
    assert_eq!(
        text.tooltip,
        "Codex 5h window\nLasts until reset at this pace"
    );
    assert!(text.description.contains("observed 4h30m into 5h"));
}

#[test]
fn quota_reset_follows_local_days_offsets_and_expiry() {
    let at = |zone: &jiff::tz::TimeZone, date: jiff::civil::Date, hour, minute| {
        date.at(hour, minute, 0, 0).to_zoned(zone.clone()).unwrap()
    };
    let utc = |date: jiff::civil::Date, hour| {
        at(&jiff::tz::TimeZone::UTC, date, hour, 0)
            .timestamp()
            .as_second() as u64
    };
    let seconds = |zoned: &jiff::Zoned| zoned.timestamp().as_second() as u64;
    let zone = jiff::tz::TimeZone::fixed(jiff::tz::offset(-3));
    let wednesday = jiff::civil::date(2026, 11, 4);

    // Days are local: at 23:30 on Wednesday (02:30 Thursday in UTC), 00:30 is Thursday and
    // 23:50 still today, though both fall on Thursday in UTC.
    let late = at(&zone, wednesday, 23, 30);
    assert_eq!(late.timestamp().to_zoned(jiff::tz::TimeZone::UTC).day(), 5);
    for (reset, line) in [
        (seconds(&late) + 3_600, "Resets Thu 00:30"),
        (seconds(&late) + 1_200, "Resets at 23:50"),
    ] {
        assert_eq!(quota_reset(reset, &late).as_deref(), Some(line));
    }

    // The weekday covers the next six days, through Tuesday 23:59, and the date follows.
    let noon = at(&zone, wednesday, 12, 0);
    for (reset, line) in [
        (seconds(&noon) + 561_540, "Resets Tue 23:59"),
        (seconds(&noon) + 561_600, "Resets 11 Nov 00:00"),
        // A reset at now has passed; a minute later has not.
        (seconds(&noon) + 60, "Resets at 12:01"),
    ] {
        assert_eq!(quota_reset(reset, &noon).as_deref(), Some(line));
    }
    assert_eq!(quota_reset(seconds(&noon), &noon), None);

    // The reset takes its own instant's offset: New York leaves daylight saving at 02:00 on
    // Sunday 1 November 2026, so 23:00 UTC that day reads 18:00, not the 19:00 that
    // Saturday's offset would give. A POSIX rule names the zone without needing a zoneinfo
    // database in the test sandbox.
    let new_york = jiff::tz::TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
    let saturday = at(&new_york, jiff::civil::date(2026, 10, 31), 12, 0);
    assert_eq!(saturday.offset(), jiff::tz::offset(-4));
    assert_eq!(
        quota_reset(utc(jiff::civil::date(2026, 11, 1), 23), &saturday).as_deref(),
        Some("Resets Sun 18:00")
    );

    // A fresh reading whose reset has passed drops the pace from hover, which would look
    // ahead to it, and keeps it in the accessible description.
    let window = QuotaWindow {
        duration_minutes: 300,
        remaining_percent: 50,
        resets_at: Some(seconds(&noon) + 1_800),
        scope: None,
    };
    for minutes in [30, 31] {
        let now = noon
            .checked_add(jiff::Span::new().minutes(minutes))
            .unwrap();
        let text = quota_text(
            QuotaProvider::Codex,
            CodexQuotaState::Fresh,
            seconds(&noon),
            Some(&window),
            Some(&now),
        );
        assert_eq!(
            text.tooltip, "Codex 5h window\nHas reset since this reading",
            "{minutes}"
        );
        assert!(
            text.description
                .ends_with("At this pace it lasts until reset."),
            "{}",
            text.description
        );
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
        QuotaWindow {
            duration_minutes: 300,
            remaining_percent: 99,
            resets_at: Some(observed_at + 17_940),
            scope: None,
        },
        QuotaWindow {
            duration_minutes: 10_080,
            remaining_percent: 85,
            resets_at: Some(observed_at + 586_800),
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
        (
            &windows[2],
            "1m/5h 99%",
            2.0 / 3.0,
            "5h window: 99% remaining; observed 1m into 5h. At this pace it runs out about 3h20m before reset.",
        ),
        (
            &windows[3],
            "5h/7d 85%",
            1.0 - (5.0 / 168.0) / 0.15,
            "7d window: 85% remaining; observed 5h into 7d. At this pace it runs out about 5d15h before reset.",
        ),
    ] {
        for provider in [QuotaProvider::Codex, QuotaProvider::Claude] {
            let text = quota_text(
                provider,
                CodexQuotaState::Fresh,
                observed_at,
                Some(window),
                None,
            );
            assert_eq!(text.label, label);
            let pace = text.pace.expect("each fresh chip judges its one window");
            assert!((pace - shortfall).abs() < 1e-3);
            assert_eq!(
                text.description,
                format!("{} quota state: fresh. {detail}", provider.name())
            );
        }
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
        None,
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
        None,
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
        None,
    );
    assert!(
        text.description
            .ends_with("At this pace it runs out about 2h before reset."),
        "{}",
        text.description
    );

    // Stale, blocked, unknown and unused windows keep neutral labels and descriptions.
    let unused = QuotaWindow {
        remaining_percent: 100,
        resets_at: Some(observed_at + 17_940),
        ..windows[0].clone()
    };
    let unused_old = QuotaWindow {
        resets_at: Some(observed_at + 1_800),
        ..unused.clone()
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
        (CodexQuotaState::Fresh, Some(&unused), "1m/5h 100%"),
        (CodexQuotaState::Fresh, Some(&unused_old), "4h30m/5h 100%"),
        (CodexQuotaState::Stale, Some(&windows[2]), "1m/5h 99% old"),
    ] {
        let text = quota_text(QuotaProvider::Codex, state, observed_at, window, None);
        assert_eq!(text.label, label);
        assert!(text.pace.is_none());
        assert!(!text.description.contains("pace") && !text.description.contains("Used up"));
    }
}

/// Hints shaped like Venus's defaults while the tabs have focus: four modifier groups, then two
/// expanded hints.
fn default_like_hints() -> Vec<ShortcutHint> {
    let hint = |modifiers: &str, names: &str, key: &str, label: &str, priority| ShortcutHint {
        modifiers: modifiers.into(),
        modifier_names: names.into(),
        key: key.into(),
        label: label.into(),
        short: label.into(),
        description: format!("{label} in full"),
        section: crate::HintSection::Actions,
        accessible_label: format!("{names}+{key} — {label} in full"),
        priority,
    };
    vec![
        hint("A", "Alt", "HJKL · 1..9", "focus", 1),
        hint("A", "Alt", "M", "pane", 1),
        hint("CA", "Ctrl+Alt", "M", "col", 3),
        hint("AS", "Alt+Shift", "T", "tab", 1),
        hint("CA", "Ctrl+Alt", "HJKL", "mv pane", 3),
        hint("CAS", "Ctrl+Alt+Shift", "HL", "mv tab", 3),
        hint("AS", "Alt+Shift", "B", "hints", 0),
        hint("", "", "Escape", "terminal", 0),
        hint("", "", "Left/Right", "tabs", 1),
    ]
}

/// Sweeps window widths at one scale, with the clock `clock` pixels wide when given, checking
/// each step of the narrow-window order, and returns which steps showed: centred beside the
/// version, shifted to clear it or the clock, version hidden, clock hidden, expanded hints
/// dropped, and short legends.
fn sweep_footer(
    hints: &[ShortcutHint],
    scale: f64,
    metrics: CellMetrics,
    clock: Option<f32>,
    mut measure: impl FnMut(&str) -> f32,
) -> [bool; 6] {
    // As the application measures it: ASCII length times the terminal cell width.
    let version = "EON 0.1.18".len() as f32 * metrics.width;
    let padding = metrics.padding;
    let mut seen = [false; 6];
    let mut previous: Option<ShortcutHintsScene> = None;
    for logical in (200..=2400).step_by(2) {
        let width = (f64::from(logical) * scale) as u32;
        let scene = ShortcutHintsScene::new(
            hints.to_vec(),
            PhysicalSize::new(width, (800.0 * scale) as u32),
            metrics,
            Some(version),
            clock,
            &mut measure,
        );
        let width = width as f32;
        let (start, end) = (padding, width - padding);
        // The clock's text sits at the start, on the version's row and centred on the keycaps'
        // line; the cluster keeps one padding after it.
        let low = scene.clock.map_or(start, |rect| rect.right() + padding);
        if let Some(rect) = scene.clock {
            assert_eq!(rect.left, start);
            assert!((rect.width - clock.unwrap()).abs() < 0.01);
            assert!(rect.top >= scene.rect.top && rect.bottom() <= scene.rect.bottom());
            let line = scene.groups[0].rect;
            assert!(((rect.top + rect.bottom()) - (line.top + line.bottom())).abs() < 0.01);
        }
        // Keycaps never drop: one per modifier group, in the hints' order, each with its
        // group's rows and accessible chords.
        let names = scene
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Alt", "Ctrl+Alt", "Alt+Shift", "Ctrl+Alt+Shift"]);
        for group in &scene.groups {
            let rows = hints
                .iter()
                .filter(|hint| hint.modifier_names == group.name)
                .map(|hint| {
                    (
                        hint.key.clone(),
                        hint.short.clone(),
                        hint.description.clone(),
                        hint.section,
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                group
                    .rows
                    .iter()
                    .map(|row| (
                        row.key.clone(),
                        row.short.clone(),
                        row.description.clone(),
                        row.section
                    ))
                    .collect::<Vec<_>>(),
                rows
            );
            assert!(
                group
                    .accessible_label
                    .starts_with(&format!("Actions: {}+", group.name))
            );
            let legends = [
                group.name.as_str(),
                &hints
                    .iter()
                    .find(|hint| hint.modifier_names == group.name)
                    .unwrap()
                    .modifiers,
            ];
            assert!(legends.contains(&group.legend.as_str()));
            assert!((group.rect.width - measure(&group.legend) - padding).abs() < 0.01);
            assert!(group.hover.contains(group.rect.left, group.rect.top));
            assert_eq!(group.hover.top, scene.rect.top);
            assert_eq!(group.hover.bottom(), scene.rect.bottom());
        }
        let short = scene.groups[0].legend == "A";
        assert!(
            scene
                .groups
                .iter()
                .all(|group| (group.legend == group.name) != short)
        );
        // The cluster: keycaps, then expanded hints, one padding apart.
        let items = scene
            .groups
            .iter()
            .map(|group| group.rect)
            .chain(scene.pieces.iter().map(|piece| piece.rect))
            .collect::<Vec<_>>();
        for pair in items.windows(2) {
            assert!((pair[1].left - pair[0].right() - padding).abs() < 0.01);
        }
        let left = items[0].left;
        let right = items.last().unwrap().right();
        let centred = ((left + right) / 2.0 - width / 2.0).abs() < 0.5;
        if !short {
            assert!(left >= low - 0.01 && right <= end + 0.01, "{logical}");
        }
        match scene.version {
            Some(rect) => {
                assert!((rect.right() - end).abs() < 0.01);
                assert!((rect.width - version).abs() < 0.01);
                assert!(right + padding <= rect.left + 0.01, "{logical}");
                assert_eq!(scene.pieces.len(), 2);
                // The version hides before the clock.
                assert_eq!(scene.clock.is_some(), clock.is_some(), "{logical}");
                // Off centre only to clear the version or the clock.
                let at_version = (right + padding - rect.left).abs() < 0.01;
                assert!(
                    centred || at_version || (left - low).abs() < 0.01,
                    "{logical}"
                );
                if !centred && at_version {
                    assert!(width / 2.0 + (right - left) / 2.0 + padding > rect.left);
                }
            }
            None => {
                // Hidden only once the cluster would cross the start beside it.
                let full = hints.iter().fold(Vec::new(), |mut names, hint| {
                    if !hint.modifiers.is_empty() && !names.contains(&hint.modifier_names) {
                        names.push(hint.modifier_names.clone());
                    }
                    names
                });
                let mut cluster = padding * 5.0;
                for name in &full {
                    cluster += measure(name) + padding;
                }
                for hint in hints.iter().filter(|hint| hint.modifiers.is_empty()) {
                    cluster += measure(&hint.key) + measure(" ") + measure(&hint.label);
                }
                let room = clock.map_or(0.0, |clock| clock + padding);
                assert!(
                    end - version - padding - cluster < start + room,
                    "{logical}"
                );
                assert!(
                    centred || (left - low).abs() < 0.01 || short && left == start,
                    "{logical}"
                );
                // The clock hides only once the whole cluster no longer fits after it, and
                // before any expanded hint drops.
                match (clock, scene.clock) {
                    (Some(clock), None) => {
                        assert!(end - cluster < start + clock + padding, "{logical}");
                    }
                    (Some(_), Some(_)) => assert_eq!(scene.pieces.len(), 2, "{logical}"),
                    _ => {}
                }
            }
        }
        // Short legends only once no expanded hint is left.
        assert!(!short || scene.pieces.is_empty());
        assert_eq!(
            scene.complete,
            scene.version.is_some()
                && scene.pieces.len() == 2
                && !short
                && scene.clock.is_some() == clock.is_some()
        );
        // Widening never hides anything: each step gives way in its order.
        if let Some(previous) = previous {
            assert!(previous.version.is_none() || scene.version.is_some());
            assert!(previous.clock.is_none() || scene.clock.is_some());
            assert!(scene.pieces.len() >= previous.pieces.len());
            assert!(previous.groups[0].legend == "A" || scene.groups[0].legend == "Alt");
        }
        seen[match (scene.version.is_some(), centred, scene.pieces.len(), short) {
            (true, true, ..) => 0,
            (true, false, ..) => 1,
            _ if scene.clock.is_some() => 2,
            (false, _, 2, false) => 3,
            (false, _, _, false) => 4,
            (false, _, _, true) => 5,
        }] = true;
        previous = Some(scene);
    }
    assert!(previous.unwrap().complete);
    seen
}

#[test]
fn footer_centres_full_name_keycaps_and_keeps_the_version_clear() {
    let hints = default_like_hints();
    // As the application measures it: `HH:MM` in cells. Without a clock no step keeps one;
    // with it every step shows.
    let clocks = |metrics: CellMetrics| {
        [
            (None, [true, true, false, true, true, true]),
            (Some(5.0 * metrics.width), [true; 6]),
        ]
    };
    for scale in [1.0, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        for (clock, steps) in clocks(metrics) {
            let seen = sweep_footer(&hints, scale, metrics, clock, |text: &str| {
                text.chars().count() as f32 * metrics.width
            });
            assert_eq!(seen, steps, "{clock:?} at scale {scale}");
        }
    }
    // The same order holds with Eon's packaged font, whose measured widths decide each step,
    // and a 1280-pixel window keeps every hint centred beside the version.
    let font = std::env::var_os("EON_VENUS_TEST_SANS_FONT")
        .expect("EON_VENUS_TEST_SANS_FONT names Eon's packaged DejaVu Sans; run in nix develop");
    let mut fonts = crate::render::FontSetup::new(&crate::render::FontSettings::default()).unwrap();
    fonts
        .set_header_font_file(std::path::Path::new(&font))
        .unwrap();
    for scale in [1.0, 2.0] {
        let metrics = fonts.metrics(scale);
        for (clock, steps) in clocks(metrics) {
            let seen = sweep_footer(&hints, scale, metrics, clock, |text: &str| {
                fonts.measure_header_text(metrics, text)
            });
            assert_eq!(seen, steps, "{clock:?} in DejaVu Sans at scale {scale}");
            let scene = ShortcutHintsScene::new(
                hints.clone(),
                PhysicalSize::new((1280.0 * scale) as u32, (800.0 * scale) as u32),
                metrics,
                Some("EON 0.1.18".len() as f32 * metrics.width),
                clock,
                |text| fonts.measure_header_text(metrics, text),
            );
            let left = scene.groups[0].rect.left;
            let right = scene.pieces.last().unwrap().rect.right();
            assert!(scene.complete);
            assert!(((left + right) / 2.0 - 640.0 * scale as f32).abs() < 0.5);
            // The clock's text starts as far from the left edge as the version's ends from the
            // right, on the same row.
            if let Some(clock) = scene.clock {
                let version = scene.version.unwrap();
                assert_eq!(clock.left, 1280.0 * scale as f32 - version.right());
                assert_eq!((clock.top, clock.height), (version.top, version.height));
            }
        }
        // With the terminal focused (keycaps only), at the widths the bead names: 1280 and 800
        // show the clock, the centred full names and the version; 400 keeps the full names
        // without either; 330 takes the short legends.
        let keycaps = hints
            .iter()
            .filter(|hint| !hint.modifiers.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        for (logical, shown, full) in [
            (1280, true, true),
            (800, true, true),
            (400, false, true),
            (330, false, false),
        ] {
            let width = (logical as f64 * scale) as u32;
            let scene = ShortcutHintsScene::new(
                keycaps.clone(),
                PhysicalSize::new(width, (800.0 * scale) as u32),
                metrics,
                Some("EON 0.1.18".len() as f32 * metrics.width),
                Some(5.0 * metrics.width),
                |text| fonts.measure_header_text(metrics, text),
            );
            assert_eq!(scene.clock.is_some(), shown, "{logical} at {scale}");
            assert_eq!(scene.version.is_some(), shown, "{logical} at {scale}");
            assert_eq!(
                scene.groups[0].legend == "Alt",
                full,
                "{logical} at {scale}"
            );
            let left = scene.groups[0].rect.left;
            let right = scene.groups.last().unwrap().rect.right();
            assert!(
                ((left + right) / 2.0 - width as f32 / 2.0).abs() < 0.5,
                "{logical}"
            );
        }
    }
    // While the inbox is open, its keys alone centre.
    let metrics = CellMetrics::for_scale(1.0);
    let inbox = ShortcutHintsScene::new(
        attention_inbox_hints(),
        PhysicalSize::new(1280, 800),
        metrics,
        Some(100.0),
        None,
        |text| text.chars().count() as f32 * metrics.width,
    );
    assert!(inbox.groups.is_empty() && inbox.pieces.len() == 4 && inbox.complete);
    let (first, last) = (&inbox.pieces[0], &inbox.pieces[3]);
    assert!(((first.rect.left + last.rect.right()) / 2.0 - 640.0).abs() < 0.5);
    // A row too short for text shows nothing.
    for height in [0, 1, 10, 40] {
        let scene = ShortcutHintsScene::new(
            hints.clone(),
            PhysicalSize::new(1280, height),
            CellMetrics::for_scale(1.0),
            Some(100.0),
            None,
            |text| text.chars().count() as f32 * 10.0,
        );
        assert!(scene.rect.top >= 0.0 && scene.rect.bottom() <= height as f32);
        if scene.rect.height < CellMetrics::for_scale(1.0).height {
            assert!(scene.groups.is_empty() && scene.pieces.is_empty());
            assert!(scene.version.is_none() && !scene.complete);
        }
    }
    let small_metrics = CellMetrics {
        width: 3.0,
        height: 6.0,
        font_size: 6.0,
        ..CellMetrics::for_scale(1.0)
    };
    let small = ShortcutHintsScene::new(
        hints,
        PhysicalSize::new(320, 600),
        small_metrics,
        None,
        None,
        |text| text.chars().count() as f32 * small_metrics.width,
    );
    assert!(
        small.rect.height >= small_metrics.padding,
        "the hints row keeps at least the edge padding height"
    );
}

#[test]
fn eon_bar_keeps_tabs_controls_and_drag_hits_disjoint_under_pressure() {
    use eon_workspace_protocol::v42::{Pane, PopupGeometry, Tab};

    let snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v42::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
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
                columns: vec![eon_workspace_protocol::v42::Column {
                    panes: vec![Pane {
                        label: format!("p{index}"),
                        pinned_name: None,
                        tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
            scene.controls.each_ref().map(|control| scene.hit_test(
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
            hints.then(|| ShortcutHintsScene::new(Vec::new(), size, metrics, None, None, |_| 0.0)),
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
    // The bar-to-card margin yields before the previous layout loses any space.
    for (height, offset) in [
        (102, 0.0),
        (108, 0.0),
        (110, 0.0),
        (112, 2.0),
        (116, 6.0),
        (122, 12.0),
        (130, 12.0),
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
    use eon_workspace_protocol::v42::{
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v42::WorkspaceInfo {
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
                            tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Closed,
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
    open.sidebar = eon_workspace_protocol::v42::Sidebar::Open;
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
fn rounded_chip_percentages_select_the_shown_grade() {
    use crate::{CpuReadings, CpuTimes, DiskUse, Load, LoadLevels, RamUse};
    use std::time::{Duration, Instant};

    for (busy, shown, grade) in [
        (595, 60, Load::Notice),
        (745, 75, Load::High),
        (895, 90, Load::Critical),
    ] {
        let now = Instant::now();
        let levels = LoadLevels::default();
        let times = |busy, idle| {
            CpuTimes::parse(&format!(
                "cpu {busy} 0 0 {idle} 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0"
            ))
        };
        let mut readings = CpuReadings::default();
        readings.push(now, times(0, 0), Duration::from_secs(3), levels);
        let cpu = readings
            .push(
                now + Duration::from_secs(2),
                times(busy, 1000 - busy),
                Duration::from_secs(3),
                levels,
            )
            .unwrap();
        let ram = RamUse::parse(&format!(
            "MemTotal: 1000 kB\nMemAvailable: {} kB",
            1000 - busy
        ))
        .unwrap()
        .rated(levels);
        let free = 10000 - ((shown - 1) * 100 + 1);
        let disk = DiskUse::of(10000, free, free, 4096, "/")
            .unwrap()
            .rated(levels);
        for (label, load) in [
            (&cpu.label, cpu.load),
            (&ram.label, ram.load),
            (&disk.label, disk.load),
        ] {
            assert_eq!(
                (label.ends_with(&format!(" {shown}%")), load),
                (true, grade),
                "{label}"
            );
        }
    }
}

#[test]
fn cpu_ram_and_disk_share_three_inclusive_use_levels() {
    use crate::{BarItem, CpuReadings, CpuTimes, DiskUse, Load, LoadLevels, RamUse};
    use std::time::{Duration, Instant};

    let snapshot = one_pane_snapshot();
    let levels = LoadLevels::default();
    for (percent, load, cpu_words, ram_words, disk_words) in [
        (59, Load::Normal, "", "", ""),
        (
            60,
            Load::Notice,
            "elevated use",
            "elevated use",
            "elevated use",
        ),
        (
            74,
            Load::Notice,
            "elevated use",
            "elevated use",
            "elevated use",
        ),
        (
            75,
            Load::High,
            "under high load",
            "under pressure",
            "filling up",
        ),
        (
            89,
            Load::High,
            "under high load",
            "under pressure",
            "filling up",
        ),
        (
            90,
            Load::Critical,
            "under heavy load",
            "under heavy pressure",
            "nearly full",
        ),
    ] {
        let start = Instant::now();
        let mut readings = CpuReadings::default();
        let times = |busy, idle| {
            CpuTimes::parse(&format!(
                "cpu {busy} 0 0 {idle} 0 0 0 0\ncpu0 0 0 0 0 0 0 0 0"
            ))
        };
        readings.push(start, times(0, 0), Duration::from_secs(3), levels);
        let cpu = readings
            .push(
                start + Duration::from_secs(2),
                times(percent, 100 - percent),
                Duration::from_secs(3),
                levels,
            )
            .unwrap();
        let ram = RamUse::parse(&format!(
            "MemTotal: 100 kB\nMemAvailable: {} kB",
            100 - percent
        ))
        .unwrap()
        .rated(levels);
        let disk = DiskUse::of(100, 100 - percent, 100 - percent, 4096, "/")
            .unwrap()
            .rated(levels);
        let scene = WorkspaceScene::from_snapshot_with_bar(
            &snapshot,
            EonBar {
                items: &[BarItem::Cpu, BarItem::Ram, BarItem::Disk],
                cpu: Some(&cpu),
                ram: Some(&ram),
                disk: Some(&disk),
                ..EonBar::default()
            },
            PhysicalSize::new(3000, 600),
            CellMetrics::for_scale(1.0),
            0.0,
            0.0,
            |_, text| (text.to_owned(), text.chars().count() as f32 * 10.0),
            None,
        );
        for (chip, words) in [
            (scene.cpu.unwrap(), cpu_words),
            (scene.ram.unwrap(), ram_words),
            (scene.disk.unwrap(), disk_words),
        ] {
            assert_eq!(chip.load, load, "{percent}%");
            assert!(chip.label.ends_with(&format!("{percent}%")));
            if words.is_empty() {
                assert!(!chip.description.contains(": "), "{}", chip.description);
            } else {
                assert!(
                    chip.description.ends_with(&format!(": {words}")),
                    "{}",
                    chip.description
                );
                assert!(
                    chip.tooltip.to_lowercase().ends_with(words),
                    "{}",
                    chip.tooltip
                );
            }
        }
    }
}

#[test]
fn cpu_and_ram_chips_colour_under_load_and_say_so() {
    use crate::{CpuReadings, CpuTimes, Load, LoadLevels, RamUse};
    use eon_workspace_protocol::v42::{Pane, PopupGeometry, Tab};
    use std::time::{Duration, Instant};

    let levels = LoadLevels::default();
    let ram = |percent: u64, levels| {
        RamUse::parse(&format!(
            "MemTotal: 10000000 kB\nMemAvailable: {} kB\n",
            (100 - percent) * 100_000
        ))
        .unwrap()
        .rated(levels)
    };
    for (percent, used, load, words, line) in [
        (59, "5.6", Load::Normal, "", ""),
        (60, "5.7", Load::Notice, ": elevated use", "\nElevated use"),
        (74, "7.1", Load::Notice, ": elevated use", "\nElevated use"),
        (
            75,
            "7.2",
            Load::High,
            ": under pressure",
            "\nUnder pressure",
        ),
        (
            89,
            "8.5",
            Load::High,
            ": under pressure",
            "\nUnder pressure",
        ),
        (
            90,
            "8.6",
            Load::Critical,
            ": under heavy pressure",
            "\nUnder heavy pressure",
        ),
    ] {
        let shown = ram(percent, levels);
        assert_eq!(shown.load, load, "{percent}%");
        assert_eq!(
            shown.description,
            format!("RAM use {percent}%, {used} GiB of 9.5 GiB{words}")
        );
        // Hover leaves out the percentage the chip shows, and lists pressure on its own line.
        assert_eq!(
            shown.tooltip,
            format!("RAM\n{used} GiB used of 9.5 GiB{line}")
        );
    }
    let configured = LoadLevels {
        ram_notice_percent: 50,
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
    assert_eq!(
        (busy.label.as_str(), busy.load),
        ("\u{f4bc} 100%", Load::Critical)
    );
    assert_eq!(
        busy.description,
        format!(
            "CPU use 100% across 1 logical CPU, over the last 2 seconds: under heavy load{}",
            average(16, 30)
        )
    );
    // Hover lists the average and the load on their own lines, without the use the chip shows.
    assert_eq!(
        busy.tooltip,
        "CPU\n1 logical CPU\n16% on average over the last 30 s\nUnder heavy load"
    );
    // The level is inclusive and follows the shown use, whatever the average.
    let mut readings = CpuReadings::default();
    let mut at_level = vec![10; 17];
    at_level[16] = 90;
    let shown = run(&mut readings, levels, 0, &at_level).remove(16).unwrap();
    assert_eq!(shown.load, Load::Critical);
    assert!(
        shown
            .description
            .ends_with(&format!(": under heavy load{}", average(15, 30)))
    );
    let mut readings = CpuReadings::default();
    let mut below = vec![95; 17];
    below[16] = 89;
    let shown = run(&mut readings, levels, 0, &below).remove(16).unwrap();
    assert_eq!(shown.load, Load::High);
    assert_eq!(
        shown.description,
        format!(
            "CPU use 89% across 1 logical CPU, over the last 2 seconds: under high load{}",
            average(95, 30)
        )
    );
    assert_eq!(
        shown.tooltip,
        "CPU\n1 logical CPU\n95% on average over the last 30 s\nUnder high load"
    );
    // The average shows only once the readings span the window.
    let mut readings = CpuReadings::default();
    let shown = run(&mut readings, levels, 0, &[95; 16]);
    assert!(shown[0].is_none());
    assert_eq!(shown[14].as_ref().unwrap().load, Load::Critical);
    assert_eq!(
        shown[14].as_ref().unwrap().description,
        "CPU use 95% across 1 logical CPU, over the last 2 seconds: under heavy load"
    );
    assert_eq!(
        shown[14].as_ref().unwrap().tooltip,
        "CPU\n1 logical CPU\nUnder heavy load"
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
        cpu_notice_percent: 50,
        cpu_warning_percent: 50,
        cpu_critical_percent: 50,
        cpu_average: Duration::from_secs(4),
        ..levels
    };
    let mut readings = CpuReadings::default();
    let shown = run(&mut readings, quick, 0, &[60, 40, 60]);
    assert_eq!(shown[1].as_ref().unwrap().load, Load::Normal);
    let quick_shown = shown[2].as_ref().unwrap();
    assert_eq!(quick_shown.load, Load::Critical);
    assert!(quick_shown.description.ends_with(&average(50, 4)));
    // The average covers exactly the window, even when it is odd or the readings are uneven:
    // an idle interval then a busy one is 67% over the last 3 of 4 seconds, and 50% over the last
    // 4 of 4.5 seconds when the idle interval took 2.5 of them.
    for (window, busy_at, average) in [(3, [2.0, 4.0], 67), (4, [2.5, 4.5], 50)] {
        let levels = LoadLevels {
            cpu_notice_percent: 1,
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
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v42::WorkspaceInfo {
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
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
    assert_eq!((cpu.load, ram.load), (Load::Critical, Load::Critical));
    assert_eq!(cpu.description, busy.description);
    assert_eq!(
        ram.description,
        "RAM use 91%, 8.7 GiB of 9.5 GiB: under heavy pressure"
    );
    assert_eq!(cpu.tooltip, busy.tooltip);
    assert_eq!(
        ram.tooltip,
        "RAM\n8.7 GiB used of 9.5 GiB\nUnder heavy pressure"
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
    use eon_workspace_protocol::v42::{Pane, PopupGeometry, Tab};

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
    for (percent, load, words, line) in [
        (59, Load::Normal, "", ""),
        (60, Load::Notice, ": elevated use", "\nElevated use"),
        (74, Load::Notice, ": elevated use", "\nElevated use"),
        (75, Load::High, ": filling up", "\nFilling up"),
        (89, Load::High, ": filling up", "\nFilling up"),
        (90, Load::Critical, ": nearly full", "\nNearly full"),
    ] {
        let shown = disk(percent, levels);
        assert_eq!(shown.label, format!("\u{f0a0} {percent}%"));
        assert_eq!(shown.load, load, "{percent}%");
        assert_eq!(
            shown.description,
            format!(
                "Disk use {percent}% of 1000.0 GiB on /, {}.0 GiB free{words}",
                (100 - percent) * 10
            )
        );
        // Hover leaves out the use the chip shows, and lists how full it is on its own line.
        assert_eq!(
            shown.tooltip,
            format!(
                "Disk \u{b7} /\n{}.0 GiB free of 1000.0 GiB{line}",
                (100 - percent) * 10
            )
        );
    }
    let configured = LoadLevels {
        disk_notice_percent: 50,
        disk_warning_percent: 50,
        disk_critical_percent: 70,
        ..levels
    };
    assert_eq!(disk(49, configured).load, Load::Normal);
    assert_eq!(disk(50, configured).load, Load::High);
    assert_eq!(disk(70, configured).load, Load::Critical);

    let snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![eon_workspace_protocol::v42::WorkspaceInfo {
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
        entries: Vec::new(),
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
    assert_eq!(shown.label, "\u{f0a0} 95%");
    assert_eq!(shown.load, Load::Critical);
    assert_eq!(shown.description, full.description);
    assert_eq!(
        shown.tooltip,
        "Disk \u{b7} /\n50.0 GiB free of 1000.0 GiB\nNearly full"
    );
    // Default icon 16 + gap 2 + injected two-digit percentage 30 + padding 24.
    assert_eq!(shown.rect.width, 72.0);
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
fn eon_icon_leads_the_bar_and_survives_tab_overflow() {
    use eon_workspace_protocol::v42::{Pane, PopupGeometry, Tab};
    let tab = |id: String| Tab {
        id: id.clone(),
        directory: b"/tmp".to_vec(),
        pending: false,
        selected_pane: Some(id.clone()),
        selected_popup: None,
        columns: vec![eon_workspace_protocol::v42::Column {
            panes: vec![Pane {
                label: id.clone(),
                pinned_name: None,
                tag: None,
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
                workspace: "s1".into(),
                active_workspace: "s1".into(),
                workspaces: (1..=workspaces)
                    .map(|n| eon_workspace_protocol::v42::WorkspaceInfo {
                        id: format!("s{n}"),
                        persistent_id: Some(format!("s{n}")),
                        registration_error: None,
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
                sidebar: eon_workspace_protocol::v42::Sidebar::Open,
                attention_queue: Vec::new(),
            };
            for (version, ram) in [
                (Some("9.8.7"), None),
                (None, None),
                (Some("9.8.7"), Some(&used)),
            ] {
                // At the tiny-width minimum the icon hides; once it shows as the window widens, it
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
                    // Below its minimum width the icon leaves the plain bar,
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
                    // Overflowing tabs scroll beside the icon.
                    if tab_count == 12 && logical_width == 600 {
                        assert!(iconic.tab_scroll_limit() > 0.5);
                    }
                    // The version label sits in the shortcut hints, never the header.
                    assert!(iconic.version.is_none());
                    assert_eq!(iconic.version, plain.version);
                    for tab_scroll in [5.0, 1.0e6] {
                        let scrolled = project(true, tab_scroll);
                        assert_eq!(scrolled.eon_icon, iconic.eon_icon);
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
fn reading_slots_only_grow_and_reflow_at_one_hundred_percent() {
    let snapshot = one_pane_snapshot();
    for scale in [1.0, 1.125, 1.25, 1.5, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        // Inject a widest two-digit value and one extra 11-pixel digit at 100%.
        let widest = 53.5 * scale as f32;
        for logical_width in [400, 900, 1800] {
            let mut previous = None;
            for readings in [
                [8, 61, 67],
                [61, 8, 99],
                [100, 61, 67],
                [8, 100, 67],
                [8, 61, 100],
            ] {
                let [cpu_percent, ram_percent, disk_percent] = readings;
                let cpu = crate::CpuUse {
                    percent: cpu_percent,
                    label: format!("\u{f4bc} {cpu_percent}%"),
                    tooltip: "CPU".into(),
                    description: "CPU".into(),
                    load: Load::Normal,
                };
                let ram = crate::RamUse::parse(&format!(
                    "MemTotal: 100000 kB\nMemAvailable: {} kB\n",
                    (100 - ram_percent) * 1000
                ))
                .unwrap()
                .rated(crate::LoadLevels::default());
                let disk = DiskUse::of(100, 100 - disk_percent, 100 - disk_percent, 4096, "/")
                    .unwrap()
                    .rated(crate::LoadLevels::default());
                let scene = WorkspaceScene::from_snapshot_with_bar(
                    &snapshot,
                    EonBar {
                        cpu: Some(&cpu),
                        ram: Some(&ram),
                        disk: Some(&disk),
                        ..EonBar::default()
                    },
                    PhysicalSize::new((logical_width as f64 * scale) as u32, 800),
                    metrics,
                    0.0,
                    0.0,
                    |kind, text| {
                        (
                            text.to_owned(),
                            if kind == HeaderText::Reading {
                                widest
                            } else if text == "100%" {
                                64.5 * scale as f32
                            } else {
                                text.chars().count() as f32 * 10.0 * scale as f32
                            },
                        )
                    },
                    None,
                );
                let labels = [&scene.cpu, &scene.ram, &scene.disk];
                let slots: [_; 3] = std::array::from_fn(|index| {
                    labels[index].as_ref().map(|label| {
                        if scale == 1.0 {
                            let width = if readings[index] == 100 { 106.5 } else { 95.5 };
                            assert_eq!(label.rect.width, width);
                        }
                        label.rect
                    })
                });
                let positions = (
                    slots,
                    scene.tabs.iter().map(|tab| tab.rect).collect::<Vec<_>>(),
                    scene.controls.each_ref().map(|control| control.rect),
                    scene.drag_region,
                );
                if readings.iter().all(|percent| *percent < 100) {
                    if let Some(previous) = &previous {
                        assert_eq!(&positions, previous);
                    }
                    previous = Some(positions);
                } else if logical_width == 1800 {
                    let previous = previous.as_ref().unwrap();
                    for index in 0..3 {
                        let before = previous.0[index].unwrap();
                        let after = slots[index].unwrap();
                        let growth = if readings[index] == 100 {
                            11.0 * scale as f32
                        } else {
                            0.0
                        };
                        assert!((after.width - before.width - growth).abs() < 0.01);
                        let wider = readings[index..]
                            .iter()
                            .filter(|percent| **percent == 100)
                            .count();
                        let shift = wider as f32 * 11.0 * scale as f32;
                        assert!((after.left - (before.left - shift)).abs() < 0.01);
                    }
                    assert_eq!(positions.1, previous.1);
                    assert_eq!(positions.2, previous.2);
                }
                if logical_width == 1800 {
                    assert!(slots.iter().all(Option::is_some));
                }
            }
        }
    }
}

#[test]
fn live_items_never_show_or_hide_the_icon_or_move_the_tabs() {
    use eon_workspace_protocol::v42::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
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
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
        tooltip: "Clock".into(),
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
    let cpu = |percent: u64| crate::CpuUse {
        percent,
        label: format!("\u{f4bc} {percent}%"),
        tooltip: "CPU".into(),
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
        // Quota values and resets do not displace the icon.
        for logical_width in (100..=3000).step_by(25) {
            let size = PhysicalSize::new((logical_width as f64 * scale) as u32, 600);
            let mut seen = None;
            for codex in &codex {
                for claude in &claude {
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
fn quota_facts_do_not_hide_the_window_menu() {
    use eon_workspace_protocol::v42::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
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
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
                // Quota changes yield space to the menu, never hide it.
                assert_eq!(
                    shown_before, shown_after,
                    "{scale} {logical_width} change {index}"
                );
                toggled |= shown_before != shown_after;
            }
            assert!(!toggled, "{scale}: change {index} must keep the icon");
        }
    }
}

#[test]
fn a_full_bar_with_room_shows_the_icon() {
    use eon_workspace_protocol::v42::{
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
        columns: vec![eon_workspace_protocol::v42::Column {
            panes: vec![Pane {
                label: format!("p{index}"),
                pinned_name: None,
                tag: None,
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
        persistent_id: Some(id.into()),
        registration_error: None,
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
    let levels = crate::LoadLevels::default();
    let ram = crate::RamUse::parse("MemTotal: 100000 kB\nMemAvailable: 50000 kB\n")
        .unwrap()
        .rated(levels);
    let disk = DiskUse::of(1000 << 18, 0, 0, 4096, "/")
        .unwrap()
        .rated(levels);
    let cpu = crate::CpuUse {
        percent: 12,
        label: "\u{f4bc} 12%".into(),
        tooltip: "CPU".into(),
        description: "CPU".into(),
        load: Load::Normal,
    };
    let clock = LocalClock {
        label: "09:20".into(),
        tooltip: "Clock".into(),
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
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![workspace("s1", "main", 0), workspace("s2", "evo", 1)],
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
            sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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

/// One tab with one pane.
pub(crate) fn one_pane_snapshot() -> Snapshot {
    use eon_workspace_protocol::v42::{Pane, PopupGeometry, Tab};
    Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
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
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
        attention_queue: Vec::new(),
    }
}

#[test]
fn shortcuts_and_close_end_the_bar_one_edge_from_the_corner() {
    let snapshot = one_pane_snapshot();
    let used = crate::RamUse::parse("MemTotal: 100000 kB\nMemAvailable: 50000 kB\n")
        .unwrap()
        .rated(crate::LoadLevels::default());
    for scale in [1.0, 1.25, 2.0] {
        let metrics = CellMetrics::for_scale(scale);
        let width = 1280.0 * scale as f32;
        let size = PhysicalSize::new(width as u32, (800.0 * scale) as u32);
        for (version, ram, shown) in [Some("9.8.7"), None].into_iter().flat_map(|version| {
            [(None, true), (Some(&used), true), (None, false)]
                .map(|(ram, shown)| (version, ram, shown))
        }) {
            let measure = |text: &str| text.chars().count() as f32 * metrics.width;
            let hints = shown.then(|| {
                ShortcutHintsScene::new(
                    default_like_hints(),
                    size,
                    metrics,
                    version.map(|version| measure(&format!("EON {version}"))),
                    None,
                    measure,
                )
            });
            let scene = WorkspaceScene::from_snapshot_with_bar(
                &snapshot,
                EonBar {
                    version,
                    ram,
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
                hints,
            );
            let [_, shortcuts, close] = &scene.controls;
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
            // Shortcuts, then Close; Shortcuts keeps its whole target.
            assert_eq!(
                shortcuts.centre,
                shortcuts.rect.right() - scene.controls[0].rect.width / 2.0
            );
            // After a chip the glyph keeps the existing spacing and the full target.
            if let Some(ram) = &scene.ram {
                let chip_inset = (metrics.padding / 3.0).min(height / 4.0);
                assert_eq!(ram.rect.right(), shortcuts.rect.left);
                let square = scene.control_glyph_rect(shortcuts, metrics);
                let gap = square.left - (ram.rect.right() - chip_inset);
                assert!((gap - 2.0 * chip_inset).abs() <= 0.5, "{scale}: {gap}");
                assert_eq!(
                    scene.hit_test(ram.rect.left + ram.rect.width / 2.0, height / 2.0),
                    Some(WorkspaceHit::Ram)
                );
            } else {
                assert_eq!(shortcuts.rect.width, scene.controls[0].rect.width);
            }
            assert!(shortcuts.rect.width >= metrics.height.round());
            assert_eq!(shortcuts.rect.right(), close.rect.left);
            // The version label sits at the right end of the hints, and hides with them.
            assert_eq!(scene.version.is_some(), version.is_some() && shown);
            if let Some(label) = &scene.version {
                let hints = scene.hints.as_ref().unwrap();
                assert_eq!(label.label, "EON 9.8.7");
                assert_eq!(label.tooltip, "Eon 9.8.7");
                assert!(label.rect.top >= hints.rect.top);
                assert!((label.rect.right() - (width - metrics.padding)).abs() < 0.01);
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
                let row = label.rect.top + label.rect.height / 2.0;
                assert_eq!(scene.hit_test(centre, row), Some(WorkspaceHit::Version));
            }
            // Shortcuts' ink-centering square follows the pills' height and centre line,
            // and stays inside its full target.
            let pill = scene.bar_pill_rect(scene.tabs[0].rect, metrics);
            let pill_centre = pill.top + pill.height / 2.0;
            let square = scene.control_glyph_rect(shortcuts, metrics);
            assert_eq!(
                (square.width, square.height),
                (pill.height.round(), pill.height.round())
            );
            assert!((square.left + square.width / 2.0 - shortcuts.centre).abs() <= 0.5);
            assert!((square.top + square.height / 2.0 - pill_centre).abs() <= 0.5);
            assert!(square.left >= shortcuts.rect.left && square.right() <= shortcuts.rect.right());
            assert_eq!(
                scene.hit_test(square.left + 0.5, pill_centre),
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
fn hint_keycap_hover_hits_its_group() {
    let snapshot = one_pane_snapshot();
    let metrics = CellMetrics::for_scale(1.0);
    let size = PhysicalSize::new(1280, 800);
    let measure = |text: &str| text.chars().count() as f32 * metrics.width;
    let hints = ShortcutHintsScene::new(
        default_like_hints(),
        size,
        metrics,
        Some(measure("EON 9.8.7")),
        None,
        measure,
    );
    let scene = WorkspaceScene::from_snapshot_with_bar(
        &snapshot,
        EonBar {
            version: Some("9.8.7"),
            ..EonBar::default()
        },
        size,
        metrics,
        0.0,
        0.0,
        |_, text| (text.to_owned(), measure(text)),
        Some(hints.clone()),
    );
    for (index, group) in hints.groups.iter().enumerate() {
        let hit = Some(WorkspaceHit::HintGroup(index));
        let middle = group.rect.top + group.rect.height / 2.0;
        // The keycap, its edges, and the footer band's top and bottom above and below it.
        for (x, y) in [
            (group.rect.left + 0.5, middle),
            (group.rect.right() - 0.5, middle),
            (group.rect.left + 1.0, hints.rect.top + 0.5),
            (group.rect.left + 1.0, hints.rect.bottom() - 0.5),
        ] {
            assert_eq!(scene.hit_test(x, y), hit, "{index} at {x},{y}");
        }
    }
    // Half a gap belongs to each neighbour, so the band between keycaps hits one of them.
    let [first, second, ..] = hints.groups.as_slice() else {
        unreachable!()
    };
    let between = (first.rect.right() + second.rect.left) / 2.0;
    assert_eq!(
        scene.hit_test(between - 0.5, first.rect.top),
        Some(WorkspaceHit::HintGroup(0))
    );
    assert_eq!(
        scene.hit_test(between + 0.5, first.rect.top),
        Some(WorkspaceHit::HintGroup(1))
    );
    let version = scene.version.as_ref().unwrap().rect;
    assert_eq!(
        scene.hit_test(version.left + 1.0, version.top + 1.0),
        Some(WorkspaceHit::Version)
    );
    // Expanded hints, the space beside the cluster, and the pane card are no keycap.
    let piece = hints.pieces[0].rect;
    for (x, y) in [
        (piece.left + 1.0, piece.top + 1.0),
        (metrics.padding, first.rect.top),
        (640.0, 400.0),
    ] {
        assert!(!matches!(
            scene.hit_test(x, y),
            Some(WorkspaceHit::HintGroup(_))
        ));
    }
}

#[test]
fn quota_text_width_does_not_hide_the_menu() {
    use eon_workspace_protocol::v42::{CodexQuota, Pane, PopupGeometry, Tab};
    let snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
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
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
    // Different quota text widths cannot displace the icon.
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
                scene.eon_icon.is_some(),
                "{logical_width}: quota text displaced the icon"
            );
        }
        assert_eq!(seen, [false, true]);
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
                scene.eon_icon.is_some(),
                "{logical_width}: quota text displaced the icon"
            );
            scene.eon_icon.is_some()
        });
        assert_eq!(shown[0], shown[1], "{logical_width}");
        toggled[usize::from(shown[0])] = true;
    }
    assert_eq!(toggled, [false, true]);
}

#[test]
fn quota_window_widgets_filter_order_and_reserve_only_selected_windows() {
    use eon_workspace_protocol::v42::{
        ClaudeQuota, ClaudeQuotaState, CodexQuota, Pane, PopupGeometry, Tab,
    };
    let mut snapshot = Snapshot {
        workspace: "s1".into(),
        active_workspace: "s1".into(),
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
            columns: vec![eon_workspace_protocol::v42::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
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
        sidebar: eon_workspace_protocol::v42::Sidebar::Open,
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
    let cpu = crate::CpuUse {
        percent: 12,
        label: "\u{f4bc} 12%".into(),
        tooltip: "CPU".into(),
        description: "CPU".into(),
        load: Load::Normal,
    };
    let project = |snapshot: &Snapshot, items: &[BarItem], width| {
        WorkspaceScene::from_snapshot_with_bar(
            snapshot,
            EonBar {
                items,
                icon: true,
                cpu: Some(&cpu),
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
            project(&without_scoped, &selected, width)
                .eon_icon
                .is_some(),
            "{width}"
        );
    }
}

#[test]
fn pane_actions_have_disjoint_exact_targets_and_yield_together() {
    let mut snapshot = one_pane_snapshot();
    snapshot.sidebar = eon_workspace_protocol::v42::Sidebar::Off;
    let project = |snapshot: &Snapshot, width, scale| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(width, 800),
            CellMetrics::for_scale(scale),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    let lone = project(&snapshot, 1280, 1.0);
    let pane = &lone.panes[0];
    assert!(
        !lone
            .pane_action(&pane.id, PaneHeaderAction::Zoom)
            .unwrap()
            .1
    );
    let mut second = snapshot.tabs[0].columns[0].panes[0].clone();
    second.id = "p2".into();
    second.label = "p2  a long pane title".repeat(20);
    snapshot.tabs[0].columns[0].panes.push(second);
    for scale in [1.0, 1.3, 4.0 / 3.0, 2.0] {
        for width in [80, 760, 1280] {
            let scene = project(&snapshot, width, scale);
            for pane in &scene.panes {
                if let Some(actions) = pane.actions {
                    assert!(
                        actions[0].top >= scene.header_viewport.top
                            && actions[0].bottom() <= scene.header_viewport.bottom()
                    );
                    assert!(actions[0].intersection(actions[1]).is_none());
                    assert!(
                        pane.header_content(CellMetrics::for_scale(scale)).right()
                            < actions[0].left
                    );
                    for action in PaneHeaderAction::ALL {
                        let target = actions[action.index()];
                        assert_eq!(
                            scene.hit_test(
                                target.left + target.width / 2.0,
                                target.top + target.height / 2.0
                            ),
                            Some(if pane.selected {
                                WorkspaceHit::PaneAction(&pane.id, action)
                            } else {
                                WorkspaceHit::Pane(&pane.id)
                            })
                        );
                        assert!(scene.pane_action(&pane.id, action).unwrap().1);
                    }
                } else {
                    assert!(width == 80);
                    assert!(
                        scene
                            .pane_action(&pane.id, PaneHeaderAction::Close)
                            .is_none()
                    );
                }
            }
        }
    }
}

#[test]
fn pane_buttons_follow_tab_selection_and_keep_every_reserved_target() {
    let mut snapshot = one_pane_snapshot();
    snapshot.sidebar = eon_workspace_protocol::v42::Sidebar::Off;
    let template = snapshot.tabs[0].columns[0].panes[0].clone();
    let pane = |id: &str| {
        let mut pane = template.clone();
        pane.id = id.into();
        pane.label = format!("{id}  a long pane title");
        pane
    };
    snapshot.tabs[0].columns = vec![
        eon_workspace_protocol::v42::Column {
            panes: vec![pane("p1"), pane("p2"), pane("p3")],
            expanded: "p2".into(),
        },
        eon_workspace_protocol::v42::Column {
            panes: vec![pane("p4"), pane("p5")],
            expanded: "p4".into(),
        },
    ];
    let project = |snapshot: &Snapshot, scale| {
        WorkspaceScene::from_snapshot(
            snapshot,
            PhysicalSize::new(2400, 1200),
            CellMetrics::for_scale(scale),
            0.0,
            0.0,
            |_, text| (text.into(), text.len() as f32 * 10.0),
        )
    };
    for scale in [1.0, 1.3, 2.0] {
        snapshot.tabs[0].selected_pane = Some("p2".into());
        let first = project(&snapshot, scale);
        snapshot.tabs[0].selected_pane = Some("p4".into());
        let second = project(&snapshot, scale);
        for (scene, selected) in [(&first, "p2"), (&second, "p4")] {
            for (panes, _) in scene.column_panes() {
                for pane in panes {
                    let before = first
                        .column_panes()
                        .flat_map(|(panes, _)| panes)
                        .find(|before| before.id == pane.id)
                        .unwrap();
                    assert_eq!(pane.actions, before.actions);
                    assert_eq!(
                        pane.header_content(CellMetrics::for_scale(scale)),
                        before.header_content(CellMetrics::for_scale(scale)),
                        "focus must not change title fitting"
                    );
                    for action in PaneHeaderAction::ALL {
                        let (target, enabled) = scene.pane_action(&pane.id, action).unwrap();
                        assert!(enabled, "semantic actions remain available on every pane");
                        assert_eq!(
                            scene.hit_test(
                                target.left + target.width / 2.0,
                                target.top + target.height / 2.0,
                            ),
                            Some(if pane.id == selected {
                                WorkspaceHit::PaneAction(&pane.id, action)
                            } else {
                                WorkspaceHit::Pane(&pane.id)
                            }),
                            "{} at scale {scale}",
                            pane.id
                        );
                    }
                }
            }
        }
        snapshot.tabs[0].selected_pane = Some("p2".into());
        snapshot.tabs[0].columns[1].panes.truncate(1);
        let lone = project(&snapshot, scale);
        let (close, _) = lone.pane_action("p4", PaneHeaderAction::Close).unwrap();
        assert_eq!(
            lone.hit_test(
                close.left + close.width / 2.0,
                close.top + close.height / 2.0
            ),
            Some(WorkspaceHit::PaneAction("p4", PaneHeaderAction::Close)),
            "a one-pane Stack retains its controls in an unselected column"
        );
        snapshot.tabs[0].columns[1].panes.push(pane("p5"));
    }
}

#[test]
fn header_and_viewer_dismissal_labels_follow_shared_chords_and_policy() {
    use eon_workspace_protocol::shortcuts::{Action, Platform, PopupPolicy, shortcut};
    use eon_workspace_protocol::v42::{PopupKey, Shortcut};
    for (control, action, label) in [
        (WorkspaceHeaderControl::NewTab, Action::CreateTab, "New tab"),
        (
            WorkspaceHeaderControl::Shortcuts,
            Action::ToggleViewer,
            "Keyboard shortcuts",
        ),
        (
            WorkspaceHeaderControl::CloseTab,
            Action::CloseTab,
            "Close tab",
        ),
    ] {
        let descriptor = shortcut(action);
        let chord = descriptor.trigger.primary_chord(Platform::CURRENT);
        let expected = format!("{label} — {}", chord.display(Platform::CURRENT));
        assert_eq!(control.label(&[]), expected);
        let popup = PopupEntry {
            id: "takes-key".into(),
            label: "Takes key".into(),
            shortcut: PopupKey::Bound(Shortcut {
                key: chord.key.into(),
                modifiers: chord.modifiers,
            }),
        };
        assert_eq!(
            control.label(&[popup]),
            if descriptor.popup_policy() == PopupPolicy::Overridable {
                label.to_owned()
            } else {
                expected
            }
        );
    }
    let toggle = shortcut(Action::ToggleViewer)
        .trigger
        .display_label_for(Platform::CURRENT);
    assert!(DialogKind::Shortcuts.footer().join(" ").contains(&toggle));
}
