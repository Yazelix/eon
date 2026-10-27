use super::managed_environment::{PopupCatalog, PopupCommand, PopupDefinition};
use eon_workspace_protocol::v22::{
    Action, Attention, AttentionState, Column as SnapshotColumn, Direction, Failure, InvokeIntent,
    MAX_COLUMNS, MAX_DIRECTORY_BYTES, MAX_SESSIONS, MAX_TABS, Pane as SnapshotPane,
    Popup as SnapshotPopup, PopupEntry as SnapshotEntry, PopupKey, PopupTarget, Side, Sidebar,
    Snapshot, Tab as SnapshotTab, WorkspaceAction, check_popup_action,
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const MAX_RECENT_REQUESTS: usize = 256;
/// The packaged Helix on Eon's terminal PATH, which `eon edit` runs.
const EDITOR: &str = "hx";

pub(crate) fn is_directory_picker_session(id: &str) -> bool {
    id.strip_prefix("directory-picker-")
        .and_then(canonical_number)
        .is_some()
}

pub(crate) fn is_directory_picker_endpoint(name: &str) -> bool {
    name == "pick.sock"
        || name
            .strip_prefix('k')
            .and_then(|name| name.strip_suffix(".sock"))
            .and_then(canonical_number)
            .is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) endpoint: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LaunchCommand {
    Pane,
    Project { tab: String, instance: String },
    Tool(Vec<OsString>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    Exited,
    /// Stopped by a tab or pane close request.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionOperation {
    Start {
        session: Session,
        directory: PathBuf,
        command: LaunchCommand,
        /// The owning tab of a popup terminal, exported to it as `EON_TAB`.
        tab: Option<String>,
    },
    Stop(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: Session,
    pinned_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Popup {
    id: String,
    entry: String,
    session: Session,
    launch_directory: PathBuf,
}

/// A tab's selected column and one column's expanded pane before a caller's
/// pane action took them.
struct Selection {
    tab: String,
    column: Option<usize>,
    expanded: (usize, usize),
}

/// One accordion of panes; `expanded` indexes the pane it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Column {
    panes: Vec<Pane>,
    expanded: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tab {
    id: String,
    directory: PathBuf,
    pending: bool,
    /// Side-by-side columns; `selected_column` is set exactly when this is nonempty.
    columns: Vec<Column>,
    selected_column: Option<usize>,
    popups: Vec<Popup>,
    selected_popup: Option<String>,
}

impl Tab {
    fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.columns.iter().flat_map(|column| &column.panes)
    }

    fn find_pane(&self, matches: impl Fn(&Pane) -> bool) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(column, item)| {
            item.panes
                .iter()
                .position(&matches)
                .map(|pane| (column, pane))
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IdAllocator {
    runtime: PathBuf,
    next_workspace: usize,
    next_tab: usize,
    next_pane: usize,
    next_session: usize,
    next_popup: usize,
}

impl IdAllocator {
    fn new(runtime: PathBuf) -> Self {
        Self {
            runtime,
            next_workspace: 1,
            next_tab: 1,
            next_pane: 1,
            next_session: 1,
            next_popup: 1,
        }
    }

    fn pane_session(&self) -> Session {
        Session {
            id: format!("session-{}", self.next_session),
            endpoint: self.runtime.join(if self.next_session == 1 {
                "orbit.sock".into()
            } else {
                format!("session-{}.sock", self.next_session)
            }),
        }
    }

    fn popup_session(&self, project: bool) -> Session {
        let number = if project {
            self.next_popup
        } else {
            self.next_session
        };
        let id = if project {
            format!("directory-picker-{number}")
        } else {
            format!("session-{number}")
        };
        let endpoint = self.runtime.join(if project {
            format!("k{number}.sock")
        } else {
            format!("{id}.sock")
        });
        Session { id, endpoint }
    }
}

/// Supervisor-wide admission, identities and replay protection; topology stays local.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceSet {
    workspaces: Vec<Workspace>,
    shown: usize,
    ids: IdAllocator,
    recent_inspections: VecDeque<String>,
    recent_mutations: VecDeque<String>,
    /// Every live workspace once, most recently shown first, so the shown one leads.
    recent_workspaces: VecDeque<String>,
}

impl WorkspaceSet {
    pub(crate) fn pending(
        runtime: PathBuf,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut set = Self {
            workspaces: Vec::new(),
            shown: 0,
            ids: IdAllocator::new(runtime),
            recent_inspections: VecDeque::new(),
            recent_workspaces: VecDeque::new(),
            recent_mutations: VecDeque::new(),
        };
        set.insert_pending(directory, home, catalog, prepare, operate)?;
        Ok(set)
    }

    fn insert_pending(
        &mut self,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.workspaces.len() == 32 || self.ids.next_workspace == usize::MAX {
            return Err("a window holds at most 32 workspaces".into());
        }
        if self.ids.next_tab == usize::MAX {
            return Err("workspace exhausted tab identities".into());
        }
        let allowance = MAX_SESSIONS
            - self
                .workspaces
                .iter()
                .map(Workspace::session_count)
                .sum::<usize>();
        let workspace = Workspace::pending(
            &mut self.ids,
            allowance,
            directory,
            home,
            catalog,
            prepare,
            operate,
        )?;
        self.ids.next_workspace += 1;
        self.ids.next_tab += 1;
        self.recent_workspaces.push_back(workspace.id.clone());
        self.workspaces.push(workspace);
        Ok(())
    }

    pub(crate) fn with_recovered_sessions(
        runtime: PathBuf,
        directory: PathBuf,
        home: Option<PathBuf>,
        sessions: Vec<(usize, Session)>,
        catalog: PopupCatalog,
    ) -> Result<Self, String> {
        let next_session = sessions
            .last()
            .ok_or("cannot recover an empty workspace")?
            .0
            .checked_add(1)
            .ok_or("recovered terminal identity leaves no next terminal identity")?;
        let mut ids = IdAllocator::new(runtime);
        let workspace =
            Workspace::with_recovered_sessions(&ids, directory, home, sessions, catalog);
        ids.next_workspace = 2;
        ids.next_tab = 2;
        ids.next_pane = next_session;
        ids.next_session = next_session;
        let recent_workspaces = VecDeque::from([workspace.id.clone()]);
        Ok(Self {
            workspaces: vec![workspace],
            shown: 0,
            ids,
            recent_inspections: VecDeque::new(),
            recent_workspaces,
            recent_mutations: VecDeque::new(),
        })
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        self.snapshot_in(None).expect("the shown workspace exists")
    }

    pub(crate) fn caller_workspace_id(
        &self,
        caller: Option<&str>,
    ) -> Result<Option<String>, Failure> {
        let index = self.caller_workspace(caller)?;
        Ok(caller.map(|_| self.workspaces[index].id.clone()))
    }

    pub(crate) fn snapshot_in(&self, id: Option<&str>) -> Result<Snapshot, Failure> {
        let index = match id {
            Some(id) => self
                .workspaces
                .iter()
                .position(|workspace| workspace.id == id)
                .ok_or_else(|| {
                    action_error("unknown-workspace", format!("workspace {id} is not live"))
                })?,
            None => self.shown,
        };
        let mut snapshot = self.workspaces[index].snapshot();
        snapshot.active_workspace = self.workspaces[self.shown].id.clone();
        snapshot.workspaces = self
            .workspaces
            .iter()
            .map(|workspace| eon_workspace_protocol::v22::WorkspaceInfo {
                id: workspace.id.clone(),
                name: workspace.name.clone(),
                tab_count: workspace.tabs.len(),
                // The attention owner sets each rollup.
                attention: None,
                recency: self
                    .recent_workspaces
                    .iter()
                    .position(|recent| *recent == workspace.id)
                    .expect("every live workspace has a recency") as u8,
            })
            .collect();
        Ok(snapshot)
    }

    /// Every workspace's own snapshot in window order, each naming the shown
    /// one, for the attention owner (EON-C31, EON-C33). Each lists only its own
    /// workspace, so none is a valid EONW reply.
    pub(crate) fn snapshots(&self) -> Vec<Snapshot> {
        let shown = &self.workspaces[self.shown].id;
        self.workspaces
            .iter()
            .map(|workspace| {
                let mut snapshot = workspace.snapshot();
                snapshot.active_workspace = shown.clone();
                snapshot
            })
            .collect()
    }

    pub(crate) fn has_sessions(&self) -> bool {
        self.workspaces
            .iter()
            .any(|workspace| workspace.session_count() != 0)
    }

    pub(crate) fn pane_sessions(&self) -> Vec<Session> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .flat_map(|tab| &tab.columns)
            .flat_map(|column| &column.panes)
            .map(|pane| pane.session.clone())
            .collect()
    }

    pub(crate) fn home(&self) -> Option<&Path> {
        self.workspaces
            .first()
            .and_then(|workspace| workspace.home.as_deref())
    }

    fn resolve_workspace(&self, target: &str) -> Result<usize, Failure> {
        eon_workspace_protocol::v22::validate_workspace_name(target)
            .map_err(|error| action_error("malformed-action", error.to_string()))?;
        resolve_selector(
            self.workspaces
                .iter()
                .map(|workspace| (workspace.id.as_str(), workspace.name.as_deref())),
            target,
        )
    }

    fn show(&mut self, index: usize) {
        self.shown = index;
        let id = &self.workspaces[index].id;
        self.recent_workspaces.retain(|recent| recent != id);
        self.recent_workspaces.push_front(id.clone());
    }

    fn prune_empty(&mut self) {
        if self.workspaces.len() == 1 {
            return;
        }
        let shown = self.workspaces[self.shown].id.clone();
        // Keep one owner until the supervisor returns Ended.
        let keep_last = !self.has_sessions();
        self.workspaces.retain(|workspace| {
            workspace.session_count() != 0 || keep_last && workspace.id == shown
        });
        self.recent_workspaces
            .retain(|id| self.workspaces.iter().any(|workspace| workspace.id == *id));
        self.shown = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == shown)
            .or_else(|| {
                self.recent_workspaces.front().and_then(|id| {
                    self.workspaces
                        .iter()
                        .position(|workspace| workspace.id == *id)
                })
            })
            .unwrap_or(0);
    }

    pub(crate) fn prepare_close_workspace(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        target: &str,
    ) -> Result<Vec<String>, Failure> {
        self.reject_duplicate(request_id)?;
        self.caller_workspace(caller)?;
        let index = self.resolve_workspace(target)?;
        let sessions: Vec<String> = self.workspaces[index]
            .sessions()
            .map(|session| session.id.clone())
            .collect();
        self.remember(request_id, true);
        if sessions.is_empty() {
            self.prune_empty();
        }
        Ok(sessions)
    }

    fn caller_workspace(&self, caller: Option<&str>) -> Result<usize, Failure> {
        match caller {
            None => Ok(self.shown),
            Some(id) => self
                .workspaces
                .iter()
                .position(|workspace| workspace.sessions().any(|session| session.id == id))
                .ok_or_else(|| {
                    action_error(
                        "unknown-caller",
                        "the calling terminal is not part of this workspace",
                    )
                }),
        }
    }

    pub(crate) fn owns_session(&self, session: &Session) -> bool {
        self.workspaces
            .iter()
            .any(|workspace| workspace.sessions().any(|item| item == session))
    }

    fn allowance(&self, index: usize) -> usize {
        MAX_SESSIONS
            - self
                .workspaces
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, workspace)| workspace.session_count())
                .sum::<usize>()
    }

    pub(crate) fn dispatch(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        action: Action,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        let mutating = !matches!(action, Action::Workspace(WorkspaceAction::Inspect));
        match action {
            Action::RenamePane { pane, name } => {
                let name = name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty());
                if let Some(name) = name {
                    eon_workspace_protocol::v22::validate_pane_name(name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                let target = self.workspaces[index]
                    .tabs
                    .iter_mut()
                    .flat_map(|tab| &mut tab.columns)
                    .flat_map(|column| &mut column.panes)
                    .find(|target| target.id == pane)
                    .ok_or_else(|| {
                        action_error("unknown-pane", format!("pane {pane} is not live"))
                    })?;
                target.pinned_name = name.map(str::to_owned);
            }
            Action::Workspace(WorkspaceAction::NewWorkspace { name, focus }) => {
                if let Some(name) = &name {
                    eon_workspace_protocol::v22::validate_workspace_name(name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                if self.workspaces.len() >= eon_workspace_protocol::v22::MAX_WORKSPACES {
                    return Err(action_error(
                        "workspace-limit",
                        "a window holds at most 32 workspaces",
                    ));
                }
                let shown = &self.workspaces[self.shown];
                let home = shown.home.clone().ok_or_else(|| {
                    action_error(
                        "invalid-directory",
                        "cannot resolve the current user home directory",
                    )
                })?;
                let directory = validated_directory(home.as_os_str().as_bytes().to_vec())?;
                let catalog = shown.catalog.clone();
                self.insert_pending(directory, Some(home), catalog, prepare, operate)
                    .map_err(|detail| action_error("session-start", detail))?;
                let mut workspace = self.workspaces.pop().expect("new workspace was inserted");
                workspace.name = name;
                let added = self.shown + 1;
                self.workspaces.insert(added, workspace);
                if focus {
                    self.show(added);
                }
            }
            Action::Workspace(WorkspaceAction::SwitchWorkspace { workspace }) => {
                let target = self.resolve_workspace(&workspace)?;
                if target != self.shown {
                    self.show(target);
                }
            }
            Action::Workspace(WorkspaceAction::RenameWorkspace { workspace, name }) => {
                if !name.is_empty() {
                    eon_workspace_protocol::v22::validate_workspace_name(&name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                let target = workspace
                    .as_deref()
                    .map(|id| self.resolve_workspace(id))
                    .transpose()?
                    .unwrap_or(index);
                self.workspaces[target].name = (!name.is_empty()).then_some(name);
            }
            action => {
                let allowance = self.allowance(index);
                let result = self.workspaces[index].dispatch(
                    &mut self.ids,
                    allowance,
                    caller,
                    action,
                    prepare,
                    operate,
                );
                if mutating {
                    self.prune_empty();
                }
                result?;
            }
        }
        self.remember(request_id, mutating);
        Ok(())
    }

    /// A queue jump: the `FocusAttention` of `tab` and `target` in
    /// `workspace`, which then becomes the shown workspace (EON-C33). Only
    /// this crosses workspaces.
    pub(crate) fn jump(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        workspace: &str,
        tab: &str,
        target: &str,
    ) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        self.caller_workspace(caller)?;
        let index = self
            .workspaces
            .iter()
            .position(|item| item.id == workspace)
            .ok_or_else(|| {
                action_error(
                    "unknown-workspace",
                    format!("workspace {workspace} is not live"),
                )
            })?;
        self.workspaces[index].focus_attention(tab, target)?;
        self.show(index);
        self.remember(request_id, true);
        Ok(())
    }

    pub(crate) fn prepare_close_tab(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        tab: &str,
    ) -> Result<Vec<String>, Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        let sessions = self.workspaces[index].prepare_close_tab(tab)?;
        self.remember(request_id, true);
        Ok(sessions)
    }

    pub(crate) fn prepare_close_pane(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        pane: &str,
    ) -> Result<Vec<String>, Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        let sessions = self.workspaces[index].prepare_close_pane(caller, pane)?;
        self.remember(request_id, true);
        Ok(sessions)
    }

    pub(crate) fn session_ended(
        &mut self,
        id: &str,
        end: SessionEnd,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let index = self
            .workspaces
            .iter()
            .position(|workspace| workspace.sessions().any(|session| session.id == id))
            .ok_or_else(|| {
                action_error(
                    "unknown-session",
                    format!("terminal {id} is not in the workspace"),
                )
            })?;
        let allowance = self.allowance(index);
        let result =
            self.workspaces[index].session_ended(&mut self.ids, allowance, id, end, operate);
        self.prune_empty();
        result
    }

    pub(crate) fn transient_sessions(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .flat_map(Workspace::transient_sessions)
            .collect()
    }

    fn reject_duplicate(&self, request_id: &str) -> Result<(), Failure> {
        if self
            .recent_mutations
            .iter()
            .chain(&self.recent_inspections)
            .any(|seen| seen == request_id)
        {
            Err(action_error(
                "duplicate-request",
                format!("request {request_id} was already accepted"),
            ))
        } else {
            Ok(())
        }
    }

    fn remember(&mut self, request_id: &str, mutating: bool) {
        // Inspections must not evict a mutation's replay protection.
        let recent = if mutating {
            &mut self.recent_mutations
        } else {
            &mut self.recent_inspections
        };
        if recent.len() == MAX_RECENT_REQUESTS {
            recent.pop_front();
        }
        recent.push_back(request_id.into());
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Workspace {
    id: String,
    name: Option<String>,
    catalog: PopupCatalog,
    home: Option<PathBuf>,
    tabs: Vec<Tab>,
    active: usize,
}

impl Workspace {
    fn pending(
        ids: &mut IdAllocator,
        allowance: usize,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut workspace = Self {
            id: format!("w{}", ids.next_workspace),
            name: None,
            catalog,
            home,
            tabs: vec![Tab {
                id: format!("t{}", ids.next_tab),
                directory,
                pending: true,
                columns: Vec::new(),
                selected_column: None,
                popups: Vec::new(),
                selected_popup: None,
            }],
            active: 0,
        };
        if workspace
            .open_popup(
                ids,
                allowance,
                0,
                "project",
                &mut prepare,
                &mut operate,
                true,
            )
            .is_err()
        {
            workspace.tabs[0].pending = false;
            workspace.start_pane(ids, allowance, 0, &mut operate)?;
        }
        Ok(workspace)
    }

    fn with_recovered_sessions(
        ids: &IdAllocator,
        directory: PathBuf,
        home: Option<PathBuf>,
        sessions: Vec<(usize, Session)>,
        catalog: PopupCatalog,
    ) -> Self {
        let panes = sessions
            .into_iter()
            .map(|(number, session)| Pane {
                pinned_name: None,
                id: format!("p{number}"),
                session,
            })
            .collect::<Vec<_>>();
        Self {
            id: format!("w{}", ids.next_workspace),
            name: None,
            catalog,
            home,
            tabs: vec![Tab {
                id: format!("t{}", ids.next_tab),
                directory,
                pending: false,
                columns: vec![Column { panes, expanded: 0 }],
                selected_column: Some(0),
                popups: Vec::new(),
                selected_popup: None,
            }],
            active: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            workspace: self.id.clone(),
            active_workspace: self.id.clone(),
            workspaces: vec![eon_workspace_protocol::v22::WorkspaceInfo {
                id: self.id.clone(),
                name: self.name.clone(),
                tab_count: self.tabs.len(),
                attention: None,
                recency: 0,
            }],
            active_tab: self.tabs[self.active].id.clone(),
            geometry: self.catalog.geometry,
            entries: self
                .catalog
                .entries
                .iter()
                .map(|entry| SnapshotEntry {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    shortcut: entry.shortcut.clone(),
                })
                .collect(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| SnapshotTab {
                    id: tab.id.clone(),
                    directory: tab.directory.as_os_str().as_bytes().to_vec(),
                    pending: tab.pending,
                    selected_pane: tab.selected_column.map(|selected| {
                        let column = &tab.columns[selected];
                        column.panes[column.expanded].id.clone()
                    }),
                    selected_popup: tab.selected_popup.clone(),
                    columns: tab
                        .columns
                        .iter()
                        .map(|column| SnapshotColumn {
                            panes: column
                                .panes
                                .iter()
                                .map(|pane| SnapshotPane {
                                    label: super::pane_labels::compose(
                                        &pane.id,
                                        true,
                                        pane.pinned_name.as_deref(),
                                        &super::pane_labels::Metadata::Unavailable,
                                        &tab.directory,
                                        self.home.as_deref(),
                                    ),
                                    pinned_name: pane.pinned_name.clone(),
                                    id: pane.id.clone(),
                                    session: pane.session.id.clone(),
                                    endpoint: pane.session.endpoint.as_os_str().as_bytes().to_vec(),
                                    live: true,
                                    attention: None,
                                })
                                .collect(),
                            expanded: column.panes[column.expanded].id.clone(),
                        })
                        .collect(),
                    popups: tab
                        .popups
                        .iter()
                        .map(|popup| SnapshotPopup {
                            id: popup.id.clone(),
                            entry: popup.entry.clone(),
                            session: popup.session.id.clone(),
                            endpoint: popup.session.endpoint.as_os_str().as_bytes().to_vec(),
                            attention: None,
                        })
                        .collect(),
                    attention_rollup: None,
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            sidebar: Sidebar::Off,
        }
    }

    /// Applies one action. `caller` names the issuing terminal's session, whose
    /// tab and pane replace the active tab and its selection as the implicit
    /// target of pane create, edit and move actions and tab moves.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        caller: Option<&str>,
        action: Action,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        check_popup_action(&self.snapshot(), &action)
            .map_err(|error| action_error("stale-popup", error.to_string()))?;
        let (target, previous) = match &action {
            Action::Workspace(WorkspaceAction::Move(Direction::Left | Direction::Right)) => {
                (self.target_tab(caller)?.0, None)
            }
            Action::Workspace(WorkspaceAction::CreatePane { .. } | WorkspaceAction::Move(_))
            | Action::MovePaneSideways(_)
            | Action::Edit { .. }
            | Action::CreateCommandPane { .. } => {
                let (target, pane) = self.target_tab(caller)?;
                (target, pane.map(|pane| self.select(target, pane)))
            }
            _ => (self.active, None),
        };
        let focus = !matches!(
            action,
            Action::Workspace(WorkspaceAction::CreatePane { focus: false })
                | Action::CreateCommandPane { focus: false, .. }
        );
        let result = self.apply(
            ids,
            allowance,
            action,
            target,
            caller.is_some(),
            &mut prepare,
            &mut operate,
        );
        // An unfocused pane leaves the tab's selection as it was.
        if (result.is_err() || !focus)
            && let Some(previous) = previous
        {
            self.restore(previous);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        action: Action,
        target: usize,
        called: bool,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match action {
            Action::Workspace(action) => self.dispatch_workspace(
                ids,
                allowance,
                action,
                target,
                called,
                &mut prepare,
                &mut operate,
            )?,
            Action::RenamePane { .. } | Action::ClosePane { .. } => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor stops a closed pane's terminal before pruning it",
                ));
            }
            // The supervisor's attention owner flips visibility once this
            // request is accepted.
            Action::ToggleSidebar => {}
            Action::FocusAttention { tab, target } => self.focus_attention(&tab, &target)?,
            Action::NextAttention | Action::PreviousAttention => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor resolves queue jumps to their target first",
                ));
            }
            Action::InvokePopup {
                tab, entry, intent, ..
            } => self.invoke_popup(
                ids,
                allowance,
                &tab,
                &entry,
                intent,
                &mut prepare,
                &mut operate,
            )?,
            Action::DismissPopup(target) => {
                self.dismiss_popup(ids, allowance, &target, &mut operate)?
            }
            Action::CommitDirectory { target, directory } => {
                self.commit_directory(ids, allowance, &target, directory, &mut operate)?
            }
            // A tab without columns gains its first one exactly as a new pane would.
            Action::CreateColumn if self.tabs[self.active].columns.is_empty() => self
                .dispatch_workspace(
                    ids,
                    allowance,
                    WorkspaceAction::CreatePane { focus: true },
                    self.active,
                    false,
                    &mut prepare,
                    &mut operate,
                )?,
            Action::CreateColumn => {
                self.require_established(self.active)?;
                if self.tabs[self.active].columns.len() >= MAX_COLUMNS {
                    return Err(action_error(
                        "unavailable",
                        format!("a tab holds at most {MAX_COLUMNS} columns"),
                    ));
                }
                let active =
                    self.dismiss_for_pane_action(ids, allowance, self.active, &mut operate)?;
                self.start_column(ids, allowance, active, &mut operate)
                    .map_err(|detail| action_error("session-start", detail))?;
            }
            Action::MovePaneSideways(side) => self.move_pane_sideways(target, side)?,
            Action::Edit { file } => {
                self.require_established(target)?;
                let argv = prepare(
                    &PopupCommand::Argv(vec![EDITOR.into(), OsString::from_vec(file)]),
                    &self.tabs[target].directory,
                )
                .map_err(|detail| action_error("edit-command", detail))?;
                self.create_pane(
                    ids,
                    allowance,
                    target,
                    called,
                    true,
                    LaunchCommand::Tool(argv),
                    &mut operate,
                )?;
            }
            Action::CreateCommandPane { argv, focus } => {
                if !called {
                    return Err(action_error(
                        "caller-required",
                        "a command pane must be requested from an Eon terminal",
                    ));
                }
                self.require_established(target)?;
                let argv = prepare(
                    &PopupCommand::Argv(argv.into_iter().map(OsString::from_vec).collect()),
                    &self.tabs[target].directory,
                )
                .map_err(|detail| action_error("pane-command", detail))?;
                self.create_pane(
                    ids,
                    allowance,
                    target,
                    called,
                    focus,
                    LaunchCommand::Tool(argv),
                    &mut operate,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_workspace(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        action: WorkspaceAction,
        target: usize,
        called: bool,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match action {
            WorkspaceAction::Inspect => {}
            WorkspaceAction::NewWorkspace { .. }
            | WorkspaceAction::SwitchWorkspace { .. }
            | WorkspaceAction::RenameWorkspace { .. }
            | WorkspaceAction::CloseWorkspace { .. }
            | WorkspaceAction::InspectRuntime
            | WorkspaceAction::InspectPresentation
            | WorkspaceAction::Present { .. }
            | WorkspaceAction::CloseTab { .. }
            | WorkspaceAction::Stop { .. } => {
                return Err(action_error(
                    "unavailable",
                    "supervisor lifecycle actions do not mutate workspace topology",
                ));
            }
            WorkspaceAction::CreateTab => self.create_tab(ids, allowance, prepare, operate)?,
            WorkspaceAction::CreatePane { focus } => self.create_pane(
                ids,
                allowance,
                target,
                called,
                focus,
                LaunchCommand::Pane,
                operate,
            )?,
            WorkspaceAction::FocusId(id) => self.focus_id(ids, allowance, &id, operate)?,
            WorkspaceAction::Focus(direction) => {
                self.focus_direction(ids, allowance, direction, operate)?
            }
            WorkspaceAction::Move(direction) => self.move_direction(target, direction)?,
            WorkspaceAction::SetTabDirectory { tab, directory } => {
                self.set_tab_directory(ids, &tab, directory)?;
            }
        }
        Ok(())
    }

    /// Adds a pane to the target tab's selected column. A focused pane is
    /// selected and dismisses the tab's popup; an unfocused one joins its
    /// column collapsed, unless it is the first pane of a tab without panes.
    #[allow(clippy::too_many_arguments)]
    fn create_pane(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        target: usize,
        called: bool,
        focus: bool,
        command: LaunchCommand,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.require_established(target)?;
        let target = if focus {
            if called {
                self.require_surviving_dismissal(target)?;
            }
            self.dismiss_for_pane_action(ids, allowance, target, operate)?
        } else {
            target
        };
        let tab = &self.tabs[target];
        let directory = tab.directory.clone();
        let kept = tab
            .selected_column
            .filter(|_| !focus)
            .map(|column| (column, tab.columns[column].expanded));
        self.start_pane_in(ids, allowance, target, &directory, false, command, operate)
            .map_err(|detail| action_error("session-start", detail))?;
        if let Some((column, expanded)) = kept {
            self.tabs[target].columns[column].expanded = expanded;
        }
        Ok(())
    }

    fn create_tab(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if self.tabs.len() >= MAX_TABS || ids.next_tab == usize::MAX {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_TABS} tabs"),
            ));
        }
        let id = format!("t{}", ids.next_tab);
        let home = self.home.as_ref().ok_or_else(|| {
            action_error(
                "invalid-directory",
                "cannot resolve the current user home directory",
            )
        })?;
        let directory = validated_directory(home.as_os_str().as_bytes().to_vec())?;
        self.tabs.push(Tab {
            id,
            directory,
            pending: true,
            columns: Vec::new(),
            selected_column: None,
            popups: Vec::new(),
            selected_popup: None,
        });
        let index = self.tabs.len() - 1;
        if let Err(error) =
            self.open_popup(ids, allowance, index, "project", prepare, operate, true)
        {
            self.tabs.pop();
            return Err(error);
        }
        ids.next_tab += 1;
        self.active = index;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_popup(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab: &str,
        entry: &str,
        intent: InvokeIntent,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let tab_index = self
            .tabs
            .iter()
            .position(|item| item.id == tab)
            .ok_or_else(|| action_error("unknown-tab", format!("tab {tab} is not live")))?;
        if tab_index != self.active {
            return Err(action_error(
                "not-active",
                "popup target is not the active tab",
            ));
        }
        if self.tabs[tab_index].pending && entry != "project" {
            return Err(action_error(
                "choose-directory-first",
                "choose the pending tab directory before opening a tool",
            ));
        }
        let definition = self.definition(entry)?.clone();
        if let Some(index) = self.tabs[tab_index]
            .popups
            .iter()
            .position(|popup| popup.entry == entry)
        {
            let popup = self.tabs[tab_index].popups[index].clone();
            // Files keeps its Yazi, and its location, across tab-directory changes.
            if popup.entry != "files" && popup.launch_directory != self.tabs[tab_index].directory {
                let candidate =
                    self.popup_candidate(ids, allowance, tab_index, &definition, true)?;
                let launch = self.prepare_launch(tab_index, &candidate, &definition, prepare)?;
                if let Some(index) = self.displaced_transient(tab_index, Some(&popup.id))? {
                    self.stop_popup(tab_index, index, operate)?;
                }
                let index = self.tabs[tab_index]
                    .popups
                    .iter()
                    .position(|item| item.id == popup.id)
                    .expect("the popup being replaced remains live");
                self.stop_popup(tab_index, index, operate)?;
                if let Err(detail) = operate(launch) {
                    self.remove_empty_tab(tab_index);
                    return Err(action_error("popup-start", detail));
                }
                self.commit_popup(ids, tab_index, candidate);
                return Ok(());
            }
            let selected = self.tabs[tab_index].selected_popup.as_deref() == Some(&popup.id);
            if intent == InvokeIntent::Toggle && selected {
                return self.dismiss_popup(
                    ids,
                    allowance,
                    &PopupTarget {
                        tab: tab.into(),
                        instance: popup.id.clone(),
                    },
                    operate,
                );
            }
            if let Some(index) = self.displaced_transient(tab_index, Some(&popup.id))? {
                self.stop_popup(tab_index, index, operate)?;
            }
            self.tabs[tab_index].selected_popup = Some(popup.id.clone());
            return Ok(());
        }
        self.open_popup(ids, allowance, tab_index, entry, prepare, operate, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_popup(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        entry: &str,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
        pending: bool,
    ) -> Result<(), Failure> {
        let definition = self.definition(entry)?.clone();
        let displaced = self.displaced_transient(tab_index, None)?;
        let stops_selected = displaced.is_some();
        let candidate =
            self.popup_candidate(ids, allowance, tab_index, &definition, stops_selected)?;
        let launch = self.prepare_launch(tab_index, &candidate, &definition, prepare)?;
        if let Some(index) = displaced {
            self.stop_popup(tab_index, index, operate)?;
        }
        if let Err(detail) = operate(launch) {
            if stops_selected {
                self.remove_empty_tab(tab_index);
            }
            return Err(action_error("popup-start", detail));
        }
        self.commit_popup(ids, tab_index, candidate);
        self.tabs[tab_index].pending = pending;
        Ok(())
    }

    fn displaced_transient(
        &self,
        tab_index: usize,
        retained: Option<&str>,
    ) -> Result<Option<usize>, Failure> {
        let tab = &self.tabs[tab_index];
        let Some(selected) = tab
            .selected_popup
            .as_deref()
            .filter(|id| retained != Some(*id))
        else {
            return Ok(None);
        };
        let index = tab
            .popups
            .iter()
            .position(|popup| popup.id == selected)
            .expect("selected popup is live");
        let popup = &tab.popups[index];
        Ok((!self.definition(&popup.entry)?.keep_alive).then_some(index))
    }

    fn popup_candidate(
        &self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        definition: &PopupDefinition,
        replacing: bool,
    ) -> Result<Popup, Failure> {
        if !replacing && self.session_count() >= allowance {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_SESSIONS} terminals"),
            ));
        }
        if ids.next_popup == usize::MAX || ids.next_session == usize::MAX {
            return Err(action_error(
                "capacity",
                "workspace exhausted terminal identities",
            ));
        }
        let project = matches!(definition.command, PopupCommand::Project);
        Ok(Popup {
            id: format!("u{}", ids.next_popup),
            entry: definition.id.clone(),
            session: ids.popup_session(project),
            launch_directory: self.tabs[tab_index].directory.clone(),
        })
    }

    fn prepare_launch(
        &self,
        tab_index: usize,
        popup: &Popup,
        definition: &PopupDefinition,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
    ) -> Result<SessionOperation, Failure> {
        let command = match &definition.command {
            PopupCommand::Project => LaunchCommand::Project {
                tab: self.tabs[tab_index].id.clone(),
                instance: popup.id.clone(),
            },
            command => LaunchCommand::Tool(
                prepare(command, &popup.launch_directory)
                    .map_err(|detail| action_error("popup-command", detail))?,
            ),
        };
        Ok(SessionOperation::Start {
            session: popup.session.clone(),
            directory: popup.launch_directory.clone(),
            command,
            tab: Some(self.tabs[tab_index].id.clone()),
        })
    }

    fn commit_popup(&mut self, ids: &mut IdAllocator, tab_index: usize, popup: Popup) {
        let project = is_directory_picker_session(&popup.session.id);
        self.tabs[tab_index].selected_popup = Some(popup.id.clone());
        self.tabs[tab_index].popups.push(popup);
        ids.next_popup += 1;
        if !project {
            ids.next_session += 1;
        }
    }

    fn dismiss_popup(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        target: &PopupTarget,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let tab_index = self
            .tabs
            .iter()
            .position(|tab| tab.id == target.tab)
            .ok_or_else(|| action_error("stale-popup", "popup tab is no longer live"))?;
        let popup_index = self.tabs[tab_index]
            .popups
            .iter()
            .position(|popup| popup.id == target.instance)
            .ok_or_else(|| action_error("stale-popup", "popup instance is no longer live"))?;
        let popup = &self.tabs[tab_index].popups[popup_index];
        if self.definition(&popup.entry)?.keep_alive {
            if self.tabs[tab_index].selected_popup.as_deref() == Some(&popup.id) {
                self.tabs[tab_index].selected_popup = None;
            }
            return Ok(());
        }
        let project = popup.entry == "project";
        self.stop_popup(tab_index, popup_index, operate)?;
        if project && self.tabs[tab_index].pending {
            self.finish_pending_tab(ids, allowance, tab_index, operate)?;
        } else {
            self.remove_empty_tab(tab_index);
        }
        Ok(())
    }

    fn commit_directory(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        target: &PopupTarget,
        directory: Vec<u8>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let tab_index = self
            .tabs
            .iter()
            .position(|tab| tab.id == target.tab)
            .ok_or_else(|| action_error("stale-popup", "chooser tab is no longer live"))?;
        let directory = validated_directory(directory)?;
        if self.tabs[tab_index].pending {
            self.start_pane_at(ids, allowance, tab_index, &directory, operate)
                .map_err(|detail| action_error("session-start", detail))?;
            self.tabs[tab_index].directory = directory;
            self.tabs[tab_index].pending = false;
        } else {
            self.retarget(ids, tab_index, directory)?;
        }
        Ok(())
    }

    pub(crate) fn session_ended(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        session_id: &str,
        end: SessionEnd,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if let Some((tab_index, popup_index)) =
            self.tabs.iter().enumerate().find_map(|(tab, item)| {
                item.popups
                    .iter()
                    .position(|popup| popup.session.id == session_id)
                    .map(|popup| (tab, popup))
            })
        {
            let project = self.tabs[tab_index].popups[popup_index].entry == "project";
            self.remove_popup(tab_index, popup_index);
            if project && self.tabs[tab_index].pending && end == SessionEnd::Exited {
                self.finish_pending_tab(ids, allowance, tab_index, &mut operate)?;
            } else {
                self.remove_empty_tab(tab_index);
            }
            return Ok(());
        }
        let (tab_index, (column_index, pane_index)) = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(tab, item)| {
                item.find_pane(|pane| pane.session.id == session_id)
                    .map(|found| (tab, found))
            })
            .ok_or_else(|| {
                action_error(
                    "unknown-session",
                    format!("terminal {session_id} is not in the workspace"),
                )
            })?;
        let tab = &mut self.tabs[tab_index];
        let column = &mut tab.columns[column_index];
        column.panes.remove(pane_index);
        if column.panes.is_empty() {
            tab.columns.remove(column_index);
            tab.selected_column = if tab.columns.is_empty() {
                None
            } else {
                Some(index_after_removal(
                    tab.selected_column
                        .expect("a tab with columns has a selected column"),
                    column_index,
                    tab.columns.len(),
                ))
            };
        } else {
            column.expanded = index_after_removal(column.expanded, pane_index, column.panes.len());
        }
        self.remove_empty_tab(tab_index);
        Ok(())
    }

    pub(crate) fn transient_sessions(&self) -> Vec<String> {
        self.tabs
            .iter()
            .flat_map(|tab| &tab.popups)
            .filter(|popup| popup.entry == "project")
            .map(|popup| popup.session.id.clone())
            .collect()
    }

    pub(crate) fn prepare_close_tab(&mut self, id: &str) -> Result<Vec<String>, Failure> {
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .ok_or_else(|| action_error("unknown-tab", format!("tab {id} is not live")))?;
        if index != self.active {
            return Err(action_error(
                "not-active",
                format!("tab {id} is not the active tab"),
            ));
        }
        let sessions = self.tab_sessions(index);
        Ok(sessions)
    }

    fn tab_sessions(&self, index: usize) -> Vec<String> {
        self.tabs[index]
            .panes()
            .map(|pane| pane.session.id.clone())
            .chain(
                self.tabs[index]
                    .popups
                    .iter()
                    .map(|popup| popup.session.id.clone()),
            )
            .collect()
    }

    /// Validates closing one pane of the caller's tab, else of the active tab,
    /// and names the terminals to stop: the pane's, and when it is the final
    /// tab's last pane, every terminal of that tab, since the close removes the
    /// workspace, or ends the window when it is the last one.
    pub(crate) fn prepare_close_pane(
        &mut self,
        caller: Option<&str>,
        id: &str,
    ) -> Result<Vec<String>, Failure> {
        let tab_index = self.target_tab(caller)?.0;
        let tab = &self.tabs[tab_index];
        let Some((column, pane)) = tab.find_pane(|pane| pane.id == id) else {
            return Err(
                if self
                    .tabs
                    .iter()
                    .any(|tab| tab.find_pane(|pane| pane.id == id).is_some())
                {
                    let tab = if caller.is_some() {
                        "calling"
                    } else {
                        "active"
                    };
                    action_error("not-active", format!("pane {id} is not in the {tab} tab"))
                } else {
                    action_error("unknown-pane", format!("pane {id} is not live"))
                },
            );
        };
        // A covering popup hides the pane Alt+W would close, and closing the
        // tab's last pane would stop the popup too, so only the tab's own
        // terminals may close a named pane under it, and never its last.
        if tab.selected_popup.is_some() {
            if caller.is_none() {
                return Err(action_error(
                    "unavailable",
                    "close the popup before closing a pane",
                ));
            }
            if tab.panes().count() == 1 {
                return Err(action_error(
                    "unavailable",
                    "close the popup before closing the tab's last pane",
                ));
            }
        }
        let sessions = if self.tabs.len() == 1 && tab.panes().count() == 1 {
            self.tab_sessions(tab_index)
        } else {
            vec![tab.columns[column].panes[pane].session.id.clone()]
        };
        Ok(sessions)
    }

    fn start_pane(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let directory = self.tabs[tab_index].directory.clone();
        self.start_pane_at(ids, allowance, tab_index, &directory, operate)
    }

    fn start_column(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let directory = self.tabs[tab_index].directory.clone();
        self.start_pane_in(
            ids,
            allowance,
            tab_index,
            &directory,
            true,
            LaunchCommand::Pane,
            operate,
        )
    }

    fn start_pane_at(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        directory: &Path,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        self.start_pane_in(
            ids,
            allowance,
            tab_index,
            directory,
            false,
            LaunchCommand::Pane,
            operate,
        )
    }

    /// Starts a pane in the selected column, or in a new column right of it.
    #[allow(clippy::too_many_arguments)]
    fn start_pane_in(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        directory: &Path,
        new_column: bool,
        command: LaunchCommand,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check_session_capacity(allowance)?;
        if ids.next_pane == usize::MAX || ids.next_session == usize::MAX {
            return Err("workspace exhausted pane or terminal identities".into());
        }
        let pane = Pane {
            pinned_name: None,
            id: format!("p{}", ids.next_pane),
            session: ids.pane_session(),
        };
        operate(SessionOperation::Start {
            session: pane.session.clone(),
            directory: directory.to_path_buf(),
            command,
            tab: None,
        })?;
        ids.next_pane += 1;
        ids.next_session += 1;
        let tab = &mut self.tabs[tab_index];
        match tab.selected_column {
            Some(selected) if !new_column => {
                let column = &mut tab.columns[selected];
                column.panes.push(pane);
                column.expanded = column.panes.len() - 1;
            }
            selected => {
                let index = selected.map_or(0, |selected| selected + 1);
                tab.columns.insert(
                    index,
                    Column {
                        panes: vec![pane],
                        expanded: 0,
                    },
                );
                tab.selected_column = Some(index);
            }
        }
        Ok(())
    }

    fn finish_pending_tab(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.tabs[tab_index].pending = false;
        if let Err(detail) = self.start_pane(ids, allowance, tab_index, operate) {
            self.remove_empty_tab(tab_index);
            return Err(action_error("session-start", detail));
        }
        Ok(())
    }

    /// Refuses a caller's pane action, before anything changes, when dismissing
    /// its tab's popup would stop the tab's last terminal and remove the tab.
    fn require_surviving_dismissal(&self, tab_index: usize) -> Result<(), Failure> {
        let tab = &self.tabs[tab_index];
        let Some(selected) = &tab.selected_popup else {
            return Ok(());
        };
        let transient = tab
            .popups
            .iter()
            .find(|popup| &popup.id == selected)
            .map(|popup| self.definition(&popup.entry).map(|entry| !entry.keep_alive))
            .transpose()?
            .unwrap_or(false);
        if transient && tab.columns.is_empty() && tab.popups.len() == 1 && self.tabs.len() > 1 {
            return Err(action_error(
                "unavailable",
                "the popup is this tab's last terminal; close it before changing panes",
            ));
        }
        Ok(())
    }

    /// Dismisses the tab's popup before a pane action and returns the tab's
    /// index afterwards. Dismissing the last popup of a tab without panes
    /// removes it, and the action falls to the tab that became active.
    fn dismiss_for_pane_action(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<usize, Failure> {
        let Some(instance) = self.tabs[tab_index].selected_popup.clone() else {
            return Ok(tab_index);
        };
        let tab = self.tabs[tab_index].id.clone();
        self.dismiss_popup(
            ids,
            allowance,
            &PopupTarget {
                tab: tab.clone(),
                instance,
            },
            operate,
        )?;
        Ok(self
            .tabs
            .iter()
            .position(|item| item.id == tab)
            .unwrap_or(self.active))
    }

    fn focus_id(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        id: &str,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == id) {
            self.active = tab_index;
            return Ok(());
        }
        for tab_index in 0..self.tabs.len() {
            if let Some((column_index, pane_index)) =
                self.tabs[tab_index].find_pane(|pane| pane.id == id)
            {
                self.dismiss_for_pane_action(ids, allowance, tab_index, operate)?;
                self.active = tab_index;
                let tab = &mut self.tabs[tab_index];
                tab.selected_column = Some(column_index);
                tab.columns[column_index].expanded = pane_index;
                return Ok(());
            }
        }
        Err(action_error(
            "unknown-id",
            format!("{id} is not a live tab or pane identity"),
        ))
    }

    /// Focuses the exact pane or popup `target` of `tab` (EON-C31). A shown
    /// keep-alive popup that is not the target is hidden first; a shown
    /// transient one, a pending picker included, is left alone, and only the
    /// tab is selected.
    fn focus_attention(&mut self, tab: &str, target: &str) -> Result<(), Failure> {
        let stale = || action_error("stale-target", format!("{tab} holds no terminal {target}"));
        let tab_index = self
            .tabs
            .iter()
            .position(|item| item.id == tab)
            .ok_or_else(stale)?;
        let pane = self.tabs[tab_index].find_pane(|pane| pane.id == target);
        let popup = self.tabs[tab_index]
            .popups
            .iter()
            .any(|popup| popup.id == target);
        if pane.is_none() && !popup {
            return Err(stale());
        }
        let covered = self.displaced_transient(tab_index, Some(target))?.is_some();
        self.active = tab_index;
        if covered {
            return Ok(());
        }
        let tab = &mut self.tabs[tab_index];
        match pane {
            Some((column, pane)) => {
                tab.selected_popup = None;
                tab.selected_column = Some(column);
                tab.columns[column].expanded = pane;
            }
            None => tab.selected_popup = Some(target.into()),
        }
        Ok(())
    }

    fn focus_direction(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        direction: Direction,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match direction {
            Direction::Left | Direction::Right => {
                // Visible columns and tabs form one strip: the outer columns continue to
                // the adjacent tab, entering at its nearest edge column.
                let shows_columns = |tab: &Tab| !tab.pending && tab.selected_popup.is_none();
                let tab = &mut self.tabs[self.active];
                let neighbor =
                    tab.selected_column
                        .filter(|_| shows_columns(tab))
                        .and_then(|selected| {
                            if direction == Direction::Left {
                                selected.checked_sub(1)
                            } else {
                                Some(selected + 1).filter(|next| *next < tab.columns.len())
                            }
                        });
                if let Some(neighbor) = neighbor {
                    tab.selected_column = Some(neighbor);
                    return Ok(());
                }
                self.active = if direction == Direction::Left {
                    self.active.checked_sub(1).unwrap_or(self.tabs.len() - 1)
                } else {
                    (self.active + 1) % self.tabs.len()
                };
                let tab = &mut self.tabs[self.active];
                if !tab.columns.is_empty() && shows_columns(tab) {
                    tab.selected_column = Some(if direction == Direction::Left {
                        tab.columns.len() - 1
                    } else {
                        0
                    });
                }
            }
            Direction::Up | Direction::Down => {
                self.require_established(self.active)?;
                let active = self.dismiss_for_pane_action(ids, allowance, self.active, operate)?;
                let tab = &mut self.tabs[active];
                let selected = tab
                    .selected_column
                    .ok_or_else(|| action_error("unavailable", "active tab has no pane"))?;
                let column = &mut tab.columns[selected];
                column.expanded = if direction == Direction::Up {
                    column
                        .expanded
                        .checked_sub(1)
                        .unwrap_or(column.panes.len() - 1)
                } else {
                    (column.expanded + 1) % column.panes.len()
                };
            }
        }
        Ok(())
    }

    fn move_direction(&mut self, tab_index: usize, direction: Direction) -> Result<(), Failure> {
        match direction {
            Direction::Left | Direction::Right => {
                let neighbor = if direction == Direction::Left {
                    tab_index.checked_sub(1)
                } else {
                    Some(tab_index + 1).filter(|next| *next < self.tabs.len())
                };
                if let Some(neighbor) = neighbor {
                    self.tabs.swap(tab_index, neighbor);
                    // The active tab stays active wherever it moves.
                    if self.active == tab_index {
                        self.active = neighbor;
                    } else if self.active == neighbor {
                        self.active = tab_index;
                    }
                }
            }
            Direction::Up | Direction::Down => {
                self.require_established(tab_index)?;
                let tab = &mut self.tabs[tab_index];
                if tab.selected_popup.is_some() {
                    return Err(action_error(
                        "unavailable",
                        "close the popup before moving a pane up or down",
                    ));
                }
                let column = tab
                    .selected_column
                    .ok_or_else(|| action_error("unavailable", "tab has no pane"))?;
                let column = &mut tab.columns[column];
                let selected = column.expanded;
                let target = if direction == Direction::Up {
                    let Some(target) = selected.checked_sub(1) else {
                        return Ok(());
                    };
                    target
                } else {
                    if selected + 1 == column.panes.len() {
                        return Ok(());
                    }
                    selected + 1
                };
                column.panes.swap(selected, target);
                column.expanded = target;
            }
        }
        Ok(())
    }

    /// Moves the selected pane into the neighbouring column's stack, below its
    /// expanded pane; past the outer column a stacked pane opens a new column.
    fn move_pane_sideways(&mut self, tab_index: usize, side: Side) -> Result<(), Failure> {
        self.require_established(tab_index)?;
        let tab = &mut self.tabs[tab_index];
        if tab.selected_popup.is_some() {
            return Err(action_error(
                "unavailable",
                "close the popup before moving a pane sideways",
            ));
        }
        let source = tab
            .selected_column
            .ok_or_else(|| action_error("unavailable", "tab has no pane"))?;
        let neighbor = match side {
            Side::Left => source.checked_sub(1),
            Side::Right => Some(source + 1).filter(|next| *next < tab.columns.len()),
        };
        let stacked = tab.columns[source].panes.len() > 1;
        if neighbor.is_none() && (!stacked || tab.columns.len() >= MAX_COLUMNS) {
            return Ok(());
        }
        let column = &mut tab.columns[source];
        let pane = column.panes.remove(column.expanded);
        if stacked {
            column.expanded =
                index_after_removal(column.expanded, column.expanded, column.panes.len());
        } else {
            tab.columns.remove(source);
        }
        let selected = match neighbor {
            Some(target) => {
                let target = if !stacked && target > source {
                    target - 1
                } else {
                    target
                };
                let column = &mut tab.columns[target];
                column.expanded += 1;
                column.panes.insert(column.expanded, pane);
                target
            }
            None => {
                let index = match side {
                    Side::Left => 0,
                    Side::Right => tab.columns.len(),
                };
                tab.columns.insert(
                    index,
                    Column {
                        panes: vec![pane],
                        expanded: 0,
                    },
                );
                index
            }
        };
        tab.selected_column = Some(selected);
        Ok(())
    }

    fn set_tab_directory(
        &mut self,
        ids: &mut IdAllocator,
        id: &str,
        directory: Vec<u8>,
    ) -> Result<(), Failure> {
        if tab_number(id).is_none() {
            return Err(action_error(
                "invalid-tab",
                "tab directory targets require a canonical tN identity",
            ));
        }
        let tab_index = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .ok_or_else(|| action_error("unknown-tab", format!("tab {id} is not live")))?;
        if self.tabs[tab_index].pending {
            return Err(action_error(
                "choose-directory-first",
                "the pending tab requires its exact Project chooser result",
            ));
        }
        self.retarget(ids, tab_index, validated_directory(directory)?)
    }

    /// Changes an established tab's directory and parks a live Agent launched
    /// elsewhere as an ordinary pane, keeping its terminal: at the end of the
    /// selected column, expanded and selected when its popup was shown, or as
    /// the first column of a tab without panes.
    fn retarget(
        &mut self,
        ids: &mut IdAllocator,
        tab_index: usize,
        directory: PathBuf,
    ) -> Result<(), Failure> {
        let tab = &self.tabs[tab_index];
        let parked = tab
            .popups
            .iter()
            .position(|popup| popup.entry == "agent" && popup.launch_directory != directory);
        if parked.is_some() && ids.next_pane == usize::MAX {
            return Err(action_error(
                "capacity",
                "workspace exhausted pane identities",
            ));
        }
        let tab = &mut self.tabs[tab_index];
        tab.directory = directory;
        let Some(index) = parked else {
            return Ok(());
        };
        let popup = tab.popups.remove(index);
        let shown = tab.selected_popup.as_deref() == Some(&popup.id);
        if shown {
            tab.selected_popup = None;
        }
        let pane = Pane {
            pinned_name: None,
            id: format!("p{}", ids.next_pane),
            session: popup.session,
        };
        ids.next_pane += 1;
        match tab.selected_column {
            Some(selected) => {
                let column = &mut tab.columns[selected];
                column.panes.push(pane);
                if shown {
                    column.expanded = column.panes.len() - 1;
                }
            }
            None => {
                tab.columns.push(Column {
                    panes: vec![pane],
                    expanded: 0,
                });
                tab.selected_column = Some(0);
            }
        }
        Ok(())
    }

    fn definition(&self, id: &str) -> Result<&PopupDefinition, Failure> {
        self.catalog
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| action_error("unknown-popup", format!("popup entry {id} is disabled")))
    }

    fn remove_popup(&mut self, tab_index: usize, popup_index: usize) {
        let removed = self.tabs[tab_index].popups.remove(popup_index);
        if self.tabs[tab_index].selected_popup.as_deref() == Some(&removed.id) {
            self.tabs[tab_index].selected_popup = None;
        }
    }

    fn stop_popup(
        &mut self,
        tab_index: usize,
        popup_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let session = self.tabs[tab_index].popups[popup_index].session.id.clone();
        operate(SessionOperation::Stop(session))
            .map_err(|detail| action_error("popup-stop", detail))?;
        self.remove_popup(tab_index, popup_index);
        Ok(())
    }

    fn remove_empty_tab(&mut self, tab_index: usize) {
        if self.tabs[tab_index].columns.is_empty()
            && self.tabs[tab_index].popups.is_empty()
            && self.tabs.len() > 1
        {
            self.tabs.remove(tab_index);
            self.active = index_after_removal(self.active, tab_index, self.tabs.len());
        }
    }

    /// The implicit target tab: the active one, or the tab holding the caller's
    /// terminal, with the caller's column and pane when it is a pane.
    fn target_tab(&self, caller: Option<&str>) -> Result<(usize, Option<(usize, usize)>), Failure> {
        let Some(session) = caller else {
            return Ok((self.active, None));
        };
        for (index, tab) in self.tabs.iter().enumerate() {
            if let Some(pane) = tab.find_pane(|pane| pane.session.id == session) {
                return Ok((index, Some(pane)));
            }
            if tab.popups.iter().any(|popup| popup.session.id == session) {
                return Ok((index, None));
            }
        }
        Err(action_error(
            "unknown-caller",
            "the calling terminal is not part of this workspace",
        ))
    }

    /// Makes a calling pane its tab's selected pane, so pane actions apply to
    /// it, and returns the selection it replaced.
    fn select(&mut self, tab_index: usize, (column, pane): (usize, usize)) -> Selection {
        let tab = &mut self.tabs[tab_index];
        let previous = Selection {
            tab: tab.id.clone(),
            column: tab.selected_column,
            expanded: (column, tab.columns[column].expanded),
        };
        tab.selected_column = Some(column);
        tab.columns[column].expanded = pane;
        previous
    }

    /// Restores a selection after the caller's action failed or added an
    /// unfocused pane.
    fn restore(&mut self, previous: Selection) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == previous.tab) else {
            return;
        };
        let (column, expanded) = previous.expanded;
        if let Some(item) = tab.columns.get_mut(column)
            && expanded < item.panes.len()
        {
            item.expanded = expanded;
        }
        if previous
            .column
            .is_none_or(|selected| selected < tab.columns.len())
        {
            tab.selected_column = previous.column;
        }
    }

    fn require_established(&self, tab_index: usize) -> Result<(), Failure> {
        if self.tabs[tab_index].pending {
            Err(action_error(
                "choose-directory-first",
                "choose the pending tab directory before changing panes",
            ))
        } else {
            Ok(())
        }
    }

    fn check_session_capacity(&self, allowance: usize) -> Result<(), String> {
        if self.session_count() >= allowance {
            Err(format!("workspace is limited to {MAX_SESSIONS} terminals"))
        } else {
            Ok(())
        }
    }

    fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.tabs.iter().flat_map(|tab| {
            tab.panes()
                .map(|pane| &pane.session)
                .chain(tab.popups.iter().map(|popup| &popup.session))
        })
    }

    fn session_count(&self) -> usize {
        self.tabs
            .iter()
            .map(|tab| tab.panes().count() + tab.popups.len())
            .sum()
    }
}

