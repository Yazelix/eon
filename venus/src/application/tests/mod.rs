use super::*;
use eon_workspace_protocol::v35::{Popup, PopupKey, Shortcut, Snapshot};

mod chrome;
mod native;
mod retry;
mod selection;
mod shortcuts;

pub(super) fn show_test_popup(snapshot: &mut Snapshot) {
    // Alt+Z's Project shows the tab's Files pair, the popup it lists.
    snapshot.entries = vec![
        PopupEntry {
            id: "project".into(),
            label: "Quick search".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v35::ALT,
                key: "KeyZ".into(),
            }),
        },
        PopupEntry {
            id: "files".into(),
            label: "Files".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: eon_workspace_protocol::v35::ALT | eon_workspace_protocol::v35::SHIFT,
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
