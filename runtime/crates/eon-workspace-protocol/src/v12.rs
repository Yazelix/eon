//! EONW v12 adds a request that moves the selected pane into the neighbouring
//! column.
//!
//! Snapshots, failures, lifecycle results and every older request keep their
//! v11 bytes under the v12 version.

pub use super::v11::*;
use super::{v2, v6, v11};

pub const VERSION: u16 = 12;
const MOVE_PANE_LEFT: u8 = 24;
const MOVE_PANE_RIGHT: u8 = 25;

/// The column a sideways pane move goes towards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

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
    /// Move the selected pane into the neighbouring column on `Side`.
    MovePaneSideways(Side),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: Action,
}

/// Applies the v11 popup rules; `MovePaneSideways` names no popup.
pub fn check_popup_action(snapshot: &Snapshot, action: &Action) -> Result<()> {
    match v11_action(action) {
        Some(action) => v11::check_popup_action(snapshot, &action),
        None => Ok(()),
    }
}

fn v11_action(action: &Action) -> Option<v11::Action> {
    Some(match action.clone() {
        Action::Workspace(action) => v11::Action::Workspace(action),
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        } => v11::Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        },
        Action::DismissPopup(target) => v11::Action::DismissPopup(target),
        Action::CommitDirectory { target, directory } => {
            v11::Action::CommitDirectory { target, directory }
        }
        Action::CreateColumn => v11::Action::CreateColumn,
        Action::ClosePane { pane } => v11::Action::ClosePane { pane },
        Action::MovePaneSideways(_) => return None,
    })
}

/// Rewrites a v11 message as v12 after encoding.
fn as_v12(mut bytes: Vec<u8>) -> Vec<u8> {
    v6::set_version(&mut bytes, VERSION);
    bytes
}

/// Checks the v12 header, then hands the bytes to the v11 decoder.
fn as_v11(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = v6::reframe(bytes, VERSION)?;
    v6::set_version(&mut bytes, v11::VERSION);
    Ok(bytes)
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    v11::encode_response(response).map(as_v12)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    v11::decode_response(&as_v11(bytes)?)
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    let Some(action) = v11_action(&request.action) else {
        let Action::MovePaneSideways(side) = request.action else {
            unreachable!("only MovePaneSideways has no v11 form");
        };
        v2::identity("request id", &request.id)?;
        let mut payload = v2::Encoder::default();
        payload.string(&request.id);
        payload.byte(match side {
            Side::Left => MOVE_PANE_LEFT,
            Side::Right => MOVE_PANE_RIGHT,
        });
        return v2::frame_version(VERSION, v2::REQUEST, payload.bytes);
    };
    v11::encode_request(&v11::Request {
        id: request.id.clone(),
        action,
    })
    .map(as_v12)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let (kind, payload) = v2::unframe_version(bytes, VERSION)?;
    if kind == v2::REQUEST {
        let mut decoder = v2::Decoder::new(payload);
        let id = decoder.string("request id", v2::MAX_ID_BYTES)?;
        let side = match decoder.byte()? {
            MOVE_PANE_LEFT => Some(Side::Left),
            MOVE_PANE_RIGHT => Some(Side::Right),
            _ => None,
        };
        if let Some(side) = side {
            decoder.finish()?;
            v2::identity("request id", &id)?;
            return Ok(Request {
                id,
                action: Action::MovePaneSideways(side),
            });
        }
    }
    let request = v11::decode_request(&as_v11(bytes)?)?;
    Ok(Request {
        id: request.id,
        action: match request.action {
            v11::Action::Workspace(action) => Action::Workspace(action),
            v11::Action::InvokePopup {
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
            v11::Action::DismissPopup(target) => Action::DismissPopup(target),
            v11::Action::CommitDirectory { target, directory } => {
                Action::CommitDirectory { target, directory }
            }
            v11::Action::CreateColumn => Action::CreateColumn,
            v11::Action::ClosePane { pane } => Action::ClosePane { pane },
        },
    })
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    v11::encode_lifecycle_response(response).map(as_v12)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    v11::decode_lifecycle_response(&as_v11(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sideways_pane_moves_reject_malformed_requests() {
        let overlong = "r".repeat(v2::MAX_ID_BYTES + 1);
        for (side, tag) in [(Side::Left, MOVE_PANE_LEFT), (Side::Right, MOVE_PANE_RIGHT)] {
            for id in ["", "r 1", overlong.as_str()] {
                assert!(
                    encode_request(&Request {
                        id: id.into(),
                        action: Action::MovePaneSideways(side),
                    })
                    .is_err()
                );
                let mut payload = v2::Encoder::default();
                payload.string(id);
                payload.byte(tag);
                let bytes = v2::frame_version(VERSION, v2::REQUEST, payload.bytes).unwrap();
                assert!(decode_request(&bytes).is_err());
            }

            let mut payload = v2::Encoder::default();
            payload.string("r1");
            let truncated = v2::frame_version(VERSION, v2::REQUEST, payload.bytes.clone()).unwrap();
            assert_eq!(decode_request(&truncated), Err(Error::Truncated));
            payload.byte(tag);
            payload.byte(0);
            let trailing = v2::frame_version(VERSION, v2::REQUEST, payload.bytes).unwrap();
            assert_eq!(
                decode_request(&trailing),
                Err(Error::TrailingBytes { count: 1 })
            );
        }
    }

    #[test]
    fn sideways_pane_moves_are_v12_requests_and_older_traffic_keeps_v11_bytes() {
        for (action, previous) in [
            (Action::MovePaneSideways(Side::Left), None),
            (Action::MovePaneSideways(Side::Right), None),
            (
                Action::ClosePane { pane: "p2".into() },
                Some(v11::Action::ClosePane { pane: "p2".into() }),
            ),
            (
                Action::Workspace(WorkspaceAction::Move(Direction::Left)),
                Some(v11::Action::Workspace(WorkspaceAction::Move(
                    Direction::Left,
                ))),
            ),
        ] {
            let request = Request {
                id: "r1".into(),
                action,
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
            assert!(v11::decode_request(&bytes).is_err());
            if let Some(action) = previous {
                let mut expected = v11::encode_request(&v11::Request {
                    id: request.id,
                    action,
                })
                .unwrap();
                v6::set_version(&mut expected, VERSION);
                assert_eq!(bytes, expected);
            }
        }
        let v11_bytes = v11::encode_request(&v11::Request {
            id: "r1".into(),
            action: v11::Action::ClosePane { pane: "p2".into() },
        })
        .unwrap();
        assert!(decode_request(&v11_bytes).is_err());

        let failure = Response::Failure(Failure {
            code: "unavailable".into(),
            detail: "no".into(),
        });
        let bytes = encode_response(&failure).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), failure);
        assert!(v11::decode_response(&bytes).is_err());
        let mut expected = v11::encode_response(&failure).unwrap();
        v6::set_version(&mut expected, VERSION);
        assert_eq!(bytes, expected);
    }
}