/// Each pane with its zero-based column and whether that column expands it.
fn column_panes(tab: &SnapshotTab) -> impl Iterator<Item = (usize, bool, &SnapshotPane)> {
    tab.columns.iter().enumerate().flat_map(|(index, column)| {
        column
            .panes
            .iter()
            .map(move |pane| (index, pane.id == column.expanded, pane))
    })
}

/// The CLI's name for an attention state.
fn attention_name(state: AttentionState) -> &'static str {
    match state {
        AttentionState::NeedsInput => "needs-input",
        AttentionState::Error => "error",
        AttentionState::Finished => "finished",
        AttentionState::Working => "working",
        AttentionState::Idle => "idle",
        AttentionState::Unavailable => "unavailable",
    }
}

/// A terminal's attention state, app and name, the only attention values the
/// CLI projects (EON-C31); the program's report text never leaves EONW. An
/// empty app or name is none.
fn attention_fields(
    attention: Option<&Attention>,
) -> (Option<&'static str>, Option<&str>, Option<&str>) {
    let record = attention.and_then(|attention| attention.record.as_ref());
    let nonempty = |text: &&str| !text.is_empty();
    (
        attention.map(|attention| attention_name(attention.state)),
        record.map(|record| record.app.as_str()).filter(nonempty),
        record.map(|record| record.name.as_str()).filter(nonempty),
    )
}

