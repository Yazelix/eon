use super::*;
use eon_workspace_protocol::v47::{Popup, PopupKey, Shortcut, Snapshot};

mod chrome;
mod native;
mod retry;
mod selection;
mod shortcuts;

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
                modifiers: eon_workspace_protocol::v47::ALT,
                key: "KeyZ".into(),
            }),
        },
        PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v47::ALT | eon_workspace_protocol::v47::SHIFT,
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
