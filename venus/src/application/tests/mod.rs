use super::*;
use eon_workspace_protocol::v43::{Popup, PopupKey, Shortcut, Snapshot};

mod chrome;
mod native;
mod retry;
mod selection;
mod shortcuts;

#[test]
fn presented_scene_timing_does_not_follow_a_column_across_focus_switches() {
    const CHILD: &str = "EON_VENUS_TEST_SCENE_TIMINGS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "application::tests::presented_scene_timing_does_not_follow_a_column_across_focus_switches"])
            .env(CHILD, "1")
            .env("EON_VENUS_FRAME_TIMINGS", "unused-scene-test.jsonl")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let mut terminal = TerminalView::new(None);
    let mut columns = [TerminalView::new(None)];
    for view in std::iter::once(&mut terminal).chain(&mut columns) {
        view.model.apply(ServerMessage::Attached).unwrap();
        let TransportEvent::Server(frame) = native::frame(1, orbit_protocol::Screen::Primary)
        else {
            unreachable!()
        };
        view.model.apply(frame).unwrap();
    }
    assert!(take_scene_build_ns(&mut terminal, &mut columns) > 0);
    std::mem::swap(&mut terminal, &mut columns[0]);
    assert_eq!(
        take_scene_build_ns(&mut terminal, &mut columns),
        0,
        "switching focus must not charge a previously presented background scene again"
    );
    let TransportEvent::Server(frame) = native::frame(2, orbit_protocol::Screen::Primary) else {
        unreachable!()
    };
    columns[0].model.apply(frame).unwrap();
    assert!(
        take_scene_build_ns(&mut terminal, &mut columns) > 0,
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
                modifiers: eon_workspace_protocol::v43::ALT,
                key: "KeyZ".into(),
            }),
        },
        PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v43::ALT | eon_workspace_protocol::v43::SHIFT,
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