fn sidebar_name(sidebar: Sidebar) -> &'static str {
    match sidebar {
        Sidebar::Open => "open",
        Sidebar::Closed => "closed",
        Sidebar::Off => "off",
    }
}

fn human_attention(attention: Option<&Attention>) -> String {
    let (state, app, name) = attention_fields(attention);
    format!(
        "attention={} app={} name={}",
        state.unwrap_or("none"),
        app.unwrap_or("none").escape_debug(),
        name.unwrap_or("none").escape_debug()
    )
}

pub(crate) fn human(snapshot: &Snapshot) -> String {
    let mut output = format!(
        "active {}\nsidebar {}\n",
        snapshot.active_tab,
        sidebar_name(snapshot.sidebar)
    );
    for tab in &snapshot.tabs {
        output.push_str(&format!(
            "tab {} directory={} selected={} popup={} pending={} active={} attention={}\n",
            tab.id,
            String::from_utf8_lossy(&tab.directory).escape_debug(),
            tab.selected_pane.as_deref().unwrap_or("none"),
            tab.selected_popup.as_deref().unwrap_or("none"),
            tab.pending,
            tab.id == snapshot.active_tab,
            tab.attention_rollup.map_or("none", attention_name),
        ));
        for (column, expanded, pane) in column_panes(tab) {
            output.push_str(&format!(
                "  pane {} column={} expanded={} terminal={} live={} endpoint={} label={} {}\n",
                pane.id,
                column + 1,
                expanded,
                pane.session,
                pane.live,
                String::from_utf8_lossy(&pane.endpoint).escape_debug(),
                pane.label.escape_debug(),
                human_attention(pane.attention.as_ref()),
            ));
        }
        for popup in &tab.popups {
            let entry = snapshot
                .entries
                .iter()
                .find(|entry| entry.id == popup.entry);
            output.push_str(&format!(
                "  popup {} entry={} label={} key={} terminal={} chosen={} endpoint={} {}\n",
                popup.id,
                popup.entry,
                entry.map_or("?", |entry| entry.label.as_str()),
                entry.map_or_else(
                    || "?".into(),
                    |entry| entry
                        .shortcut
                        .bound()
                        .map_or_else(|| "none".into(), shortcut_text)
                ),
                popup.session,
                tab.selected_popup.as_deref() == Some(&popup.id),
                String::from_utf8_lossy(&popup.endpoint).escape_debug(),
                human_attention(popup.attention.as_ref()),
            ));
        }
    }
    output
}

