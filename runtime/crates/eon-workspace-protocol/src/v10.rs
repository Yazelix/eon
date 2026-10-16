//! EONW v10 arranges each tab's panes into one to four side-by-side columns.
//!
//! A snapshot carries each tab's column layout ahead of the unchanged v9
//! snapshot of its flattened panes, so every v9 rule still applies.

pub use super::v9::*;
use super::{v2, v6, v9};

pub const VERSION: u16 = 10;
pub const MAX_COLUMNS: usize = 4;
const CREATE_COLUMN: u8 = 22;

/// One accordion of panes; `expanded` names the pane it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub panes: Vec<Pane>,
    pub expanded: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: String,
    pub directory: Vec<u8>,
    pub pending: bool,
    /// The focused pane: one column's expanded pane.
    pub selected_pane: Option<String>,
    /// Retained when another tab is active; not native keyboard-focus state.
    pub selected_popup: Option<String>,
    pub columns: Vec<Column>,
    pub popups: Vec<Popup>,
}

impl Tab {
    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.columns.iter().flat_map(|column| &column.panes)
    }
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

impl Snapshot {
    pub fn check_popup_action(&self, action: &Action) -> Result<()> {
        match action {
            Action::CreateColumn => Ok(()),
            action => flatten(self).check_popup_action(&v9_action(action)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Snapshot(Snapshot),
    Failure(Failure),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: Action,
}

fn v9_action(action: &Action) -> v9::Action {
    match action.clone() {
        Action::Workspace(action) => v9::Action::Workspace(action),
        Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        } => v9::Action::InvokePopup {
            tab,
            entry,
            expected_instance,
            intent,
        },
        Action::DismissPopup(target) => v9::Action::DismissPopup(target),
        Action::CommitDirectory { target, directory } => {
            v9::Action::CommitDirectory { target, directory }
        }
        Action::CreateColumn => unreachable!("CreateColumn has no v9 form"),
    }
}

fn from_v9_action(action: v9::Action) -> Action {
    match action {
        v9::Action::Workspace(action) => Action::Workspace(action),
        v9::Action::InvokePopup {
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
        v9::Action::DismissPopup(target) => Action::DismissPopup(target),
        v9::Action::CommitDirectory { target, directory } => {
            Action::CommitDirectory { target, directory }
        }
    }
}

fn flatten(snapshot: &Snapshot) -> v9::Snapshot {
    v9::Snapshot {
        active_tab: snapshot.active_tab.clone(),
        geometry: snapshot.geometry,
        entries: snapshot.entries.clone(),
        tabs: snapshot
            .tabs
            .iter()
            .map(|tab| v9::Tab {
                id: tab.id.clone(),
                directory: tab.directory.clone(),
                pending: tab.pending,
                selected_pane: tab.selected_pane.clone(),
                selected_popup: tab.selected_popup.clone(),
                panes: tab.panes().cloned().collect(),
                popups: tab.popups.clone(),
            })
            .collect(),
        codex_quota: snapshot.codex_quota.clone(),
        claude_quota: snapshot.claude_quota.clone(),
    }
}

/// Checks the column rules; v9 validation of the flattened panes covers the rest.
fn validate_columns(tab: &Tab) -> Result<()> {
    let invalid = |field| Err(Error::InvalidSnapshot { field });
    if tab.columns.len() > MAX_COLUMNS {
        return invalid("columns");
    }
    for column in &tab.columns {
        if column.panes.is_empty() {
            return invalid("empty column");
        }
        if !column.panes.iter().any(|pane| pane.id == column.expanded) {
            return invalid("expanded pane");
        }
    }
    if tab.selected_pane.as_ref().is_some_and(|selected| {
        !tab.columns
            .iter()
            .any(|column| column.expanded == *selected)
    }) {
        return invalid("selected pane");
    }
    Ok(())
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
    let snapshot = match response {
        Response::Snapshot(snapshot) => snapshot,
        Response::Failure(failure) => {
            let mut bytes = v9::encode_response(&v9::Response::Failure(failure.clone()))?;
            v6::set_version(&mut bytes, VERSION);
            return Ok(bytes);
        }
    };
    let flat = v9::encode_response(&v9::Response::Snapshot(flatten(snapshot)))?;
    let mut layout = v2::Encoder::default();
    layout.count(snapshot.tabs.len());
    for tab in &snapshot.tabs {
        validate_columns(tab)?;
        layout.count(tab.columns.len());
        for column in &tab.columns {
            layout.count(column.panes.len());
            let expanded = column
                .panes
                .iter()
                .position(|pane| pane.id == column.expanded)
                .expect("validated expanded pane");
            layout.count(expanded);
        }
    }
    let (kind, payload) = v2::unframe_version(&flat, v9::VERSION)?;
    let mut bytes = layout.bytes;
    bytes.extend_from_slice(payload);
    v2::frame_version(VERSION, kind, bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response> {
    let (kind, payload) = v2::unframe_version(bytes, VERSION)?;
    if kind != v2::SNAPSHOT {
        let mut bytes = bytes.to_vec();
        v6::set_version(&mut bytes, v9::VERSION);
        return match v9::decode_response(&bytes)? {
            v9::Response::Failure(failure) => Ok(Response::Failure(failure)),
            v9::Response::Snapshot(_) => Err(Error::InvalidKind { value: kind }),
        };
    }
    let mut decoder = v2::Decoder::new(payload);
    let tabs = decoder.count("tabs", MAX_TABS)?;
    let mut layouts = Vec::with_capacity(tabs);
    for _ in 0..tabs {
        let columns = decoder.count("columns", MAX_COLUMNS)?;
        let mut layout = Vec::with_capacity(columns);
        for _ in 0..columns {
            let panes = decoder.count("column panes", MAX_SESSIONS)?;
            layout.push((panes, decoder.count("expanded pane", MAX_SESSIONS)?));
        }
        layouts.push(layout);
    }
    let flat = v2::frame_version(v9::VERSION, kind, decoder.rest().to_vec())?;
    let v9::Response::Snapshot(flat) = v9::decode_response(&flat)? else {
        return Err(Error::InvalidKind { value: kind });
    };
    if flat.tabs.len() != layouts.len() {
        return Err(Error::InvalidSnapshot { field: "columns" });
    }
    let mut tabs = Vec::with_capacity(layouts.len());
    for (tab, layout) in flat.tabs.into_iter().zip(layouts) {
        if layout.iter().map(|(panes, _)| panes).sum::<usize>() != tab.panes.len() {
            return Err(Error::InvalidSnapshot { field: "columns" });
        }
        let mut panes = tab.panes.into_iter();
        let mut columns = Vec::with_capacity(layout.len());
        for (count, expanded) in layout {
            let panes: Vec<_> = panes.by_ref().take(count).collect();
            let Some(pane) = panes.get(expanded) else {
                return Err(Error::InvalidSnapshot {
                    field: "expanded pane",
                });
            };
            columns.push(Column {
                expanded: pane.id.clone(),
                panes,
            });
        }
        let tab = Tab {
            id: tab.id,
            directory: tab.directory,
            pending: tab.pending,
            selected_pane: tab.selected_pane,
            selected_popup: tab.selected_popup,
            columns,
            popups: tab.popups,
        };
        validate_columns(&tab)?;
        tabs.push(tab);
    }
    Ok(Response::Snapshot(Snapshot {
        active_tab: flat.active_tab,
        geometry: flat.geometry,
        entries: flat.entries,
        tabs,
        codex_quota: flat.codex_quota,
        claude_quota: flat.claude_quota,
    }))
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
    if request.action == Action::CreateColumn {
        v2::identity("request id", &request.id)?;
        let mut payload = v2::Encoder::default();
        payload.string(&request.id);
        payload.byte(CREATE_COLUMN);
        return v2::frame_version(VERSION, v2::REQUEST, payload.bytes);
    }
    let mut bytes = v9::encode_request(&v9::Request {
        id: request.id.clone(),
        action: v9_action(&request.action),
    })?;
    v6::set_version(&mut bytes, VERSION);
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request> {
    let (kind, payload) = v2::unframe_version(bytes, VERSION)?;
    if kind == v2::REQUEST {
        let mut decoder = v2::Decoder::new(payload);
        let id = decoder.string("request id", v2::MAX_ID_BYTES)?;
        if decoder.byte()? == CREATE_COLUMN {
            decoder.finish()?;
            v2::identity("request id", &id)?;
            return Ok(Request {
                id,
                action: Action::CreateColumn,
            });
        }
    }
    let mut bytes = bytes.to_vec();
    v6::set_version(&mut bytes, v9::VERSION);
    let request = v9::decode_request(&bytes)?;
    Ok(Request {
        id: request.id,
        action: from_v9_action(request.action),
    })
}

pub fn encode_lifecycle_response(response: &LifecycleResponse) -> Result<Vec<u8>> {
    let mut bytes = v9::encode_lifecycle_response(response)?;
    v6::set_version(&mut bytes, VERSION);
    Ok(bytes)
}

pub fn decode_lifecycle_response(bytes: &[u8]) -> Result<LifecycleResponse> {
    let mut bytes = v6::reframe(bytes, VERSION)?;
    v6::set_version(&mut bytes, v9::VERSION);
    v9::decode_lifecycle_response(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> Pane {
        Pane {
            id: id.into(),
            session: format!("session-{id}"),
            endpoint: format!("/run/eon/{id}.sock").into_bytes(),
            live: true,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
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
                selected_pane: Some("p3".into()),
                selected_popup: None,
                columns: vec![
                    Column {
                        panes: vec![pane("p1"), pane("p2")],
                        expanded: "p2".into(),
                    },
                    Column {
                        panes: vec![pane("p3")],
                        expanded: "p3".into(),
                    },
                ],
                popups: vec![],
            }],
            codex_quota: None,
            claude_quota: None,
        }
    }

    #[test]
    fn columns_roundtrip_and_keep_every_v9_rule() {
        let response = Response::Snapshot(snapshot());
        let bytes = encode_response(&response).unwrap();
        assert_eq!(decode_response(&bytes).unwrap(), response);
        assert!(v9::decode_response(&bytes).is_err());
        assert!(decode_response(&bytes[..bytes.len() - 1]).is_err());

        for change in [
            |tab: &mut Tab| tab.columns[1].panes.clear(),
            |tab: &mut Tab| tab.columns[0].expanded = "p3".into(),
            |tab: &mut Tab| tab.selected_pane = Some("p1".into()),
            |tab: &mut Tab| tab.columns[1].panes[0].id = "p1".into(),
            |tab: &mut Tab| {
                for id in ["p4", "p5", "p6"] {
                    tab.columns.push(Column {
                        panes: vec![pane(id)],
                        expanded: id.into(),
                    });
                }
            },
        ] {
            let mut invalid = snapshot();
            change(&mut invalid.tabs[0]);
            assert!(encode_response(&Response::Snapshot(invalid)).is_err());
        }

        // A layout listing fewer panes than the flattened snapshot cannot drop the rest.
        let mut first = snapshot();
        first.tabs[0].selected_pane = Some("p1".into());
        first.tabs[0].columns.swap(0, 1);
        first.tabs[0].columns[0].panes = vec![pane("p1")];
        first.tabs[0].columns[0].expanded = "p1".into();
        first.tabs[0].columns[1].panes = vec![pane("p2"), pane("p3")];
        first.tabs[0].columns[1].expanded = "p2".into();
        let mut forged = encode_response(&Response::Snapshot(first)).unwrap();
        // Header, tab count, column count, first column (panes, expanded), second column panes.
        forged[v2::HEADER_BYTES + 2 * 4] = 1;
        assert!(matches!(
            decode_response(&forged),
            Err(Error::InvalidSnapshot { field: "columns" })
        ));

        let failure = Response::Failure(Failure {
            code: "invalid-transition".into(),
            detail: "no".into(),
        });
        assert_eq!(
            decode_response(&encode_response(&failure).unwrap()).unwrap(),
            failure
        );
    }

    #[test]
    fn create_column_is_a_v10_request_and_older_actions_still_roundtrip() {
        for action in [
            Action::CreateColumn,
            Action::Workspace(WorkspaceAction::CreatePane),
            Action::DismissPopup(PopupTarget {
                tab: "t1".into(),
                instance: "u1".into(),
            }),
        ] {
            let request = Request {
                id: "r1".into(),
                action,
            };
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
            assert!(v9::decode_request(&bytes).is_err());
        }
        let v9_bytes = v9::encode_request(&v9::Request {
            id: "r1".into(),
            action: v9::Action::Workspace(WorkspaceAction::CreatePane),
        })
        .unwrap();
        assert!(decode_request(&v9_bytes).is_err());
    }
}
