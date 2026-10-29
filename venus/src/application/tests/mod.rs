use super::*;
use eon_workspace_protocol::v24::{Popup, PopupKey, Shortcut, Snapshot};

mod chrome;
mod native;
mod retry;
mod selection;
mod shortcuts;

pub(super) fn show_test_popup(snapshot: &mut Snapshot) {
    snapshot.entries = vec![PopupEntry {
        id: "project".into(),
        label: "Project".into(),
        shortcut: PopupKey::Bound(Shortcut {
            modifiers: eon_workspace_protocol::v24::ALT,
            key: "KeyZ".into(),
        }),
    }];
    snapshot.tabs[0].popups = vec![Popup {
        id: "u1".into(),
        entry: "project".into(),
        session: "popup-session".into(),
        endpoint: b"/tmp/popup.sock".to_vec(),
        attention: None,
    }];
    snapshot.tabs[0].selected_popup = Some("u1".into());
}
