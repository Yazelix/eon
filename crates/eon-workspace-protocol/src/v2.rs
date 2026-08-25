//! Unactivated EONW v2 codec surface for the exact Venus consumer.

use std::collections::HashSet;

pub use super::{
    Availability, Direction, Error, Failure, HEADER_BYTES, LifecycleResponse, MAX_DETAIL_BYTES,
    MAX_PANES, MAX_TABS, Pane, Result, Runtime, Stopped, declared_message_len,
};

pub const VERSION: u16 = 2;
pub const MAX_DIRECTORY_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Inspect,
    InspectRuntime,
    InspectPresentation,
    Present { workspace: bool },
    CreateTab,
    CreatePane,
    FocusId(String),
    Focus(Direction),
    Stop { generation: String },
    SetTabDirectory { tab: String, directory: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub directory: Vec<u8>,
    pub selected_pane: String,
    pub panes: Vec<Pane>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub active_tab: String,
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Snapshot(Snapshot),
    Failure(Failure),
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    super::identity("request id", &request.id)?;
    let mut payload = super::Encoder::default();
    payload.string(&request.id);
    match &request.action {
        Action::Inspect => payload.byte(0),
        Action::CreateTab => payload.byte(1),
        Action::CreatePane => payload.byte(2),
        Action::FocusId(id) => {
            super::nonempty("focus id", id, super::MAX_ID_BYTES)?;
            payload.byte(3);
            payload.string(id);
        }
        Action::Focus(Direction::Left) => payload.byte(4),
        Action::Focus(Direction::Right) => payload.byte(5),
        Action::Focus(Direction::Up) => payload.byte(6),
        Action::Focus(Direction::Down) => payload.byte(7),
        Action::InspectRuntime => payload.byte(8),
        Action::Stop { generation } => {
            super::identity("generation", generation)?;
            payload.byte(9);
            payload.string(generation);
        }
        Action::Present { workspace } => {
            payload.byte(10);
            payload.byte(u8::from(*workspace));
        }
        Action::InspectPresentation => payload.byte(11),
        Action::SetTabDirectory { tab, directory } => {
            super::identity("tab id", tab)?;
            validate_directory(directory)?;
            payload.byte(12);
            payload.string(tab);
            payload.raw(directory);
        }
    }
    frame(super::REQUEST, payload.bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let (kind, payload) = unframe(bytes)?;
    if kind != super::REQUEST {
        return Err(Error::InvalidKind { value: kind });
    }
    let mut decoder = super::Decoder::new(payload);
    let id = decoder.string("request id", super::MAX_ID_BYTES)?;
    super::identity("request id", &id)?;
    let action = match decoder.byte()? {
        0 => Action::Inspect,
        1 => Action::CreateTab,
        2 => Action::CreatePane,
        3 => {
            let id = decoder.string("focus id", super::MAX_ID_BYTES)?;
            super::nonempty("focus id", &id, super::MAX_ID_BYTES)?;
            Action::FocusId(id)
        }
        4 => Action::Focus(Direction::Left),
        5 => Action::Focus(Direction::Right),
        6 => Action::Focus(Direction::Up),
        7 => Action::Focus(Direction::Down),
        8 => Action::InspectRuntime,
        9 => {
            let generation = decoder.string("generation", super::MAX_ID_BYTES)?;
            super::identity("generation", &generation)?;
            Action::Stop { generation }
        }
        10 => Action::Present {
            workspace: match decoder.byte()? {
                0 => false,
                1 => true,
                _ => {
                    return Err(Error::InvalidValue {
                        field: "presentation mode",
                    });
                }
            },
        },
        11 => Action::InspectPresentation,
        12 => {
            let tab = decoder.string("tab id", super::MAX_ID_BYTES)?;
            super::identity("tab id", &tab)?;
            let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
            validate_directory(&directory)?;
            Action::SetTabDirectory { tab, directory }
        }
        value => {
            return Err(Error::InvalidTag {
                field: "action",
                value,
            });
        }
    };
    decoder.finish()?;
    Ok(Request { id, action })
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    let mut payload = super::Encoder::default();
    let kind = match response {
        Response::Snapshot(snapshot) => {
            validate_snapshot(snapshot)?;
            payload.string(&snapshot.active_tab);
            payload.count(snapshot.tabs.len());
            for tab in &snapshot.tabs {
                payload.string(&tab.id);
                payload.raw(&tab.directory);
                payload.string(&tab.selected_pane);
                payload.count(tab.panes.len());
                for pane in &tab.panes {
                    payload.string(&pane.id);
                    payload.string(&pane.session);
                    payload.raw(&pane.endpoint);
                    payload.byte(u8::from(pane.live));
                }
            }
            super::SNAPSHOT
        }
        Response::Failure(failure) => {
            super::identity("failure code", &failure.code)?;
            super::nonempty("failure detail", &failure.detail, MAX_DETAIL_BYTES)?;
            payload.string(&failure.code);
            payload.string(&failure.detail);
            super::FAILURE
        }
    };
    frame(kind, payload.bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    let (kind, payload) = unframe(bytes)?;
    let mut decoder = super::Decoder::new(payload);
    let response = match kind {
        super::SNAPSHOT => {
            let active_tab = decoder.string("active tab", super::MAX_ID_BYTES)?;
            let tab_count = decoder.count("tabs", MAX_TABS)?;
            let mut tabs = Vec::with_capacity(tab_count);
            let mut pane_count = 0;
            for _ in 0..tab_count {
                let id = decoder.string("tab id", super::MAX_ID_BYTES)?;
                let directory = decoder.raw("directory", MAX_DIRECTORY_BYTES)?.to_vec();
                let selected_pane = decoder.string("selected pane", super::MAX_ID_BYTES)?;
                let count = decoder.count("panes", MAX_PANES)?;
                pane_count += count;
                if pane_count > MAX_PANES {
                    return Err(Error::InvalidSnapshot { field: "panes" });
                }
                let mut panes = Vec::with_capacity(count);
                for _ in 0..count {
                    panes.push(Pane {
                        id: decoder.string("pane id", super::MAX_ID_BYTES)?,
                        session: decoder.string("session id", super::MAX_ID_BYTES)?,
                        endpoint: decoder.raw("endpoint", super::MAX_ENDPOINT_BYTES)?.to_vec(),
                        live: match decoder.byte()? {
                            0 => false,
                            1 => true,
                            _ => return Err(Error::InvalidValue { field: "liveness" }),
                        },
                    });
                }
                tabs.push(Tab {
                    id,
                    directory,
                    selected_pane,
                    panes,
                });
            }
            let snapshot = Snapshot { active_tab, tabs };
            validate_snapshot(&snapshot)?;
            Response::Snapshot(snapshot)
        }
        super::FAILURE => {
            let code = decoder.string("failure code", super::MAX_ID_BYTES)?;
            super::identity("failure code", &code)?;
            let detail = decoder.string("failure detail", MAX_DETAIL_BYTES)?;
            super::nonempty("failure detail", &detail, MAX_DETAIL_BYTES)?;
            Response::Failure(Failure { code, detail })
        }
        value => return Err(Error::InvalidKind { value }),
    };
    decoder.finish()?;
    Ok(response)
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    let mut encoded = super::encode_lifecycle_response(response)?;
    encoded[4..6].copy_from_slice(&VERSION.to_le_bytes());
    Ok(encoded)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    unframe(bytes)?;
    let mut encoded = bytes.to_vec();
    encoded[4..6].copy_from_slice(&super::VERSION.to_le_bytes());
    super::decode_lifecycle_response(&encoded)
}

fn validate_directory(directory: &[u8]) -> Result<()> {
    if directory.is_empty() || directory.contains(&0) {
        return Err(Error::InvalidValue { field: "directory" });
    }
    if directory.len() > MAX_DIRECTORY_BYTES {
        return Err(Error::FieldTooLong {
            field: "directory",
            length: directory.len(),
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<()> {
    if snapshot.tabs.is_empty() || snapshot.tabs.len() > MAX_TABS {
        return Err(Error::InvalidSnapshot { field: "tabs" });
    }
    super::identity("active tab", &snapshot.active_tab)?;

    let mut focus_ids = HashSet::new();
    let mut sessions = HashSet::new();
    let mut endpoints = HashSet::new();
    let mut pane_count = 0;
    let mut active_tab_present = false;
    for tab in &snapshot.tabs {
        super::identity("tab id", &tab.id)?;
        validate_directory(&tab.directory)?;
        super::identity("selected pane", &tab.selected_pane)?;
        if !focus_ids.insert(tab.id.as_str()) {
            return Err(Error::InvalidSnapshot { field: "tab id" });
        }
        active_tab_present |= tab.id == snapshot.active_tab;
        if tab.panes.is_empty() {
            return Err(Error::InvalidSnapshot { field: "panes" });
        }
        pane_count += tab.panes.len();
        if pane_count > MAX_PANES {
            return Err(Error::InvalidSnapshot { field: "panes" });
        }
        let mut selected = false;
        for pane in &tab.panes {
            super::identity("pane id", &pane.id)?;
            super::identity("session id", &pane.session)?;
            if !focus_ids.insert(pane.id.as_str()) {
                return Err(Error::InvalidSnapshot { field: "pane id" });
            }
            if !sessions.insert(pane.session.as_str()) {
                return Err(Error::InvalidSnapshot {
                    field: "session id",
                });
            }
            if pane.endpoint.is_empty() {
                return Err(Error::InvalidSnapshot { field: "endpoint" });
            }
            if pane.endpoint.len() > super::MAX_ENDPOINT_BYTES {
                return Err(Error::FieldTooLong {
                    field: "endpoint",
                    length: pane.endpoint.len(),
                });
            }
            if !endpoints.insert(pane.endpoint.as_slice()) {
                return Err(Error::InvalidSnapshot { field: "endpoint" });
            }
            selected |= pane.id == tab.selected_pane;
        }
        if !selected {
            return Err(Error::InvalidSnapshot {
                field: "selected_pane",
            });
        }
    }
    if !active_tab_present {
        return Err(Error::InvalidSnapshot {
            field: "active_tab",
        });
    }
    Ok(())
}

fn frame(kind: u8, payload: Vec<u8>) -> Result<Vec<u8>> {
    let size = HEADER_BYTES + payload.len();
    if size > super::MAX_MESSAGE_BYTES {
        return Err(Error::MessageTooLarge { size });
    }
    let mut message = Vec::with_capacity(size);
    message.extend_from_slice(super::MAGIC);
    message.extend_from_slice(&VERSION.to_le_bytes());
    message.push(kind);
    message.push(0);
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&payload);
    Ok(message)
}

fn unframe(bytes: &[u8]) -> Result<(u8, &[u8])> {
    if bytes.len() > super::MAX_MESSAGE_BYTES {
        return Err(Error::MessageTooLarge { size: bytes.len() });
    }
    if bytes.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    if &bytes[..4] != super::MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(Error::UnsupportedVersion { version });
    }
    if bytes[7] != 0 {
        return Err(Error::InvalidFlags { value: bytes[7] });
    }
    let declared = declared_message_len(bytes)?;
    if bytes.len() < declared {
        return Err(Error::Truncated);
    }
    if bytes.len() > declared {
        return Err(Error::TrailingBytes {
            count: bytes.len() - declared,
        });
    }
    Ok((bytes[6], &bytes[HEADER_BYTES..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            active_tab: "t1".into(),
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/tmp/eon-\xff".to_vec(),
                selected_pane: "p1".into(),
                panes: vec![Pane {
                    id: "p1".into(),
                    session: "session-1".into(),
                    endpoint: b"/run/eon/orbit.sock".to_vec(),
                    live: true,
                }],
            }],
        }
    }

    #[test]
    fn round_trips_raw_directories_and_rejects_v1_and_invalid_paths() {
        let request = Request {
            id: "client-1".into(),
            action: Action::SetTabDirectory {
                tab: "t1".into(),
                directory: b"/tmp/eon-\xff".to_vec(),
            },
        };
        let encoded_request = encode_request(&request).unwrap();
        assert_eq!(decode_request(&encoded_request).unwrap(), request);
        assert_eq!(
            super::super::decode_request(&encoded_request),
            Err(Error::UnsupportedVersion { version: VERSION })
        );

        let response = Response::Snapshot(snapshot());
        let encoded_response = encode_response(&response).unwrap();
        assert_eq!(decode_response(&encoded_response).unwrap(), response);
        let v1 = super::super::encode_request(&super::super::Request {
            id: "client-1".into(),
            action: super::super::Action::Inspect,
        })
        .unwrap();
        assert_eq!(
            decode_request(&v1),
            Err(Error::UnsupportedVersion {
                version: super::super::VERSION,
            })
        );

        for directory in [Vec::new(), b"/tmp/eon\0bad".to_vec()] {
            let invalid = Request {
                id: "client-1".into(),
                action: Action::SetTabDirectory {
                    tab: "t1".into(),
                    directory,
                },
            };
            assert_eq!(
                encode_request(&invalid),
                Err(Error::InvalidValue { field: "directory" })
            );
        }

        let mut oversized = snapshot();
        oversized.tabs[0].directory = vec![b'x'; MAX_DIRECTORY_BYTES + 1];
        assert_eq!(
            encode_response(&Response::Snapshot(oversized)),
            Err(Error::FieldTooLong {
                field: "directory",
                length: MAX_DIRECTORY_BYTES + 1,
            })
        );
    }
}
