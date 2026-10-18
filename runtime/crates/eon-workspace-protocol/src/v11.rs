//! EONW v11 adds a request that closes one pane by its stable identity.
//!
//! Snapshots, failures, lifecycle results and every older request keep their
//! v10 bytes under the v11 version.

pub use super::v10::*;
use super::{v2, v6, v10};

pub const VERSION: u16 = 11;
const CLOSE_PANE: u8 = 23;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Workspace(WorkspaceAction),
    InvokePopup {
        tab: String,
        entry: String,
        /// The tab-scoped instance the requester last observed.
        expected_instance: Option<String>,
        intent: InvokeIntent,
    },
    DismissPopup(PopupTarget),
    CommitDirectory {
        target: PopupTarget,
        directory: Vec<u8>,
    },
    /// Add a column right of the selected column with one new pane.
    CreateColumn,
    /// Stop the terminal of one pane in the active tab.
    ClosePane {
        pane: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: Action,
}

/// Applies the v10 popup rules; `ClosePane` names no popup.
pub fn check_popup_action(snapshot: &Snapshot, action: &Action) -> Result<()> {
    match v10_action(action) {
        Some(action) => snapshot.check_popup_action(&action),
        None => Ok(()),
    }
}

fn v10_action(action: &Action) -> Option<v10::Action> {
    Some(match action.clone() {
        Action::Workspace(action) => v10::Action::Workspace(action),
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        } => v10::Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        },
        Action::DismissPopup(target) => v10::Action::DismissPopup(target),
        Action::CommitDirectory { target, directory } => {
            v10::Action::CommitDirectory { target, directory }
        }
        Action::CreateColumn => v10::Action::CreateColumn,
        Action::ClosePane { .. } => return None,
    })
}

fn from_v10_action(action: v10::Action) -> Action {
    match action {
        v10::Action::Workspace(action) => Action::Workspace(action),
        v10::Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        } => Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        },
        v10::Action::DismissPopup(target) => Action::DismissPopup(target),
        v10::Action::CommitDirectory { target, directory } => {
            Action::CommitDirectory { target, directory }
        }
        v10::Action::CreateColumn => Action::CreateColumn,
    }
}

/// Rewrites a v10 message as v11 after encoding.
fn as_v11(mut bytes: Vec<u8>) -> Vec<u8> {
    v6::set_version(&mut bytes, VERSION);
    bytes
}

/// Checks the v11 header, then hands the bytes to the v10 decoder.
fn as_v10(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = v6::reframe(bytes, VERSION)?;
    v6::set_version(&mut bytes, v10::VERSION);
    Ok(bytes)
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    v10::encode_response(response).map(as_v11)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    v10::decode_response(&as_v10(bytes)?)
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    let Some(action) = v10_action(&request.action) else {
        let Action::ClosePane { pane } = &request.action else {
            unreachable!("only ClosePane has no v10 form");
        };
        v2::identity("request id", &request.id)?;
        v2::identity("pane id", pane)?;
        let mut payload = v2::Encoder::default();
        payload.string(&request.id);
        payload.byte(CLOSE_PANE);
        payload.string(pane);
        return v2::frame_version(VERSION, v2::REQUEST, payload.bytes);
    };
    v10::encode_request(&v10::Request {
        id: request.id.clone(),
        action,
    })
    .map(as_v11)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let (kind, payload) = v2::unframe_version(bytes, VERSION)?;
    if kind == v2::REQUEST {
        let mut decoder = v2::Decoder::new(payload);
        let id = decoder.string("request id", v2::MAX_ID_BYTES)?;
        if decoder.byte()? == CLOSE_PANE {
            let pane = decoder.string("pane id", v2::MAX_ID_BYTES)?;
            decoder.finish()?;
            v2::identity("request id", &id)?;
            v2::identity("pane id", &pane)?;
            return Ok(Request {
                id,
                action: Action::ClosePane { pane },
            });
        }
    }
    let request = v10::decode_request(&as_v10(bytes)?)?;
    Ok(Request {
        id: request.id,
        action: from_v10_action(request.action),
    })
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    v10::encode_lifecycle_response(response).map(as_v11)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    v10::decode_lifecycle_response(&as_v10(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_pane_is_a_v11_request_and_older_traffic_keeps_v10_bytes() {
        for action in [
            Action::ClosePane { pane: "p2".into() },
            Action::CreateColumn,
            Action::Workspace(WorkspaceAction::CloseTab { tab: "t1".into() }),
        ] {
            let request = Request {
                id: "r1".into(),
                action,
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
            assert!(v10::decode_request(&bytes).is_err());
        }
        let v10_bytes = v10::encode_request(&v10::Request {
            id: "r1".into(),
            action: v10::Action::CreateColumn,
        })
        .unwrap();
        assert!(decode_request(&v10_bytes).is_err());

        for pane in ["", "p 2"] {
            let request = Request {
                id: "r1".into(),
                action: Action::ClosePane { pane: pane.into() },
            };
            assert!(encode_request(&request).is_err());
        }

        let failure = Response::Failure(Failure {
            code: "unavailable".into(),
            detail: "no".into(),
        });
        let bytes = encode_response(&failure).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), failure);
        assert!(v10::decode_response(&bytes).is_err());
    }
}