pub(crate) fn json(snapshot: &Snapshot) -> String {
    let mut output = format!(
        "{{\"active_tab\":\"{}\",\"geometry\":{{\"side_margin\":{},\"vertical_margin\":{}}},\"entries\":[",
        json_escape(&snapshot.active_tab),
        snapshot.geometry.side_margin,
        snapshot.geometry.vertical_margin
    );
    for (index, entry) in snapshot.entries.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"label\":\"{}\",\"keybinding\":",
            json_escape(&entry.id),
            json_escape(&entry.label),
        ));
        let (keybinding, overridden_by, default) = match &entry.shortcut {
            PopupKey::Bound(shortcut) => (Some(shortcut_text(shortcut)), None, None),
            PopupKey::Unbound => (None, None, None),
            PopupKey::Overridden { default, by } => {
                (None, Some(by.as_str()), Some(shortcut_text(default)))
            }
        };
        json_option(&mut output, keybinding.as_deref());
        output.push_str(",\"overridden_by\":");
        json_option(&mut output, overridden_by);
        output.push_str(",\"default_keybinding\":");
        json_option(&mut output, default.as_deref());
        output.push('}');
    }
    output.push_str("],\"tabs\":[");
    for (tab_index, tab) in snapshot.tabs.iter().enumerate() {
        if tab_index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"directory\":{},\"pending\":{},\"selected_pane\":",
            json_escape(&tab.id),
            json_path(&tab.directory),
            tab.pending
        ));
        json_option(&mut output, tab.selected_pane.as_deref());
        output.push_str(",\"selected_popup\":");
        json_option(&mut output, tab.selected_popup.as_deref());
        output.push_str(",\"attention_rollup\":");
        json_option(&mut output, tab.attention_rollup.map(attention_name));
        output.push_str(",\"panes\":[");
        for (index, (column, expanded, pane)) in column_panes(tab).enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"column\":{},\"expanded\":{},\"session\":\"{}\",\"endpoint\":{},\"live\":{},\"label\":\"{}\",\"pinned_name\":",
                json_escape(&pane.id),
                column + 1,
                expanded,
                json_escape(&pane.session),
                json_path(&pane.endpoint),
                pane.live,
                json_escape(&pane.label),
            ));
            json_option(&mut output, pane.pinned_name.as_deref());
            output.push_str(",\"attention\":");
            json_attention(&mut output, pane.attention.as_ref());
            output.push('}');
        }
        output.push_str("],\"popups\":[");
        for (index, popup) in tab.popups.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"entry\":\"{}\",\"session\":\"{}\",\"endpoint\":{},\"chosen\":{},\"attention\":",
                json_escape(&popup.id), json_escape(&popup.entry), json_escape(&popup.session),
                json_path(&popup.endpoint), tab.selected_popup.as_deref() == Some(&popup.id)
            ));
            json_attention(&mut output, popup.attention.as_ref());
            output.push('}');
        }
        output.push_str("]}");
    }
    output.push_str(&format!(
        "],\"sidebar\":\"{}\"}}\n",
        sidebar_name(snapshot.sidebar)
    ));
    output
}

/// `{"state":...,"app":...,"name":...}`, or `null` without attention.
fn json_attention(output: &mut String, attention: Option<&Attention>) {
    let (Some(state), app, name) = attention_fields(attention) else {
        output.push_str("null");
        return;
    };
    output.push_str(&format!("{{\"state\":\"{state}\",\"app\":"));
    json_option(output, app);
    output.push_str(",\"name\":");
    json_option(output, name);
    output.push('}');
}

/// The window's workspaces in order, one line each (`eon workspaces`).
pub(crate) fn list_human(snapshot: &Snapshot) -> String {
    snapshot
        .workspaces
        .iter()
        .map(|workspace| {
            format!(
                "{} shown={} tabs={} attention={} recency={} name={}\n",
                workspace.id,
                workspace.id == snapshot.active_workspace,
                workspace.tab_count,
                workspace.attention.map_or("none", attention_name),
                workspace.recency,
                workspace
                    .name
                    .as_ref()
                    .map_or_else(|| "none".into(), |name| format!("{name:?}")),
            )
        })
        .collect()
}

/// `{"workspaces":[{"id":...,"name":...,"shown":...,"tabs":...,"attention":...,"recency":...}]}`
/// (`eon workspaces --json`).
pub(crate) fn list_json(snapshot: &Snapshot) -> String {
    let mut output = String::from("{\"workspaces\":[");
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"name\":",
            json_escape(&workspace.id)
        ));
        json_option(&mut output, workspace.name.as_deref());
        output.push_str(&format!(
            ",\"shown\":{},\"tabs\":{},\"attention\":",
            workspace.id == snapshot.active_workspace,
            workspace.tab_count
        ));
        json_option(&mut output, workspace.attention.map(attention_name));
        output.push_str(&format!(",\"recency\":{}}}", workspace.recency));
    }
    output.push_str("]}\n");
    output
}

pub(crate) fn failure_human(failure: &Failure) -> String {
    format!(
        "error {}: {}\n",
        failure.code,
        failure.detail.escape_debug()
    )
}

pub(crate) fn failure_json(failure: &Failure) -> String {
    format!(
        "{{\"error\":{{\"code\":\"{}\",\"detail\":\"{}\"}}}}\n",
        json_escape(&failure.code),
        json_escape(&failure.detail)
    )
}

pub(crate) fn validate_initial_directory(directory: &Path) -> Result<(), String> {
    let bytes = directory.as_os_str().as_bytes();
    if !directory.is_absolute() || bytes.len() > MAX_DIRECTORY_BYTES || bytes.contains(&0) {
        return Err("Eon launch directory must be one bounded absolute Unix path".into());
    }
    if !fs::metadata(directory)
        .map_err(|error| format!("cannot inspect Eon launch directory: {error}"))?
        .is_dir()
    {
        return Err("Eon launch directory is not a directory".into());
    }
    Ok(())
}

fn validated_directory(directory: Vec<u8>) -> Result<PathBuf, Failure> {
    if directory.is_empty()
        || directory.len() > MAX_DIRECTORY_BYTES
        || directory.contains(&0)
        || directory[0] != b'/'
    {
        return Err(action_error(
            "invalid-directory",
            "tab launch directory must be one bounded absolute Unix path",
        ));
    }
    let directory = PathBuf::from(OsString::from_vec(directory));
    if !fs::metadata(&directory).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(action_error(
            "invalid-directory",
            "tab launch directory is unavailable or not a directory",
        ));
    }
    Ok(directory)
}

fn tab_number(value: &str) -> Option<usize> {
    value.strip_prefix('t').and_then(canonical_number)
}

fn canonical_number(value: &str) -> Option<usize> {
    if value.starts_with('0') || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|number| *number > 0 && number.to_string() == value)
}

fn index_after_removal(selected: usize, removed: usize, remaining: usize) -> usize {
    if selected > removed {
        selected - 1
    } else {
        selected.min(remaining.saturating_sub(1))
    }
}

