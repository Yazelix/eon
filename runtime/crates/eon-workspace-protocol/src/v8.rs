//! EONW v8 adds bounded, presentation-safe Claude quota facts.

use super::{v2, v5, v6, v7};

pub use super::v6::CodexQuotaWindow as QuotaWindow;
pub use super::v7::{
    ALT, Action, Availability, CTRL, CodexQuota, CodexQuotaState, CodexQuotaWindow, Direction,
    Error, Failure, HEADER_BYTES, InvokeIntent, LifecycleResponse, MAX_CODEX_QUOTA_WINDOWS,
    MAX_DETAIL_BYTES, MAX_DIRECTORY_BYTES, MAX_ENTRIES, MAX_ENTRY_ID_BYTES, MAX_KEY_BYTES,
    MAX_LABEL_BYTES, MAX_SESSIONS, MAX_TABS, PROJECT_ENTRY, Pane, Popup, PopupEntry, PopupGeometry,
    PopupTarget, Request, Runtime, SHIFT, SUPER, Shortcut, Stopped, Tab, WorkspaceAction,
    declared_message_len,
};

pub const VERSION: u16 = 8;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeQuotaState {
    Fresh,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeQuota {
    pub state: ClaudeQuotaState,
    pub observed_at: u64,
    pub windows: Vec<QuotaWindow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub active_tab: String,
    pub geometry: PopupGeometry,
    pub entries: Vec<PopupEntry>,
    pub tabs: Vec<Tab>,
    pub codex_quota: Option<CodexQuota>,
    pub claude_quota: Option<ClaudeQuota>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Snapshot(Snapshot),
    Failure(Failure),
}

impl Snapshot {
    pub fn check_popup_action(&self, action: &Action) -> Result<()> {
        v5::check_popup_action(&self.entries, &self.tabs, action)
    }
}

fn as_codex(quota: &ClaudeQuota) -> CodexQuota {
    CodexQuota {
        state: match quota.state {
            ClaudeQuotaState::Fresh => CodexQuotaState::Fresh,
            ClaudeQuotaState::Stale => CodexQuotaState::Stale,
        },
        observed_at: quota.observed_at,
        windows: quota.windows.clone(),
    }
}

fn validate_claude(quota: Option<&CodexQuota>) -> Result<()> {
    v6::validate_quota(quota)?;
    let Some(quota) = quota else { return Ok(()) };
    if !matches!(quota.state, CodexQuotaState::Fresh | CodexQuotaState::Stale)
        || quota.windows.iter().enumerate().any(|(index, window)| {
            !matches!(window.duration_minutes, 300 | 10080)
                || window.resets_at.is_none()
                || quota.windows[..index]
                    .iter()
                    .any(|previous| previous.duration_minutes == window.duration_minutes)
        })
    {
        return Err(Error::InvalidSnapshot {
            field: "Claude quota",
        });
    }
    Ok(())
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    let mut payload = v2::Encoder::default();
    let kind = match response {
        Response::Snapshot(snapshot) => {
            let workspace = v5::Snapshot {
                active_tab: snapshot.active_tab.clone(),
                geometry: snapshot.geometry,
                entries: snapshot.entries.clone(),
                tabs: snapshot.tabs.clone(),
            };
            let claude = snapshot.claude_quota.as_ref().map(as_codex);
            v5::validate_snapshot_with_pending(&workspace, true)?;
            v6::validate_quota(snapshot.codex_quota.as_ref())?;
            validate_claude(claude.as_ref())?;
            v5::encode_workspace(&mut payload, &workspace);
            v6::encode_quota(&mut payload, snapshot.codex_quota.as_ref());
            v6::encode_quota(&mut payload, claude.as_ref());
            v2::SNAPSHOT
        }
        Response::Failure(failure) => {
            v2::encode_failure(&mut payload, failure)?;
            v2::FAILURE
        }
    };
    v2::frame_version(VERSION, kind, payload.bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    let (kind, payload) = v2::unframe_version(bytes, VERSION)?;
    let mut decoder = v2::Decoder::new(payload);
    let response = match kind {
        v2::SNAPSHOT => {
            let workspace = v5::decode_workspace(&mut decoder)?;
            let codex_quota = v6::decode_quota(&mut decoder)?;
            let claude = v6::decode_quota(&mut decoder)?;
            v5::validate_snapshot_with_pending(&workspace, true)?;
            v6::validate_quota(codex_quota.as_ref())?;
            validate_claude(claude.as_ref())?;
            let claude_quota = claude.map(|quota| ClaudeQuota {
                state: if quota.state == CodexQuotaState::Stale {
                    ClaudeQuotaState::Stale
                } else {
                    ClaudeQuotaState::Fresh
                },
                observed_at: quota.observed_at,
                windows: quota.windows,
            });
            Response::Snapshot(Snapshot {
                active_tab: workspace.active_tab,
                geometry: workspace.geometry,
                entries: workspace.entries,
                tabs: workspace.tabs,
                codex_quota,
                claude_quota,
            })
        }
        v2::FAILURE => Response::Failure(v2::decode_failure(&mut decoder)?),
        value => return Err(Error::InvalidKind { value }),
    };
    decoder.finish()?;
    Ok(response)
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    let mut bytes = v7::encode_request(request)?;
    v6::set_version(&mut bytes, VERSION);
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let mut bytes = v6::reframe(bytes, VERSION)?;
    v6::set_version(&mut bytes, v7::VERSION);
    v7::decode_request(&bytes)
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    let mut bytes = v7::encode_lifecycle_response(response)?;
    v6::set_version(&mut bytes, VERSION);
    Ok(bytes)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    let mut bytes = v6::reframe(bytes, VERSION)?;
    v6::set_version(&mut bytes, v7::VERSION);
    v7::decode_lifecycle_response(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_providers_roundtrip_with_historical_and_malformed_boundaries() {
        let snapshot = Snapshot {
            active_tab: "t1".into(),
            geometry: PopupGeometry {
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
                panes: vec![Pane {
                    id: "p1".into(),
                    session: "s1".into(),
                    endpoint: b"/run/eon/orbit.sock".to_vec(),
                    live: true,
                }],
                popups: vec![],
            }],
            codex_quota: Some(CodexQuota {
                state: CodexQuotaState::Blocked,
                observed_at: 100,
                windows: vec![],
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Fresh,
                observed_at: 100,
                windows: vec![QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 98,
                    resets_at: Some(200),
                }],
            }),
        };
        for present in [false, true] {
            let mut candidate = snapshot.clone();
            if !present {
                candidate.claude_quota = None;
            }
            let response = Response::Snapshot(candidate);
            let bytes = encode_response(&response).unwrap();
            assert_eq!(decode_response(&bytes).unwrap(), response);
            assert!(v7::decode_response(&bytes).is_err());
            assert!(decode_response(&bytes[..bytes.len() - 1]).is_err());
            let mut extra = bytes;
            extra.push(0);
            assert!(decode_response(&extra).is_err());
        }
        for change in [
            |q: &mut ClaudeQuota| q.windows.clear(),
            |q: &mut ClaudeQuota| q.windows[0].duration_minutes = 60,
            |q: &mut ClaudeQuota| q.windows[0].remaining_percent = 101,
            |q: &mut ClaudeQuota| q.windows[0].resets_at = None,
            |q: &mut ClaudeQuota| q.windows[0].resets_at = Some(q.observed_at),
            |q: &mut ClaudeQuota| q.windows.push(q.windows[0].clone()),
        ] {
            let mut invalid = snapshot.clone();
            change(invalid.claude_quota.as_mut().unwrap());
            assert!(encode_response(&Response::Snapshot(invalid)).is_err());
        }
        let old = v7::Response::Snapshot(v7::Snapshot {
            active_tab: snapshot.active_tab,
            geometry: snapshot.geometry,
            entries: snapshot.entries,
            tabs: snapshot.tabs,
            codex_quota: snapshot.codex_quota,
        });
        let old_bytes = v7::encode_response(&old).unwrap();
        assert_eq!(v7::decode_response(&old_bytes).unwrap(), old);
        assert!(decode_response(&old_bytes).is_err());
        let request = Request {
            id: "create".into(),
            action: Action::Workspace(WorkspaceAction::CreateTab),
        };
        let bytes = encode_request(&request).unwrap();
        assert_eq!(decode_request(&bytes).unwrap(), request);
        assert!(v7::decode_request(&bytes).is_err());
    }
}
