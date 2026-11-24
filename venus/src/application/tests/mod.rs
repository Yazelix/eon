use super::*;
use eon_workspace_protocol::v50::{Popup, PopupKey, Shortcut, Snapshot};

mod chrome;
mod native;
mod retry;
mod selection;
mod shortcuts;

#[cfg(any(target_os = "linux", feature = "candidate-rune"))]
/// The probe window's generation, which the Reserved Stop carries.
pub(super) const GENERATION_G7: &str = "g1-00000000000000000000000000000007";

#[cfg(any(target_os = "linux", feature = "candidate-rune"))]
/// A workspace snapshot whose one window, g7, is running with p1 (at `endpoint`) busy: a close
/// request asks before it quits (VEN-C8).
pub(super) fn busy_window_snapshot(endpoint: Vec<u8>) -> Snapshot {
    use eon_workspace_protocol::v50::{
        self as workspace, BusyTerminal, BusyTerminals, Pane, RunningWindows, Tab, TargetId,
        TerminalTarget, WindowEntry, WindowState,
    };
    workspace::Snapshot {
        focus: None,
        released_shortcuts: Default::default(),
        input_locked: false,
        input_lock: crate::fixture_input_lock(),
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: "s1".into(),
        active_workspace: "s1".into(),
        workspaces: vec![workspace::WorkspaceInfo {
            id: "s1".into(),
            persistent_id: Some("s1".into()),
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        }],
        active_tab: "t1".into(),
        geometry: workspace::PopupGeometry {
            side_margin: 8.0,
            vertical_margin: 4.0,
        },
        entries: vec![],
        tabs: vec![Tab {
            id: "t1".into(),
            directory: b"/tmp".to_vec(),
            pending: false,
            selected_pane: Some("p1".into()),
            selected_popup: None,
            columns: vec![workspace::Column {
                panes: vec![Pane {
                    label: "p1".into(),
                    pinned_name: None,
                    tag: None,
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint,
                    live: true,
                    attention: None,
                }],
                expanded: "p1".into(),
            }],
            popups: vec![],
            attention_rollup: None,
        }],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: Some(RunningWindows {
            this: WindowEntry {
                created_build: Some("g1-00000000000000000000000000000001".into()),
                active_build: Some("g1-00000000000000000000000000000001".into()),
                window: None,
                generation: GENERATION_G7.into(),
                name: None,
                started_at: None,
                live_terminals: Some(3),
                state: WindowState::Live,
            },
            others: None,
            omitted: 0,
        }),
        busy: Some(BusyTerminals {
            terminals: vec![BusyTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p1".into()),
                    terminal: "s1".into(),
                },
                name: "p1".into(),
            }],
            omitted: 0,
        }),
        sidebar: workspace::Sidebar::Open,
        attention_queue: Vec::new(),
    }
}

#[test]
fn presented_scene_timing_does_not_follow_a_column_across_focus_switches() {
    let mut terminal = TerminalView::new(None);
    let mut columns = [TerminalView::new(None)];
    terminal.model.record_scene_build_ns(5);
    terminal.model.record_scene_build_ns(6);
    columns[0].model.record_scene_build_ns(17);
    assert_eq!(take_scene_build_ns(&mut terminal, &mut columns), 28);
    std::mem::swap(&mut terminal, &mut columns[0]);
    columns[0].model.record_scene_build_ns(0);
    assert_eq!(
        take_scene_build_ns(&mut terminal, &mut columns),
        0,
        "switching focus must not charge a previously presented background scene again"
    );
    columns[0].model.record_scene_build_ns(23);
    assert_eq!(
        take_scene_build_ns(&mut terminal, &mut columns),
        23,
        "new background scene work belongs to this window presentation"
    );
    std::mem::swap(&mut terminal, &mut columns[0]);
    assert_eq!(take_scene_build_ns(&mut terminal, &mut columns), 0);
}

pub(super) fn show_test_popup(snapshot: &mut Snapshot) {
    // Alt+Z's Project shows the tab's Files pair, the popup it lists.
    snapshot.entries = vec![
        PopupEntry {
            id: "project".into(),
            label: "Quick search".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v50::ALT,
                key: "KeyZ".into(),
            }),
        },
        PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v50::ALT | eon_workspace_protocol::v50::SHIFT,
                key: "KeyY".into(),
            }),
        },
    ];
    snapshot.tabs[0].popups = vec![Popup {
        id: "u1".into(),
        entry: "files".into(),
        session: "popup-session".into(),
        endpoint: b"/tmp/popup.sock".to_vec(),
        attention: None,
    }];
    snapshot.tabs[0].selected_popup = Some("u1".into());
}
