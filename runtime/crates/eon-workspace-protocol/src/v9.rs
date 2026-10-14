//! EONW v9 preserves quota windows whose providers do not supply reset times.

pub use super::v8::*;
use super::{v2, v5, v6, v7};
pub const VERSION: u16 = 9;

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

fn validate_quota(quota: Option<&CodexQuota>) -> Result<()> {
    let mut quota = quota.cloned();
    if let Some(quota) = quota.as_mut() {
        // v9 freshness does not require the provider to report a reset.
        if quota.state == CodexQuotaState::Stale {
            quota.state = CodexQuotaState::Fresh;
        }
    }
    v6::validate_quota(quota.as_ref())
}

fn validate_claude(quota: Option<&CodexQuota>) -> Result<()> {
    validate_quota(quota)?;
    let Some(quota) = quota else { return Ok(()) };
    if !matches!(quota.state, CodexQuotaState::Fresh | CodexQuotaState::Stale)
        || quota.windows.iter().enumerate().any(|(index, window)| {
            !matches!(window.duration_minutes, 300 | 10080)
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
            validate_quota(snapshot.codex_quota.as_ref())?;
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
            validate_quota(codex_quota.as_ref())?;
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
    fn optional_resets_roundtrip_without_relaxing_historical_or_value_boundaries() {
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
                state: CodexQuotaState::Stale,
                observed_at: 100,
                windows: vec![QuotaWindow {
                    duration_minutes: 10080,
                    remaining_percent: 89,
                    resets_at: None,
                }],
            }),
            claude_quota: Some(ClaudeQuota {
                state: ClaudeQuotaState::Fresh,
                observed_at: 100,
                windows: vec![QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 100,
                    resets_at: None,
                }],
            }),
        };
        let response = Response::Snapshot(snapshot.clone());
        let bytes = encode_response(&response).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), response);
        assert!(super::super::v8::encode_response(&response).is_err());
        assert!(super::super::v8::decode_response(&bytes).is_err());
        for change in [
            |q: &mut ClaudeQuota| q.windows.clear(),
            |q: &mut ClaudeQuota| q.windows[0].duration_minutes = 60,
            |q: &mut ClaudeQuota| q.windows[0].remaining_percent = 101,
            |q: &mut ClaudeQuota| q.windows[0].resets_at = Some(q.observed_at),
            |q: &mut ClaudeQuota| q.windows.push(q.windows[0].clone()),
        ] {
            let mut invalid = snapshot.clone();
            change(invalid.claude_quota.as_mut().unwrap());
            assert!(encode_response(&Response::Snapshot(invalid)).is_err());
        }
        assert!(decode_response(&bytes[..bytes.len() - 1]).is_err());
        let request = Request {
            id: "create".into(),
            action: Action::Workspace(WorkspaceAction::CreateTab),
        };
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
    }
}