/// Resolve a workspace selector over `(wN, name)` pairs in order (EON-C33):
/// an exact `wN`, else the one workspace with that exact name. The CLI's
/// close prompt applies it to a snapshot's summaries.
pub(crate) fn resolve_selector<'a>(
    workspaces: impl Iterator<Item = (&'a str, Option<&'a str>)>,
    target: &str,
) -> Result<usize, Failure> {
    let workspaces = workspaces.collect::<Vec<_>>();
    if let Some(index) = workspaces.iter().position(|(id, _)| *id == target) {
        return Ok(index);
    }
    let matches = workspaces
        .iter()
        .enumerate()
        .filter(|(_, (_, name))| *name == Some(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err(action_error(
            "unknown-workspace",
            format!("workspace {target} is not live"),
        )),
        _ => Err(action_error(
            "ambiguous-workspace",
            format!(
                "workspace {target} matches {}",
                matches
                    .iter()
                    .map(|(_, (id, _))| *id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

fn action_error(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        detail: detail.into(),
    }
}

fn shortcut_text(shortcut: &eon_workspace_protocol::v22::Shortcut) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (eon_workspace_protocol::v22::CTRL, "Ctrl"),
        (eon_workspace_protocol::v22::ALT, "Alt"),
        (eon_workspace_protocol::v22::SHIFT, "Shift"),
        (eon_workspace_protocol::v22::SUPER, "Super"),
    ] {
        if shortcut.modifiers & bit != 0 {
            parts.push(name.to_string());
        }
    }
    parts.push(
        shortcut
            .key
            .strip_prefix("Key")
            .or_else(|| shortcut.key.strip_prefix("Digit"))
            .unwrap_or(&shortcut.key)
            .to_string(),
    );
    parts.join("+")
}

fn json_option(output: &mut String, value: Option<&str>) {
    match value {
        Some(value) => output.push_str(&format!("\"{}\"", json_escape(value))),
        None => output.push_str("null"),
    }
}

/// UTF-8 paths become JSON strings; other paths keep their exact bytes as an array.
fn json_path(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(path) => format!("\"{}\"", json_escape(path)),
        Err(_) => format!(
            "[{}]",
            bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

pub(crate) fn json_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v22::{ALT, PopupGeometry, Shortcut};

    fn two_workspace_set(
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> WorkspaceSet {
        let mut set = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            &mut operate,
        )
        .unwrap();
        set.insert_pending(
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            &mut operate,
        )
        .unwrap();
        for index in 0..2 {
            let picker = set.workspaces[index].transient_sessions()[0].clone();
            set.shown = index;
            set.session_ended(&picker, SessionEnd::Exited, &mut operate)
                .unwrap();
        }
        set.shown = 0;
        set
    }

    fn invoke(tab: &str, entry: &str) -> Action {
        Action::InvokePopup {
            tab: tab.into(),
            entry: entry.into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        }
    }

    #[test]
    fn pane_pin_targets_hidden_tabs_without_selection_changes_and_follows_moves() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "tab",
            None,
            Action::Workspace(WorkspaceAction::CreateTab),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let before = set.snapshot();
        set.dispatch(
            "name",
            None,
            Action::RenamePane {
                pane: "p1".into(),
                name: Some("  Review  ".into()),
            },
            prepared,
            |_| panic!("rename runs no terminal operation"),
        )
        .unwrap();
        let named = set.snapshot();
        assert_eq!(named.active_tab, before.active_tab);
        assert_eq!(named.active_workspace, before.active_workspace);
        for (before, after) in before.tabs.iter().zip(&named.tabs) {
            assert_eq!(before.selected_pane, after.selected_pane);
            assert_eq!(before.selected_popup, after.selected_popup);
        }
        let pin = |set: &WorkspaceSet| {
            set.snapshot()
                .tabs
                .iter()
                .flat_map(|tab| tab.panes())
                .find(|pane| pane.id == "p1")
                .unwrap()
                .pinned_name
                .clone()
        };
        assert_eq!(pin(&set).as_deref(), Some("Review"));
        assert!(json(&named).contains("\"pinned_name\":\"Review\""));
        assert!(human(&named).contains("Review"));
        for (id, name, expected) in [
            ("p99", "ok", "unknown-pane"),
            ("p1", "bad\u{202e}name", "malformed-action"),
        ] {
            let before = set.clone();
            assert_eq!(
                set.dispatch(
                    "bad",
                    None,
                    Action::RenamePane {
                        pane: id.into(),
                        name: Some(name.into())
                    },
                    prepared,
                    |_| Ok(())
                )
                .unwrap_err()
                .code,
                expected
            );
            assert_eq!(set, before);
        }
        set.dispatch(
            "focus",
            None,
            Action::Workspace(WorkspaceAction::FocusId("p1".into())),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "stack",
            None,
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "focus-again",
            None,
            Action::Workspace(WorkspaceAction::FocusId("p1".into())),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "column",
            None,
            Action::MovePaneSideways(Side::Right),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "tab-move",
            None,
            Action::Workspace(WorkspaceAction::Move(Direction::Right)),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(pin(&set).as_deref(), Some("Review"));
        set.dispatch(
            "clear",
            None,
            Action::RenamePane {
                pane: "p1".into(),
                name: Some("  ".into()),
            },
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(pin(&set), None);
        set.dispatch(
            "name-again",
            None,
            Action::RenamePane {
                pane: "p1".into(),
                name: Some("Again".into()),
            },
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let session = set
            .snapshot()
            .tabs
            .iter()
            .flat_map(|tab| tab.panes())
            .find(|pane| pane.id == "p1")
            .unwrap()
            .session
            .clone();
        set.session_ended(&session, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        assert!(
            !set.snapshot()
                .tabs
                .iter()
                .flat_map(|tab| tab.panes())
                .any(|pane| pane.id == "p1")
        );
    }

    #[test]
    fn workspace_requests_keep_order_projection_names_and_bounds() {
        let mut set = two_workspace_set(|_| Ok(()));
        let caller = set.workspaces[0].sessions().next().unwrap().id.clone();
        let original = set.snapshot_in(Some("w1")).unwrap().tabs;
        let mut apply = |id: &str, action| {
            set.dispatch(
                id,
                Some(&caller),
                Action::Workspace(action),
                prepared,
                |_| Ok(()),
            )
        };
        apply(
            "new",
            WorkspaceAction::NewWorkspace {
                name: Some("work".into()),
                focus: true,
            },
        )
        .unwrap();
        // Replaying a creation neither allocates another workspace nor starts a picker.
        assert_eq!(
            apply(
                "new",
                WorkspaceAction::NewWorkspace {
                    name: None,
                    focus: true
                }
            )
            .unwrap_err()
            .code,
            "duplicate-request"
        );
        apply(
            "background",
            WorkspaceAction::NewWorkspace {
                name: Some("work".into()),
                focus: false,
            },
        )
        .unwrap();
        let shown = set.snapshot();
        assert_eq!(shown.workspace, "w3");
        assert_eq!(
            shown
                .workspaces
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["w1", "w3", "w4", "w2"]
        );
        assert!(shown.tabs[0].pending);
        assert_eq!(shown.tabs[0].popups[0].entry, "project");
        let caller_view = set
            .snapshot_in(set.caller_workspace_id(Some(&caller)).unwrap().as_deref())
            .unwrap();
        assert_eq!(
            (
                caller_view.workspace.as_str(),
                caller_view.active_workspace.as_str()
            ),
            ("w1", "w3")
        );
        assert_eq!(caller_view.tabs, original);
        eon_workspace_protocol::v22::encode_response(
            &eon_workspace_protocol::v22::Response::Snapshot(caller_view),
        )
        .unwrap();

        let before = set.clone();
        let error = set
            .dispatch(
                "ambiguous",
                None,
                Action::Workspace(WorkspaceAction::SwitchWorkspace {
                    workspace: "work".into(),
                }),
                prepared,
                |_| panic!("a switch starts nothing"),
            )
            .unwrap_err();
        assert_eq!(error.code, "ambiguous-workspace");
        assert!(error.detail.contains("w3") && error.detail.contains("w4"));
        assert_eq!(set, before);
        set.dispatch(
            "same",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: "w3".into(),
            }),
            prepared,
            |_| panic!("a switch starts nothing"),
        )
        .unwrap();
        assert_eq!(set.snapshot(), shown);
        set.dispatch(
            "rename",
            Some(&caller),
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: "w3".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            set.resolve_workspace("w3").unwrap(),
            1,
            "an ID wins over another workspace's name"
        );
        set.workspaces[0].name = Some("w99".into());
        assert_eq!(
            set.resolve_workspace("w99").unwrap(),
            0,
            "an absent ID falls through to an exact name"
        );
        set.workspaces[0].name = Some("w01".into());
        assert_eq!(
            set.resolve_workspace("w01").unwrap(),
            0,
            "a noncanonical ID spelling can be a name"
        );
        set.dispatch(
            "clear",
            Some(&caller),
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: String::new(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(set.workspaces[0].name, None);
        for target in ["missing", "w999"] {
            assert_eq!(
                set.resolve_workspace(target).unwrap_err().code,
                "unknown-workspace"
            );
        }
        assert_eq!(
            set.dispatch(
                "bad-name",
                None,
                Action::Workspace(WorkspaceAction::RenameWorkspace {
                    workspace: None,
                    name: "bad\nname".into()
                }),
                prepared,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "malformed-action"
        );
        while set.workspaces.len() < eon_workspace_protocol::v22::MAX_WORKSPACES {
            set.dispatch(
                &format!("new-{}", set.workspaces.len()),
                None,
                Action::Workspace(WorkspaceAction::NewWorkspace {
                    name: None,
                    focus: false,
                }),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        }
        let before = set.clone();
        assert_eq!(
            set.dispatch(
                "limit",
                None,
                Action::Workspace(WorkspaceAction::NewWorkspace {
                    name: None,
                    focus: false
                }),
                prepared,
                |_| panic!("a refused creation starts nothing")
            )
            .unwrap_err()
            .code,
            "workspace-limit"
        );
        assert_eq!(set, before);
    }

    #[test]
    fn popup_dismissal_prunes_an_empty_workspace_and_final_emptying_keeps_an_owner() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch("project", None, invoke("t1", "project"), prepared, |_| {
            Ok(())
        })
        .unwrap();
        let tab = &set.snapshot().tabs[0];
        let pane = tab.panes().next().unwrap().session.clone();
        let popup = tab.popups[0].id.clone();
        set.session_ended(&pane, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        set.dispatch(
            "dismiss",
            None,
            Action::DismissPopup(PopupTarget {
                tab: "t1".into(),
                instance: popup,
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let snapshot = set.snapshot();
        eon_workspace_protocol::v22::encode_response(
            &eon_workspace_protocol::v22::Response::Snapshot(snapshot.clone()),
        )
        .unwrap();
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspace, "w2");

        let mut last_exit = set.clone();
        let last_terminal = last_exit.workspaces[0]
            .sessions()
            .next()
            .unwrap()
            .id
            .clone();
        last_exit
            .session_ended(&last_terminal, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        assert!(!last_exit.has_sessions());
        assert_eq!(
            last_exit.snapshot().workspaces.len(),
            1,
            "the supervisor needs its last owner until Ended"
        );
    }

    #[test]
    fn failed_pending_picker_exit_prunes_the_workspace() {
        assert_failed_pending_picker_prunes_workspace(true);
    }

    #[test]
    fn failed_pending_picker_dismissal_prunes_the_workspace() {
        assert_failed_pending_picker_prunes_workspace(false);
    }

    fn assert_failed_pending_picker_prunes_workspace(exited: bool) {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "new",
            None,
            Action::Workspace(WorkspaceAction::NewWorkspace {
                name: None,
                focus: true,
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let pending = set.snapshot();
        assert_eq!(pending.workspace, "w3");
        let popup = &pending.tabs[0].popups[0];
        let fail_start = |operation| match operation {
            SessionOperation::Start { .. } => Err("start failed".into()),
            SessionOperation::Stop(_) => Ok(()),
        };
        let error = if exited {
            set.session_ended(&popup.session, SessionEnd::Exited, fail_start)
        } else {
            set.dispatch(
                "dismiss",
                None,
                Action::DismissPopup(PopupTarget {
                    tab: pending.active_tab,
                    instance: popup.id.clone(),
                }),
                prepared,
                fail_start,
            )
        }
        .unwrap_err();
        assert_eq!(error.code, "session-start");
        assert_eq!(error.detail, "start failed");
        let snapshot = set.snapshot();
        eon_workspace_protocol::v22::encode_response(
            &eon_workspace_protocol::v22::Response::Snapshot(snapshot.clone()),
        )
        .unwrap();
        assert_eq!(snapshot.workspace, "w1");
        assert_eq!(snapshot.workspaces.len(), 2);
    }

    #[test]
    fn workspace_requests_resolve_id_shaped_names_after_live_ids() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some("w1".into()),
                name: "w2".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "live-id",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: "w2".into(),
            }),
            prepared,
            |_| panic!("switching starts nothing"),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "w2");
        set.dispatch(
            "absent-id-name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some("w1".into()),
                name: "w99".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "switch-name",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: "w99".into(),
            }),
            prepared,
            |_| panic!("switching starts nothing"),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "w1");
        set.dispatch(
            "rename-name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some("w99".into()),
                name: "w100".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let expected = set.workspaces[0]
            .sessions()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            set.prepare_close_workspace("close-name", None, "w100")
                .unwrap(),
            expected
        );
    }

    #[test]
    fn workspace_recency_ranks_the_shown_first_then_the_most_recently_shown() {
        let mut set = two_workspace_set(|_| Ok(()));
        let ranks = |set: &WorkspaceSet| {
            set.snapshot()
                .workspaces
                .iter()
                .map(|workspace| (workspace.id.clone(), workspace.recency))
                .collect::<Vec<_>>()
        };
        let new = Action::Workspace(WorkspaceAction::NewWorkspace {
            name: None,
            focus: false,
        });
        set.dispatch("new", None, new, prepared, |_| Ok(()))
            .unwrap();
        // A workspace never shown ranks last.
        assert_eq!(
            ranks(&set),
            [("w1".into(), 0), ("w3".into(), 2), ("w2".into(), 1)]
        );
        for (request, workspace) in [("third", "w3"), ("second", "w2")] {
            let switch = Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: workspace.into(),
            });
            set.dispatch(request, None, switch, prepared, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(
            ranks(&set),
            [("w1".into(), 2), ("w3".into(), 1), ("w2".into(), 0)]
        );
    }

    #[test]
    fn only_a_queue_jump_crosses_workspaces_and_it_shows_its_target() {
        let mut set = two_workspace_set(|_| Ok(()));
        let hidden = set.snapshot_in(Some("w2")).unwrap();
        let tab = hidden.tabs[0].id.clone();
        let pane = hidden.tabs[0].panes().next().unwrap().id.clone();
        let focus = Action::FocusAttention {
            tab: tab.clone(),
            target: pane.clone(),
        };
        let error = set
            .dispatch("row", None, focus, prepared, |_| Ok(()))
            .unwrap_err();
        assert_eq!(
            error.code, "stale-target",
            "a sidebar row stays in its workspace"
        );
        assert_eq!(set.snapshot().workspace, "w1");

        set.jump("jump", None, "w2", &tab, &pane).unwrap();
        assert_eq!(set.snapshot().workspace, "w2");
        assert_eq!(set.recent_workspaces, ["w2", "w1"]);
        assert_eq!(
            set.jump("jump", None, "w2", &tab, &pane).unwrap_err().code,
            "duplicate-request"
        );
        let stale = set.jump("stale", None, "w1", &tab, &pane).unwrap_err();
        assert_eq!(stale.code, "stale-target");
        assert_eq!(
            set.snapshot().workspace,
            "w2",
            "a failed jump shows nothing"
        );
    }

    #[test]
    fn workspace_closes_prune_only_confirmed_ends_and_show_the_recent_survivor() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "show-second",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: "w2".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "new",
            None,
            Action::Workspace(WorkspaceAction::NewWorkspace {
                name: None,
                focus: true,
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let survivor = set.snapshot_in(Some("w1")).unwrap().tabs;
        let closing = set.prepare_close_workspace("close", None, "w3").unwrap();
        assert_eq!(closing.len(), 1, "a fresh workspace has only its picker");
        assert_eq!(
            set.prepare_close_workspace("close", None, "w3")
                .unwrap_err()
                .code,
            "duplicate-request"
        );
        // Before a confirmed stop, the workspace remains intact.
        assert_eq!(set.snapshot().workspace, "w3");
        set.session_ended(&closing[0], SessionEnd::Closed, |_| {
            panic!("closing never starts a terminal")
        })
        .unwrap();
        assert_eq!(set.snapshot().workspace, "w2");
        assert_eq!(set.snapshot().workspaces.len(), 2);
        assert_eq!(set.snapshot_in(Some("w1")).unwrap().tabs, survivor);
        let background = set
            .prepare_close_workspace("background", None, "w1")
            .unwrap();
        for id in background {
            set.session_ended(&id, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(set.snapshot().workspace, "w2");
        assert_eq!(set.snapshot().workspaces.len(), 1);
        let last = set.prepare_close_workspace("last", None, "w2").unwrap();
        for id in last {
            set.session_ended(&id, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        assert!(
            !set.has_sessions(),
            "the last workspace is left for the supervisor's Ended reply"
        );
    }

    #[test]
    fn workspace_set_ids_and_socket_paths_are_unique() {
        let mut operations = Vec::new();
        let mut record = |op| {
            operations.push(op);
            Ok(())
        };
        let mut set = two_workspace_set(&mut record);
        assert_eq!(set.workspaces[0].id, "w1");
        assert_eq!(set.workspaces[1].id, "w2");
        let initial = set.snapshot();
        assert_eq!(initial.active_tab, "t1");
        assert_eq!(initial.tabs[0].columns[0].panes[0].id, "p1");
        assert_eq!(initial.tabs[0].columns[0].panes[0].session, "session-1");
        for index in 0..2 {
            set.shown = index;
            let tab = set.snapshot().active_tab;
            set.dispatch(
                &format!("pane-{index}"),
                None,
                Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
                prepared,
                &mut record,
            )
            .unwrap();
            set.dispatch(
                &format!("agent-{index}"),
                None,
                invoke(&tab, "agent"),
                prepared,
                &mut record,
            )
            .unwrap();
            // Retarget parks Agent as a pane without starting another terminal.
            set.dispatch(
                &format!("park-{index}"),
                None,
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: tab.clone(),
                    directory: b"/tmp".to_vec(),
                }),
                prepared,
                &mut record,
            )
            .unwrap();
            set.dispatch(
                &format!("git-{index}"),
                None,
                invoke(&tab, "git"),
                prepared,
                &mut record,
            )
            .unwrap();
            set.dispatch(
                &format!("tab-{index}"),
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                &mut record,
            )
            .unwrap();
        }
        let mut ids = std::collections::HashSet::new();
        for workspace in &set.workspaces {
            for tab in workspace.snapshot().tabs {
                assert!(ids.insert(tab.id.clone()));
                for pane in tab.panes() {
                    assert!(ids.insert(pane.id.clone()));
                }
                for popup in tab.popups {
                    assert!(ids.insert(popup.id));
                }
            }
        }
        let mut sessions = std::collections::HashSet::new();
        let mut sockets = std::collections::HashSet::new();
        for op in operations {
            if let SessionOperation::Start { session, .. } = op {
                assert!(sessions.insert(session.id.clone()), "reused {}", session.id);
                assert!(
                    sockets.insert(session.endpoint.clone()),
                    "reused {:?}",
                    session.endpoint
                );
            }
        }
        for _ in 2..32 {
            set.insert_pending(
                "/".into(),
                Some("/".into()),
                catalog(),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        }
        let before = set.clone();
        assert!(
            set.insert_pending(
                "/".into(),
                Some("/".into()),
                catalog(),
                prepared,
                |_| panic!("capacity starts nothing")
            )
            .is_err()
        );
        assert_eq!(set, before);
    }

    #[test]
    fn workspace_set_refuses_foreign_targets() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.shown = 1;
        let foreign = set.snapshot().tabs[0].clone();
        set.dispatch(
            "foreign-popup",
            None,
            invoke(&foreign.id, "agent"),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let popup = set.snapshot().tabs[0].popups[0].id.clone();
        let caller = set.workspaces[0].tabs[0].columns[0].panes[0]
            .session
            .clone();
        assert!(set.owns_session(&caller));
        assert!(!set.owns_session(&Session {
            id: caller.id.clone(),
            endpoint: "/wrong.sock".into()
        }));
        for shown in [0, 1] {
            set.shown = shown;
            let before = set.clone();
            for action in [
                Action::Workspace(WorkspaceAction::FocusId(foreign.id.clone())),
                Action::DismissPopup(PopupTarget {
                    tab: foreign.id.clone(),
                    instance: popup.clone(),
                }),
            ] {
                assert!(
                    set.dispatch("refused", Some(&caller.id), action, prepared, |_| panic!(
                        "foreign target performs no operations"
                    ))
                    .is_err()
                );
                assert_eq!(set, before);
            }
            assert!(
                set.prepare_close_tab("refused", Some(&caller.id), &foreign.id)
                    .is_err()
            );
            assert!(
                set.prepare_close_pane(
                    "refused",
                    Some(&caller.id),
                    &foreign.columns[0].panes[0].id
                )
                .is_err()
            );
            assert_eq!(set, before);
        }
        set.dispatch(
            "local",
            Some(&caller.id),
            Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(set.workspaces[0].session_count(), 2);
        assert_eq!(set.snapshot().active_tab, foreign.id);
    }

    #[test]
    fn workspace_set_terminal_capacity_spans_workspaces() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.shown = 0;
        let tab = set.snapshot().active_tab;
        set.dispatch("agent", None, invoke(&tab, "agent"), prepared, |_| Ok(()))
            .unwrap();
        for index in 0..2 {
            set.shown = index;
            let count = if index == 0 { 126 } else { 127 };
            for number in 0..count {
                set.dispatch(
                    &format!("fill-{index}-{number}"),
                    None,
                    Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
                    prepared,
                    |_| Ok(()),
                )
                .unwrap();
            }
        }
        assert_eq!(
            set.workspaces
                .iter()
                .map(Workspace::session_count)
                .sum::<usize>(),
            MAX_SESSIONS
        );
        for index in 0..2 {
            set.shown = index;
            let tab = set.snapshot().active_tab;
            for action in [
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
                invoke(&tab, "git"),
            ] {
                assert!(
                    set.dispatch("full", None, action, prepared, |_| panic!(
                        "full window starts nothing"
                    ))
                    .is_err()
                );
            }
        }
        set.shown = 0;
        let old = set.workspaces[1].tabs[0].columns[0].panes[0]
            .session
            .id
            .clone();
        set.session_ended(&old, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        set.dispatch("git", None, invoke(&tab, "git"), prepared, |_| Ok(()))
            .unwrap();
        set.dispatch(
            "directory",
            None,
            Action::Workspace(WorkspaceAction::SetTabDirectory {
                tab: tab.clone(),
                directory: b"/tmp".to_vec(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let popup = set.snapshot().tabs[0]
            .popups
            .iter()
            .find(|popup| popup.entry == "git")
            .unwrap()
            .id
            .clone();
        let mut operations = Vec::new();
        set.dispatch(
            "replace",
            None,
            Action::InvokePopup {
                tab: tab.clone(),
                entry: "git".into(),
                expected_instance: Some(popup),
                intent: InvokeIntent::Toggle,
            },
            prepared,
            |op| {
                operations.push(op);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(
            operations.as_slice(),
            [SessionOperation::Stop(_), SessionOperation::Start { .. }]
        ));
        assert_eq!(
            set.workspaces
                .iter()
                .map(Workspace::session_count)
                .sum::<usize>(),
            MAX_SESSIONS
        );
    }

    #[test]
    fn workspace_replay_windows_are_bounded_and_inspections_do_not_evict_mutations() {
        let mut set = two_workspace_set(|_| Ok(()));
        let create = Action::Workspace(WorkspaceAction::NewWorkspace {
            name: None,
            focus: false,
        });
        let switch = Action::Workspace(WorkspaceAction::SwitchWorkspace {
            workspace: "w2".into(),
        });
        set.dispatch("create", None, create.clone(), prepared, |_| Ok(()))
            .unwrap();
        set.dispatch("switch", None, switch.clone(), prepared, |_| Ok(()))
            .unwrap();
        set.prepare_close_workspace("close", None, "w3").unwrap();
        for number in 0..MAX_RECENT_REQUESTS * 2 {
            set.dispatch(
                &format!("inspect-{number}"),
                None,
                Action::Workspace(WorkspaceAction::Inspect),
                prepared,
                |_| panic!("inspection operates on no terminals"),
            )
            .unwrap();
            assert!(set.recent_inspections.len() <= MAX_RECENT_REQUESTS);
            assert!(set.recent_mutations.len() <= MAX_RECENT_REQUESTS);
        }
        let before = set.clone();
        for (id, action) in [("create", create.clone()), ("switch", switch.clone())] {
            assert_eq!(
                set.dispatch(id, None, action, prepared, |_| {
                    panic!("replay performs no terminal operations")
                })
                .unwrap_err()
                .code,
                "duplicate-request"
            );
        }
        assert_eq!(
            set.prepare_close_workspace("close", None, "w3")
                .unwrap_err()
                .code,
            "duplicate-request"
        );
        assert_eq!(set, before);

        for number in 0..MAX_RECENT_REQUESTS {
            if number == MAX_RECENT_REQUESTS - 1 {
                assert_eq!(
                    set.prepare_close_workspace("close", None, "w3")
                        .unwrap_err()
                        .code,
                    "duplicate-request",
                    "255 later mutations still retain the close ID"
                );
            }
            set.dispatch(
                &format!("rename-{number}"),
                None,
                Action::Workspace(WorkspaceAction::RenameWorkspace {
                    workspace: None,
                    name: number.to_string(),
                }),
                prepared,
                |_| panic!("renaming operates on no terminals"),
            )
            .unwrap();
            assert!(set.recent_inspections.len() <= MAX_RECENT_REQUESTS);
            assert!(set.recent_mutations.len() <= MAX_RECENT_REQUESTS);
        }
        set.prepare_close_workspace("close", None, "w3")
            .expect("256 later accepted mutations forget the close ID");
        set.dispatch("switch", None, switch, prepared, |_| Ok(()))
            .unwrap();
        set.dispatch("create", None, create, prepared, |_| Ok(()))
            .unwrap();
        assert_eq!(set.snapshot().workspaces.len(), 4);
        assert!(set.recent_inspections.len() <= MAX_RECENT_REQUESTS);
        assert!(set.recent_mutations.len() <= MAX_RECENT_REQUESTS);
    }

    #[test]
    fn workspace_set_replays_span_workspaces() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "accepted",
            None,
            Action::Workspace(WorkspaceAction::Inspect),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.shown = 1;
        let before = set.clone();
        assert_eq!(
            set.dispatch(
                "accepted",
                None,
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
                prepared,
                |_| panic!("replay performs no operations")
            )
            .unwrap_err()
            .code,
            "duplicate-request"
        );
        assert_eq!(set, before);
        assert!(set.prepare_close_tab("retry", None, "unknown").is_err());
        set.shown = 0;
        let tab = set.snapshot().active_tab;
        let pane = set.snapshot().tabs[0].columns[0].panes[0].id.clone();
        let sessions = set.prepare_close_tab("retry", None, &tab).unwrap();
        assert!(!sessions.is_empty()); // Accepted before any potentially partial stop.
        set.shown = 1;
        let tab = set.snapshot().active_tab;
        assert_eq!(
            set.prepare_close_tab("retry", None, &tab).unwrap_err().code,
            "duplicate-request"
        );
        set.shown = 0;
        set.prepare_close_pane("pane-close", None, &pane).unwrap();
        set.shown = 1;
        assert_eq!(
            set.dispatch(
                "pane-close",
                None,
                Action::Workspace(WorkspaceAction::Inspect),
                prepared,
                |_| panic!("replay performs no operations")
            )
            .unwrap_err()
            .code,
            "duplicate-request"
        );
    }

    #[test]
    fn json_paths_are_strings_unless_their_bytes_are_not_utf8() {
        assert_eq!(json_path(b"/home/a \"b\""), "\"/home/a \\\"b\\\"\"");
        assert_eq!(json_path(b"/tmp/\xff"), "[47,116,109,112,47,255]");
    }

    #[test]
    fn cli_output_carries_attention_state_and_app_but_never_report_text() {
        let workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            None,
            vec![
                (
                    1,
                    Session {
                        id: "session-1".into(),
                        endpoint: "/runtime/orbit.sock".into(),
                    },
                ),
                (
                    2,
                    Session {
                        id: "session-2".into(),
                        endpoint: "/runtime/session-2.sock".into(),
                    },
                ),
            ],
            catalog(),
        )
        .unwrap();
        let mut snapshot = workspace.snapshot();
        snapshot.tabs[0].columns[0].panes[0].attention = Some(Attention {
            state: AttentionState::NeedsInput,
            record: Some(eon_workspace_protocol::v22::AttentionRecord {
                kind: None,
                progress: Some(40),
                app: "codex".into(),
                name: "eon \"9a\"".into(),
                title: "secret title".into(),
                message: "secret message".into(),
                changed_at: 1,
                stale: true,
                others: 3,
            }),
        });
        snapshot.tabs[0].attention_rollup = Some(AttentionState::NeedsInput);
        snapshot.sidebar = Sidebar::Closed;
        let json = json(&snapshot);
        assert!(json.contains(r#""attention_rollup":"needs-input","panes":[{"id":"p1","#));
        assert!(
            json.contains(
                r#""attention":{"state":"needs-input","app":"codex","name":"eon \"9a\""}}"#
            )
        );
        assert!(json.contains(r#""label":"p2  unavailable","pinned_name":null,"attention":null}"#));
        assert!(json.ends_with("],\"sidebar\":\"closed\"}\n"));
        let human = human(&snapshot);
        assert!(human.starts_with("active t1\nsidebar closed\ntab t1 "));
        assert!(human.contains(" attention=needs-input\n"));
        assert!(human.contains(" attention=needs-input app=codex name=eon \\\"9a\\\"\n"));
        assert!(human.contains(" attention=none app=none name=none\n"));
        for output in [json, human] {
            assert!(!output.contains("secret") && !output.contains("progress"));
        }
    }

    fn catalog() -> PopupCatalog {
        PopupCatalog {
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![
                PopupDefinition {
                    id: "project".into(),
                    label: "Project".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: ALT,
                        key: "KeyZ".into(),
                    }),
                    command: PopupCommand::Project,
                    keep_alive: false,
                },
                PopupDefinition {
                    id: "agent".into(),
                    label: "Agent".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: ALT,
                        key: "KeyA".into(),
                    }),
                    command: PopupCommand::Argv(vec!["agent".into()]),
                    keep_alive: true,
                },
                PopupDefinition {
                    id: "git".into(),
                    label: "Git".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: ALT,
                        key: "KeyG".into(),
                    }),
                    command: PopupCommand::Argv(vec!["git".into()]),
                    keep_alive: true,
                },
                PopupDefinition {
                    id: "files".into(),
                    label: "Files".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: ALT,
                        key: "KeyY".into(),
                    }),
                    command: PopupCommand::Argv(vec!["files".into()]),
                    keep_alive: true,
                },
            ],
        }
    }

    fn prepared(command: &PopupCommand, _: &Path) -> Result<Vec<OsString>, String> {
        match command {
            PopupCommand::Argv(argv) => Ok(argv.clone()),
            PopupCommand::Project => Ok(Vec::new()),
            PopupCommand::AgentAuto => unreachable!(),
        }
    }

    #[test]
    fn create_column_in_a_popup_only_tab_never_overfills_a_neighbor() {
        let root = std::env::temp_dir().join(format!("eon-column-limit-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let project = |tab: &str| Action::InvokePopup {
            tab: tab.into(),
            entry: "project".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        };
        for id in ["c1", "c2", "c3"] {
            workspace
                .dispatch(id, None, Action::CreateColumn, prepared, ok)
                .unwrap();
        }
        workspace
            .dispatch(
                "t2",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                ok,
            )
            .unwrap();
        let picker = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&picker, SessionEnd::Exited, ok)
            .unwrap();
        workspace
            .dispatch("open", None, project("t2"), prepared, ok)
            .unwrap();
        let pane = workspace.snapshot().tabs[1].columns[0].panes[0]
            .session
            .clone();
        workspace
            .session_ended(&pane, SessionEnd::Exited, ok)
            .unwrap();
        let tab = &workspace.snapshot().tabs[1];
        assert!(tab.columns.is_empty() && tab.selected_popup.is_some());

        workspace
            .dispatch("c4", None, Action::CreateColumn, prepared, ok)
            .unwrap();
        let after = workspace.snapshot();
        assert_eq!(after.tabs.len(), 1);
        assert_eq!(after.tabs[0].columns.len(), MAX_COLUMNS);
        eon_workspace_protocol::v22::encode_response(
            &eon_workspace_protocol::v22::Response::Snapshot(after),
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_transient_popup_caller_never_removes_its_paneless_tab() {
        let root = std::env::temp_dir().join(format!("eon-popup-caller-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        workspace
            .dispatch(
                "t2",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                ok,
            )
            .unwrap();
        let picker = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&picker, SessionEnd::Exited, ok)
            .unwrap();
        let project = Action::InvokePopup {
            tab: "t2".into(),
            entry: "project".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        };
        workspace
            .dispatch("open", None, project, prepared, ok)
            .unwrap();
        let pane = workspace.snapshot().tabs[1].columns[0].panes[0]
            .session
            .clone();
        workspace
            .session_ended(&pane, SessionEnd::Exited, ok)
            .unwrap();
        workspace
            .dispatch(
                "away",
                None,
                Action::Workspace(WorkspaceAction::FocusId("t1".into())),
                prepared,
                ok,
            )
            .unwrap();
        let caller = workspace.snapshot().tabs[1].popups[0].session.clone();

        // Its own pane create would stop the tab's last terminal, and a tab
        // without panes has nothing to move: both change nothing.
        let before = workspace.snapshot();
        for action in [
            WorkspaceAction::CreatePane { focus: true },
            WorkspaceAction::Move(Direction::Down),
        ] {
            let failure = workspace
                .dispatch(
                    "refused",
                    Some(&caller),
                    Action::Workspace(action),
                    prepared,
                    ok,
                )
                .unwrap_err();
            assert_eq!(failure.code, "unavailable");
            assert_eq!(workspace.snapshot(), before);
        }

        // An unfocused pane dismisses nothing, so it becomes the tab's first,
        // selected pane under its still-shown popup.
        workspace
            .dispatch(
                "unfocused",
                Some(&caller),
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
                prepared,
                ok,
            )
            .unwrap();
        let snapshot = workspace.snapshot();
        let tab = &snapshot.tabs[1];
        assert_eq!(tab.panes().count(), 1);
        assert_eq!(
            tab.selected_pane,
            tab.columns[0].panes.first().map(|pane| pane.id.clone())
        );
        assert_eq!(tab.selected_popup, before.tabs[1].selected_popup);
        assert_eq!(snapshot.active_tab, "t1");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unfocused_panes_keep_the_selection_and_the_shown_popup() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let mut request = |id: &str, caller: Option<&str>, action| {
            workspace
                .dispatch(id, caller, action, prepared, |_| Ok(()))
                .unwrap();
            workspace.snapshot()
        };
        let command = |focus| Action::CreateCommandPane {
            argv: vec![b"claude".to_vec()],
            focus,
        };
        request(
            "p2",
            None,
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        );

        // From the unselected p1, the pane joins the column collapsed.
        let snapshot = request("p3", Some("session-1"), command(false));
        let tab = &snapshot.tabs[0];
        let panes = tab.panes().map(|pane| pane.id.as_str()).collect::<Vec<_>>();
        assert_eq!(panes, ["p1", "p2", "p3"]);
        assert_eq!(tab.selected_pane.as_deref(), Some("p2"));

        // From a shown popup, the popup stays shown over the same selection.
        let snapshot = request(
            "agent",
            None,
            Action::InvokePopup {
                tab: "t1".into(),
                entry: "agent".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        );
        let popup = snapshot.tabs[0].popups[0].clone();
        for (id, action) in [
            ("p4", command(false)),
            (
                "p5",
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
            ),
        ] {
            let tab = request(id, Some(&popup.session), action).tabs.remove(0);
            assert_eq!(tab.panes().last().map(|pane| pane.id.as_str()), Some(id));
            assert_eq!(tab.selected_pane.as_deref(), Some("p2"));
            assert_eq!(tab.selected_popup.as_deref(), Some(popup.id.as_str()));
        }

        // Focused, the same caller's pane is selected and the popup dismissed.
        let tab = request("p6", Some(&popup.session), command(true))
            .tabs
            .remove(0);
        assert_eq!(tab.selected_pane.as_deref(), Some("p6"));
        assert_eq!(tab.selected_popup, None);
    }

    #[test]
    fn attention_focus_hides_keep_alive_popups_and_never_stops_a_transient_one() {
        let mut catalog = catalog();
        catalog.entries.push(PopupDefinition {
            id: "anima".into(),
            label: "Anima".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT,
                key: "KeyN".into(),
            }),
            command: PopupCommand::Argv(vec!["anima".into()]),
            keep_alive: false,
        });
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog,
        )
        .unwrap();
        let mut stops = 0;
        let mut request = |id: &str, action| {
            workspace
                .dispatch(id, None, action, prepared, |operation| {
                    stops += usize::from(matches!(operation, SessionOperation::Stop(_)));
                    Ok(())
                })
                .map(|()| workspace.snapshot())
        };
        let invoke = |entry: &str| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: None,
            intent: InvokeIntent::Focus,
        };
        let focus = |tab: &str, target: &str| Action::FocusAttention {
            tab: tab.into(),
            target: target.into(),
        };
        request(
            "p2",
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        )
        .unwrap();
        let agent = request("agent", invoke("agent")).unwrap().tabs[0].popups[0]
            .id
            .clone();

        // A shown keep-alive popup is hidden, and survives, for another target.
        let tab = request("f1", focus("t1", "p1")).unwrap().tabs.remove(0);
        assert_eq!(tab.selected_pane.as_deref(), Some("p1"));
        assert_eq!(tab.selected_popup, None);
        assert_eq!(tab.popups.len(), 1);
        let tab = request("f2", focus("t1", &agent)).unwrap().tabs.remove(0);
        assert_eq!(tab.selected_popup.as_deref(), Some(agent.as_str()));

        // A shown transient popup is left alone: the action only selects the
        // tab, from another active tab too, and stops nothing.
        request("anima", invoke("anima")).unwrap();
        request("t2", Action::Workspace(WorkspaceAction::CreateTab)).unwrap();
        for (id, target) in [("f3", "p2"), ("f4", agent.as_str())] {
            let snapshot = request(id, focus("t1", target)).unwrap();
            assert_eq!(snapshot.active_tab, "t1");
            let tab = &snapshot.tabs[0];
            assert_eq!(tab.selected_pane.as_deref(), Some("p1"));
            assert_ne!(tab.selected_popup.as_deref(), Some(agent.as_str()));
            assert_eq!(tab.popups.len(), 2);
        }
        assert_eq!(stops, 0);

        // A target its tab does not hold is stale, with no fallback.
        let before = workspace.snapshot();
        let mut request = |id: &str, action| {
            workspace
                .dispatch(id, None, action, prepared, |_| Ok(()))
                .unwrap_err()
                .code
        };
        for (id, tab, target) in [("s1", "t1", "p9"), ("s2", "t2", "p1"), ("s3", "t9", "p1")] {
            assert_eq!(request(id, focus(tab, target)), "stale-target");
        }
        assert_eq!(workspace.snapshot(), before);
    }

    #[test]
    fn columns_hold_accordions_traverse_into_tabs_and_collapse_on_exit() {
        let root = std::env::temp_dir().join(format!("eon-columns-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let request = |workspace: &mut WorkspaceSet, id: &str, action| {
            workspace.dispatch(id, None, action, prepared, ok)
        };
        let shape = |workspace: &WorkspaceSet| {
            let tab = &workspace.snapshot().tabs[0];
            let columns = tab
                .columns
                .iter()
                .map(|column| {
                    column
                        .panes
                        .iter()
                        .map(|pane| pane.id.clone())
                        .collect::<Vec<_>>()
                        .join(",")
                        + "@"
                        + &column.expanded
                })
                .collect::<Vec<_>>();
            (columns, tab.selected_pane.clone().unwrap())
        };
        let single = workspace.snapshot();

        // A new column opens right of the selected one and takes the selection.
        request(&mut workspace, "c1", Action::CreateColumn).unwrap();
        request(
            &mut workspace,
            "c2",
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        )
        .unwrap();
        assert_eq!(
            shape(&workspace),
            (vec!["p1@p1".into(), "p2,p3@p3".into()], "p3".into())
        );
        request(
            &mut workspace,
            "l1",
            Action::Workspace(WorkspaceAction::Focus(Direction::Left)),
        )
        .unwrap();
        request(&mut workspace, "c3", Action::CreateColumn).unwrap();
        assert_eq!(
            shape(&workspace),
            (
                vec!["p1@p1".into(), "p4@p4".into(), "p2,p3@p3".into()],
                "p4".into()
            )
        );
        request(&mut workspace, "c4", Action::CreateColumn).unwrap();
        let full = workspace.snapshot();
        let refused = request(&mut workspace, "c5", Action::CreateColumn).unwrap_err();
        assert_eq!(refused.code, "unavailable");
        assert_eq!(workspace.snapshot(), full);

        // Up/down stay within the selected column; left/right cross columns, then tabs.
        request(
            &mut workspace,
            "r1",
            Action::Workspace(WorkspaceAction::Focus(Direction::Right)),
        )
        .unwrap();
        request(
            &mut workspace,
            "u1",
            Action::Workspace(WorkspaceAction::Focus(Direction::Up)),
        )
        .unwrap();
        assert_eq!(shape(&workspace).1, "p2");
        request(
            &mut workspace,
            "f1",
            Action::Workspace(WorkspaceAction::FocusId("p1".into())),
        )
        .unwrap();
        assert_eq!(shape(&workspace).1, "p1");
        request(
            &mut workspace,
            "t2",
            Action::Workspace(WorkspaceAction::CreateTab),
        )
        .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t2");
        request(
            &mut workspace,
            "l2",
            Action::Workspace(WorkspaceAction::Focus(Direction::Left)),
        )
        .unwrap();
        // Entering a tab by moving left lands on its rightmost column.
        assert_eq!(workspace.snapshot().active_tab, "t1");
        assert_eq!(shape(&workspace).1, "p2");
        request(
            &mut workspace,
            "l3",
            Action::Workspace(WorkspaceAction::Focus(Direction::Left)),
        )
        .unwrap();
        assert_eq!(shape(&workspace).1, "p5");
        // Holding one direction walks every column of every tab, entering at the edge.
        let mut walk = Vec::new();
        for id in ["r2", "r3", "r4"] {
            request(
                &mut workspace,
                id,
                Action::Workspace(WorkspaceAction::Focus(Direction::Right)),
            )
            .unwrap();
            let snapshot = workspace.snapshot();
            walk.push(if snapshot.active_tab == "t1" {
                shape(&workspace).1
            } else {
                snapshot.active_tab
            });
        }
        assert_eq!(walk, ["p2", "t2", "p1"]);

        // Ending a column's last pane removes the column; selection moves on.
        request(
            &mut workspace,
            "f2",
            Action::Workspace(WorkspaceAction::FocusId("p5".into())),
        )
        .unwrap();
        workspace
            .session_ended("session-5", SessionEnd::Exited, ok)
            .unwrap();
        assert_eq!(shape(&workspace).1, "p2");
        for session in ["session-4", "session-2", "session-3"] {
            workspace
                .session_ended(session, SessionEnd::Exited, ok)
                .unwrap();
        }
        let mut collapsed = workspace.snapshot();
        collapsed.tabs.truncate(1);
        collapsed.active_tab = "t1".into();
        assert_eq!(collapsed.tabs, single.tabs);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pane_close_targets_the_active_tab_and_prunes_like_an_exit() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let request = |workspace: &mut WorkspaceSet, id: &str, action| {
            workspace.dispatch(id, None, action, prepared, ok).unwrap()
        };
        let close = |workspace: &mut WorkspaceSet, id: &str, pane: &str| {
            let sessions = workspace.prepare_close_pane(id, None, pane)?;
            let [session] = sessions.as_slice() else {
                panic!("a non-final pane close stops only its pane");
            };
            workspace.session_ended(session, SessionEnd::Closed, ok)?;
            Ok::<_, Failure>(session.clone())
        };
        let refused = |workspace: &mut WorkspaceSet, id: &str, pane: &str| {
            let before = workspace.snapshot();
            let failure = close(workspace, id, pane).unwrap_err();
            assert_eq!(workspace.snapshot(), before);
            failure
        };

        request(&mut workspace, "c1", Action::CreateColumn);
        request(
            &mut workspace,
            "c2",
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        );
        assert_eq!(close(&mut workspace, "x2", "p3").unwrap(), "session-3");
        assert_eq!(
            refused(&mut workspace, "x2", "p2").code,
            "duplicate-request"
        );
        assert_eq!(refused(&mut workspace, "x3", "p3").code, "unknown-pane");
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.columns.len(), 2);
        assert_eq!(tab.selected_pane.as_deref(), Some("p2"));

        // A column's last pane takes the column with it.
        close(&mut workspace, "x4", "p2").unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.columns.len(), 1);
        assert_eq!(tab.selected_pane.as_deref(), Some("p1"));

        // A covering popup and an inactive tab refuse; a non-final tab's last pane removes it.
        request(
            &mut workspace,
            "a1",
            Action::InvokePopup {
                tab: "t1".into(),
                entry: "agent".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        );
        assert_eq!(
            refused(&mut workspace, "x5", "p1").detail,
            "close the popup before closing a pane"
        );
        // Under the popup, the tab's own terminals may close a named pane but not its last.
        let agent = workspace.snapshot().tabs[0].popups[0].session.clone();
        let before = workspace.snapshot();
        assert_eq!(
            workspace
                .prepare_close_pane("x5a", Some(&agent), "p1")
                .unwrap_err()
                .detail,
            "close the popup before closing the tab's last pane"
        );
        assert_eq!(workspace.snapshot(), before);
        for (id, caller) in [("x5b", agent.as_str()), ("x5c", "session-1")] {
            request(
                &mut workspace,
                id,
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
            );
            let pane = workspace.snapshot().tabs[0]
                .panes()
                .last()
                .unwrap()
                .id
                .clone();
            // Alt+W, which names no caller, still may not close a hidden pane.
            assert_eq!(
                refused(&mut workspace, &format!("{id}-alt-w"), &pane).detail,
                "close the popup before closing a pane"
            );
            let [session] = workspace
                .prepare_close_pane(&format!("{id}-close"), Some(caller), &pane)
                .unwrap()
                .try_into()
                .unwrap();
            workspace
                .session_ended(&session, SessionEnd::Closed, ok)
                .unwrap();
            assert_eq!(workspace.snapshot(), before);
        }
        request(
            &mut workspace,
            "t2",
            Action::Workspace(WorkspaceAction::CreateTab),
        );
        assert_eq!(refused(&mut workspace, "x6", "p1").code, "not-active");
        let picker = workspace.snapshot().tabs[1].popups[0].clone();
        request(
            &mut workspace,
            "t2-dir",
            Action::CommitDirectory {
                target: PopupTarget {
                    tab: "t2".into(),
                    instance: picker.id,
                },
                directory: b"/".to_vec(),
            },
        );
        workspace
            .session_ended(&picker.session, SessionEnd::Exited, ok)
            .unwrap();
        let pane = workspace.snapshot().tabs[1].selected_pane.clone().unwrap();
        close(&mut workspace, "x7", &pane).unwrap();
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.tabs.len(), 1);
        assert_eq!(snapshot.active_tab, "t1");

        // The final tab's last pane ends the window, so its hidden popup stops too.
        let popup = snapshot.tabs[0].popups[0].clone();
        request(
            &mut workspace,
            "a2",
            Action::InvokePopup {
                tab: "t1".into(),
                entry: "agent".into(),
                expected_instance: Some(popup.id),
                intent: InvokeIntent::Toggle,
            },
        );
        assert_eq!(workspace.snapshot().tabs[0].selected_popup, None);
        assert_eq!(
            workspace.prepare_close_pane("x8", None, "p1").unwrap(),
            ["session-1".to_owned(), popup.session]
        );
    }

    #[test]
    fn callers_act_in_their_own_tab_without_changing_the_active_tab() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let request = |workspace: &mut WorkspaceSet, id: &str, caller: Option<&str>, action| {
            workspace.dispatch(id, caller, action, prepared, |_| Ok(()))
        };
        request(
            &mut workspace,
            "t2",
            None,
            Action::Workspace(WorkspaceAction::CreateTab),
        )
        .unwrap();
        let picker = workspace.snapshot().tabs[1].popups[0].clone();
        request(
            &mut workspace,
            "t2-dir",
            None,
            Action::CommitDirectory {
                target: PopupTarget {
                    tab: "t2".into(),
                    instance: picker.id,
                },
                directory: b"/".to_vec(),
            },
        )
        .unwrap();
        workspace
            .session_ended(&picker.session, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        let panes = |workspace: &WorkspaceSet, tab: &str| {
            let snapshot = workspace.snapshot();
            let tab = snapshot.tabs.iter().find(|item| item.id == tab).unwrap();
            let panes = tab.panes().map(|pane| pane.id.clone()).collect::<Vec<_>>();
            (panes, tab.selected_pane.clone().unwrap())
        };
        let active = panes(&workspace, "t2");

        // A pane in the inactive tab creates beside itself, and the user stays put.
        request(
            &mut workspace,
            "c1",
            Some("session-1"),
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        )
        .unwrap();
        let (created, selected) = panes(&workspace, "t1");
        assert_eq!(created.len(), 2);
        assert_eq!(selected, created[1]);
        assert_eq!(workspace.snapshot().active_tab, "t2");
        assert_eq!(panes(&workspace, "t2"), active);

        // A failed action restores the selection the caller's pane took.
        let before = workspace.snapshot();
        workspace
            .dispatch(
                "c-fails",
                Some("session-1"),
                Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
                prepared,
                |_| Err("start failed".into()),
            )
            .unwrap_err();
        assert_eq!(workspace.snapshot(), before);

        // The caller's tab moves; the active tab stays active.
        request(
            &mut workspace,
            "m1",
            Some("session-1"),
            Action::Workspace(WorkspaceAction::Move(Direction::Right)),
        )
        .unwrap();
        let snapshot = workspace.snapshot();
        let order = snapshot
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            (order, snapshot.active_tab.as_str()),
            (vec!["t2", "t1"], "t2")
        );

        // Closing resolves the pane in the caller's tab, not the active one.
        let [session] = &workspace
            .prepare_close_pane("x1", Some("session-1"), &created[1])
            .unwrap()[..]
        else {
            panic!("a non-final pane close stops only its pane");
        };
        workspace
            .session_ended(session, SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(panes(&workspace, "t1").0, ["p1"]);

        // A popup's terminal acts for its tab's selected column.
        request(
            &mut workspace,
            "a1",
            None,
            Action::InvokePopup {
                tab: "t2".into(),
                entry: "agent".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        )
        .unwrap();
        let agent = workspace.snapshot().tabs[0].popups[0].session.clone();

        // A popup covering the active tab leaves another tab's pane free to
        // move; with that other tab active, the popup's own terminal is
        // refused.
        request(
            &mut workspace,
            "m2",
            Some("session-1"),
            Action::Workspace(WorkspaceAction::Move(Direction::Up)),
        )
        .unwrap();
        request(
            &mut workspace,
            "to-t1",
            None,
            Action::Workspace(WorkspaceAction::FocusId("t1".into())),
        )
        .unwrap();
        let before = workspace.snapshot();
        let failure = request(
            &mut workspace,
            "m3",
            Some(&agent),
            Action::Workspace(WorkspaceAction::Move(Direction::Down)),
        )
        .unwrap_err();
        assert_eq!(failure.code, "unavailable");
        assert_eq!(workspace.snapshot(), before);
        request(
            &mut workspace,
            "c2",
            Some(&agent),
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        )
        .unwrap();
        assert_eq!(panes(&workspace, "t2").0.len(), active.0.len() + 1);
        assert_eq!(panes(&workspace, "t1").0, ["p1"]);

        // A caller that is no terminal of this workspace changes nothing.
        let before = workspace.snapshot();
        let failure = request(
            &mut workspace,
            "c3",
            Some("session-99"),
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        )
        .unwrap_err();
        assert_eq!(failure.code, "unknown-caller");
        assert_eq!(workspace.snapshot(), before);
    }

    #[test]
    fn sideways_pane_moves_join_neighbor_stacks_split_at_edges_and_drop_emptied_columns() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let request = |workspace: &mut WorkspaceSet, id: &str, action| {
            workspace.dispatch(id, None, action, prepared, ok)
        };
        let workspace_action = |workspace: &mut WorkspaceSet, id: &str, action| {
            request(workspace, id, Action::Workspace(action))
        };
        let sideways = |workspace: &mut WorkspaceSet, id: &str, side| {
            request(workspace, id, Action::MovePaneSideways(side))
        };
        // Each column's panes with the expanded one marked, then the selected pane.
        let layout = |workspace: &WorkspaceSet| {
            let tab = &workspace.snapshot().tabs[0];
            let columns = tab
                .columns
                .iter()
                .map(|column| {
                    column
                        .panes
                        .iter()
                        .map(|pane| {
                            if pane.id == column.expanded {
                                format!("[{}]", pane.id)
                            } else {
                                pane.id.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>();
            (columns, tab.selected_pane.clone().unwrap())
        };
        workspace_action(
            &mut workspace,
            "p",
            WorkspaceAction::CreatePane { focus: true },
        )
        .unwrap();

        // A stacked pane past the outer column opens a new column there.
        sideways(&mut workspace, "r1", Side::Right).unwrap();
        assert_eq!(
            layout(&workspace),
            (vec!["[p1]".into(), "[p2]".into()], "p2".into())
        );
        // A pane alone in the outer column stops quietly.
        let edge = workspace.snapshot();
        sideways(&mut workspace, "r2", Side::Right).unwrap();
        assert_eq!(workspace.snapshot(), edge);

        // A lone pane joins the neighbour's stack and takes its emptied column away.
        sideways(&mut workspace, "l1", Side::Left).unwrap();
        assert_eq!(layout(&workspace), (vec!["p1 [p2]".into()], "p2".into()));

        // The moved pane lands directly below the neighbour's expanded pane, not at the end.
        workspace_action(&mut workspace, "f1", WorkspaceAction::FocusId("p1".into())).unwrap();
        sideways(&mut workspace, "r3", Side::Right).unwrap();
        workspace_action(&mut workspace, "f2", WorkspaceAction::FocusId("p2".into())).unwrap();
        workspace_action(
            &mut workspace,
            "p3",
            WorkspaceAction::CreatePane { focus: true },
        )
        .unwrap();
        workspace_action(&mut workspace, "f3", WorkspaceAction::FocusId("p2".into())).unwrap();
        workspace_action(&mut workspace, "f4", WorkspaceAction::FocusId("p1".into())).unwrap();
        assert_eq!(
            layout(&workspace),
            (vec!["[p2] p3".into(), "[p1]".into()], "p1".into())
        );
        sideways(&mut workspace, "l2", Side::Left).unwrap();
        assert_eq!(layout(&workspace), (vec!["p2 [p1] p3".into()], "p1".into()));

        // The source column expands the following pane; identities and terminals stay.
        sideways(&mut workspace, "l3", Side::Left).unwrap();
        assert_eq!(
            layout(&workspace),
            (vec!["[p1]".into(), "p2 [p3]".into()], "p1".into())
        );
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.columns[0].panes[0].session, "session-1");

        // A stacked pane cannot open a fifth column; the move stops quietly.
        request(&mut workspace, "c1", Action::CreateColumn).unwrap();
        request(&mut workspace, "c2", Action::CreateColumn).unwrap();
        workspace_action(&mut workspace, "f5", WorkspaceAction::FocusId("p3".into())).unwrap();
        let full = workspace.snapshot();
        assert_eq!(full.tabs[0].columns.len(), MAX_COLUMNS);
        sideways(&mut workspace, "r4", Side::Right).unwrap();
        assert_eq!(workspace.snapshot(), full);
        assert_eq!(
            sideways(&mut workspace, "r4", Side::Right)
                .unwrap_err()
                .code,
            "duplicate-request"
        );

        // Removing a lone source left of its target shifts the target's index.
        workspace_action(&mut workspace, "f6", WorkspaceAction::FocusId("p5".into())).unwrap();
        sideways(&mut workspace, "r5", Side::Right).unwrap();
        assert_eq!(
            layout(&workspace),
            (
                vec!["[p1]".into(), "[p4]".into(), "p2 p3 [p5]".into()],
                "p5".into()
            )
        );
        // Moving the last pane out of a stack expands its preceding pane.
        sideways(&mut workspace, "r6", Side::Right).unwrap();
        assert_eq!(
            layout(&workspace),
            (
                vec![
                    "[p1]".into(),
                    "[p4]".into(),
                    "p2 [p3]".into(),
                    "[p5]".into()
                ],
                "p5".into()
            )
        );

        // The picker-bound pending tab refuses without changing topology.
        workspace_action(&mut workspace, "t2", WorkspaceAction::CreateTab).unwrap();
        let pending = workspace.snapshot();
        assert_eq!(
            sideways(&mut workspace, "l4", Side::Left).unwrap_err().code,
            "choose-directory-first"
        );
        assert_eq!(workspace.snapshot(), pending);
    }

    #[test]
    fn pane_moves_refuse_covering_popups_without_changing_tabs() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        for (id, action) in [
            ("stack", WorkspaceAction::CreatePane { focus: true }),
            ("other-tab", WorkspaceAction::CreateTab),
            ("select", WorkspaceAction::FocusId("t1".into())),
        ] {
            workspace
                .dispatch(id, None, Action::Workspace(action), prepared, |_| Ok(()))
                .unwrap();
        }
        for entry in ["agent", "project"] {
            for with_panes in [true, false] {
                let mut covered = workspace.clone();
                covered
                    .dispatch(
                        "open",
                        None,
                        Action::InvokePopup {
                            tab: "t1".into(),
                            entry: entry.into(),
                            expected_instance: None,
                            intent: InvokeIntent::Toggle,
                        },
                        prepared,
                        |_| Ok(()),
                    )
                    .unwrap();
                if !with_panes {
                    for session in ["session-1", "session-2"] {
                        covered
                            .session_ended(session, SessionEnd::Exited, |_| Ok(()))
                            .unwrap();
                    }
                }
                let before = covered.clone();
                for action in [
                    Action::MovePaneSideways(Side::Left),
                    Action::MovePaneSideways(Side::Right),
                    Action::Workspace(WorkspaceAction::Move(Direction::Up)),
                    Action::Workspace(WorkspaceAction::Move(Direction::Down)),
                ] {
                    assert_eq!(
                        covered
                            .dispatch("move", None, action, prepared, |_| {
                                panic!("a refused pane move must not start or stop a terminal")
                            })
                            .unwrap_err()
                            .code,
                        "unavailable"
                    );
                    assert_eq!(covered, before);
                }
            }
        }
    }

    #[test]
    fn column_moves_stay_in_their_column_popups_skip_columns_and_the_cli_reports_positions() {
        let root = std::env::temp_dir().join(format!("eon-column-moves-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let request = |workspace: &mut WorkspaceSet, id: &str, action| {
            workspace.dispatch(id, None, action, prepared, ok)
        };
        let workspace_action = |workspace: &mut WorkspaceSet, id: &str, action| {
            request(workspace, id, Action::Workspace(action))
        };
        let panes = |workspace: &WorkspaceSet| {
            workspace.snapshot().tabs[0]
                .columns
                .iter()
                .map(|column| {
                    column
                        .panes
                        .iter()
                        .map(|pane| pane.id.clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        request(&mut workspace, "c1", Action::CreateColumn).unwrap();
        workspace_action(
            &mut workspace,
            "p",
            WorkspaceAction::CreatePane { focus: true },
        )
        .unwrap();

        // Moving a pane in the second column reorders only that column.
        workspace_action(&mut workspace, "m1", WorkspaceAction::Move(Direction::Up)).unwrap();
        assert_eq!(panes(&workspace), [vec!["p1"], vec!["p3", "p2"]]);
        assert_eq!(
            workspace.snapshot().tabs[0].selected_pane.as_deref(),
            Some("p3")
        );
        let moved = workspace.snapshot();
        workspace_action(&mut workspace, "m2", WorkspaceAction::Move(Direction::Up)).unwrap();
        assert_eq!(workspace.snapshot(), moved);

        let text = human(&moved);
        for line in [
            "  pane p1 column=1 expanded=true ",
            "  pane p3 column=2 expanded=true ",
            "  pane p2 column=2 expanded=false ",
        ] {
            assert!(text.contains(line), "{line:?} missing from {text}");
        }
        assert!(json(&moved).contains("{\"id\":\"p2\",\"column\":2,\"expanded\":false,"));

        // While a popup covers a tab, left/right leave it instead of crossing its columns.
        workspace_action(&mut workspace, "t2", WorkspaceAction::CreateTab).unwrap();
        workspace_action(
            &mut workspace,
            "l1",
            WorkspaceAction::Focus(Direction::Left),
        )
        .unwrap();
        workspace_action(&mut workspace, "f1", WorkspaceAction::FocusId("p1".into())).unwrap();
        request(
            &mut workspace,
            "open",
            Action::InvokePopup {
                tab: "t1".into(),
                entry: "agent".into(),
                expected_instance: None,
                intent: InvokeIntent::Toggle,
            },
        )
        .unwrap();
        workspace_action(
            &mut workspace,
            "r1",
            WorkspaceAction::Focus(Direction::Right),
        )
        .unwrap();
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.active_tab, "t2");
        assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p1"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retarget_parks_the_agent_and_restarts_git_only_on_its_next_invocation() {
        let root = std::env::temp_dir().join(format!("eon-workspace-{}", std::process::id()));
        let old = root.join("old");
        let new = root.join("new");
        let newest = root.join("newest");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::create_dir_all(&newest).unwrap();
        let mut operations = Vec::new();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            old,
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let mut operate = |operation| {
            operations.push(operation);
            Ok(())
        };
        let invoke = |entry: &str, expected: Option<&str>| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: expected.map(Into::into),
            intent: InvokeIntent::Toggle,
        };
        let retarget = |directory: &Path| {
            Action::Workspace(WorkspaceAction::SetTabDirectory {
                tab: "t1".into(),
                directory: directory.as_os_str().as_bytes().to_vec(),
            })
        };
        for (request, entry) in [("agent", "agent"), ("git", "git")] {
            workspace
                .dispatch(request, None, invoke(entry, None), prepared, &mut operate)
                .unwrap();
        }
        let [agent, git] = workspace.snapshot().tabs[0]
            .popups
            .clone()
            .try_into()
            .unwrap();
        workspace
            .dispatch(
                "hide",
                None,
                invoke("git", Some(&git.id)),
                prepared,
                &mut operate,
            )
            .unwrap();
        assert_eq!(
            workspace.snapshot().tabs[0].popups,
            [agent.clone(), git.clone()]
        );

        // Git, opened over the Agent and then hidden, leaves the Agent hidden: it
        // parks collapsed at the end of the selected column.
        workspace
            .dispatch("retarget", None, retarget(&new), prepared, &mut operate)
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.popups, std::slice::from_ref(&git));
        assert_eq!(tab.selected_popup, None);
        assert_eq!(tab.selected_pane.as_deref(), Some("p1"));
        let panes = &tab.columns[0].panes;
        assert_eq!(panes.len(), 2);
        assert_eq!(
            (panes[1].session.as_str(), panes[1].endpoint.as_slice()),
            (agent.session.as_str(), agent.endpoint.as_slice())
        );
        let parked = panes[1].id.clone();

        // The next Agent starts fresh at the new directory; Git restarts there.
        workspace
            .dispatch("fresh", None, invoke("agent", None), prepared, &mut operate)
            .unwrap();
        workspace
            .dispatch(
                "replace",
                None,
                invoke("git", Some(&git.id)),
                prepared,
                &mut operate,
            )
            .unwrap();
        let popups = workspace.snapshot().tabs[0].popups.clone();
        assert_eq!(popups.len(), 2);
        assert!(
            popups
                .iter()
                .all(|popup| popup.id != agent.id && popup.id != git.id)
        );
        assert_eq!(
            operations
                .iter()
                .map(|operation| match operation {
                    SessionOperation::Start { directory, .. } => directory.clone(),
                    SessionOperation::Stop(id) => PathBuf::from(format!("stop {id}")),
                })
                .collect::<Vec<_>>(),
            [
                root.join("old"),
                root.join("old"),
                new.clone(),
                PathBuf::from(format!("stop {}", git.session)),
                new.clone(),
            ]
        );

        // A shown Agent parks as the selected, expanded pane.
        let live_git = popups
            .iter()
            .find(|popup| popup.entry == "git")
            .unwrap()
            .clone();
        let fresh = popups
            .iter()
            .find(|popup| popup.entry == "agent")
            .unwrap()
            .clone();
        workspace
            .dispatch(
                "show",
                None,
                invoke("agent", Some(&fresh.id)),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        workspace
            .dispatch("retarget-again", None, retarget(&newest), prepared, |_| {
                Err("parking starts and stops nothing".into())
            })
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.popups, std::slice::from_ref(&live_git));
        assert_eq!(tab.selected_popup, None);
        let panes = &tab.columns[0].panes;
        assert_eq!(panes[1].id, parked);
        assert_eq!(panes[2].session, fresh.session);
        assert_eq!(tab.selected_pane.as_ref(), Some(&panes[2].id));

        // Git's restart keeps the old instance when its stop fails and records
        // no replacement when its start fails.
        assert_eq!(
            workspace
                .dispatch(
                    "stop-fails",
                    None,
                    invoke("git", Some(&live_git.id)),
                    prepared,
                    |operation| match operation {
                        SessionOperation::Stop(_) => Err("stop failed".into()),
                        SessionOperation::Start { .. } => Ok(()),
                    }
                )
                .unwrap_err()
                .code,
            "popup-stop"
        );
        assert_eq!(
            workspace.snapshot().tabs[0].popups,
            std::slice::from_ref(&live_git)
        );
        assert_eq!(
            workspace
                .dispatch(
                    "start-fails",
                    None,
                    invoke("git", Some(&live_git.id)),
                    prepared,
                    |operation| match operation {
                        SessionOperation::Stop(_) => Ok(()),
                        SessionOperation::Start { .. } => Err("start failed".into()),
                    }
                )
                .unwrap_err()
                .code,
            "popup-start"
        );
        assert!(workspace.snapshot().tabs[0].popups.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn popup_switching_stops_transients_without_leaving_empty_tabs() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let mut operations = Vec::new();
        let mut operate = |operation| {
            operations.push(operation);
            Ok(())
        };
        let invoke = |entry: &str, expected_instance: Option<String>| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance,
            intent: InvokeIntent::Toggle,
        };

        workspace
            .dispatch(
                "agent-open",
                None,
                invoke("agent", None),
                prepared,
                &mut operate,
            )
            .unwrap();
        let agent = workspace.snapshot().tabs[0].popups[0].clone();
        workspace
            .dispatch(
                "agent-hide",
                None,
                invoke("agent", Some(agent.id.clone())),
                prepared,
                &mut operate,
            )
            .unwrap();
        workspace
            .dispatch(
                "project-open",
                None,
                invoke("project", None),
                prepared,
                &mut operate,
            )
            .unwrap();
        workspace
            .dispatch(
                "agent-return",
                None,
                invoke("agent", Some(agent.id.clone())),
                prepared,
                &mut operate,
            )
            .unwrap();

        let snapshot = workspace.snapshot();
        assert_eq!(
            snapshot.tabs[0].popups.as_slice(),
            std::slice::from_ref(&agent)
        );
        assert_eq!(snapshot.tabs[0].selected_popup.as_ref(), Some(&agent.id));
        assert!(matches!(
            operations.last(),
            Some(SessionOperation::Stop(id)) if id == "directory-picker-2"
        ));

        workspace.workspaces[0].tabs.push(Tab {
            id: "t2".into(),
            directory: "/".into(),
            pending: false,
            columns: vec![Column {
                panes: vec![Pane {
                    pinned_name: None,
                    id: "p99".into(),
                    session: Session {
                        id: "session-99".into(),
                        endpoint: "/runtime/session-99.sock".into(),
                    },
                }],
                expanded: 0,
            }],
            selected_column: Some(0),
            popups: Vec::new(),
            selected_popup: None,
        });
        workspace
            .dispatch(
                "project-again",
                None,
                invoke("project", None),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        workspace
            .session_ended(&agent.session, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        workspace
            .session_ended("session-1", SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot().tabs[0].popups[0].entry, "project");

        let failure = workspace
            .dispatch(
                "replacement-fails",
                None,
                invoke("agent", None),
                prepared,
                |operation| match operation {
                    SessionOperation::Stop(_) => Ok(()),
                    SessionOperation::Start { .. } => Err("start failed".into()),
                },
            )
            .unwrap_err();
        assert_eq!(failure.code, "popup-start");
        assert_eq!(workspace.snapshot().tabs[0].id, "t2");
    }

    #[test]
    fn chooser_result_requires_its_exact_live_invocation() {
        let root = std::env::temp_dir().join(format!("eon-chooser-{}", std::process::id()));
        let selected = root.join("selected");
        fs::create_dir_all(&selected).unwrap();
        let mut workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let project = workspace.snapshot().tabs[0].popups[0].clone();
        let commit = || Action::CommitDirectory {
            target: PopupTarget {
                tab: "t1".into(),
                instance: project.id.clone(),
            },
            directory: selected.as_os_str().as_bytes().to_vec(),
        };
        assert_eq!(
            workspace
                .dispatch("commit-fails", None, commit(), prepared, |_| {
                    Err("start failed".into())
                })
                .unwrap_err()
                .code,
            "session-start"
        );
        let unchanged = workspace.snapshot();
        assert!(unchanged.tabs[0].pending);
        assert_eq!(unchanged.tabs[0].directory, root.as_os_str().as_bytes());
        assert!(unchanged.tabs[0].columns.is_empty());
        workspace
            .dispatch("commit", None, commit(), prepared, |_| Ok(()))
            .unwrap();
        assert!(!workspace.snapshot().tabs[0].pending);
        assert!(
            workspace
                .dispatch(
                    "stale",
                    None,
                    Action::DismissPopup(PopupTarget {
                        tab: "t1".into(),
                        instance: "u999".into(),
                    }),
                    prepared,
                    |_| Ok(()),
                )
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn files_keeps_its_terminal_across_retargets() {
        let root = std::env::temp_dir().join(format!("eon-files-{}", std::process::id()));
        let next = root.join("next");
        fs::create_dir_all(&next).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let invoke = |expected: Option<&str>| Action::InvokePopup {
            tab: "t1".into(),
            entry: "files".into(),
            expected_instance: expected.map(Into::into),
            intent: InvokeIntent::Toggle,
        };
        let mut started = Vec::new();
        workspace
            .dispatch("open", None, invoke(None), prepared, |operation| {
                started.push(operation);
                Ok(())
            })
            .unwrap();
        // A popup terminal names its owning tab for `EON_TAB`.
        assert!(matches!(
            started.as_slice(),
            [SessionOperation::Start { tab: Some(tab), .. }] if tab == "t1"
        ));
        let files = workspace.snapshot().tabs[0].popups[0].clone();
        workspace
            .dispatch("hide", None, invoke(Some(&files.id)), prepared, |_| Ok(()))
            .unwrap();
        workspace
            .dispatch(
                "retarget",
                None,
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: next.as_os_str().as_bytes().to_vec(),
                }),
                prepared,
                |_| Err("retargeting keeps Files".into()),
            )
            .unwrap();
        workspace
            .dispatch("reopen", None, invoke(Some(&files.id)), prepared, |_| {
                Err("reopening keeps Files".into())
            })
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.popups, std::slice::from_ref(&files));
        assert_eq!(tab.selected_popup.as_ref(), Some(&files.id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn files_edits_a_file_in_a_new_helix_pane_of_its_tab() {
        let root = std::env::temp_dir().join(format!("eon-edit-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        workspace
            .dispatch(
                "open",
                None,
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "files".into(),
                    expected_instance: None,
                    intent: InvokeIntent::Toggle,
                },
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let files = workspace.snapshot().tabs[0].popups[0].clone();
        let edit = || Action::Edit {
            file: b"/src/\xff.rs".to_vec(),
        };

        // An unavailable editor changes nothing.
        let before = workspace.snapshot();
        let failure = workspace
            .dispatch(
                "missing",
                Some(&files.session),
                edit(),
                |_, _| Err("no hx".into()),
                |_| Err("nothing starts".into()),
            )
            .unwrap_err();
        assert_eq!(failure.code, "edit-command");
        assert_eq!(workspace.snapshot(), before);

        let mut started = Vec::new();
        workspace
            .dispatch(
                "edit",
                Some(&files.session),
                edit(),
                prepared,
                |operation| {
                    started.push(operation);
                    Ok(())
                },
            )
            .unwrap();
        let [
            SessionOperation::Start {
                directory,
                command: LaunchCommand::Tool(argv),
                tab: None,
                ..
            },
        ] = started.as_slice()
        else {
            panic!("one editor pane starts: {started:?}");
        };
        assert_eq!(directory, &root);
        assert_eq!(
            argv,
            &["hx".into(), OsString::from_vec(b"/src/\xff.rs".to_vec())]
        );
        // The editor is the tab's selected pane, and Files is hidden but kept.
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.panes().count(), 2);
        assert_eq!(tab.selected_pane.as_deref(), Some("p2"));
        assert_eq!(tab.popups, std::slice::from_ref(&files));
        assert_eq!(tab.selected_popup, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_command_pane_runs_its_exact_argv_only_for_a_calling_terminal() {
        let root = std::env::temp_dir().join(format!("eon-command-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let command = || Action::CreateCommandPane {
            argv: vec![b"claude".to_vec(), b"prime \xff".to_vec()],
            focus: true,
        };
        let before = workspace.snapshot();

        // Without a calling terminal, or with an unavailable program, nothing
        // changes.
        let failure = workspace
            .dispatch("uncalled", None, command(), prepared, |_| {
                Err("nothing starts".into())
            })
            .unwrap_err();
        assert_eq!(failure.code, "caller-required");
        let failure = workspace
            .dispatch(
                "missing",
                Some("session-1"),
                command(),
                |_, _| Err("no claude".into()),
                |_| Err("nothing starts".into()),
            )
            .unwrap_err();
        assert_eq!(failure.code, "pane-command");
        assert_eq!(workspace.snapshot(), before);

        let mut started = Vec::new();
        workspace
            .dispatch("run", Some("session-1"), command(), prepared, |operation| {
                started.push(operation);
                Ok(())
            })
            .unwrap();
        let [
            SessionOperation::Start {
                directory,
                command: LaunchCommand::Tool(argv),
                ..
            },
        ] = started.as_slice()
        else {
            panic!("one command pane starts: {started:?}");
        };
        assert_eq!(directory, &root);
        assert_eq!(
            argv,
            &["claude".into(), OsString::from_vec(b"prime \xff".to_vec())]
        );
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.panes().count(), 2);
        assert_eq!(tab.selected_pane.as_deref(), Some("p2"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chooser_retarget_parks_a_hidden_agent_as_the_first_pane_of_a_paneless_tab() {
        let root = std::env::temp_dir().join(format!("eon-park-{}", std::process::id()));
        let next = root.join("next");
        fs::create_dir_all(&next).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let invoke = |entry: &str, expected: Option<&str>| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: expected.map(Into::into),
            intent: InvokeIntent::Toggle,
        };
        workspace
            .dispatch("agent", None, invoke("agent", None), prepared, |_| Ok(()))
            .unwrap();
        let agent = workspace.snapshot().tabs[0].popups[0].clone();
        workspace
            .dispatch(
                "hide",
                None,
                invoke("agent", Some(&agent.id)),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        workspace
            .session_ended("session-1", SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        assert!(workspace.snapshot().tabs[0].columns.is_empty());
        workspace
            .dispatch("project", None, invoke("project", None), prepared, |_| {
                Ok(())
            })
            .unwrap();
        let project = workspace.snapshot().tabs[0]
            .popups
            .iter()
            .find(|popup| popup.entry == "project")
            .unwrap()
            .clone();
        workspace
            .dispatch(
                "commit",
                None,
                Action::CommitDirectory {
                    target: PopupTarget {
                        tab: "t1".into(),
                        instance: project.id.clone(),
                    },
                    directory: next.as_os_str().as_bytes().to_vec(),
                },
                prepared,
                |_| Err("parking starts and stops nothing".into()),
            )
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.directory, next.as_os_str().as_bytes());
        assert_eq!(tab.popups, std::slice::from_ref(&project));
        let [column] = tab.columns.as_slice() else {
            panic!("the parked Agent is the tab's only column");
        };
        assert_eq!(column.panes.len(), 1);
        assert_eq!(column.panes[0].session, agent.session);
        assert_eq!(tab.selected_pane.as_ref(), Some(&column.panes[0].id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_tabs_start_at_home_and_reject_unavailable_home_without_mutation() {
        let root = std::env::temp_dir().join(format!("eon-home-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some(root.clone()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let initial = workspace.snapshot();
        let mut launches = Vec::new();
        workspace
            .dispatch(
                "new",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |operation| {
                    launches.push(operation);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(workspace.snapshot().tabs[0], initial.tabs[0]);
        assert_eq!(
            workspace.snapshot().tabs[1].directory,
            root.as_os_str().as_bytes()
        );
        assert!(
            matches!(&launches[..], [SessionOperation::Start { directory, command: LaunchCommand::Project { .. }, .. }] if directory == &root)
        );

        let picker = workspace.snapshot().tabs[1].popups[0].session.clone();
        let error = workspace
            .session_ended(&picker, SessionEnd::Exited, |_| Err("start failed".into()))
            .unwrap_err();
        assert_eq!(error.code, "session-start");
        assert_eq!(workspace.snapshot(), initial);
        eon_workspace_protocol::v22::encode_response(
            &eon_workspace_protocol::v22::Response::Snapshot(workspace.snapshot()),
        )
        .unwrap();

        for home in [None, Some("relative".into()), Some(root.join("missing"))] {
            workspace.workspaces[0].home = home;
            let before = workspace.snapshot();
            let error = workspace
                .dispatch(
                    "invalid",
                    None,
                    Action::Workspace(WorkspaceAction::CreateTab),
                    prepared,
                    |_| panic!("invalid home launched a terminal"),
                )
                .unwrap_err();
            assert_eq!(error.code, "invalid-directory");
            assert_eq!(workspace.snapshot(), before);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_keyless_project_still_opens_for_a_pending_tab() {
        let overridden = PopupKey::Overridden {
            default: Shortcut {
                modifiers: ALT,
                key: "KeyA".into(),
            },
            by: "agent".into(),
        };
        for (key, text, json_key) in [
            (
                PopupKey::Unbound,
                "key=none",
                r#""keybinding":null,"overridden_by":null,"default_keybinding":null}"#,
            ),
            (
                overridden,
                "key=none",
                r#""keybinding":null,"overridden_by":"agent","default_keybinding":"Alt+A"}"#,
            ),
        ] {
            let mut catalog = catalog();
            catalog.entries[0].shortcut = key.clone();
            let workspace = WorkspaceSet::pending(
                "/runtime".into(),
                "/".into(),
                Some("/".into()),
                catalog,
                prepared,
                |_| Ok(()),
            )
            .unwrap();
            let state = workspace.snapshot();
            assert!(state.tabs[0].pending);
            assert_eq!(state.tabs[0].popups[0].entry, "project");
            assert_eq!(state.entries[0].shortcut, key);
            assert!(human(&state).contains(&format!("entry=project label=Project {text} ")));
            assert!(
                json(&state).contains(&format!(r#"{{"id":"project","label":"Project",{json_key}"#))
            );
            assert!(json(&state).contains(
                r#"{"id":"agent","label":"Agent","keybinding":"Alt+A","overridden_by":null,"default_keybinding":null}"#
            ));
            eon_workspace_protocol::v22::encode_response(
                &eon_workspace_protocol::v22::Response::Snapshot(state),
            )
            .unwrap();
        }
    }

    #[test]
    fn pending_tabs_keep_independent_pickers_and_close_only_the_selected_tab() {
        let mut workspace = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        workspace
            .dispatch(
                "new-t2",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let state = workspace.snapshot();
        assert_eq!(state.active_tab, "t2");
        assert_eq!(state.tabs.len(), 2);
        assert!(state.tabs.iter().all(|tab| tab.pending));
        assert_ne!(state.tabs[0].popups[0].id, state.tabs[1].popups[0].id);
        assert_ne!(
            state.tabs[0].popups[0].endpoint,
            state.tabs[1].popups[0].endpoint
        );
        let first = state.tabs[0].popups[0].clone();
        let second = state.tabs[1].popups[0].clone();

        workspace
            .dispatch(
                "focus-t1",
                None,
                Action::Workspace(WorkspaceAction::FocusId("t1".into())),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            workspace.prepare_close_tab("close-t1", None, "t1").unwrap(),
            std::slice::from_ref(&first.session)
        );
        workspace
            .session_ended(&first.session, SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t2");
        assert_eq!(workspace.snapshot().tabs[0].popups[0], second);
        assert!(
            workspace
                .dispatch(
                    "stale-t1",
                    None,
                    Action::CommitDirectory {
                        target: PopupTarget {
                            tab: "t1".into(),
                            instance: first.id,
                        },
                        directory: b"/".to_vec(),
                    },
                    prepared,
                    |_| Ok(()),
                )
                .is_err()
        );

        let before_failed_launch = workspace.snapshot();
        assert_eq!(
            workspace
                .dispatch(
                    "new-fails",
                    None,
                    Action::Workspace(WorkspaceAction::CreateTab),
                    prepared,
                    |_| Err("launch failed".into()),
                )
                .unwrap_err()
                .code,
            "popup-start"
        );
        assert_eq!(workspace.snapshot(), before_failed_launch);

        workspace
            .dispatch(
                "new-t3",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let third = workspace.snapshot().tabs[1].popups[0].clone();
        assert_eq!(
            workspace.prepare_close_tab("close-t3", None, "t3").unwrap(),
            std::slice::from_ref(&third.session)
        );
        workspace
            .session_ended(&third.session, SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot(), before_failed_launch);

        workspace
            .dispatch(
                "accept-t2",
                None,
                Action::CommitDirectory {
                    target: PopupTarget {
                        tab: "t2".into(),
                        instance: second.id,
                    },
                    directory: b"/".to_vec(),
                },
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert!(!workspace.snapshot().tabs[0].pending);
        assert_eq!(workspace.snapshot().tabs[0].panes().count(), 1);
    }

    #[test]
    fn background_pending_picker_exit_keeps_the_selected_tab() {
        let mut workspace = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        for number in 2..=3 {
            workspace
                .dispatch(
                    &format!("new-t{number}"),
                    None,
                    Action::Workspace(WorkspaceAction::CreateTab),
                    prepared,
                    |_| Ok(()),
                )
                .unwrap();
        }
        let second_picker = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&second_picker, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.active_tab, "t3");
        assert_eq!(
            snapshot
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["t1", "t2", "t3"]
        );
        assert!(!snapshot.tabs[1].pending);
        assert!(snapshot.tabs[1].popups.is_empty());
        assert_eq!(snapshot.tabs[1].panes().count(), 1);
    }
}
