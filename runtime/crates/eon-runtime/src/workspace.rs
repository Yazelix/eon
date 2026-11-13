use super::cli::WORKSPACE_PICKER;
use super::managed_environment::{PopupCatalog, PopupCommand, PopupDefinition, PreparedCommand};
use super::session_registry::{Completion, Operation, Registration, Registry};
use eon_workspace_protocol::v42::{
    Action, Attention, AttentionKind, AttentionState, BusyTerminals, ClaudeQuotaState, CloseEnd,
    CodexQuotaState, Column as SnapshotColumn, Direction, Editing, FILES_ENTRY, Failure,
    InvokeIntent, MAX_COLUMNS, MAX_DIRECTORY_BYTES, MAX_SESSIONS, MAX_TABS, PROJECT_ENTRY,
    Pane as SnapshotPane, Popup as SnapshotPopup, PopupEntry as SnapshotEntry, PopupKey,
    PopupTarget, QuotaFailure, QuotaWindow, Screen, Sent, Side, Sidebar, Snapshot,
    Tab as SnapshotTab, TargetId, TerminalHistory, TerminalTarget, TerminalText, WorkspaceAction,
    WorkspaceTarget, check_popup_action,
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const MAX_RECENT_REQUESTS: usize = 256;
// Fixed partitions keep every supervisor's local identities disjoint (EON-C15).
const IDS_PER_SUPERVISOR: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) endpoint: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LaunchCommand {
    Pane,
    /// The tab's Files pair (EON-C18): `quick` starts it on quick search, and
    /// `pending` marks the pair of a pending tab until that tab commits.
    Files {
        argv: Vec<OsString>,
        quick: bool,
        pending: bool,
    },
    Tool(Vec<OsString>),
    Editor(PreparedCommand),
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
    /// The pending pair `session` stays on as its committed tab's Files.
    SettlePair(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: Session,
    pinned_name: Option<String>,
    tag: Option<String>,
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
    /// The popup a shown transient popup covers, shown again when it ends.
    covered_popup: Option<String>,
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
    limit: usize,
    next_workspace: usize,
    next_tab: usize,
    next_pane: usize,
    next_session: usize,
    next_popup: usize,
    /// terminal-1, whose endpoint is orbit.sock, waits for the window's first
    /// pane after the first pending tab's pair took the next terminal.
    first_reserved: bool,
}

impl IdAllocator {
    pub(crate) fn new(runtime: PathBuf, range: usize) -> Result<Self, String> {
        let first = range
            .checked_mul(IDS_PER_SUPERVISOR)
            .and_then(|base| base.checked_add(1))
            .ok_or("capacity: workspace identity range overflow")?;
        let limit = first
            .checked_add(IDS_PER_SUPERVISOR)
            .ok_or("capacity: workspace identity range overflow")?;
        Ok(Self {
            runtime,
            limit,
            next_workspace: first,
            next_tab: first,
            next_pane: first,
            next_session: 1,
            next_popup: first,
            first_reserved: false,
        })
    }

    fn check_pane_identity(&self) -> Result<(), Failure> {
        if self.next_pane >= self.limit {
            return Err(action_error(
                "capacity",
                "session exhausted pane identities",
            ));
        }
        Ok(())
    }

    fn pane_number(&self) -> usize {
        if self.first_reserved {
            1
        } else {
            self.next_session
        }
    }

    fn pane_session(&self) -> Session {
        let number = self.pane_number();
        Session {
            id: format!("terminal-{number}"),
            endpoint: self.runtime.join(if number == 1 {
                "orbit.sock".into()
            } else {
                format!("terminal-{number}.sock")
            }),
        }
    }

    fn commit_pane_session(&mut self) {
        if self.first_reserved {
            self.first_reserved = false;
        } else {
            self.next_session += 1;
        }
    }

    /// A popup never takes terminal-1: it stays the first pane's, which
    /// `eon run -- COMMAND` starts in (EON-C18).
    fn popup_session(&mut self) -> Session {
        if self.next_session == 1 {
            self.next_session = 2;
            self.first_reserved = true;
        }
        let id = format!("terminal-{}", self.next_session);
        let endpoint = self.runtime.join(format!("{id}.sock"));
        Session { id, endpoint }
    }
}

/// Supervisor-wide admission, identities and replay protection; topology stays local.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceSet {
    registry: Option<Registry>,
    workspaces: Vec<Workspace>,
    shown: usize,
    ids: IdAllocator,
    recent_inspections: VecDeque<String>,
    recent_mutations: VecDeque<String>,
    in_flight: Option<String>,
    /// Every live workspace once, most recently shown first, so the shown one leads.
    recent_workspaces: VecDeque<String>,
}

impl WorkspaceSet {
    #[cfg(test)]
    pub(crate) fn pending(
        runtime: PathBuf,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let registry =
            Registry::in_test_directory(super::supervisor::temporary_directory().join("sessions"));
        let registration = registry.create(None).unwrap();
        Self::registered_pending(
            Some(registry),
            registration,
            IdAllocator::new(runtime, 0)?,
            directory,
            home,
            catalog,
            true,
            prepare,
            operate,
        )
    }

    #[cfg(test)]
    fn insert_pending(
        &mut self,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let registration = self.registry.as_ref().unwrap().create(None).unwrap();
        self.insert_registered_pending(
            registration,
            directory,
            home,
            catalog,
            true,
            prepare,
            operate,
        )
    }

    #[cfg(test)]
    pub(crate) fn with_recovered_sessions(
        runtime: PathBuf,
        directory: PathBuf,
        home: Option<PathBuf>,
        sessions: Vec<(usize, Session)>,
        catalog: PopupCatalog,
    ) -> Result<Self, String> {
        let registry =
            Registry::in_test_directory(super::supervisor::temporary_directory().join("sessions"));
        let registration = registry.create(None).unwrap();
        Self::registered_recovered(
            Some(registry),
            Some(registration),
            IdAllocator::new(runtime, 0)?,
            directory,
            home,
            sessions,
            catalog,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn registered_pending(
        registry: Option<Registry>,
        registration: Registration,
        ids: IdAllocator,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        pair: bool,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut set = Self {
            registry,
            workspaces: Vec::new(),
            shown: 0,
            ids,
            recent_inspections: VecDeque::new(),
            recent_workspaces: VecDeque::new(),
            recent_mutations: VecDeque::new(),
            in_flight: None,
        };
        set.insert_registered_pending(
            registration,
            directory,
            home,
            catalog,
            pair,
            prepare,
            operate,
        )?;
        Ok(set)
    }

    fn pending_allowance(&self) -> Result<usize, Failure> {
        if self.workspaces.len() >= eon_workspace_protocol::v42::MAX_WORKSPACES {
            return Err(action_error(
                "session-limit",
                "a window holds at most 32 sessions",
            ));
        }
        if self.ids.next_workspace >= self.ids.limit
            || self.ids.next_tab >= self.ids.limit
            || self.ids.next_pane >= self.ids.limit
            || self.ids.next_session == usize::MAX
            || self.ids.next_popup >= self.ids.limit
        {
            return Err(action_error(
                "capacity",
                "session exhausted local identities",
            ));
        }
        let allowance = MAX_SESSIONS
            - self
                .workspaces
                .iter()
                .map(Workspace::session_count)
                .sum::<usize>();
        if allowance == 0 {
            return Err(action_error(
                "capacity",
                format!("window is limited to {MAX_SESSIONS} terminals"),
            ));
        }
        Ok(allowance)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_registered_pending(
        &mut self,
        registration: Registration,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        pair: bool,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let allowance = self.pending_allowance().map_err(|error| error.detail)?;
        let workspace = Workspace::pending(
            registration,
            &mut self.ids,
            allowance,
            directory,
            home,
            catalog,
            pair,
            prepare,
            operate,
        )?;
        self.ids.next_workspace += 1;
        self.ids.next_tab += 1;
        self.recent_workspaces.push_back(workspace.id.clone());
        self.workspaces.push(workspace);
        Ok(())
    }

    pub(crate) fn registered_recovered(
        registry: Option<Registry>,
        registration: Option<Registration>,
        mut ids: IdAllocator,
        directory: PathBuf,
        home: Option<PathBuf>,
        sessions: Vec<(usize, Session)>,
        catalog: PopupCatalog,
    ) -> Result<Self, String> {
        let next_session = sessions
            .last()
            .ok_or("cannot recover an empty session")?
            .0
            .checked_add(1)
            .ok_or("recovered terminal identity leaves no next terminal identity")?;
        if sessions.len() > ids.limit.saturating_sub(ids.next_pane) {
            return Err("capacity: recovered panes exceed identity range".into());
        }
        let workspace = Workspace::with_recovered_sessions(
            registration,
            &mut ids,
            directory,
            home,
            sessions,
            catalog,
        );
        ids.next_workspace += 1;
        ids.next_tab += 1;
        ids.next_session = next_session;
        let recent_workspaces = VecDeque::from([workspace.id.clone()]);
        Ok(Self {
            registry,
            workspaces: vec![workspace],
            shown: 0,
            ids,
            recent_inspections: VecDeque::new(),
            recent_workspaces,
            recent_mutations: VecDeque::new(),
            in_flight: None,
        })
    }

    pub(crate) fn prepare_registration(
        &self,
        request_id: &str,
        caller: Option<&str>,
        action: &Action,
    ) -> Result<Operation, Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        match action {
            Action::Workspace(WorkspaceAction::NewWorkspace { name, .. }) => {
                if let Some(name) = name {
                    eon_workspace_protocol::v42::validate_workspace_name(name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                self.pending_allowance()?;
                let home = self.workspaces[self.shown].home.as_ref().ok_or_else(|| {
                    action_error(
                        "invalid-directory",
                        "cannot resolve the current user home directory",
                    )
                })?;
                validated_directory(home.as_os_str().as_bytes().to_vec())?;
                Ok(Operation::Create { name: name.clone() })
            }
            Action::Workspace(WorkspaceAction::RenameWorkspace { workspace, name }) => {
                if !name.is_empty() {
                    eon_workspace_protocol::v42::validate_workspace_name(name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                let target = workspace
                    .as_ref()
                    .map(|target| self.resolve_workspace(target))
                    .transpose()?
                    .unwrap_or(index);
                let id = self.workspaces[target]
                    .persistent_id
                    .clone()
                    .ok_or_else(|| {
                        action_error(
                            "session-registry",
                            "recovered session has no confirmed registration",
                        )
                    })?;
                Ok(Operation::Rename {
                    id,
                    name: (!name.is_empty()).then(|| name.clone()),
                })
            }
            _ => Err(action_error(
                "malformed-action",
                "action does not mutate the registry",
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn registry(&self) -> &Registry {
        self.registry.as_ref().unwrap()
    }

    pub(crate) fn reserve_registration(&mut self, request_id: &str) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        if self.in_flight.is_some() {
            return Err(super::session_registry::Failure::busy().into_control());
        }
        self.in_flight = Some(request_id.to_owned());
        Ok(())
    }

    pub(crate) fn release_registration(&mut self, request_id: &str) {
        if self.in_flight.as_deref() == Some(request_id) {
            self.in_flight = None;
        }
    }

    pub(crate) fn remember_registration(&mut self, request_id: &str) {
        if !self.recent_mutations.iter().any(|id| id == request_id) {
            self.remember(request_id, true);
        }
    }

    pub(crate) fn registered_recovery(&mut self, target: &str, result: Completion) {
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == target && workspace.persistent_id.is_none())
        else {
            return;
        };
        match result {
            Ok(Some(record)) => {
                workspace.persistent_id = Some(record.id);
                workspace.name = record.name;
                workspace.registration_error = None;
            }
            Ok(None) => {
                workspace.registration_error = Some(action_error(
                    "session-registry",
                    "recovered registration is missing",
                ))
            }
            Err(error) => workspace.registration_error = Some(error.into_control()),
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        action: Action,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        // State tests execute prepared I/O directly; supervisor tests exercise the worker.
        let registration = if matches!(
            action,
            Action::Workspace(
                WorkspaceAction::NewWorkspace { .. } | WorkspaceAction::RenameWorkspace { .. }
            )
        ) {
            let operation = self.prepare_registration(request_id, caller, &action)?;
            match self.registry.as_ref().unwrap().perform(operation) {
                Ok(record) => record,
                Err(error) => {
                    if error.reserved_id.is_some()
                        || error.publication == super::session_registry::Publication::Uncertain
                    {
                        self.remember_registration(request_id);
                    }
                    return Err(error.into_control());
                }
            }
        } else {
            None
        };
        self.apply_action(request_id, caller, action, registration, prepare, operate)
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
                    action_error("unknown-session", format!("session {id} is not live"))
                })?,
            None => self.shown,
        };
        let mut snapshot = self.workspaces[index].snapshot();
        snapshot.active_workspace = self.workspaces[self.shown].id.clone();
        snapshot.workspaces = self
            .workspaces
            .iter()
            .map(|workspace| eon_workspace_protocol::v42::WorkspaceInfo {
                id: workspace.id.clone(),
                persistent_id: workspace.persistent_id.clone(),
                registration_error: workspace.registration_error.clone(),
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

    /// Each tab's Files pair terminal, and whether its tab shows it (EON-C18).
    pub(crate) fn pairs(&self) -> impl Iterator<Item = (&str, bool)> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .flat_map(|tab| {
                tab.popups
                    .iter()
                    .filter(|popup| popup.entry == FILES_ENTRY)
                    .map(move |popup| {
                        (
                            popup.session.id.as_str(),
                            tab.selected_popup.as_deref() == Some(&popup.id),
                        )
                    })
            })
    }

    /// The terminals of pending tabs' pairs. Lifecycle inspection and Stop
    /// do not count them: each is a popup, not a durable terminal (EON-C18).
    pub(crate) fn pending_pair_sessions(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .filter(|tab| tab.pending)
            .flat_map(|tab| &tab.popups)
            .map(|popup| popup.session.id.clone())
            .collect()
    }

    /// Every pane and popup terminal of the window, in every workspace.
    fn terminal_targets(&self) -> impl Iterator<Item = TerminalTarget> + '_ {
        self.workspaces.iter().flat_map(|workspace| {
            workspace.tabs.iter().flat_map(move |tab| {
                let target = move |id, session: &Session| TerminalTarget {
                    workspace: workspace.id.clone(),
                    tab: tab.id.clone(),
                    id,
                    terminal: session.id.clone(),
                };
                tab.panes()
                    .map(move |pane| target(TargetId::Pane(pane.id.clone()), &pane.session))
                    .chain(tab.popups.iter().map(move |popup| {
                        target(TargetId::Popup(popup.id.clone()), &popup.session)
                    }))
            })
        })
    }

    /// The pane or popup `id` in any workspace of the window, shown or hidden
    /// (EON-C35, EON-C36). Accepts `request_id` as an inspection or, when
    /// `mutating`, a mutation, under the same replay protection as other
    /// requests; it changes no workspace state.
    pub(crate) fn terminal_target(
        &mut self,
        request_id: &str,
        mutating: bool,
        id: &str,
    ) -> Result<TerminalTarget, Failure> {
        self.reject_duplicate(request_id)?;
        let target = self.find_terminal_target(id)?;
        self.remember(request_id, mutating);
        Ok(target)
    }

    /// The pane or popup `id` in any workspace of the window.
    fn find_terminal_target(&self, id: &str) -> Result<TerminalTarget, Failure> {
        self.terminal_targets()
            .find(|target| matches!(&target.id, TargetId::Pane(item) | TargetId::Popup(item) if item == id))
            .ok_or_else(|| {
                action_error(
                    "unknown-target",
                    format!("no pane or popup {id} is live in this window"),
                )
            })
    }

    /// The pane or popup that runs terminal `session`.
    pub(crate) fn session_target(&self, session: &str) -> Option<TerminalTarget> {
        self.terminal_targets()
            .find(|target| target.terminal == session)
    }

    pub(crate) fn window_name(&self) -> String {
        let workspace = &self.workspaces[self.shown];
        let directory = &workspace.tabs[workspace.active].directory;
        let name = workspace.name.clone().unwrap_or_else(|| {
            if workspace.home.as_ref() == Some(directory) {
                "~".into()
            } else {
                directory
                    .file_name()
                    .unwrap_or(directory.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            }
        });
        name.chars()
            .map(|character| {
                if eon_workspace_protocol::v42::pane_label_character(character) {
                    character
                } else {
                    '\u{fffd}'
                }
            })
            .take(eon_workspace_protocol::v42::MAX_WINDOW_NAME_CHARS)
            .collect()
    }

    pub(crate) fn home(&self) -> Option<&Path> {
        self.workspaces
            .first()
            .and_then(|workspace| workspace.home.as_deref())
    }

    fn resolve_workspace(&self, target: &WorkspaceTarget) -> Result<usize, Failure> {
        resolve_target(
            self.workspaces.iter().map(|workspace| {
                (
                    workspace.id.as_str(),
                    workspace.name.as_deref(),
                    workspace.persistent_id.as_deref(),
                )
            }),
            target,
        )
    }

    pub(crate) fn inspected_workspace_id(
        &self,
        target: &WorkspaceTarget,
    ) -> Result<String, Failure> {
        Ok(self.workspaces[self.resolve_workspace(target)?].id.clone())
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
        target: &WorkspaceTarget,
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
                        "the calling terminal is not part of this session",
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

    /// A direct attention focus crosses the caller's boundary only for another live tab.
    pub(crate) fn attention_destination(&self, caller: Option<&str>, tab: &str) -> Option<&str> {
        let caller = self.caller_workspace(caller).ok()?;
        self.workspaces
            .iter()
            .enumerate()
            .find(|(index, workspace)| {
                *index != caller && workspace.tabs.iter().any(|item| item.id == tab)
            })
            .map(|(_, workspace)| workspace.id.as_str())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn edit(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        file: Vec<u8>,
        mut prepare: impl FnMut(&Path, &Path) -> Result<super::editor::EditCommand, String>,
        mut reuse: impl FnMut(&Session, &Path) -> Result<bool, Failure>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        let allowance = self.allowance(index);
        // An exchange can execute despite a lost reply: never replay its request ID.
        self.remember(request_id, true);
        let workspace = &mut self.workspaces[index];
        let (target, pane) = workspace.target_tab(caller)?;
        workspace.require_established(target)?;
        let file = PathBuf::from(OsString::from_vec(file));
        let mut command = prepare(&workspace.tabs[target].directory, &file)
            .map_err(|detail| action_error("edit-command", detail))?;
        if command.popup && workspace.definition("editor").is_ok() {
            let live = workspace.tabs[target]
                .popups
                .iter()
                .find(|popup| popup.entry == "editor")
                .cloned();
            if let Some(popup) = live {
                let displaced = workspace.displaced_transient(target, Some(&popup.id))?;
                if reuse(&popup.session, &file)? {
                    if let Some(displaced) = displaced {
                        workspace.stop_popup(target, displaced, &mut operate)?;
                    }
                    let tab = &mut workspace.tabs[target];
                    tab.covered_popup = None;
                    tab.selected_popup = Some(popup.id);
                    return Ok(());
                }
            } else {
                command.command.argv.push(file.into_os_string());
                return workspace.open_popup(
                    &mut self.ids,
                    allowance,
                    target,
                    "editor",
                    &mut |_, _| Ok(command.command.clone()),
                    &mut operate,
                    false,
                );
            }
        }
        command.command.argv.push(file.into_os_string());
        let previous = pane.map(|pane| workspace.select(target, pane));
        let result = workspace.create_pane(
            &mut self.ids,
            allowance,
            target,
            caller.is_some(),
            true,
            LaunchCommand::Tool(command.command.argv),
            &mut operate,
        );
        if result.is_err()
            && let Some(previous) = previous
        {
            workspace.restore(previous);
        }
        result
    }

    pub(crate) fn apply_action(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        action: Action,
        registration: Option<Registration>,
        prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        if let Action::FocusAttention { tab, target } = &action
            && let Some(workspace) = self.attention_destination(caller, tab).map(str::to_owned)
        {
            return self.jump(request_id, caller, &workspace, tab, target);
        }
        let mutating = !matches!(
            action,
            Action::Workspace(WorkspaceAction::Inspect | WorkspaceAction::InspectWorkspace { .. })
        );
        match action {
            Action::RenamePane { pane, name, tag } => {
                let name = name.as_deref().map(str::trim);
                let tag = tag.as_deref().map(str::trim);
                eon_workspace_protocol::v42::validate_pane_rename(name, tag)
                    .map_err(|error| action_error("malformed-action", error.to_string()))?;
                let target = self.workspaces[index]
                    .tabs
                    .iter_mut()
                    .flat_map(|tab| &mut tab.columns)
                    .flat_map(|column| &mut column.panes)
                    .find(|target| target.id == pane)
                    .ok_or_else(|| {
                        action_error("unknown-pane", format!("pane {pane} is not live"))
                    })?;
                target.pinned_name = name.filter(|name| !name.is_empty()).map(str::to_owned);
                if let Some(tag) = tag {
                    target.tag = (!tag.is_empty()).then(|| tag.to_owned());
                }
            }
            Action::Workspace(WorkspaceAction::NewWorkspace { name, focus }) => {
                if let Some(name) = &name {
                    eon_workspace_protocol::v42::validate_workspace_name(name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                self.pending_allowance()?;
                let shown = &self.workspaces[self.shown];
                let home = shown.home.clone().ok_or_else(|| {
                    action_error(
                        "invalid-directory",
                        "cannot resolve the current user home directory",
                    )
                })?;
                let directory = validated_directory(home.as_os_str().as_bytes().to_vec())?;
                let catalog = shown.catalog.clone();
                let registration = registration.ok_or_else(|| {
                    action_error(
                        "session-registry",
                        "creation requires confirmed registration",
                    )
                })?;
                self.remember(request_id, true);
                let persistent_id = registration.id.clone();
                self.insert_registered_pending(
                    registration,
                    directory,
                    Some(home),
                    catalog,
                    true,
                    prepare,
                    operate,
                )
                .map_err(|detail| {
                    action_error(
                        "terminal-start",
                        format!("{detail}; registered persistent_id={persistent_id}"),
                    )
                })?;
                let workspace = self.workspaces.pop().expect("new workspace was inserted");
                let added = self.shown + 1;
                self.workspaces.insert(added, workspace);
                if focus {
                    self.show(added);
                }
                return Ok(());
            }
            Action::Workspace(WorkspaceAction::InspectWorkspace { workspace }) => {
                self.resolve_workspace(&workspace)?;
            }
            Action::Workspace(WorkspaceAction::SwitchWorkspace { workspace }) => {
                let target = self.resolve_workspace(&workspace)?;
                if target != self.shown {
                    self.show(target);
                }
            }
            Action::Workspace(WorkspaceAction::RenameWorkspace { workspace, name }) => {
                if !name.is_empty() {
                    eon_workspace_protocol::v42::validate_workspace_name(&name)
                        .map_err(|error| action_error("malformed-action", error.to_string()))?;
                }
                let target = workspace
                    .as_ref()
                    .map(|id| self.resolve_workspace(id))
                    .transpose()?
                    .unwrap_or(index);
                let registration = registration.ok_or_else(|| {
                    action_error("session-registry", "rename requires confirmed persistence")
                })?;
                if self.workspaces[target].persistent_id.as_deref()
                    != Some(registration.id.as_str())
                {
                    return Err(action_error(
                        "unknown-session",
                        "registered target left this window",
                    ));
                }
                self.workspaces[target].name = registration.name;
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
    /// `workspace`, which then becomes the shown workspace (EON-C33). Also used by
    /// `FocusAttention` when its tab belongs to another workspace (EON-C31).
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
                    "unknown-session",
                    format!("session {workspace} is not live"),
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

    pub(crate) fn prepare_close_column(
        &mut self,
        request_id: &str,
        caller: Option<&str>,
        pane: &str,
    ) -> Result<(Vec<String>, CloseEnd), Failure> {
        self.reject_duplicate(request_id)?;
        let index = self.caller_workspace(caller)?;
        let (sessions, whole_tab) = self.workspaces[index].prepare_close_column(caller, pane)?;
        let end = if !whole_tab {
            CloseEnd::Column
        } else if self
            .workspaces
            .iter()
            .enumerate()
            .any(|(other, workspace)| other != index && workspace.sessions().next().is_some())
        {
            CloseEnd::Session
        } else {
            CloseEnd::Window
        };
        self.remember(request_id, true);
        Ok((sessions, end))
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
                    "unknown-terminal",
                    format!("terminal {id} is not in the session"),
                )
            })?;
        let allowance = self.allowance(index);
        let result =
            self.workspaces[index].session_ended(&mut self.ids, allowance, id, end, operate);
        self.prune_empty();
        result
    }

    /// Dismisses every pending tab's pair, as presentation detachment does
    /// (EON-C18): each stays on as its tab's Files and the tab's first pane
    /// starts. Every pair is attempted; the first failure is returned.
    pub(crate) fn dismiss_pending_pairs(
        &mut self,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let mut result = Ok(());
        for index in 0..self.workspaces.len() {
            for target in self.workspaces[index].pending_pairs() {
                let allowance = self.allowance(index);
                let dismissed = self.workspaces[index].dismiss_popup(
                    &mut self.ids,
                    allowance,
                    &target,
                    &mut operate,
                );
                if result.is_ok() {
                    result = dismissed;
                }
            }
        }
        self.prune_empty();
        result
    }

    /// The terminal of the Files pair that invoking `entry` in `tab` of the
    /// caller's workspace acts on, and whether it is shown; none for other
    /// entries or a tab without a pair.
    pub(crate) fn pair(
        &self,
        caller: Option<&str>,
        tab: &str,
        entry: &str,
    ) -> Option<(String, bool)> {
        if !matches!(entry, PROJECT_ENTRY | FILES_ENTRY) {
            return None;
        }
        self.workspaces[self.caller_workspace(caller).ok()?].pair(tab)
    }

    /// The terminal of the Workspaces picker that toggling `entry` in `tab` of the caller's
    /// workspace would hide: the tab's selected popup, when `entry` runs the runtime's own
    /// picker. Its key confirms the highlighted row there instead (EON-C33). A picker launched in
    /// another directory than its tab's has none: the invocation relaunches it.
    pub(crate) fn shown_picker(
        &self,
        caller: Option<&str>,
        tab: &str,
        entry: &str,
    ) -> Option<String> {
        self.workspaces[self.caller_workspace(caller).ok()?].shown_picker(tab, entry)
    }

    /// Whether `entry` of the caller's workspace is a Scrollback popup.
    pub(crate) fn is_scrollback(&self, caller: Option<&str>, entry: &str) -> bool {
        self.caller_workspace(caller).is_ok_and(|index| {
            self.workspaces[index]
                .definition(entry)
                .is_ok_and(|definition| definition.command == PopupCommand::Scrollback)
        })
    }

    /// The terminal invoking `entry` in `tab` of the caller's workspace
    /// would snapshot, as its pane or popup ID and its terminal: set only when
    /// `entry` is Scrollback and the invocation would open a new one in the
    /// active tab (EON-C10).
    pub(crate) fn scrollback_source(
        &self,
        caller: Option<&str>,
        tab: &str,
        entry: &str,
    ) -> Option<(String, String)> {
        if !self.is_scrollback(caller, entry) {
            return None;
        }
        let workspace = &self.workspaces[self.caller_workspace(caller).ok()?];
        let index = workspace.tabs.iter().position(|item| item.id == tab)?;
        let tab = &workspace.tabs[index];
        if index != workspace.active
            || tab.pending
            || tab.popups.iter().any(|popup| popup.entry == entry)
        {
            return None;
        }
        workspace.shown_terminal(index)
    }

    /// The pane or popup `id` that `eon pane history --edit` snapshots, and
    /// the Scrollback entry whose popup opens it over `id`'s tab: only the
    /// shown workspace's active tab, and only while that tab shows no
    /// Scrollback and no other transient popup, which opening it would stop
    /// (EON-C35, EON-C10). It changes nothing; the dispatch that opens the
    /// popup accepts `request_id`.
    pub(crate) fn scrollback_target(
        &self,
        request_id: &str,
        id: &str,
    ) -> Result<(TerminalTarget, String), Failure> {
        self.reject_duplicate(request_id)?;
        let target = self.find_terminal_target(id)?;
        let workspace = &self.workspaces[self.shown];
        let tab = &workspace.tabs[workspace.active];
        if target.workspace != workspace.id || target.tab != tab.id {
            return Err(action_error(
                "not-active",
                format!("{id} is not in the shown session's active tab"),
            ));
        }
        if tab.pending {
            return Err(action_error(
                "choose-directory-first",
                "choose the pending tab directory before opening a tool",
            ));
        }
        let entry = workspace
            .catalog
            .entries
            .iter()
            .find(|entry| entry.command == PopupCommand::Scrollback)
            .ok_or_else(|| action_error("unknown-popup", "the Scrollback popup is disabled"))?;
        if tab.popups.iter().any(|popup| popup.entry == entry.id) {
            return Err(action_error(
                "unavailable",
                format!("tab {} already shows a Scrollback popup", tab.id),
            ));
        }
        // An agent's edit never stops a transient popup the person has open.
        if let Some(shown) = tab.selected_popup.as_deref()
            && let Some(popup) = tab.popups.iter().find(|popup| popup.id == shown)
            && workspace
                .definition(&popup.entry)
                .is_ok_and(|definition| !definition.keep_alive)
        {
            return Err(action_error(
                "unavailable",
                format!("tab {} shows the transient popup {shown}", tab.id),
            ));
        }
        Ok((target, entry.id.clone()))
    }

    fn reject_duplicate(&self, request_id: &str) -> Result<(), Failure> {
        if self.in_flight.as_deref() == Some(request_id)
            || self
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
    persistent_id: Option<String>,
    registration_error: Option<Failure>,
    name: Option<String>,
    catalog: PopupCatalog,
    home: Option<PathBuf>,
    tabs: Vec<Tab>,
    active: usize,
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    fn pending(
        registration: Registration,
        ids: &mut IdAllocator,
        allowance: usize,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        pair: bool,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut workspace = Self {
            id: format!("s{}", ids.next_workspace),
            persistent_id: Some(registration.id),
            registration_error: None,
            name: registration.name,
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
                covered_popup: None,
            }],
            active: 0,
        };
        // Without its pair, as for a launch `--dir` (EON-C18), t1
        // starts its first pane at once.
        if !pair
            || workspace
                .open_popup(
                    ids,
                    allowance,
                    0,
                    FILES_ENTRY,
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
        registration: Option<Registration>,
        ids: &mut IdAllocator,
        directory: PathBuf,
        home: Option<PathBuf>,
        sessions: Vec<(usize, Session)>,
        catalog: PopupCatalog,
    ) -> Self {
        let panes = sessions
            .into_iter()
            .map(|(_, session)| {
                let id = format!("p{}", ids.next_pane);
                ids.next_pane += 1;
                Pane {
                    pinned_name: None,
                    tag: None,
                    id,
                    session,
                }
            })
            .collect::<Vec<_>>();
        Self {
            id: format!("s{}", ids.next_workspace),
            persistent_id: registration.as_ref().map(|record| record.id.clone()),
            registration_error: None,
            name: registration.and_then(|record| record.name),
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
                covered_popup: None,
            }],
            active: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            workspace: self.id.clone(),
            active_workspace: self.id.clone(),
            workspaces: vec![eon_workspace_protocol::v42::WorkspaceInfo {
                id: self.id.clone(),
                persistent_id: self.persistent_id.clone(),
                registration_error: self.registration_error.clone(),
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
                                    tag: pane.tag.clone(),
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
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: Sidebar::Off,
            attention_queue: Vec::new(),
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
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        check_popup_action(&self.snapshot(), &action)
            .map_err(|error| action_error("stale-popup", error.to_string()))?;
        if matches!(
            action,
            Action::CreateColumn
                | Action::Edit { .. }
                | Action::CreateCommandPane { .. }
                | Action::LaunchTab { .. }
                | Action::Workspace(WorkspaceAction::CreatePane { .. })
        ) {
            ids.check_pane_identity()?;
        }
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
                | Action::LaunchTab { focus: false, .. }
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
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
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
            Action::RenamePane { .. } | Action::ClosePane { .. } | Action::CloseColumn { .. } => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor stops a closed pane's terminal before pruning it",
                ));
            }
            // The supervisor's attention owner flips visibility once this
            // request is accepted.
            Action::ToggleSidebar | Action::SetSidebar { .. } => {}
            Action::FocusAttention { tab, target } => self.focus_attention(&tab, &target)?,
            Action::NextAttention | Action::PreviousAttention => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor resolves queue jumps to their target first",
                ));
            }
            Action::ReadTerminal { .. }
            | Action::ReadTerminalHistory { .. }
            | Action::EditTerminalHistory { .. }
            | Action::SendTerminal { .. } => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor reads and sends to terminals over their leases",
                ));
            }
            // The supervisor serves subscriptions on their own connection
            // (EON-C39), so none reaches the workspace owner.
            Action::Subscribe { .. } => {
                return Err(action_error(
                    "unavailable",
                    "this supervisor does not serve event subscriptions",
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
                    .map_err(|detail| action_error("terminal-start", detail))?;
            }
            Action::MovePaneSideways(side) => self.move_pane_sideways(target, side)?,
            Action::Edit { .. } => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor resolves editor routing for each edit",
                ));
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
                    LaunchCommand::Tool(argv.argv),
                    &mut operate,
                )?;
            }
            Action::LaunchTab {
                directory,
                argv,
                focus,
            } => self.launch_tab(
                ids,
                allowance,
                directory,
                argv,
                focus,
                called,
                &mut prepare,
                &mut operate,
            )?,
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
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match action {
            WorkspaceAction::Inspect => {}
            WorkspaceAction::InspectWorkspace { .. }
            | WorkspaceAction::NewWorkspace { .. }
            | WorkspaceAction::SwitchWorkspace { .. }
            | WorkspaceAction::RenameWorkspace { .. }
            | WorkspaceAction::CloseWorkspace { .. }
            | WorkspaceAction::InspectRuntime
            | WorkspaceAction::InspectPresentation
            | WorkspaceAction::Present { .. }
            | WorkspaceAction::ReloadPresentation { .. }
            | WorkspaceAction::CloseTab { .. } => {
                return Err(action_error(
                    "unavailable",
                    "supervisor lifecycle actions do not change sessions, tabs or panes",
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
                self.set_tab_directory(ids, allowance, &tab, directory, operate)?;
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
            .map_err(|detail| action_error("terminal-start", detail))?;
        if let Some((column, expanded)) = kept {
            self.tabs[target].columns[column].expanded = expanded;
        }
        Ok(())
    }

    fn create_tab(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if ids.next_tab >= ids.limit {
            return Err(action_error("capacity", "session exhausted tab identities"));
        }
        if self.tabs.len() >= MAX_TABS {
            return Err(action_error(
                "capacity",
                format!("session is limited to {MAX_TABS} tabs"),
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
            covered_popup: None,
        });
        let index = self.tabs.len() - 1;
        if let Err(error) =
            self.open_popup(ids, allowance, index, FILES_ENTRY, prepare, operate, true)
        {
            self.tabs.pop();
            return Err(error);
        }
        ids.next_tab += 1;
        self.active = index;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_tab(
        &mut self,
        ids: &mut IdAllocator,
        allowance: usize,
        directory: Vec<u8>,
        argv: Vec<Vec<u8>>,
        focus: bool,
        called: bool,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if !argv.is_empty() && !called {
            return Err(action_error(
                "caller-required",
                "a command tab must be requested from an Eon terminal",
            ));
        }
        let directory = validated_directory(directory)?;
        if ids.next_tab >= ids.limit {
            return Err(action_error("capacity", "session exhausted tab identities"));
        }
        if self.tabs.len() >= MAX_TABS {
            return Err(action_error(
                "capacity",
                format!("session is limited to {MAX_TABS} tabs"),
            ));
        }
        let command = if argv.is_empty() {
            LaunchCommand::Pane
        } else {
            LaunchCommand::Tool(
                prepare(
                    &PopupCommand::Argv(argv.into_iter().map(OsString::from_vec).collect()),
                    &directory,
                )
                .map_err(|detail| action_error("pane-command", detail))?
                .argv,
            )
        };
        self.tabs.push(Tab {
            id: format!("t{}", ids.next_tab),
            directory: directory.clone(),
            pending: false,
            columns: Vec::new(),
            selected_column: None,
            popups: Vec::new(),
            selected_popup: None,
            covered_popup: None,
        });
        let index = self.tabs.len() - 1;
        if let Err(detail) =
            self.start_pane_in(ids, allowance, index, &directory, false, command, operate)
        {
            self.tabs.pop();
            return Err(action_error("terminal-start", detail));
        }
        ids.next_tab += 1;
        if focus {
            self.active = index;
        }
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
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
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
        // Project and Files both show the tab's one pair (EON-C18).
        let pair = matches!(entry, PROJECT_ENTRY | FILES_ENTRY);
        if self.tabs[tab_index].pending && !pair {
            return Err(action_error(
                "choose-directory-first",
                "choose the pending tab directory before opening a tool",
            ));
        }
        self.definition(entry)?;
        let shown_entry = if pair { FILES_ENTRY } else { entry };
        let definition = self.definition(shown_entry)?.clone();
        if let Some(index) = self.tabs[tab_index]
            .popups
            .iter()
            .position(|popup| popup.entry == shown_entry)
        {
            let popup = self.tabs[tab_index].popups[index].clone();
            // Retargeting preserves Files' location and Editor's buffers.
            // A Scrollback snapshot belongs to no directory.
            if popup.entry != "files"
                && !matches!(
                    definition.command,
                    PopupCommand::Editor | PopupCommand::Scrollback
                )
                && popup.launch_directory != self.tabs[tab_index].directory
            {
                let candidate =
                    self.popup_candidate(ids, allowance, tab_index, &definition, true)?;
                let launch =
                    self.prepare_launch(tab_index, &candidate, &definition, false, prepare)?;
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
            if !selected {
                self.tabs[tab_index].covered_popup = None;
            }
            self.tabs[tab_index].selected_popup = Some(popup.id.clone());
            return Ok(());
        }
        // A tab showing no terminal has nothing to snapshot (EON-C10).
        if definition.command == PopupCommand::Scrollback
            && self.shown_terminal(tab_index).is_none()
        {
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
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
        pending: bool,
    ) -> Result<(), Failure> {
        // Project opens the pair on quick search, as a pending tab does.
        let quick = pending || entry == PROJECT_ENTRY;
        let entry = if entry == PROJECT_ENTRY {
            FILES_ENTRY
        } else {
            entry
        };
        let definition = self.definition(entry)?.clone();
        let displaced = self.displaced_transient(tab_index, None)?;
        let stops_selected = displaced.is_some();
        let candidate =
            self.popup_candidate(ids, allowance, tab_index, &definition, stops_selected)?;
        let launch = self.prepare_launch(tab_index, &candidate, &definition, quick, prepare)?;
        if let Some(index) = displaced {
            self.stop_popup(tab_index, index, operate)?;
        }
        if let Err(detail) = operate(launch) {
            // A start that timed out can still leave its terminal live under
            // this identity, so a later terminal never reuses it.
            ids.next_session += 1;
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
                format!("session is limited to {MAX_SESSIONS} terminals"),
            ));
        }
        if ids.next_popup >= ids.limit || ids.next_session == usize::MAX {
            return Err(action_error(
                "capacity",
                "session exhausted terminal identities",
            ));
        }
        Ok(Popup {
            id: format!("u{}", ids.next_popup),
            entry: definition.id.clone(),
            session: ids.popup_session(),
            launch_directory: self.tabs[tab_index].directory.clone(),
        })
    }

    fn prepare_launch(
        &self,
        tab_index: usize,
        popup: &Popup,
        definition: &PopupDefinition,
        quick: bool,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<PreparedCommand, String>,
    ) -> Result<SessionOperation, Failure> {
        let prepared = prepare(&definition.command, &popup.launch_directory)
            .map_err(|detail| action_error("popup-command", detail))?;
        let command = if definition.id == FILES_ENTRY {
            LaunchCommand::Files {
                argv: prepared.argv,
                quick,
                pending: self.tabs[tab_index].pending,
            }
        } else if definition.command == PopupCommand::Editor {
            if let Some((kind, _)) = prepared.listener {
                super::editor::validate_listener_path(&popup.session.endpoint, kind)
                    .map_err(|detail| action_error("popup-command", detail))?;
            }
            LaunchCommand::Editor(prepared)
        } else {
            LaunchCommand::Tool(prepared.argv)
        };
        Ok(SessionOperation::Start {
            session: popup.session.clone(),
            directory: popup.launch_directory.clone(),
            command,
            tab: Some(self.tabs[tab_index].id.clone()),
        })
    }

    fn commit_popup(&mut self, ids: &mut IdAllocator, tab_index: usize, popup: Popup) {
        let transient = self
            .definition(&popup.entry)
            .is_ok_and(|definition| !definition.keep_alive);
        let tab = &mut self.tabs[tab_index];
        let covered = tab.selected_popup.replace(popup.id.clone());
        tab.covered_popup = covered.filter(|_| transient);
        tab.popups.push(popup);
        ids.next_popup += 1;
        ids.next_session += 1;
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
        // Closing a pending tab's pair is dismissal: its first pane starts at
        // the tab directory (EON-C18).
        let pending = self.tabs[tab_index].pending;
        if pending {
            ids.check_pane_identity()?;
        }
        if self.definition(&popup.entry)?.keep_alive {
            if self.tabs[tab_index].selected_popup.as_deref() == Some(&popup.id) {
                self.tabs[tab_index].selected_popup = None;
            }
            if self.tabs[tab_index].covered_popup.as_deref() == Some(&target.instance) {
                self.tabs[tab_index].covered_popup = None;
            }
            if pending {
                self.finish_pending_tab(ids, allowance, tab_index, operate)?;
            }
            return Ok(());
        }
        self.stop_popup(tab_index, popup_index, operate)?;
        if pending {
            self.finish_pending_tab(ids, allowance, tab_index, operate)?;
        } else {
            self.remove_empty_tab(tab_index);
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
            // A pending tab's pair that exits is dismissed (EON-C18).
            self.remove_popup(tab_index, popup_index);
            if self.tabs[tab_index].pending && end == SessionEnd::Exited {
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
                    "unknown-terminal",
                    format!("terminal {session_id} is not in the session"),
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

    /// The pair of each pending tab, which presentation detachment dismisses.
    pub(crate) fn pending_pairs(&self) -> Vec<PopupTarget> {
        self.tabs
            .iter()
            .filter(|tab| tab.pending)
            .flat_map(|tab| {
                tab.popups.iter().map(|popup| PopupTarget {
                    tab: tab.id.clone(),
                    instance: popup.id.clone(),
                })
            })
            .collect()
    }

    /// The terminal of `tab`'s Files pair and whether it is shown.
    fn pair(&self, tab: &str) -> Option<(String, bool)> {
        let tab = self.tabs.iter().find(|item| item.id == tab)?;
        let popup = tab.popups.iter().find(|popup| popup.entry == FILES_ENTRY)?;
        Some((
            popup.session.id.clone(),
            tab.selected_popup.as_deref() == Some(&popup.id),
        ))
    }

    /// The terminal a tab shows, as its pane or popup ID and its terminal:
    /// its shown popup, else its selected pane.
    fn shown_terminal(&self, tab_index: usize) -> Option<(String, String)> {
        let tab = &self.tabs[tab_index];
        if let Some(selected) = tab.selected_popup.as_deref() {
            return tab
                .popups
                .iter()
                .find(|popup| popup.id == selected)
                .map(|popup| (popup.id.clone(), popup.session.id.clone()));
        }
        let column = &tab.columns[tab.selected_column?];
        let pane = &column.panes[column.expanded];
        Some((pane.id.clone(), pane.session.id.clone()))
    }

    fn shown_picker(&self, tab: &str, entry: &str) -> Option<String> {
        if self.definition(entry).ok()?.command != PopupCommand::Argv(vec![WORKSPACE_PICKER.into()])
        {
            return None;
        }
        let tab = self.tabs.iter().find(|item| item.id == tab)?;
        let selected = tab.selected_popup.as_deref()?;
        tab.popups
            .iter()
            .find(|popup| {
                popup.id == selected
                    && popup.entry == entry
                    && popup.launch_directory == tab.directory
            })
            .map(|popup| popup.session.id.clone())
    }

    pub(crate) fn prepare_close_tab(&mut self, id: &str) -> Result<Vec<String>, Failure> {
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .ok_or_else(|| action_error("unknown-tab", format!("tab {id} is not live")))?;
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

    /// Validates closing one pane of this workspace, and names the terminals to
    /// stop: the pane's, and when it is the final tab's last pane, every
    /// terminal of that tab, since the close removes the workspace, or ends the
    /// window when it is the last one.
    pub(crate) fn prepare_close_pane(
        &mut self,
        caller: Option<&str>,
        id: &str,
    ) -> Result<Vec<String>, Failure> {
        let (tab_index, (column, pane)) = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| tab.find_pane(|pane| pane.id == id).map(|pos| (index, pos)))
            .ok_or_else(|| action_error("unknown-pane", format!("pane {id} is not live")))?;
        let tab = &self.tabs[tab_index];
        // A covering popup hides the pane Alt+W would close, and closing the
        // tab's last pane would stop the popup too, so only validated calling
        // terminals of this workspace may close a named pane under it, and
        // never its last.
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

    /// Names only the runs this column close may stop; pruning follows their exits.
    fn prepare_close_column(
        &self,
        caller: Option<&str>,
        id: &str,
    ) -> Result<(Vec<String>, bool), Failure> {
        let (tab_index, column) = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| {
                tab.find_pane(|pane| pane.id == id)
                    .map(|(column, _)| (index, column))
            })
            .ok_or_else(|| action_error("unknown-pane", format!("pane {id} is not live")))?;
        let tab = &self.tabs[tab_index];
        if tab.selected_popup.is_some() {
            if caller.is_none() {
                return Err(action_error(
                    "unavailable",
                    "close the popup before closing a column",
                ));
            }
            if tab.columns.len() == 1 {
                return Err(action_error(
                    "unavailable",
                    "close the popup before closing the tab's last column",
                ));
            }
        }
        let whole_tab = self.tabs.len() == 1 && tab.columns.len() == 1;
        let sessions = if whole_tab {
            self.tab_sessions(tab_index)
        } else {
            tab.columns[column]
                .panes
                .iter()
                .map(|pane| pane.session.id.clone())
                .collect()
        };
        Ok((sessions, whole_tab))
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
        ids.check_pane_identity().map_err(|error| error.detail)?;
        if ids.next_session == usize::MAX {
            return Err("session exhausted pane or terminal identities".into());
        }
        let pane = Pane {
            pinned_name: None,
            tag: None,
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
        ids.commit_pane_session();
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
            // A tab whose first pane cannot start goes, its pair with it.
            while !self.tabs[tab_index].popups.is_empty()
                && self.stop_popup(tab_index, 0, operate).is_ok()
            {}
            self.remove_empty_tab(tab_index);
            return Err(action_error("terminal-start", detail));
        }
        self.settle_pair(tab_index, operate);
        Ok(())
    }

    /// The pair of a tab that just committed stays on as its Files.
    fn settle_pair(
        &self,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) {
        for popup in &self.tabs[tab_index].popups {
            // The supervisor logs a failure; it changes nothing else.
            let _ = operate(SessionOperation::SettlePair(popup.session.id.clone()));
        }
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
        // The action shows a pane, not the popup a transient one covered; a
        // failed dismissal changes nothing, so the tab keeps it.
        let covered = self.tabs[tab_index].covered_popup.take();
        let tab = self.tabs[tab_index].id.clone();
        if let Err(error) = self.dismiss_popup(
            ids,
            allowance,
            &PopupTarget {
                tab: tab.clone(),
                instance,
            },
            operate,
        ) {
            self.tabs[tab_index].covered_popup = covered;
            return Err(error);
        }
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
        if tab_number(id).is_some() {
            return Err(action_error("unknown-tab", format!("tab {id} is not live")));
        }
        Err(action_error(
            "unknown-id",
            format!("{id} is not a live tab or pane identity"),
        ))
    }

    /// Focuses the exact pane or popup `target` of `tab` (EON-C31). A shown
    /// keep-alive popup that is not the target is hidden first; a shown
    /// transient one is left alone, and only the tab is selected.
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
        if tab.selected_popup.as_deref() != Some(target) {
            tab.covered_popup = None;
        }
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
        allowance: usize,
        id: &str,
        directory: Vec<u8>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
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
        let directory = validated_directory(directory)?;
        if !self.tabs[tab_index].pending {
            return self.retarget(ids, tab_index, directory);
        }
        ids.check_pane_identity()?;
        // A pick on a pending tab commits it: one first pane at the directory,
        // and the pair hides (EON-C18).
        self.start_pane_at(ids, allowance, tab_index, &directory, operate)
            .map_err(|detail| action_error("terminal-start", detail))?;
        let tab = &mut self.tabs[tab_index];
        tab.directory = directory;
        tab.pending = false;
        tab.selected_popup = None;
        tab.covered_popup = None;
        self.settle_pair(tab_index, operate);
        Ok(())
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
        if parked.is_some() {
            ids.check_pane_identity()?;
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
            tab.covered_popup = None;
        }
        let pane = Pane {
            pinned_name: None,
            tag: None,
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
        let tab = &mut self.tabs[tab_index];
        let removed = tab.popups.remove(popup_index);
        if tab.selected_popup.as_deref() == Some(&removed.id) {
            // A transient popup that ends shows the popup it covered again.
            tab.selected_popup = tab
                .covered_popup
                .take()
                .filter(|covered| tab.popups.iter().any(|popup| &popup.id == covered));
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
            "the calling terminal is not part of this session",
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
            Err(format!("session is limited to {MAX_SESSIONS} terminals"))
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
pub(crate) fn attention_name(state: AttentionState) -> &'static str {
    match state {
        AttentionState::NeedsInput => "needs-input",
        AttentionState::Error => "error",
        AttentionState::Finished => "finished",
        AttentionState::Working => "working",
        AttentionState::Background => "background",
        AttentionState::Idle => "idle",
        AttentionState::Unavailable => "unavailable",
    }
}

/// The CLI's name for a needs-input kind.
fn attention_kind_name(kind: AttentionKind) -> &'static str {
    match kind {
        AttentionKind::Permission => "permission",
        AttentionKind::Question => "question",
        AttentionKind::Auth => "auth",
    }
}

/// A terminal's attention state, app and name, the human projection's subset;
/// JSON carries the full record (EON-C31). An empty app or name is none.
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

pub(crate) fn human_attention(attention: Option<&Attention>) -> String {
    let (state, app, name) = attention_fields(attention);
    format!(
        "attention={} app={} name={}",
        state.unwrap_or("none"),
        app.unwrap_or("none").escape_debug(),
        name.unwrap_or("none").escape_debug()
    )
}

/// The calling terminal's JSON envelope field or human header line.
pub(crate) fn caller_output(snapshot: &Snapshot, session: Option<&str>, json: bool) -> String {
    let caller = session.and_then(|session| {
        snapshot.tabs.iter().find_map(|tab| {
            if let Some((_, _, pane)) =
                column_panes(tab).find(|(_, _, pane)| pane.session == session)
            {
                Some((session, tab.id.as_str(), Some(pane.id.as_str()), None))
            } else {
                tab.popups
                    .iter()
                    .find(|popup| popup.session == session)
                    .map(|popup| (session, tab.id.as_str(), None, Some(popup.id.as_str())))
            }
        })
    });
    let Some((terminal, tab, pane, popup)) = caller else {
        return if json {
            "\"caller\":null,".into()
        } else {
            String::new()
        };
    };
    if json {
        let mut output = format!(
            "\"caller\":{{\"terminal\":\"{}\",\"tab\":\"{}\",\"pane\":",
            json_escape(terminal),
            json_escape(tab)
        );
        json_option(&mut output, pane);
        output.push_str(",\"popup\":");
        json_option(&mut output, popup);
        output.push_str("},");
        output
    } else {
        format!(
            "caller terminal={terminal} tab={tab} pane={} popup={}\n",
            pane.unwrap_or("none"),
            popup.unwrap_or("none")
        )
    }
}

/// `eon pane read --json` without the generation and window the CLI puts in
/// front (EON-C35).
pub(crate) fn terminal_text_json(read: &TerminalText) -> String {
    format!(
        "{{{},\"revision\":{},\"screen\":\"{}\",\"rows\":{},\"columns\":{},\"title\":\"{}\",\"directory\":{},\"text\":\"{}\",\"omitted_rows\":{},\"lossy\":true}}\n",
        terminal_json(&read.target, read.caller.as_ref()),
        read.revision,
        match read.screen {
            Screen::Primary => "primary",
            Screen::Alternate => "alternate",
        },
        read.rows,
        read.columns,
        json_escape(&read.title),
        json_path(&read.directory),
        json_escape(&read.text),
        read.omitted_rows,
    )
}

/// `eon pane history --json` without the generation and window the CLI puts
/// in front (EON-C35): `eon pane read`'s fields, with `omitted_lines` and
/// `cut_bytes` in place of `omitted_rows`.
pub(crate) fn terminal_history_json(history: &TerminalHistory) -> String {
    format!(
        "{{{},\"revision\":{},\"screen\":\"{}\",\"rows\":{},\"columns\":{},\"title\":\"{}\",\"directory\":{},\"text\":\"{}\",\"omitted_lines\":{},\"cut_bytes\":{},\"lossy\":true}}\n",
        terminal_json(&history.target, history.caller.as_ref()),
        history.revision,
        match history.screen {
            Screen::Primary => "primary",
            Screen::Alternate => "alternate",
        },
        history.rows,
        history.columns,
        json_escape(&history.title),
        json_path(&history.directory),
        json_escape(&history.text),
        history.omitted_lines,
        history.cut_bytes,
    )
}

/// `eon pane history --edit --json` without the generation and window the
/// CLI puts in front (EON-C35): the target, then the Scrollback popup that
/// opened its snapshot in the target's tab.
pub(crate) fn terminal_editing_json(editing: &Editing) -> String {
    let popup = match &editing.popup.id {
        TargetId::Popup(id) | TargetId::Pane(id) => id,
    };
    format!(
        "{{{},\"scrollback\":{{\"popup\":\"{}\",\"terminal\":\"{}\"}}}}\n",
        terminal_json(&editing.target, editing.caller.as_ref()),
        json_escape(popup),
        json_escape(&editing.popup.terminal),
    )
}

/// `eon pane send --json` without the generation and window the CLI puts in
/// front (EON-C36): what reached the target, and the Enter's failure when the
/// paste reached it and the Enter did not. No bytes means Enter alone, so
/// `bracketed` is null.
pub(crate) fn terminal_sent_json(
    sent: &Sent,
    bytes: usize,
    enter: bool,
    error: Option<&Failure>,
) -> String {
    let bracketed = if bytes == 0 {
        "null".into()
    } else {
        sent.bracketed.to_string()
    };
    let error = error.map_or_else(String::new, |error| {
        format!(
            ",\"error\":{{\"code\":\"{}\",\"detail\":\"{}\"}}",
            json_escape(&error.code),
            json_escape(&error.detail)
        )
    });
    format!(
        "{{{},\"bytes\":{bytes},\"bracketed\":{bracketed},\"enter\":{enter}{error}}}\n",
        terminal_json(&sent.target, sent.caller.as_ref())
    )
}

/// A target's `session`, `tab`, `pane`, `popup` and `terminal`, then the
/// calling terminal as `eon session` reports `caller`.
fn terminal_json(target: &TerminalTarget, caller: Option<&TerminalTarget>) -> String {
    let caller = caller.map_or_else(
        || "null".into(),
        |caller| {
            format!(
                "{{\"terminal\":\"{}\",\"tab\":\"{}\",{}}}",
                json_escape(&caller.terminal),
                json_escape(&caller.tab),
                target_id_json(&caller.id)
            )
        },
    );
    format!(
        "\"session\":\"{}\",\"tab\":\"{}\",{},\"terminal\":\"{}\",\"caller\":{caller}",
        json_escape(&target.workspace),
        json_escape(&target.tab),
        target_id_json(&target.id),
        json_escape(&target.terminal)
    )
}

/// `pane` and `popup`, one of them null, as `caller` reports them.
pub(crate) fn target_id_json(id: &TargetId) -> String {
    let (pane, popup) = match id {
        TargetId::Pane(id) => (Some(id.as_str()), None),
        TargetId::Popup(id) => (None, Some(id.as_str())),
    };
    let mut output = "\"pane\":".to_owned();
    json_option(&mut output, pane);
    output.push_str(",\"popup\":");
    json_option(&mut output, popup);
    output
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
    if let Some(running) = &snapshot.running {
        for (kind, entry) in std::iter::once(("this", &running.this)).chain(
            running
                .others
                .iter()
                .flatten()
                .map(|entry| ("other", entry)),
        ) {
            output.push_str(&format!("running {kind} window={} generation={} name={} started_at={} live_terminals={} state={}\n",
                entry.window.as_deref().unwrap_or("default"), entry.generation.escape_debug(),
                entry.name.as_deref().unwrap_or("unknown").escape_debug(),
                entry.started_at.map_or_else(|| "unknown".into(), |v| v.to_string()),
                entry.live_terminals.map_or_else(|| "unknown".into(), |v| v.to_string()),
                window_state(entry.state)));
        }
    }
    output.push_str(&human_busy(snapshot.busy.as_ref()));
    output
}

/// One `busy` line per busy terminal (EON-C10, EON-C11).
pub(crate) fn human_busy(busy: Option<&BusyTerminals>) -> String {
    let Some(busy) = busy else {
        return String::new();
    };
    let mut output = String::new();
    for terminal in &busy.terminals {
        let (kind, id) = match &terminal.target.id {
            TargetId::Pane(id) => ("pane", id),
            TargetId::Popup(id) => ("popup", id),
        };
        output.push_str(&format!(
            "busy session={} tab={} {kind}={} terminal={} name={}\n",
            terminal.target.workspace,
            terminal.target.tab,
            id,
            terminal.target.terminal,
            terminal.name.escape_debug()
        ));
    }
    if busy.omitted > 0 {
        output.push_str(&format!("busy omitted={}\n", busy.omitted));
    }
    output
}

/// The window's busy terminals as `busy`, null when unknown (EON-C10); each
/// terminal's name only with `names`.
pub(crate) fn json_busy(output: &mut String, busy: Option<&BusyTerminals>, names: bool) {
    let Some(busy) = busy else {
        output.push_str("null");
        return;
    };
    output.push_str("{\"terminals\":[");
    for (index, terminal) in busy.terminals.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let (pane, popup) = match &terminal.target.id {
            TargetId::Pane(id) => (Some(id.as_str()), None),
            TargetId::Popup(id) => (None, Some(id.as_str())),
        };
        output.push_str(&format!(
            "{{\"session\":\"{}\",\"tab\":\"{}\",\"pane\":",
            json_escape(&terminal.target.workspace),
            json_escape(&terminal.target.tab),
        ));
        json_option(output, pane);
        output.push_str(",\"popup\":");
        json_option(output, popup);
        output.push_str(&format!(
            ",\"terminal\":\"{}\"",
            json_escape(&terminal.target.terminal)
        ));
        if names {
            output.push_str(&format!(",\"name\":\"{}\"", json_escape(&terminal.name)));
        }
        output.push('}');
    }
    output.push_str(&format!("],\"omitted\":{}}}", busy.omitted));
}

pub(crate) fn json_pane(output: &mut String, pane: &SnapshotPane, column: usize, expanded: bool) {
    output.push_str(&format!(
        "{{\"id\":\"{}\",\"column\":{},\"expanded\":{},\"terminal\":\"{}\",\"endpoint\":{},\"live\":{},\"label\":\"{}\",\"pinned_name\":",
        json_escape(&pane.id),
        column,
        expanded,
        json_escape(&pane.session),
        json_path(&pane.endpoint),
        pane.live,
        json_escape(&pane.label),
    ));
    json_option(output, pane.pinned_name.as_deref());
    output.push_str(",\"tag\":");
    json_option(output, pane.tag.as_deref());
    output.push_str(",\"attention\":");
    json_attention(output, pane.attention.as_ref());
    output.push('}');
}

pub(crate) fn json_popup(output: &mut String, popup: &SnapshotPopup, chosen: bool) {
    output.push_str(&format!(
        "{{\"id\":\"{}\",\"entry\":\"{}\",\"terminal\":\"{}\",\"endpoint\":{},\"chosen\":{},\"attention\":",
        json_escape(&popup.id), json_escape(&popup.entry), json_escape(&popup.session),
        json_path(&popup.endpoint), chosen
    ));
    json_attention(output, popup.attention.as_ref());
    output.push('}');
}

pub(crate) fn json_tab(output: &mut String, tab: &SnapshotTab) {
    output.push_str(&format!(
        "{{\"id\":\"{}\",\"directory\":{},\"pending\":{},\"selected_pane\":",
        json_escape(&tab.id),
        json_path(&tab.directory),
        tab.pending
    ));
    json_option(output, tab.selected_pane.as_deref());
    output.push_str(",\"selected_popup\":");
    json_option(output, tab.selected_popup.as_deref());
    output.push_str(",\"attention_rollup\":");
    json_option(output, tab.attention_rollup.map(attention_name));
    output.push_str(",\"panes\":[");
    for (index, (column, expanded, pane)) in column_panes(tab).enumerate() {
        if index != 0 {
            output.push(',');
        }
        json_pane(output, pane, column + 1, expanded);
    }
    output.push_str("],\"popups\":[");
    for (index, popup) in tab.popups.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        json_popup(
            output,
            popup,
            tab.selected_popup.as_deref() == Some(&popup.id),
        );
    }
    output.push_str("]}");
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
        json_tab(&mut output, tab);
    }
    output.push_str(&format!(
        "],\"sidebar\":\"{}\",\"codex_quota\":",
        sidebar_name(snapshot.sidebar)
    ));
    json_quota(
        &mut output,
        snapshot.codex_quota.as_ref().map(|quota| {
            let state = match quota.state {
                CodexQuotaState::Fresh => "fresh",
                CodexQuotaState::Stale => "stale",
                CodexQuotaState::Blocked => "blocked",
                CodexQuotaState::Unknown => "unknown",
            };
            (state, quota.observed_at, quota.windows.as_slice())
        }),
    );
    output.push_str(",\"claude_quota\":");
    json_quota(
        &mut output,
        snapshot.claude_quota.as_ref().map(|quota| {
            let state = match quota.state {
                ClaudeQuotaState::Fresh => "fresh",
                ClaudeQuotaState::Stale => "stale",
            };
            (state, quota.observed_at, quota.windows.as_slice())
        }),
    );
    output.push_str(",\"codex_quota_failure\":");
    json_quota_failure(&mut output, snapshot.codex_quota_failure);
    output.push_str(",\"claude_quota_failure\":");
    json_quota_failure(&mut output, snapshot.claude_quota_failure);
    output.push_str(",\"attention_queue\":[");
    for (index, entry) in snapshot.attention_queue.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"session\":\"{}\",\"tab\":\"{}\",\"target\":\"{}\",\"attention\":",
            json_escape(&entry.workspace),
            json_escape(&entry.tab),
            json_escape(&entry.target),
        ));
        json_attention(&mut output, Some(&entry.attention));
        output.push('}');
    }
    output.push_str("],\"running\":");
    if let Some(running) = &snapshot.running {
        output.push_str("{\"this\":");
        json_window(&mut output, &running.this, false);
        output.push_str(",\"others\":");
        if let Some(others) = &running.others {
            output.push('[');
            for (index, entry) in others.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                json_window(&mut output, entry, true);
            }
            output.push(']');
        } else {
            output.push_str("null");
        }
        output.push_str(&format!(",\"omitted\":{}}}", running.omitted));
    } else {
        output.push_str("null");
    }
    output.push_str(",\"busy\":");
    json_busy(&mut output, snapshot.busy.as_ref(), true);
    output.push_str("}\n");
    output
}

fn window_state(state: eon_workspace_protocol::v42::WindowState) -> &'static str {
    use eon_workspace_protocol::v42::WindowState;
    match state {
        WindowState::Live => "live",
        WindowState::Incompatible => "incompatible",
        WindowState::Unreachable => "unreachable",
    }
}

fn json_window(
    output: &mut String,
    entry: &eon_workspace_protocol::v42::WindowEntry,
    include_state: bool,
) {
    output.push_str("{\"window\":");
    json_option(output, entry.window.as_deref());
    output.push_str(&format!(
        ",\"generation\":\"{}\",\"name\":",
        json_escape(&entry.generation)
    ));
    json_option(output, entry.name.as_deref());
    output.push_str(&format!(
        ",\"started_at\":{},\"live_terminals\":{}",
        entry
            .started_at
            .map_or_else(|| "null".into(), |v| v.to_string()),
        entry
            .live_terminals
            .map_or_else(|| "null".into(), |v| v.to_string())
    ));
    if include_state {
        output.push_str(&format!(",\"state\":\"{}\"", window_state(entry.state)));
    }
    output.push('}');
}

/// The full attention record, or `null` without attention.
pub(crate) fn json_attention(output: &mut String, attention: Option<&Attention>) {
    let (Some(state), app, name) = attention_fields(attention) else {
        output.push_str("null");
        return;
    };
    output.push_str(&format!("{{\"state\":\"{state}\",\"app\":"));
    json_option(output, app);
    output.push_str(",\"name\":");
    json_option(output, name);
    if let Some(record) = attention.and_then(|attention| attention.record.as_ref()) {
        output.push_str(",\"kind\":");
        json_option(output, record.kind.map(attention_kind_name));
        output.push_str(&format!(
            ",\"progress\":{},\"title\":",
            record
                .progress
                .map_or_else(|| "null".into(), |progress| progress.to_string())
        ));
        json_option(
            output,
            Some(record.title.as_str()).filter(|text| !text.is_empty()),
        );
        output.push_str(",\"message\":");
        json_option(
            output,
            Some(record.message.as_str()).filter(|text| !text.is_empty()),
        );
        output.push_str(&format!(
            ",\"changed_at\":{},\"stale\":{},\"others\":{}",
            record.changed_at, record.stale, record.others
        ));
    } else {
        output.push_str(",\"kind\":null,\"progress\":null,\"title\":null,\"message\":null,\"changed_at\":null,\"stale\":null,\"others\":null");
    }
    output.push('}');
}

fn quota_milliseconds(seconds: u64) -> u128 {
    u128::from(seconds) * 1000
}

fn json_quota(output: &mut String, quota: Option<(&str, u64, &[QuotaWindow])>) {
    let Some((state, observed_at, windows)) = quota else {
        output.push_str("null");
        return;
    };
    output.push_str(&format!(
        "{{\"state\":\"{state}\",\"observed_at\":{},\"windows\":[",
        quota_milliseconds(observed_at)
    ));
    for (index, window) in windows.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"duration_minutes\":{},\"remaining_percent\":{},\"resets_at\":{},\"scope\":",
            window.duration_minutes,
            window.remaining_percent,
            window.resets_at.map_or_else(
                || "null".into(),
                |time| quota_milliseconds(time).to_string()
            )
        ));
        json_option(output, window.scope.as_deref());
        output.push('}');
    }
    output.push_str("]}");
}

/// The provider's last failed read (EON-C27), or `null`.
fn json_quota_failure(output: &mut String, failure: Option<QuotaFailure>) {
    let Some(failure) = failure else {
        output.push_str("null");
        return;
    };
    let reason = super::quota::failure_reason(failure.reason);
    output.push_str(&format!(
        "{{\"at\":{},\"reason\":\"{reason}\"}}",
        quota_milliseconds(failure.at)
    ));
}

/// The window's sessions in order, one line each (`eon session ls`).
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

/// One session object, shared by inspection and event output.
pub(crate) fn json_workspace(
    output: &mut String,
    workspace: &eon_workspace_protocol::v42::WorkspaceInfo,
    shown: bool,
) {
    output.push_str(&format!(
        "{{\"id\":\"{}\",{},\"name\":",
        json_escape(&workspace.id),
        registration_json(workspace),
    ));
    json_option(output, workspace.name.as_deref());
    output.push_str(&format!(
        ",\"shown\":{},\"tabs\":{},\"attention\":",
        shown, workspace.tab_count
    ));
    json_option(output, workspace.attention.map(attention_name));
    output.push_str(&format!(",\"recency\":{}}}", workspace.recency));
}

pub(crate) fn registration_json(workspace: &eon_workspace_protocol::v42::WorkspaceInfo) -> String {
    let mut output = String::from("\"persistent_id\":");
    json_option(&mut output, workspace.persistent_id.as_deref());
    output.push_str(",\"registration_error\":");
    match &workspace.registration_error {
        Some(error) => output.push_str(&format!(
            "{{\"code\":\"{}\",\"detail\":\"{}\"}}",
            json_escape(&error.code),
            json_escape(&error.detail)
        )),
        None => output.push_str("null"),
    }
    output
}

/// The local handles, persistent IDs, names and summaries of this window's members.
/// (`eon session ls --json`).
pub(crate) fn list_json(snapshot: &Snapshot) -> String {
    let mut output = String::from("{\"sessions\":[");
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        json_workspace(
            &mut output,
            workspace,
            workspace.id == snapshot.active_workspace,
        );
    }
    output.push_str("]}\n");
    output
}

pub(crate) fn failure_human(failure: &Failure) -> String {
    format!(
        "error {}: {}\n",
        failure.code,
        terminal_text(&failure.detail)
    )
}

/// Text for a terminal: control and other unprintable characters are escaped,
/// so a detail cannot drive the terminal, while quotes, backslashes and line
/// breaks stay as written, so a command in it can be pasted.
pub(crate) fn terminal_text(text: &str) -> String {
    let mut printable = String::new();
    for character in text.chars() {
        match character {
            '\\' | '\'' | '"' | '\n' => printable.push(character),
            character => printable.extend(character.escape_debug()),
        }
    }
    printable
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

pub(crate) fn tab_number(value: &str) -> Option<usize> {
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

/// Explicit persistent targets never fall through to a window-local name.
pub(crate) fn resolve_target<'a>(
    workspaces: impl Iterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>,
    target: &WorkspaceTarget,
) -> Result<usize, Failure> {
    match target {
        WorkspaceTarget::Selector(selector) => {
            eon_workspace_protocol::v42::validate_workspace_name(selector)
                .map_err(|error| action_error("malformed-action", error.to_string()))?;
            resolve_selector(workspaces.map(|(id, name, _)| (id, name)), selector)
        }
        WorkspaceTarget::PersistentId(id) => {
            super::session_registry::validate_id(id)
                .map_err(|error| action_error("malformed-action", error.to_string()))?;
            workspaces
                .enumerate()
                .find_map(|(index, (_, _, persistent))| {
                    (persistent == Some(id.as_str())).then_some(index)
                })
                .ok_or_else(|| {
                    action_error(
                        "unknown-session",
                        format!("session {id} is not live in this window"),
                    )
                })
        }
    }
}

/// Resolve a workspace selector over `(sN, name)` pairs in order (EON-C33):
/// an exact `sN`, else the one workspace with that exact name. The CLI's
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
            "unknown-session",
            format!("session {target} is not live"),
        )),
        _ => Err(action_error(
            "ambiguous-session",
            format!(
                "session {target} matches {}",
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

fn shortcut_text(shortcut: &eon_workspace_protocol::v42::Shortcut) -> String {
    eon_workspace_protocol::shortcuts::config_chord(shortcut.modifiers, &shortcut.key)
}

pub(crate) fn json_option(output: &mut String, value: Option<&str>) {
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
    use eon_workspace_protocol::v42::{ALT, PopupGeometry, QuotaFailureReason, Shortcut};

    fn c38_set(root: &Path) -> (Registry, WorkspaceSet) {
        let registry = Registry::in_test_directory(root.join("sessions"));
        let registration = registry.create(Some("same")).unwrap();
        let set = WorkspaceSet::registered_pending(
            Some(registry.clone()),
            registration,
            IdAllocator::new(root.join("runtime"), 0).unwrap(),
            root.into(),
            Some(root.into()),
            catalog(),
            true,
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        (registry, set)
    }

    // A committed identity must be observable before launch, even when launch fails.
    #[test]
    fn column_close_scope_is_its_column_or_the_whole_final_tab() {
        let mut set = two_workspace_set(|_| Ok(()));
        let pane = set.snapshot().tabs[0].panes().next().unwrap().id.clone();
        let popup = invoke(&set.snapshot().tabs[0].id, "agent");
        set.dispatch("popup", None, popup, prepared, |_| Ok(()))
            .unwrap();
        assert_eq!(
            set.prepare_close_column("covered", None, &pane)
                .unwrap_err()
                .detail,
            "close the popup before closing a column"
        );
        let caller = set.snapshot().tabs[0]
            .popups
            .last()
            .unwrap()
            .session
            .clone();
        assert_eq!(
            set.prepare_close_column("last-covered", Some(&caller), &pane)
                .unwrap_err()
                .detail,
            "close the popup before closing the tab's last column"
        );
        let dismiss = Action::DismissPopup(PopupTarget {
            tab: set.snapshot().tabs[0].id.clone(),
            instance: set.snapshot().tabs[0].selected_popup.clone().unwrap(),
        });
        set.dispatch("dismiss", None, dismiss, prepared, |_| Ok(()))
            .unwrap();
        let (sessions, end) = set
            .prepare_close_column("session-end", None, &pane)
            .unwrap();
        assert_eq!(end, CloseEnd::Session);
        assert!(sessions.contains(&caller), "final tab includes its popups");
        let other: Vec<_> = set.workspaces[1]
            .sessions()
            .map(|session| session.id.clone())
            .collect();
        for id in other {
            set.session_ended(&id, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(
            set.prepare_close_column("window-end", None, &pane)
                .unwrap()
                .1,
            CloseEnd::Window
        );
        set.dispatch("column", None, Action::CreateColumn, prepared, |_| Ok(()))
            .unwrap();
        let before = set.snapshot();
        let (sessions, end) = set.prepare_close_column("column-end", None, &pane).unwrap();
        assert_eq!(end, CloseEnd::Column);
        assert_eq!(
            sessions,
            [before.tabs[0].columns[0].panes[0].session.clone()]
        );
        assert_eq!(set.snapshot(), before);
        set.dispatch(
            "new-tab",
            None,
            Action::Workspace(WorkspaceAction::CreateTab),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let last_column_pane = before.tabs[0].columns[1].panes[0].id.clone();
        for session in sessions {
            set.session_ended(&session, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        let (sessions, end) = set
            .prepare_close_column("nonfinal-only", None, &last_column_pane)
            .unwrap();
        assert_eq!(end, CloseEnd::Column);
        assert_eq!(sessions.len(), 1, "a non-final tab's popups are excluded");
        assert_eq!(
            set.prepare_close_column("unknown", None, "p99")
                .unwrap_err()
                .code,
            "unknown-pane"
        );
    }

    #[test]
    fn c38_failed_creation_retains_identity_and_replay_protection() {
        let root = super::super::supervisor::temporary_directory();
        let (registry, mut set) = c38_set(&root);
        let before = set.snapshot();
        let create = Action::Workspace(WorkspaceAction::NewWorkspace {
            name: Some("same".into()),
            focus: true,
        });
        let mut starts = 0;
        let error = set
            .dispatch("failed", None, create.clone(), prepared, |operation| {
                assert!(matches!(operation, SessionOperation::Start { .. }));
                let records = registry.list().unwrap();
                assert_eq!(records.len(), 2);
                assert_eq!(records[1].name.as_deref(), Some("same"));
                starts += 1;
                Err("launch failed".into())
            })
            .unwrap_err();
        let records = registry.list().unwrap();
        assert_eq!(error.code, "terminal-start");
        assert!(error.detail.contains(&records[1].id), "{}", error.detail);
        assert!(starts > 0);
        assert_eq!(set.snapshot(), before);
        assert_eq!(
            set.dispatch("failed", None, create, prepared, |_| panic!(
                "a replay cannot launch"
            ))
            .unwrap_err()
            .code,
            "duplicate-request"
        );
        assert_eq!(registry.list().unwrap(), records);
        fs::remove_dir_all(root).unwrap();
    }

    // Both allocator and record sync uncertainty consume the existing replay window.
    #[test]
    fn c38_uncertain_creation_never_launches_or_replays() {
        use super::super::session_registry::Fault;
        let root = super::super::supervisor::temporary_directory();
        let (registry, mut set) = c38_set(&root);
        for (index, fault) in [Fault::AllocatorDirectorySync, Fault::RecordDirectorySync]
            .into_iter()
            .enumerate()
        {
            let request = format!("uncertain-{index}");
            let before = set.snapshot();
            let create = Action::Workspace(WorkspaceAction::NewWorkspace {
                name: None,
                focus: true,
            });
            set.registry.as_mut().unwrap().inject(Some(fault));
            let error = set
                .dispatch(&request, None, create.clone(), prepared, |_| {
                    panic!("unconfirmed registration starts nothing")
                })
                .unwrap_err();
            assert_eq!(error.code, "registry-uncertain");
            assert!(error.detail.contains("uncertain"));
            if fault == Fault::RecordDirectorySync {
                assert!(
                    error
                        .detail
                        .contains(&registry.list().unwrap().last().unwrap().id)
                );
            }
            assert_eq!(set.snapshot(), before);
            set.registry.as_mut().unwrap().inject(None);
            assert_eq!(
                set.dispatch(&request, None, create, prepared, |_| panic!(
                    "a replay cannot launch"
                ))
                .unwrap_err()
                .code,
                "duplicate-request"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    // Publication failures cannot publish a new in-memory name or a success event.
    #[test]
    fn c38_rename_persists_before_memory_and_rereads_disk() {
        use super::super::session_registry::Fault;
        let root = super::super::supervisor::temporary_directory();
        let (registry, mut set) = c38_set(&root);
        let id = set.snapshot().workspaces[0].persistent_id.clone().unwrap();
        for (index, fault) in [Fault::RecordBeforePublish, Fault::RecordDirectorySync]
            .into_iter()
            .enumerate()
        {
            let before = set.snapshot();
            set.registry.as_mut().unwrap().inject(Some(fault));
            let error = set
                .dispatch(
                    &format!("rename-{index}"),
                    None,
                    Action::Workspace(WorkspaceAction::RenameWorkspace {
                        workspace: Some(WorkspaceTarget::PersistentId(id.clone())),
                        name: "unconfirmed".into(),
                    }),
                    prepared,
                    |_| panic!("rename never operates terminals"),
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                if index == 0 {
                    "session-registry"
                } else {
                    "registry-uncertain"
                }
            );
            assert_eq!(set.snapshot(), before);
            assert_eq!(
                registry.lookup(&id).unwrap().unwrap().name.as_deref(),
                Some(if index == 0 { "same" } else { "unconfirmed" })
            );
        }
        set.registry.as_mut().unwrap().inject(None);
        registry.rename(&id, Some("external")).unwrap();
        set.dispatch(
            "clear",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some(WorkspaceTarget::PersistentId(id.clone())),
                name: String::new(),
            }),
            prepared,
            |_| panic!("clear never operates terminals"),
        )
        .unwrap();
        let current = registry.lookup(&id).unwrap().unwrap();
        assert_eq!(current.name, None);
        assert_eq!(
            set.snapshot().workspaces[0].persistent_id.as_deref(),
            Some(id.as_str())
        );
        assert_eq!(set.snapshot().workspaces[0].name, None);
        fs::remove_dir_all(root).unwrap();
    }

    // Host metadata cannot admit a member, including when its ID is a local name.
    #[test]
    fn c38_exact_targets_are_window_scoped_and_pruning_retains_registrations() {
        let root = super::super::supervisor::temporary_directory();
        let (registry, mut set) = c38_set(&root);
        let sibling = set.snapshot();
        set.dispatch(
            "new",
            None,
            Action::Workspace(WorkspaceAction::NewWorkspace {
                name: Some("same".into()),
                focus: false,
            }),
            prepared,
            |_| {
                assert_eq!(registry.list().unwrap().len(), 2);
                Ok(())
            },
        )
        .unwrap();
        let created = set.snapshot().workspaces[1].clone();
        assert_ne!(created.persistent_id, sibling.workspaces[0].persistent_id);
        assert_eq!(
            set.snapshot_in(Some(&created.id)).unwrap().tabs[0].directory,
            sibling.tabs[0].directory
        );
        assert_eq!(
            set.resolve_workspace(&WorkspaceTarget::Selector("same".into()))
                .unwrap_err()
                .code,
            "ambiguous-session"
        );
        let foreign = registry.create(None).unwrap();
        set.dispatch(
            "id-name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: foreign.id.clone(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let target = WorkspaceTarget::PersistentId(foreign.id.clone());
        let before = set.snapshot();
        for (index, action) in [
            WorkspaceAction::InspectWorkspace {
                workspace: target.clone(),
            },
            WorkspaceAction::SwitchWorkspace {
                workspace: target.clone(),
            },
            WorkspaceAction::RenameWorkspace {
                workspace: Some(target.clone()),
                name: "wrong".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                set.dispatch(
                    &format!("foreign-{index}"),
                    None,
                    Action::Workspace(action),
                    prepared,
                    |_| panic!("foreign identity cannot operate terminals")
                )
                .unwrap_err()
                .code,
                "unknown-session"
            );
        }
        assert_eq!(
            set.prepare_close_workspace("foreign-close", None, &target)
                .unwrap_err()
                .code,
            "unknown-session"
        );
        assert_eq!(set.snapshot(), before);
        let exact = WorkspaceTarget::PersistentId(created.persistent_id.clone().unwrap());
        assert_eq!(set.inspected_workspace_id(&exact).unwrap(), created.id);
        set.dispatch(
            "switch",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: exact.clone(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, created.id);
        let closing = set.prepare_close_workspace("close", None, &exact).unwrap();
        for terminal in closing {
            set.session_ended(&terminal, SessionEnd::Closed, |_| {
                panic!("close starts nothing")
            })
            .unwrap();
        }
        let after = set.snapshot();
        assert_eq!(after.workspaces.len(), 1);
        assert_eq!(after.tabs, sibling.tabs);
        assert_eq!(
            after.workspaces[0].persistent_id,
            sibling.workspaces[0].persistent_id
        );
        assert!(
            registry
                .lookup(created.persistent_id.as_deref().unwrap())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            set.resolve_workspace(&exact).unwrap_err().code,
            "unknown-session"
        );
        fs::remove_dir_all(root).unwrap();
    }

    // Validation must not burn an identity or disturb existing members.
    #[test]
    fn c38_creation_validates_before_reserving_identity() {
        let root = super::super::supervisor::temporary_directory();
        let (registry, mut set) = c38_set(&root);
        let records = registry.list().unwrap();
        let new = |name| Action::Workspace(WorkspaceAction::NewWorkspace { name, focus: true });
        assert!(
            set.dispatch(
                "name",
                None,
                new(Some("bad\nname".into())),
                prepared,
                |_| panic!("invalid name")
            )
            .is_err()
        );
        set.workspaces[0].home = Some(root.join("absent"));
        assert!(
            set.dispatch("directory", None, new(None), prepared, |_| panic!(
                "invalid directory"
            ))
            .is_err()
        );
        set.workspaces[0].home = Some(root.clone());
        set.ids.next_tab = usize::MAX;
        assert!(
            set.dispatch("capacity", None, new(None), prepared, |_| panic!(
                "exhausted capacity"
            ))
            .is_err()
        );
        assert_eq!(registry.list().unwrap(), records);
        fs::remove_dir_all(root).unwrap();
    }

    /// A launch `--dir` starts t1's first pane at once, with no pair
    /// (EON-C18).
    #[test]
    fn a_launch_directory_starts_t1_with_its_first_pane_and_no_pair() {
        let registry = Registry::in_test_directory(
            super::super::supervisor::temporary_directory().join("sessions"),
        );
        let registration = registry.create(None).unwrap();
        let mut started = Vec::new();
        let set = WorkspaceSet::registered_pending(
            Some(registry),
            registration,
            IdAllocator::new("/runtime".into(), 0).unwrap(),
            "/launch".into(),
            Some("/".into()),
            catalog(),
            false,
            prepared,
            |operation| {
                if let SessionOperation::Start {
                    session,
                    directory,
                    command,
                    tab,
                } = operation
                {
                    started.push((session.id, directory, command, tab));
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            started,
            [(
                "terminal-1".to_owned(),
                PathBuf::from("/launch"),
                LaunchCommand::Pane,
                None
            )]
        );
        let snapshot = set.snapshot();
        let [tab] = snapshot.tabs.as_slice() else {
            panic!("one tab");
        };
        assert!(!tab.pending && tab.popups.is_empty() && tab.selected_popup.is_none());
        assert_eq!(tab.directory, b"/launch");
        assert_eq!(tab.selected_pane.as_deref(), Some("p1"));
        assert_eq!(tab.panes().count(), 1);
    }

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
            // Its pair exits, which dismisses it and starts the first pane.
            let pair = set.workspaces[index].tabs[0].popups[0].session.id.clone();
            set.shown = index;
            set.session_ended(&pair, SessionEnd::Exited, &mut operate)
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
    fn pane_tags_validate_atomically_and_preserve_name_only_updates() {
        let mut set = two_workspace_set(|_| Ok(()));
        for (pane, name, tag) in [
            ("p1", None, "tag".to_string()),
            ("p1", Some("new"), "bad\u{200b}tag".into()),
            ("p1", Some("new"), "🦀".repeat(81)),
            ("p1", Some("bad\nname"), "tag".into()),
            ("p99", Some("new"), "tag".into()),
        ] {
            let before = set.clone();
            let error = set
                .dispatch(
                    "bad",
                    None,
                    Action::RenamePane {
                        pane: pane.into(),
                        name: name.map(str::to_owned),
                        tag: Some(tag),
                    },
                    prepared,
                    |_| panic!("rename runs no terminal operation"),
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                if pane == "p99" {
                    "unknown-pane"
                } else {
                    "malformed-action"
                }
            );
            assert_eq!(set, before);
        }
        for (index, (name, tag, expected_pin, expected_tag)) in [
            (
                Some(" review "),
                Some(" work "),
                Some("review"),
                Some("work"),
            ),
            (Some("new"), None, Some("new"), Some("work")),
            (None, None, None, Some("work")),
            (Some(""), Some("   "), None, None),
        ]
        .into_iter()
        .enumerate()
        {
            set.dispatch(
                &format!("rename-{index}"),
                None,
                Action::RenamePane {
                    pane: "p1".into(),
                    name: name.map(str::to_owned),
                    tag: tag.map(str::to_owned),
                },
                prepared,
                |_| panic!("rename runs no terminal operation"),
            )
            .unwrap();
            let snapshot = set.snapshot();
            let pane = snapshot
                .tabs
                .iter()
                .flat_map(|tab| tab.panes())
                .find(|pane| pane.id == "p1")
                .unwrap();
            assert_eq!(pane.pinned_name.as_deref(), expected_pin);
            assert_eq!(pane.tag.as_deref(), expected_tag);
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
                tag: Some(" work ".into()),
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
        let tag = |set: &WorkspaceSet| {
            set.snapshot()
                .tabs
                .iter()
                .flat_map(|tab| tab.panes())
                .find(|pane| pane.id == "p1")
                .unwrap()
                .tag
                .clone()
        };
        assert_eq!(tag(&set).as_deref(), Some("work"));
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
                        name: Some(name.into()),
                        tag: None,
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
        assert_eq!(tag(&set).as_deref(), Some("work"));
        set.dismiss_pending_pairs(|_| Ok(())).unwrap();
        assert_eq!(tag(&set).as_deref(), Some("work"));
        set.dispatch(
            "clear",
            None,
            Action::RenamePane {
                pane: "p1".into(),
                name: Some("  ".into()),
                tag: None,
            },
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(pin(&set), None);
        assert_eq!(tag(&set).as_deref(), Some("work"));
        set.dispatch(
            "name-again",
            None,
            Action::RenamePane {
                pane: "p1".into(),
                name: Some("Again".into()),
                tag: None,
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
        let original = set.snapshot_in(Some("s1")).unwrap().tabs;
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
        // Replaying a creation neither allocates another workspace nor starts a pair.
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
        assert_eq!(shown.workspace, "s3");
        assert_eq!(
            shown
                .workspaces
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s3", "s4", "s2"]
        );
        assert!(shown.tabs[0].pending);
        assert_eq!(shown.tabs[0].popups[0].entry, FILES_ENTRY);
        let caller_view = set
            .snapshot_in(set.caller_workspace_id(Some(&caller)).unwrap().as_deref())
            .unwrap();
        assert_eq!(
            (
                caller_view.workspace.as_str(),
                caller_view.active_workspace.as_str()
            ),
            ("s1", "s3")
        );
        assert_eq!(caller_view.tabs, original);
        eon_workspace_protocol::v42::encode_response(
            &eon_workspace_protocol::v42::Response::Snapshot(caller_view),
        )
        .unwrap();

        let before = set.clone();
        let error = set
            .dispatch(
                "ambiguous",
                None,
                Action::Workspace(WorkspaceAction::SwitchWorkspace {
                    workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector(
                        "work".into(),
                    ),
                }),
                prepared,
                |_| panic!("a switch starts nothing"),
            )
            .unwrap_err();
        assert_eq!(error.code, "ambiguous-session");
        assert!(error.detail.contains("s3") && error.detail.contains("s4"));
        assert_eq!(set, before);
        set.dispatch(
            "same",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector("s3".into()),
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
                name: "s3".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            set.resolve_workspace(&WorkspaceTarget::Selector("s3".into()))
                .unwrap(),
            1,
            "an ID wins over another workspace's name"
        );
        set.workspaces[0].name = Some("s99".into());
        assert_eq!(
            set.resolve_workspace(&WorkspaceTarget::Selector("s99".into()))
                .unwrap(),
            0,
            "an absent ID falls through to an exact name"
        );
        set.workspaces[0].name = Some("s01".into());
        assert_eq!(
            set.resolve_workspace(&WorkspaceTarget::Selector("s01".into()))
                .unwrap(),
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
        for target in ["missing", "s999"] {
            assert_eq!(
                set.resolve_workspace(&WorkspaceTarget::Selector(target.into()))
                    .unwrap_err()
                    .code,
                "unknown-session"
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
        while set.workspaces.len() < eon_workspace_protocol::v42::MAX_WORKSPACES {
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
            "session-limit"
        );
        assert_eq!(set, before);
    }

    #[test]
    fn popup_dismissal_prunes_an_empty_workspace_and_final_emptying_keeps_an_owner() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch("commands", None, invoke("t1", "commands"), prepared, |_| {
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
        eon_workspace_protocol::v42::encode_response(
            &eon_workspace_protocol::v42::Response::Snapshot(snapshot.clone()),
        )
        .unwrap();
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspace, "s2");

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
    fn failed_pending_pair_exit_prunes_the_workspace() {
        assert_failed_pending_pair_prunes_workspace(true);
    }

    #[test]
    fn failed_pending_pair_dismissal_prunes_the_workspace() {
        assert_failed_pending_pair_prunes_workspace(false);
    }

    fn assert_failed_pending_pair_prunes_workspace(exited: bool) {
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
        assert_eq!(pending.workspace, "s3");
        let popup = &pending.tabs[0].popups[0];
        let fail_start = |operation| match operation {
            SessionOperation::Start { .. } => Err("start failed".into()),
            SessionOperation::Stop(_) | SessionOperation::SettlePair(_) => Ok(()),
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
        assert_eq!(error.code, "terminal-start");
        assert_eq!(error.detail, "start failed");
        let snapshot = set.snapshot();
        eon_workspace_protocol::v42::encode_response(
            &eon_workspace_protocol::v42::Response::Snapshot(snapshot.clone()),
        )
        .unwrap();
        assert_eq!(snapshot.workspace, "s1");
        assert_eq!(snapshot.workspaces.len(), 2);
    }

    #[test]
    fn workspace_requests_resolve_id_shaped_names_after_live_ids() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some(eon_workspace_protocol::v42::WorkspaceTarget::Selector(
                    "s1".into(),
                )),
                name: "s2".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "live-id",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector("s2".into()),
            }),
            prepared,
            |_| panic!("switching starts nothing"),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "s2");
        set.dispatch(
            "absent-id-name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some(eon_workspace_protocol::v42::WorkspaceTarget::Selector(
                    "s1".into(),
                )),
                name: "s99".into(),
            }),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "switch-name",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector("s99".into()),
            }),
            prepared,
            |_| panic!("switching starts nothing"),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "s1");
        set.dispatch(
            "rename-name",
            None,
            Action::Workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some(eon_workspace_protocol::v42::WorkspaceTarget::Selector(
                    "s99".into(),
                )),
                name: "s100".into(),
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
            set.prepare_close_workspace(
                "close-name",
                None,
                &WorkspaceTarget::Selector("s100".into())
            )
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
            [("s1".into(), 0), ("s3".into(), 2), ("s2".into(), 1)]
        );
        for (request, workspace) in [("third", "s3"), ("second", "s2")] {
            let switch = Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector(workspace.into()),
            });
            set.dispatch(request, None, switch, prepared, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(
            ranks(&set),
            [("s1".into(), 2), ("s3".into(), 1), ("s2".into(), 0)]
        );
    }

    #[test]
    fn attention_focus_crosses_workspaces_atomically_and_keeps_local_callers_local() {
        let mut set = two_workspace_set(|_| Ok(()));
        let hidden = set.snapshot_in(Some("s2")).unwrap();
        let tab = hidden.tabs[0].id.clone();
        let pane = hidden.tabs[0].panes().next().unwrap().id.clone();
        let focus = Action::FocusAttention {
            tab: tab.clone(),
            target: pane.clone(),
        };
        let before = set.snapshots();
        assert_eq!(
            set.dispatch(
                "stale",
                None,
                Action::FocusAttention {
                    tab: tab.clone(),
                    target: "missing-pane".into(),
                },
                prepared,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "stale-target"
        );
        assert_eq!(set.snapshots(), before);
        let caller = hidden.tabs[0].panes().next().unwrap().session.clone();
        set.dispatch("local", Some(&caller), focus.clone(), prepared, |_| Ok(()))
            .unwrap();
        assert_eq!(
            set.snapshot().workspace,
            "s1",
            "a hidden caller stays hidden"
        );
        let shown = set.snapshot();
        set.dispatch(
            "shown",
            Some(&caller),
            Action::FocusAttention {
                tab: shown.tabs[0].id.clone(),
                target: shown.tabs[0].panes().next().unwrap().id.clone(),
            },
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "s1");
        set.dispatch("row", None, focus.clone(), prepared, |_| Ok(()))
            .unwrap();
        assert_eq!(set.snapshot().workspace, "s2");
        assert_eq!(
            set.dispatch("row", None, focus, prepared, |_| Ok(()))
                .unwrap_err()
                .code,
            "duplicate-request"
        );

        set.jump("jump", None, "s2", &tab, &pane).unwrap();
        assert_eq!(set.snapshot().workspace, "s2");
        assert_eq!(set.recent_workspaces, ["s2", "s1"]);
        assert_eq!(
            set.jump("jump", None, "s2", &tab, &pane).unwrap_err().code,
            "duplicate-request"
        );
        let stale = set.jump("stale", None, "s1", &tab, &pane).unwrap_err();
        assert_eq!(stale.code, "stale-target");
        assert_eq!(
            set.snapshot().workspace,
            "s2",
            "a failed jump shows nothing"
        );
    }

    #[test]
    fn terminal_targets_name_panes_and_popups_in_any_workspace_and_change_nothing() {
        let mut set = two_workspace_set(|_| Ok(()));
        let hidden = set.snapshot_in(Some("s2")).unwrap();
        let tab = hidden.tabs[0].id.clone();
        let pane = hidden.tabs[0].panes().next().unwrap().clone();
        set.dispatch(
            "agent",
            Some(&pane.session),
            invoke(&tab, "agent"),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(set.snapshot().workspace, "s1");
        let popup = set.snapshot_in(Some("s2")).unwrap().tabs[0].popups[0].clone();
        let target = |id, terminal: &str| TerminalTarget {
            workspace: "s2".into(),
            tab: tab.clone(),
            id,
            terminal: terminal.into(),
        };
        let before = set.snapshots();
        assert_eq!(
            set.terminal_target("read", false, &pane.id),
            Ok(target(TargetId::Pane(pane.id.clone()), &pane.session))
        );
        assert_eq!(
            set.terminal_target("send", true, &popup.id),
            Ok(target(TargetId::Popup(popup.id.clone()), &popup.session))
        );
        assert_eq!(
            set.session_target(&popup.session),
            Some(target(TargetId::Popup(popup.id.clone()), &popup.session))
        );
        assert_eq!(set.session_target("terminal-99"), None);
        assert_eq!(
            set.terminal_target("missing", false, "p99")
                .unwrap_err()
                .code,
            "unknown-target"
        );
        for id in ["read", "send"] {
            assert_eq!(
                set.terminal_target(id, true, &pane.id).unwrap_err().code,
                "duplicate-request"
            );
        }
        assert_eq!(set.snapshots(), before);
        assert_eq!(set.snapshot().workspace, "s1");
    }

    #[test]
    fn workspace_closes_prune_only_confirmed_ends_and_show_the_recent_survivor() {
        let mut set = two_workspace_set(|_| Ok(()));
        set.dispatch(
            "show-second",
            None,
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector("s2".into()),
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
        let survivor = set.snapshot_in(Some("s1")).unwrap().tabs;
        let closing = set
            .prepare_close_workspace("close", None, &WorkspaceTarget::Selector("s3".into()))
            .unwrap();
        assert_eq!(closing.len(), 1, "a fresh workspace has only its pair");
        assert_eq!(
            set.prepare_close_workspace("close", None, &WorkspaceTarget::Selector("s3".into()))
                .unwrap_err()
                .code,
            "duplicate-request"
        );
        // Before a confirmed stop, the workspace remains intact.
        assert_eq!(set.snapshot().workspace, "s3");
        set.session_ended(&closing[0], SessionEnd::Closed, |_| {
            panic!("closing never starts a terminal")
        })
        .unwrap();
        assert_eq!(set.snapshot().workspace, "s2");
        assert_eq!(set.snapshot().workspaces.len(), 2);
        assert_eq!(set.snapshot_in(Some("s1")).unwrap().tabs, survivor);
        let background = set
            .prepare_close_workspace("background", None, &WorkspaceTarget::Selector("s1".into()))
            .unwrap();
        for id in background {
            set.session_ended(&id, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(set.snapshot().workspace, "s2");
        assert_eq!(set.snapshot().workspaces.len(), 1);
        let last = set
            .prepare_close_workspace("last", None, &WorkspaceTarget::Selector("s2".into()))
            .unwrap();
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
        assert_eq!(set.workspaces[0].id, "s1");
        assert_eq!(set.workspaces[1].id, "s2");
        let initial = set.snapshot();
        assert_eq!(initial.active_tab, "t1");
        assert_eq!(initial.tabs[0].columns[0].panes[0].id, "p1");
        assert_eq!(initial.tabs[0].columns[0].panes[0].session, "terminal-1");
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
            assert_eq!(
                set.prepare_close_tab("refused", Some(&caller.id), &foreign.id)
                    .unwrap_err()
                    .code,
                "unknown-tab"
            );
            assert_eq!(
                set.prepare_close_pane(
                    "refused",
                    Some(&caller.id),
                    &foreign.columns[0].panes[0].id
                )
                .unwrap_err()
                .code,
                "unknown-pane"
            );
            if shown == 0 {
                assert_eq!(
                    set.prepare_close_tab("refused-none", None, &foreign.id)
                        .unwrap_err()
                        .code,
                    "unknown-tab"
                );
                assert_eq!(
                    set.prepare_close_pane("refused-none", None, &foreign.columns[0].panes[0].id)
                        .unwrap_err()
                        .code,
                    "unknown-pane"
                );
            }
            // Stale local IDs and a stale caller refuse in the same way.
            for caller in [Some(caller.id.as_str()), None] {
                assert_eq!(
                    set.prepare_close_tab("refused-stale", caller, "t99"),
                    Err(action_error("unknown-tab", "tab t99 is not live"))
                );
                assert_eq!(
                    set.prepare_close_pane("refused-stale", caller, "p99"),
                    Err(action_error("unknown-pane", "pane p99 is not live"))
                );
            }
            let stale = action_error(
                "unknown-caller",
                "the calling terminal is not part of this session",
            );
            assert_eq!(
                set.prepare_close_tab("refused-caller", Some("terminal-99"), &foreign.id),
                Err(stale.clone())
            );
            assert_eq!(
                set.prepare_close_pane(
                    "refused-caller",
                    Some("terminal-99"),
                    &foreign.columns[0].panes[0].id
                ),
                Err(stale)
            );
            assert_eq!(set, before);
        }
        // A refused close does not use up its request ID.
        let local = set.workspaces[0].tabs[0].id.clone();
        let sessions = set
            .prepare_close_tab("refused", Some(&caller.id), &local)
            .unwrap();
        assert!(sessions.contains(&caller.id));
        assert_eq!(
            set.prepare_close_tab("refused", Some(&caller.id), &local)
                .unwrap_err()
                .code,
            "duplicate-request"
        );
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
            workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector("s2".into()),
        });
        set.dispatch("create", None, create.clone(), prepared, |_| Ok(()))
            .unwrap();
        set.dispatch("switch", None, switch.clone(), prepared, |_| Ok(()))
            .unwrap();
        set.prepare_close_workspace("close", None, &WorkspaceTarget::Selector("s3".into()))
            .unwrap();
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
            set.prepare_close_workspace("close", None, &WorkspaceTarget::Selector("s3".into()))
                .unwrap_err()
                .code,
            "duplicate-request"
        );
        assert_eq!(set, before);

        for number in 0..MAX_RECENT_REQUESTS {
            if number == MAX_RECENT_REQUESTS - 1 {
                assert_eq!(
                    set.prepare_close_workspace(
                        "close",
                        None,
                        &WorkspaceTarget::Selector("s3".into())
                    )
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
        set.prepare_close_workspace("close", None, &WorkspaceTarget::Selector("s3".into()))
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
    fn caller_output_finds_a_popup_and_omits_an_absent_terminal() {
        let mut snapshot = two_workspace_set(|_| Ok(())).snapshot();
        snapshot.tabs[0].popups.push(super::SnapshotPopup {
            id: "u9".into(),
            entry: "project".into(),
            session: "terminal-9".into(),
            endpoint: b"/runtime/terminal-9.sock".to_vec(),
            attention: None,
        });
        assert_eq!(
            super::caller_output(&snapshot, Some("terminal-9"), true),
            "\"caller\":{\"terminal\":\"terminal-9\",\"tab\":\"t1\",\"pane\":null,\"popup\":\"u9\"},"
        );
        assert_eq!(
            super::caller_output(&snapshot, Some("terminal-9"), false),
            "caller terminal=terminal-9 tab=t1 pane=none popup=u9\n"
        );
        for session in [None, Some("terminal-999")] {
            assert_eq!(
                super::caller_output(&snapshot, session, true),
                "\"caller\":null,"
            );
            assert_eq!(super::caller_output(&snapshot, session, false), "");
        }
    }

    #[test]
    fn terminal_sent_json_reports_what_was_sent_and_an_enter_failure() {
        let sent = Sent {
            target: TerminalTarget {
                workspace: "s2".into(),
                tab: "t3".into(),
                id: TargetId::Pane("p4".into()),
                terminal: "terminal-5".into(),
            },
            caller: None,
            bracketed: false,
            enter_delay_ms: 300,
        };
        let target = "\"session\":\"s2\",\"tab\":\"t3\",\"pane\":\"p4\",\"popup\":null,\
                      \"terminal\":\"terminal-5\",\"caller\":null";
        assert_eq!(
            super::terminal_sent_json(&sent, 0, true, None),
            format!("{{{target},\"bytes\":0,\"bracketed\":null,\"enter\":true}}\n")
        );
        let failure = Failure {
            code: "terminal-ended".into(),
            detail: "terminal \"x\" has ended".into(),
        };
        assert_eq!(
            super::terminal_sent_json(&sent, 5, false, Some(&failure)),
            format!(
                "{{{target},\"bytes\":5,\"bracketed\":false,\"enter\":false,\
                 \"error\":{{\"code\":\"terminal-ended\",\"detail\":\"terminal \\\"x\\\" has ended\"}}}}\n"
            )
        );
    }

    #[test]
    fn terminal_text_json_names_the_target_and_caller_and_keeps_raw_directories() {
        let target = |workspace: &str, id| TerminalTarget {
            workspace: workspace.into(),
            tab: "t2".into(),
            id,
            terminal: "terminal-4".into(),
        };
        let read = TerminalText {
            target: target("s2", TargetId::Popup("u3".into())),
            caller: Some(target("s1", TargetId::Pane("p1".into()))),
            revision: 7,
            screen: Screen::Alternate,
            rows: 2,
            columns: 10,
            title: "vim \"a\"".into(),
            directory: b"/tmp/\xff".to_vec(),
            omitted_rows: 1,
            text: "one\ntwo".into(),
        };
        assert_eq!(
            super::terminal_text_json(&read),
            "{\"session\":\"s2\",\"tab\":\"t2\",\"pane\":null,\"popup\":\"u3\",\
             \"terminal\":\"terminal-4\",\"caller\":{\"terminal\":\"terminal-4\",\"tab\":\"t2\",\
             \"pane\":\"p1\",\"popup\":null},\"revision\":7,\"screen\":\"alternate\",\"rows\":2,\
             \"columns\":10,\"title\":\"vim \\\"a\\\"\",\"directory\":[47,116,109,112,47,255],\
             \"text\":\"one\\ntwo\",\"omitted_rows\":1,\"lossy\":true}\n"
        );
        let history = TerminalHistory {
            target: read.target,
            caller: read.caller,
            revision: read.revision,
            screen: Screen::Primary,
            rows: read.rows,
            columns: read.columns,
            title: read.title,
            directory: read.directory,
            omitted_lines: 2,
            cut_bytes: 3,
            text: read.text,
        };
        assert_eq!(
            super::terminal_history_json(&history),
            "{\"session\":\"s2\",\"tab\":\"t2\",\"pane\":null,\"popup\":\"u3\",\
             \"terminal\":\"terminal-4\",\"caller\":{\"terminal\":\"terminal-4\",\"tab\":\"t2\",\
             \"pane\":\"p1\",\"popup\":null},\"revision\":7,\"screen\":\"primary\",\"rows\":2,\
             \"columns\":10,\"title\":\"vim \\\"a\\\"\",\"directory\":[47,116,109,112,47,255],\
             \"text\":\"one\\ntwo\",\"omitted_lines\":2,\"cut_bytes\":3,\"lossy\":true}\n"
        );
        let editing = Editing {
            target: history.target,
            caller: None,
            popup: target("s2", TargetId::Popup("u5".into())),
        };
        assert_eq!(
            super::terminal_editing_json(&editing),
            "{\"session\":\"s2\",\"tab\":\"t2\",\"pane\":null,\"popup\":\"u3\",\
             \"terminal\":\"terminal-4\",\"caller\":null,\
             \"scrollback\":{\"popup\":\"u5\",\"terminal\":\"terminal-4\"}}\n"
        );
    }

    #[test]
    fn json_paths_are_strings_unless_their_bytes_are_not_utf8() {
        assert_eq!(json_path(b"/home/a \"b\""), "\"/home/a \\\"b\\\"\"");
        assert_eq!(json_path(b"/tmp/\xff"), "[47,116,109,112,47,255]");
    }

    #[test]
    fn window_name_uses_shown_workspace_then_directory_and_home() {
        use std::os::unix::ffi::OsStrExt;
        let mut set = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/home/me".into(),
            Some("/home/me".into()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        assert_eq!(set.window_name(), "~");
        set.workspaces[0].tabs[0].directory =
            Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xffname")).into();
        assert_eq!(set.window_name(), "�name");
        set.workspaces[0].tabs[0].directory =
            Path::new(std::ffi::OsStr::from_bytes(b"/tmp/a\x1bb\nc")).into();
        assert_eq!(set.window_name(), "a�b�c");
        set.workspaces[0].name = Some("🦀".repeat(64));
        assert_eq!(set.window_name(), "🦀".repeat(64));
        set.workspaces[0].name = None;
        set.workspaces[0].tabs[0].directory = format!("/tmp/{}", "🦀".repeat(65)).into();
        assert_eq!(set.window_name(), "🦀".repeat(64));
    }

    #[test]
    fn cli_json_carries_running_windows() {
        use eon_workspace_protocol::v42::{RunningWindows, WindowEntry, WindowState};
        let set = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            None,
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let mut snapshot = set.snapshot();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json(&snapshot))
                .unwrap()
                .get("running"),
            Some(&serde_json::Value::Null)
        );
        let this = WindowEntry {
            window: None,
            generation: "g1".into(),
            name: Some("a\"\n🦀".into()),
            started_at: Some(123),
            live_terminals: Some(1),
            state: WindowState::Live,
        };
        snapshot.running = Some(RunningWindows {
            this: this.clone(),
            others: None,
            omitted: 0,
        });
        let initial: serde_json::Value = serde_json::from_str(&json(&snapshot)).unwrap();
        assert!(initial["running"]["others"].is_null());
        snapshot.running.as_mut().unwrap().others = Some(vec![WindowEntry {
            window: Some("1234abcd".into()),
            generation: "g2".into(),
            name: None,
            started_at: None,
            live_terminals: None,
            state: WindowState::Incompatible,
        }]);
        snapshot.running.as_mut().unwrap().omitted = 7;
        let value: serde_json::Value = serde_json::from_str(&json(&snapshot)).unwrap();
        assert_eq!(value["running"]["this"]["window"], serde_json::Value::Null);
        assert_eq!(value["running"]["this"]["name"], this.name.unwrap());
        assert_eq!(value["running"]["this"]["started_at"], 123);
        assert_eq!(value["running"]["this"]["live_terminals"], 1);
        assert_eq!(value["running"]["others"][0]["state"], "incompatible");
        assert!(value["running"]["others"][0]["name"].is_null());
        assert!(value["running"]["others"][0]["started_at"].is_null());
        assert!(value["running"]["others"][0]["live_terminals"].is_null());
        assert_eq!(value["running"]["omitted"], 7);
        let human = human(&snapshot);
        assert_eq!(
            human
                .lines()
                .filter(|line| line.starts_with("running "))
                .count(),
            2
        );
        assert!(human.contains("a\\\"\\n🦀"), "{human}");
    }

    #[test]
    fn cli_json_carries_busy_terminals() {
        use eon_workspace_protocol::v42::{BusyTerminal, BusyTerminals};
        let set = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            None,
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let mut snapshot = set.snapshot();
        let value = |snapshot: &Snapshot| {
            serde_json::from_str::<serde_json::Value>(&json(snapshot)).unwrap()["busy"].clone()
        };
        assert!(value(&snapshot).is_null());
        let target = |id| TerminalTarget {
            workspace: "s1".into(),
            tab: "t1".into(),
            id,
            terminal: "terminal-2".into(),
        };
        snapshot.busy = Some(BusyTerminals {
            terminals: vec![
                BusyTerminal {
                    target: target(TargetId::Pane("p2".into())),
                    name: "p2  \"vim\"".into(),
                },
                BusyTerminal {
                    target: target(TargetId::Popup("u1".into())),
                    name: "Agent".into(),
                },
            ],
            omitted: 3,
        });
        assert_eq!(
            value(&snapshot),
            serde_json::json!({
                "terminals": [
                    {"session": "s1", "tab": "t1", "pane": "p2", "popup": null,
                     "terminal": "terminal-2", "name": "p2  \"vim\""},
                    {"session": "s1", "tab": "t1", "pane": null, "popup": "u1",
                     "terminal": "terminal-2", "name": "Agent"},
                ],
                "omitted": 3,
            })
        );
        assert_eq!(
            human_busy(snapshot.busy.as_ref()),
            "busy session=s1 tab=t1 pane=p2 terminal=terminal-2 name=p2  \\\"vim\\\"\n\
             busy session=s1 tab=t1 popup=u1 terminal=terminal-2 name=Agent\n\
             busy omitted=3\n"
        );
    }

    #[test]
    fn cli_json_carries_the_full_attention_record_and_quotas_but_human_output_does_not() {
        use eon_workspace_protocol::v42::{
            AttentionKind, AttentionRecord, ClaudeQuota, ClaudeQuotaState, CodexQuota,
            CodexQuotaState, QuotaWindow,
        };

        let workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            None,
            (1..=3)
                .map(|number| {
                    (
                        number,
                        Session {
                            id: format!("terminal-{number}"),
                            endpoint: format!("/runtime/terminal-{number}.sock").into(),
                        },
                    )
                })
                .collect(),
            catalog(),
        )
        .unwrap();
        let mut snapshot = workspace.snapshot();
        snapshot.tabs[0].columns[0].panes[0].attention = Some(Attention {
            state: AttentionState::NeedsInput,
            record: Some(AttentionRecord {
                kind: Some(AttentionKind::Permission),
                progress: Some(40),
                app: "codex".into(),
                name: "eon \"9a\"".into(),
                title: "secret \"title\"".into(),
                message: r#"secret "message"\path"#.into(),
                changed_at: 1_720_000_000_123,
                stale: true,
                others: 3,
            }),
        });
        snapshot.tabs[0].columns[0].panes[2].attention = Some(Attention {
            state: AttentionState::Working,
            record: Some(AttentionRecord {
                kind: None,
                progress: None,
                app: String::new(),
                name: String::new(),
                title: String::new(),
                message: String::new(),
                changed_at: 1_720_000_000_456,
                stale: false,
                others: 0,
            }),
        });
        snapshot.tabs[0].popups.push(SnapshotPopup {
            id: "popup-1".into(),
            entry: "agent".into(),
            session: "terminal-4".into(),
            endpoint: b"/runtime/terminal-4.sock".to_vec(),
            attention: Some(Attention {
                state: AttentionState::Unavailable,
                record: None,
            }),
        });
        snapshot.tabs[0].attention_rollup = Some(AttentionState::NeedsInput);
        snapshot.sidebar = Sidebar::Closed;
        snapshot.codex_quota = Some(CodexQuota {
            state: CodexQuotaState::Stale,
            observed_at: 1_720_000_000,
            windows: vec![
                QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 80,
                    resets_at: Some(1_720_000_300),
                    scope: None,
                },
                QuotaWindow {
                    duration_minutes: 10080,
                    remaining_percent: 60,
                    resets_at: None,
                    scope: None,
                },
            ],
        });
        snapshot.claude_quota = Some(ClaudeQuota {
            state: ClaudeQuotaState::Fresh,
            observed_at: 1_720_000_001,
            windows: vec![
                QuotaWindow {
                    duration_minutes: 300,
                    remaining_percent: 25,
                    resets_at: Some(1_720_000_600),
                    scope: None,
                },
                QuotaWindow {
                    duration_minutes: 10080,
                    remaining_percent: 40,
                    resets_at: None,
                    scope: Some("Fable \"x\"".into()),
                },
            ],
        });
        snapshot.claude_quota_failure = Some(QuotaFailure {
            at: 1_720_000_002,
            reason: QuotaFailureReason::InvalidWindow,
        });
        snapshot.attention_queue = vec![
            eon_workspace_protocol::v42::QueueEntry {
                workspace: "s1".into(),
                tab: "t1".into(),
                target: "p1".into(),
                attention: snapshot.tabs[0].columns[0].panes[0]
                    .attention
                    .clone()
                    .unwrap(),
            },
            eon_workspace_protocol::v42::QueueEntry {
                workspace: "s2".into(),
                tab: "t\"2".into(),
                target: "p\\2".into(),
                attention: Attention {
                    state: AttentionState::Finished,
                    ..snapshot.tabs[0].columns[0].panes[2]
                        .attention
                        .clone()
                        .unwrap()
                },
            },
        ];
        let output = json(&snapshot);
        assert!(output.contains(r#""attention_rollup":"needs-input","panes":[{"id":"p1","#));
        assert!(
            output.contains(
                r#""attention":{"state":"needs-input","app":"codex","name":"eon \"9a\"","kind":"permission","progress":40,"title":"secret \"title\"","message":"secret \"message\"\\path","changed_at":1720000000123,"stale":true,"others":3}}"#
            ),
            "{output}"
        );
        assert!(output.contains(
            r#""label":"p2  unavailable","pinned_name":null,"tag":null,"attention":null}"#
        ));
        assert!(output.contains(
            r#""attention":{"state":"working","app":null,"name":null,"kind":null,"progress":null,"title":null,"message":null,"changed_at":1720000000456,"stale":false,"others":0}}"#
        ));
        assert!(output.contains(
            r#""attention":{"state":"unavailable","app":null,"name":null,"kind":null,"progress":null,"title":null,"message":null,"changed_at":null,"stale":null,"others":null}}"#
        ));
        assert!(output.ends_with(concat!(
            r#"],"sidebar":"closed","codex_quota":{"state":"stale","observed_at":1720000000000,"windows":[{"duration_minutes":300,"remaining_percent":80,"resets_at":1720000300000,"scope":null},{"duration_minutes":10080,"remaining_percent":60,"resets_at":null,"scope":null}]}"#,
            r#","claude_quota":{"state":"fresh","observed_at":1720000001000,"windows":[{"duration_minutes":300,"remaining_percent":25,"resets_at":1720000600000,"scope":null},{"duration_minutes":10080,"remaining_percent":40,"resets_at":null,"scope":"Fable \"x\""}]}"#,
            r#","codex_quota_failure":null,"claude_quota_failure":{"at":1720000002000,"reason":"invalid_window"}"#,
            r#","attention_queue":[{"session":"s1","tab":"t1","target":"p1","attention":{"state":"needs-input","app":"codex","name":"eon \"9a\"","kind":"permission","progress":40,"title":"secret \"title\"","message":"secret \"message\"\\path","changed_at":1720000000123,"stale":true,"others":3}},{"session":"s2","tab":"t\"2","target":"p\\2","attention":{"state":"finished","app":null,"name":null,"kind":null,"progress":null,"title":null,"message":null,"changed_at":1720000000456,"stale":false,"others":0}}],"running":null,"busy":null}"#,
            "\n"
        )));
        assert!(
            json(&workspace.snapshot())
                .ends_with("\"sidebar\":\"off\",\"codex_quota\":null,\"claude_quota\":null,\"codex_quota_failure\":null,\"claude_quota_failure\":null,\"attention_queue\":[],\"running\":null,\"busy\":null}\n")
        );
        snapshot.codex_quota.as_mut().unwrap().windows[0].resets_at = Some(18_446_744_073_709_552);
        assert!(json(&snapshot).contains(r#""resets_at":18446744073709552000"#));
        let human = human(&snapshot);
        assert!(human.starts_with("active t1\nsidebar closed\ntab t1 "));
        assert!(human.contains(" attention=needs-input\n"));
        assert!(human.contains(" attention=needs-input app=codex name=eon \\\"9a\\\"\n"));
        assert!(human.contains(" attention=none app=none name=none\n"));
        for omitted in ["secret", "progress", "quota", "observed", "attention_queue"] {
            assert!(!human.contains(omitted), "{human}");
        }
    }

    #[test]
    fn popup_identity_recovery_gate() {
        let pane = Session {
            id: "terminal-1".into(),
            endpoint: "/runtime/orbit.sock".into(),
        };
        let recover = |range| {
            WorkspaceSet::registered_recovered(
                None,
                None,
                IdAllocator::new("/runtime".into(), range).unwrap(),
                "/".into(),
                Some("/".into()),
                vec![(1, pane.clone())],
                catalog(),
            )
            .unwrap()
        };
        let mut before = recover(0);
        before
            .dispatch("old", None, invoke("t1", "git"), prepared, |_| Ok(()))
            .unwrap();
        let old = before.snapshot().tabs[0].popups[0].clone();
        before
            .session_ended(&old.session, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        let mut recovered = recover(1);
        let tab = recovered.snapshot().active_tab;
        recovered
            .dispatch("new", None, invoke(&tab, "git"), prepared, |_| Ok(()))
            .unwrap();
        let snapshot = recovered.snapshot();
        let stale = Action::DismissPopup(PopupTarget {
            tab,
            instance: old.id.clone(),
        });
        assert!(
            check_popup_action(&snapshot, &stale).is_err(),
            "saved {} targets successor {:?} after recovery",
            old.id,
            snapshot.tabs[0].popups[0]
        );
    }

    #[test]
    fn recovery_never_retargets_pane_tab_or_session_ids() {
        let sessions = || {
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )]
        };
        let mut before = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            sessions(),
            catalog(),
        )
        .unwrap();
        before
            .dispatch("popup", None, invoke("t1", "git"), prepared, |_| Ok(()))
            .unwrap();
        before
            .dispatch(
                "pane",
                Some("terminal-1"),
                Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            before.snapshot().tabs[0].panes().nth(1).unwrap().session,
            "terminal-3"
        );
        before
            .dispatch(
                "tab",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        before
            .insert_pending(
                "/".into(),
                Some("/".into()),
                catalog(),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let old = before.snapshots();
        for range in 1..=2 {
            let mut recovered = WorkspaceSet::registered_recovered(
                None,
                None,
                IdAllocator::new("/runtime".into(), range).unwrap(),
                "/".into(),
                Some("/".into()),
                sessions(),
                catalog(),
            )
            .unwrap();
            let snapshot = recovered.snapshot();
            for session in &old {
                assert!(recovered.snapshot_in(Some(&session.workspace)).is_err());
                for tab in &session.tabs {
                    for id in std::iter::once(&tab.id).chain(tab.panes().map(|pane| &pane.id)) {
                        assert!(
                            recovered
                                .dispatch(
                                    &format!("stale-{id}"),
                                    None,
                                    Action::Workspace(WorkspaceAction::FocusId(id.clone())),
                                    prepared,
                                    |_| panic!("stale ID operated on a terminal"),
                                )
                                .is_err()
                        );
                        assert_eq!(recovered.snapshot(), snapshot);
                    }
                }
            }
        }
    }

    #[test]
    fn identity_ranges_refuse_exhaustion_before_process_effects() {
        let zero = IdAllocator::new("/runtime".into(), 0).unwrap();
        let next = IdAllocator::new("/runtime".into(), 1).unwrap();
        assert_eq!(zero.next_popup, 1);
        assert_eq!(zero.limit, next.next_popup);
        assert_eq!(zero.limit - 1, IDS_PER_SUPERVISOR);
        assert!(IdAllocator::new("/runtime".into(), usize::MAX / IDS_PER_SUPERVISOR).is_err());
        for kind in 0..4 {
            let mut set = WorkspaceSet::with_recovered_sessions(
                "/runtime".into(),
                "/".into(),
                Some("/".into()),
                vec![(
                    1,
                    Session {
                        id: "terminal-1".into(),
                        endpoint: "/runtime/orbit.sock".into(),
                    },
                )],
                catalog(),
            )
            .unwrap();
            let mut action = match kind {
                0 => {
                    set.ids.next_pane = set.ids.limit - 1;
                    Action::Workspace(WorkspaceAction::CreatePane { focus: true })
                }
                1 => {
                    set.ids.next_tab = set.ids.limit - 1;
                    Action::Workspace(WorkspaceAction::CreateTab)
                }
                2 => {
                    set.ids.next_popup = set.ids.limit - 1;
                    invoke("t1", "git")
                }
                _ => {
                    set.ids.next_workspace = set.ids.limit - 1;
                    Action::Workspace(WorkspaceAction::NewWorkspace {
                        name: None,
                        focus: true,
                    })
                }
            };
            set.dispatch("last-identity", None, action.clone(), prepared, |_| Ok(()))
                .unwrap();
            if kind == 2 {
                action = invoke("t1", "agent");
            }
            let before = set.snapshot();
            let error = set
                .dispatch("exhausted", None, action, prepared, |_| {
                    panic!("exhausted identity range operated on a terminal")
                })
                .unwrap_err();
            assert_eq!(error.code, "capacity");
            assert_eq!(set.snapshot(), before);
        }
    }

    #[test]
    fn exhausted_pane_ids_leave_pending_pairs_unchanged() {
        let mut pending = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        pending.ids.next_pane = pending.ids.limit;
        pending
            .dispatch(
                "pending",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let before = pending.snapshot();
        let tab = before
            .tabs
            .iter()
            .find(|tab| tab.id == before.active_tab)
            .unwrap();
        let pair = tab.popups[0].clone();
        for kind in 0..4 {
            let mut set = pending.clone();
            let mut operations = Vec::new();
            let operate = |operation| {
                operations.push(operation);
                Ok(())
            };
            let result = match kind {
                0 => set.dispatch(
                    "hide",
                    None,
                    Action::DismissPopup(PopupTarget {
                        tab: tab.id.clone(),
                        instance: pair.id.clone(),
                    }),
                    prepared,
                    operate,
                ),
                1 => set.dispatch(
                    "pick",
                    None,
                    Action::Workspace(WorkspaceAction::SetTabDirectory {
                        tab: tab.id.clone(),
                        directory: b"/".to_vec(),
                    }),
                    prepared,
                    operate,
                ),
                2 => set.dispatch(
                    "toggle",
                    None,
                    Action::InvokePopup {
                        tab: tab.id.clone(),
                        entry: FILES_ENTRY.into(),
                        expected_instance: Some(pair.id.clone()),
                        intent: InvokeIntent::Toggle,
                    },
                    prepared,
                    operate,
                ),
                _ => set.dismiss_pending_pairs(operate),
            };
            let error = result.unwrap_err();
            assert!(
                error.code == "capacity" && operations.is_empty() && set.snapshot() == before,
                "pending action {kind}: {error:?}; operations {operations:?}; snapshot {:?}",
                set.snapshot()
            );
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
                    command: PopupCommand::Argv(Vec::new()),
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
                PopupDefinition {
                    id: "commands".into(),
                    label: "Commands".into(),
                    shortcut: PopupKey::Unbound,
                    command: PopupCommand::Argv(vec!["commands".into()]),
                    keep_alive: false,
                },
                PopupDefinition {
                    id: "scrollback".into(),
                    label: "Scrollback".into(),
                    shortcut: PopupKey::Unbound,
                    command: PopupCommand::Scrollback,
                    keep_alive: false,
                },
            ],
        }
    }

    fn prepared(command: &PopupCommand, _: &Path) -> Result<PreparedCommand, String> {
        let argv = match command {
            PopupCommand::Argv(argv) => argv.clone(),
            PopupCommand::Editor => vec!["editor".into()],
            PopupCommand::Edit(file) => vec!["editor".into(), file.clone()],
            PopupCommand::Scrollback => vec!["snapshot-editor".into()],
            PopupCommand::AgentAuto => unreachable!(),
        };
        Ok(PreparedCommand {
            argv,
            listener: None,
        })
    }

    /// Scrollback snapshots what its active tab shows, its shown popup else
    /// its selected pane, opens as a transient over it, and its key again
    /// hides it; a tab showing no terminal, a hidden tab and a live
    /// Scrollback name no source (EON-C10).
    #[test]
    fn scrollback_snapshots_the_shown_terminal_and_covers_it_as_a_transient() {
        let mut set = two_workspace_set(|_| Ok(()));
        let request = |set: &mut WorkspaceSet, id: &str, action: Action| {
            set.dispatch(id, None, action, prepared, |_| Ok(()))
                .unwrap();
            set.snapshot().tabs.remove(0)
        };
        let tab = set.snapshot().tabs.remove(0);
        let pane = tab.selected_pane.clone().unwrap();
        let session = |set: &WorkspaceSet, id: &str| {
            let tab = set.snapshot().tabs.remove(0);
            tab.panes()
                .find(|pane| pane.id == id)
                .map(|pane| pane.session.clone())
                .or_else(|| {
                    tab.popups
                        .iter()
                        .find(|popup| popup.id == id)
                        .map(|popup| popup.session.clone())
                })
                .unwrap()
        };
        assert_eq!(
            set.scrollback_source(None, "t1", "scrollback"),
            Some((pane.clone(), session(&set, &pane)))
        );
        assert_eq!(set.scrollback_source(None, "t1", "commands"), None);

        // Over Git, it snapshots Git and shows Git again when it ends.
        let git = request(&mut set, "git", invoke("t1", "git")).popups[0]
            .id
            .clone();
        assert_eq!(
            set.scrollback_source(None, "t1", "scrollback"),
            Some((git.clone(), session(&set, &git)))
        );
        let tab = request(&mut set, "scrollback", invoke("t1", "scrollback"));
        let scrollback = tab.popups.last().unwrap().clone();
        assert_eq!(
            (scrollback.entry.as_str(), tab.selected_popup.as_ref()),
            ("scrollback", Some(&scrollback.id))
        );
        // A live Scrollback is no source: its key again hides it.
        assert_eq!(set.scrollback_source(None, "t1", "scrollback"), None);
        let toggle = |entry: &str, instance: &str| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: Some(instance.into()),
            intent: InvokeIntent::Toggle,
        };
        let tab = request(&mut set, "hide", toggle("scrollback", &scrollback.id));
        assert_eq!(tab.selected_popup.as_ref(), Some(&git));
        assert!(tab.popups.iter().all(|popup| popup.entry != "scrollback"));

        // Over a pane, its end shows the pane.
        request(&mut set, "hide-git", toggle("git", &git));
        let tab = request(&mut set, "over-pane", invoke("t1", "scrollback"));
        let ended = tab.popups.last().unwrap().session.clone();
        set.session_ended(&ended, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        let tab = set.snapshot().tabs.remove(0);
        assert_eq!((tab.selected_popup, tab.selected_pane), (None, Some(pane)));

        // A hidden tab names none, nor does a pending one.
        request(
            &mut set,
            "new-tab",
            Action::Workspace(WorkspaceAction::CreateTab),
        );
        let snapshot = set.snapshot();
        assert_ne!(snapshot.active_tab, "t1");
        assert_eq!(set.scrollback_source(None, "t1", "scrollback"), None);
        assert_eq!(
            set.scrollback_source(None, &snapshot.active_tab, "scrollback"),
            None
        );
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
                    id: "terminal-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        let ok = |_| Ok(());
        let transient = |tab: &str| Action::InvokePopup {
            tab: tab.into(),
            entry: "commands".into(),
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
        let pair = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&pair, SessionEnd::Exited, ok)
            .unwrap();
        workspace
            .dispatch("open", None, transient("t2"), prepared, ok)
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
        eon_workspace_protocol::v42::encode_response(
            &eon_workspace_protocol::v42::Response::Snapshot(after),
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
                    id: "terminal-1".into(),
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
        let pair = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&pair, SessionEnd::Exited, ok)
            .unwrap();
        let transient = Action::InvokePopup {
            tab: "t2".into(),
            entry: "commands".into(),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        };
        workspace
            .dispatch("open", None, transient, prepared, ok)
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
                    id: "terminal-1".into(),
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
        let snapshot = request("p3", Some("terminal-1"), command(false));
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
    fn launched_tabs_skip_the_picker_preserve_focus_and_roll_back_failures() {
        let root = std::env::temp_dir().join(format!("eon-tab-launch-{}", std::process::id()));
        let directory = root.join("project");
        fs::create_dir_all(&directory).unwrap();
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: root.join("orbit.sock"),
                },
            )],
            catalog(),
        )
        .unwrap();
        workspace
            .dispatch(
                "popup",
                None,
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "agent".into(),
                    expected_instance: None,
                    intent: InvokeIntent::Toggle,
                },
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let before = workspace.clone();
        let snapshot = workspace.snapshot();
        let caller = snapshot.tabs[0].popups[0].session.clone();
        let argv = vec![b"hx".to_vec(), b"notes \xff".to_vec()];
        let launch = |directory: &Path, command: bool, focus| Action::LaunchTab {
            directory: directory.as_os_str().as_bytes().to_vec(),
            argv: if command { argv.clone() } else { vec![] },
            focus,
        };
        let missing = root.join("missing");
        for (id, caller, directory, code) in [
            ("uncalled", None, &missing, "caller-required"),
            (
                "missing",
                Some(caller.as_str()),
                &missing,
                "invalid-directory",
            ),
        ] {
            let error = workspace
                .dispatch(
                    id,
                    caller,
                    launch(directory, true, false),
                    |_, _| panic!("rejected before preparation"),
                    |_| panic!("nothing starts"),
                )
                .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(workspace, before);
        }
        let error = workspace
            .dispatch(
                "prepare",
                Some(&caller),
                launch(&directory, true, false),
                |_, at| {
                    assert_eq!(at, directory);
                    Err("no command".into())
                },
                |_| panic!("nothing starts"),
            )
            .unwrap_err();
        assert_eq!(error.code, "pane-command");
        assert_eq!(workspace, before);
        let error = workspace
            .dispatch(
                "start",
                Some(&caller),
                launch(&directory, true, false),
                prepared,
                |_| Err("start failed".into()),
            )
            .unwrap_err();
        assert_eq!(error.code, "terminal-start");
        assert_eq!(workspace, before);

        let mut full = workspace.clone();
        full.ids.next_tab = usize::MAX;
        let full_before = full.clone();
        for (directory, code) in [(&missing, "invalid-directory"), (&directory, "capacity")] {
            let error = full
                .dispatch(
                    "full",
                    Some(&caller),
                    launch(directory, true, false),
                    |_, _| panic!("rejected before preparation"),
                    |_| panic!("nothing starts"),
                )
                .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(full, full_before);
        }

        let mut started = Vec::new();
        workspace
            .dispatch(
                "t2",
                Some(&caller),
                launch(&directory, true, false),
                |command, at| {
                    assert_eq!(at, directory);
                    prepared(command, at)
                },
                |operation| {
                    started.push(operation);
                    Ok(())
                },
            )
            .unwrap();
        let [
            SessionOperation::Start {
                directory: at,
                command: LaunchCommand::Tool(command),
                tab: None,
                ..
            },
        ] = started.as_slice()
        else {
            panic!("one command pane starts: {started:?}")
        };
        assert_eq!(at, &directory);
        assert_eq!(
            command,
            &argv
                .iter()
                .cloned()
                .map(OsString::from_vec)
                .collect::<Vec<_>>()
        );
        let after = workspace.snapshot();
        assert_eq!(after.active_tab, "t1");
        assert_eq!(after.tabs[0], snapshot.tabs[0]);
        let tab = &after.tabs[1];
        assert_eq!(tab.id, "t2");
        assert_eq!(tab.directory, directory.as_os_str().as_bytes());
        assert!(!tab.pending);
        assert_eq!(tab.panes().count(), 1);
        assert_eq!(tab.selected_pane.as_deref(), Some("p2"));
        assert!(tab.popups.is_empty());

        workspace
            .dispatch(
                "t3",
                Some(&caller),
                launch(&directory, true, true),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let after = workspace.snapshot();
        assert_eq!(after.active_tab, "t3");
        assert_eq!(after.tabs[0], snapshot.tabs[0]);
        workspace
            .dispatch(
                "t4",
                None,
                launch(&directory, false, true),
                |_, _| panic!("default panes need no command preparation"),
                |operation| {
                    assert!(matches!(operation, SessionOperation::Start {
                    directory: at, command: LaunchCommand::Pane, ..
                } if at == directory));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t4");
        fs::remove_dir_all(root).unwrap();
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
                    id: "terminal-1".into(),
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
                    id: "terminal-1".into(),
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
            .session_ended("terminal-5", SessionEnd::Exited, ok)
            .unwrap();
        assert_eq!(shape(&workspace).1, "p2");
        for session in ["terminal-4", "terminal-2", "terminal-3"] {
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
    fn pane_close_targets_exact_ids_and_prunes_like_an_exit() {
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
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
        assert_eq!(close(&mut workspace, "x2", "p3").unwrap(), "terminal-3");
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

        // A covering popup refuses without a caller; a non-final tab's last pane removes it.
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
        // Under the popup, a validated caller may close a named pane but not its last.
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
        for (id, caller) in [("x5b", agent.as_str()), ("x5c", "terminal-1")] {
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
        // A pane of a background tab closes by its ID, here under that tab's
        // popup by the popup's terminal, and the active pending tab, its
        // picker and the covered tab's popup stay as they were.
        request(
            &mut workspace,
            "x6-pane",
            Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
        );
        let background = workspace.snapshot().tabs[0].panes().last().unwrap().clone();
        request(
            &mut workspace,
            "t2",
            Action::Workspace(WorkspaceAction::CreateTab),
        );
        let before = workspace.snapshot();
        assert_eq!(before.active_tab, "t2");
        assert_eq!(
            workspace
                .prepare_close_pane("x6", Some(&agent), &background.id)
                .unwrap(),
            std::slice::from_ref(&background.session)
        );
        workspace
            .session_ended(&background.session, SessionEnd::Closed, ok)
            .unwrap();
        let after = workspace.snapshot();
        assert_eq!(after.active_tab, "t2");
        assert_eq!(after.tabs[1], before.tabs[1]);
        assert_eq!(after.tabs[0].selected_popup, before.tabs[0].selected_popup);
        assert_eq!(after.tabs[0].selected_pane.as_deref(), Some("p1"));
        assert_eq!(after.tabs[0].panes().count(), 1);
        let pair = workspace.snapshot().tabs[1].popups[0].clone();
        request(
            &mut workspace,
            "t2-dir",
            Action::Workspace(WorkspaceAction::SetTabDirectory {
                tab: "t2".into(),
                directory: b"/".to_vec(),
            }),
        );
        workspace
            .session_ended(&pair.session, SessionEnd::Exited, ok)
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
            ["terminal-1".to_owned(), popup.session]
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
                    id: "terminal-1".into(),
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
        let pair = workspace.snapshot().tabs[1].popups[0].clone();
        request(
            &mut workspace,
            "t2-dir",
            None,
            Action::Workspace(WorkspaceAction::SetTabDirectory {
                tab: "t2".into(),
                directory: b"/".to_vec(),
            }),
        )
        .unwrap();
        workspace
            .session_ended(&pair.session, SessionEnd::Exited, |_| Ok(()))
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
            Some("terminal-1"),
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
                Some("terminal-1"),
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
            Some("terminal-1"),
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

        // Closing resolves the pane by its ID, here in the caller's background tab.
        let [session] = &workspace
            .prepare_close_pane("x1", Some("terminal-1"), &created[1])
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
            Some("terminal-1"),
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
            Some("terminal-99"),
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
                    id: "terminal-1".into(),
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
        assert_eq!(tab.columns[0].panes[0].session, "terminal-1");

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
                    id: "terminal-1".into(),
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
                    for session in ["terminal-1", "terminal-2"] {
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
                    id: "terminal-1".into(),
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
                    id: "terminal-1".into(),
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
                    SessionOperation::SettlePair(id) => PathBuf::from(format!("settle {id}")),
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
                        SessionOperation::Start { .. } | SessionOperation::SettlePair(_) => {
                            Ok(())
                        }
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
                        SessionOperation::Stop(_) | SessionOperation::SettlePair(_) => Ok(()),
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
                    id: "terminal-1".into(),
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
                "commands-open",
                None,
                invoke("commands", None),
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
            Some(SessionOperation::Stop(id)) if id == "terminal-3"
        ));

        workspace.workspaces[0].tabs.push(Tab {
            id: "t2".into(),
            directory: "/".into(),
            pending: false,
            columns: vec![Column {
                panes: vec![Pane {
                    pinned_name: None,
                    tag: None,
                    id: "p99".into(),
                    session: Session {
                        id: "terminal-99".into(),
                        endpoint: "/runtime/terminal-99.sock".into(),
                    },
                }],
                expanded: 0,
            }],
            selected_column: Some(0),
            popups: Vec::new(),
            selected_popup: None,
            covered_popup: None,
        });
        workspace
            .dispatch(
                "commands-again",
                None,
                invoke("commands", None),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        workspace
            .session_ended(&agent.session, SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        workspace
            .session_ended("terminal-1", SessionEnd::Exited, |_| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot().tabs[0].popups[0].entry, "commands");

        let failure = workspace
            .dispatch(
                "replacement-fails",
                None,
                invoke("agent", None),
                prepared,
                |operation| match operation {
                    SessionOperation::Stop(_) | SessionOperation::SettlePair(_) => Ok(()),
                    SessionOperation::Start { .. } => Err("start failed".into()),
                },
            )
            .unwrap_err();
        assert_eq!(failure.code, "popup-start");
        assert_eq!(workspace.snapshot().tabs[0].id, "t2");
    }

    /// A transient popup that ends, after a workspace switch or in place,
    /// shows the keep-alive popup it covered again; over a pane it returns to
    /// the pane.
    #[test]
    fn an_ended_transient_popup_shows_the_popup_it_covered_again() {
        let mut set = two_workspace_set(|_| Ok(()));
        let request = |set: &mut WorkspaceSet, id: &str, action: Action| {
            set.dispatch(id, None, action, prepared, |_| Ok(()))
                .unwrap();
            set.snapshot().tabs.remove(0)
        };
        let switch = |workspace: &str| {
            Action::Workspace(WorkspaceAction::SwitchWorkspace {
                workspace: eon_workspace_protocol::v42::WorkspaceTarget::Selector(workspace.into()),
            })
        };
        // Opens the picker over the shown terminal and returns its terminal.
        let picker = |set: &mut WorkspaceSet, id: &str| {
            let tab = request(set, id, invoke("t1", "commands"));
            let picker = tab.popups.last().unwrap();
            assert_eq!(picker.entry, "commands");
            assert_eq!(tab.selected_popup.as_ref(), Some(&picker.id));
            picker.session.clone()
        };
        let ended = |set: &mut WorkspaceSet, session: &str| {
            set.session_ended(session, SessionEnd::Exited, |_| Ok(()))
                .unwrap();
            set.snapshot().tabs.remove(0)
        };
        let pane = set.snapshot().tabs[0].selected_pane.clone();
        let agent = request(&mut set, "agent", invoke("t1", "agent")).popups[0]
            .id
            .clone();

        // The picker switches away and exits; switching back shows Agent.
        let session = picker(&mut set, "switch");
        request(&mut set, "away", switch("s2"));
        ended(&mut set, &session);
        let tab = request(&mut set, "back", switch("s1"));
        assert_eq!(tab.selected_popup.as_ref(), Some(&agent));

        // Esc exits the picker in place.
        let session = picker(&mut set, "esc");
        let tab = ended(&mut set, &session);
        assert_eq!(tab.selected_popup.as_ref(), Some(&agent));

        // A pane action whose picker stop fails changes nothing, so the
        // picker's later exit still shows Agent.
        let session = picker(&mut set, "focus-fails");
        let focus = Action::Workspace(WorkspaceAction::FocusId(pane.clone().unwrap()));
        let failed = set
            .dispatch(
                "focus",
                None,
                focus,
                prepared,
                |operation| match operation {
                    SessionOperation::Stop(_) => Err("stop failed".into()),
                    SessionOperation::Start { .. } | SessionOperation::SettlePair(_) => Ok(()),
                },
            )
            .unwrap_err();
        assert_eq!(failed.code, "popup-stop");
        let tab = ended(&mut set, &session);
        assert_eq!(tab.selected_popup.as_ref(), Some(&agent));

        // `eon popup hide` on the picker shows Agent again (EON-C10).
        picker(&mut set, "hide-picker");
        let shown = set.snapshot().tabs[0].selected_popup.clone().unwrap();
        let dismiss = Action::DismissPopup(PopupTarget {
            tab: "t1".into(),
            instance: shown,
        });
        let tab = request(&mut set, "dismiss", dismiss);
        assert_eq!(tab.selected_popup.as_ref(), Some(&agent));

        // Over a pane, the picker returns to the pane.
        let hide = Action::InvokePopup {
            tab: "t1".into(),
            entry: "agent".into(),
            expected_instance: Some(agent.clone()),
            intent: InvokeIntent::Toggle,
        };
        let tab = request(&mut set, "hide", hide.clone());
        assert_eq!(tab.selected_popup, None);
        let session = picker(&mut set, "over-pane");
        let tab = ended(&mut set, &session);
        assert_eq!(tab.selected_popup, None);
        assert_eq!(tab.selected_pane, pane);
        assert_eq!(tab.popups.len(), 1);

        // A pane action over the picker shows the pane, not Agent.
        let tab = request(&mut set, "show", hide);
        assert_eq!(tab.selected_popup.as_ref(), Some(&agent));
        picker(&mut set, "focus");
        let focus = Action::Workspace(WorkspaceAction::FocusId(pane.clone().unwrap()));
        let tab = request(&mut set, "focus-pane", focus);
        assert_eq!(tab.selected_popup, None);
        assert_eq!(tab.selected_pane, pane);
        assert_eq!(tab.popups.len(), 1);
    }

    /// A pick on a pending tab (EON-C18) commits it atomically: a refused
    /// directory or a failed first pane changes nothing, and a success starts
    /// exactly one first pane at the directory, hides the pair, keeps it as
    /// the tab's Files and settles its pending marker.
    #[test]
    fn a_pick_commits_a_pending_tab_with_one_first_pane_and_keeps_its_pair() {
        let root = std::env::temp_dir().join(format!("eon-pick-{}", std::process::id()));
        let selected = root.join("selected");
        fs::create_dir_all(&selected).unwrap();
        let mut launches = Vec::new();
        let mut workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            catalog(),
            prepared,
            |operation| {
                launches.push(operation);
                Ok(())
            },
        )
        .unwrap();
        // The pair takes terminal-2: terminal-1 waits for the first pane.
        assert_eq!(
            launches,
            [SessionOperation::Start {
                session: Session {
                    id: "terminal-2".into(),
                    endpoint: root.join("terminal-2.sock"),
                },
                directory: root.clone(),
                command: LaunchCommand::Files {
                    argv: vec!["files".into()],
                    quick: true,
                    pending: true,
                },
                tab: Some("t1".into()),
            }]
        );
        let pending = workspace.snapshot();
        let pair = pending.tabs[0].popups[0].clone();
        assert_eq!(pair.entry, FILES_ENTRY);
        assert_eq!(pending.tabs[0].selected_popup.as_ref(), Some(&pair.id));
        let pick = |directory: &Path| {
            Action::Workspace(WorkspaceAction::SetTabDirectory {
                tab: "t1".into(),
                directory: directory.as_os_str().as_bytes().to_vec(),
            })
        };
        for (id, directory) in [
            ("relative", Path::new("selected")),
            ("missing", &root.join("x")),
        ] {
            assert_eq!(
                workspace
                    .dispatch(id, None, pick(directory), prepared, |_| {
                        panic!("a refused pick started a terminal")
                    })
                    .unwrap_err()
                    .code,
                "invalid-directory"
            );
            assert_eq!(workspace.snapshot(), pending);
        }
        assert_eq!(
            workspace
                .dispatch("pick-fails", None, pick(&selected), prepared, |_| {
                    Err("start failed".into())
                })
                .unwrap_err()
                .code,
            "terminal-start"
        );
        assert_eq!(workspace.snapshot(), pending);
        let mut operations = Vec::new();
        workspace
            .dispatch("pick", None, pick(&selected), prepared, |operation| {
                operations.push(operation);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            operations,
            [
                SessionOperation::Start {
                    session: Session {
                        id: "terminal-1".into(),
                        endpoint: root.join("orbit.sock"),
                    },
                    directory: selected.clone(),
                    command: LaunchCommand::Pane,
                    tab: None,
                },
                SessionOperation::SettlePair("terminal-2".into()),
            ]
        );
        let tab = &workspace.snapshot().tabs[0];
        assert!(!tab.pending);
        assert_eq!(tab.directory, selected.as_os_str().as_bytes());
        assert_eq!(tab.panes().count(), 1);
        assert_eq!(tab.popups, std::slice::from_ref(&pair));
        assert_eq!(tab.selected_popup, None);

        // A second, racing pick finds an established tab and only retargets it.
        workspace
            .dispatch("pick-again", None, pick(&root), prepared, |_| {
                panic!("a pick on an established tab started a terminal")
            })
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert_eq!(tab.panes().count(), 1);
        assert_eq!(tab.directory, root.as_os_str().as_bytes());
        fs::remove_dir_all(root).unwrap();
    }

    /// Closing a pending tab's pair is dismissal (EON-C18): the pair stays
    /// alive as the tab's hidden Files and the first pane starts at the tab
    /// directory; a pair that exits instead is dismissed too.
    #[test]
    fn dismissing_or_exiting_a_pending_pair_starts_the_first_pane() {
        for exited in [false, true] {
            let mut workspace = WorkspaceSet::pending(
                "/runtime".into(),
                "/start".into(),
                Some("/".into()),
                catalog(),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
            let pair = workspace.snapshot().tabs[0].popups[0].clone();
            let mut operations = Vec::new();
            let operate = |operation| {
                operations.push(operation);
                Ok(())
            };
            if exited {
                workspace
                    .session_ended(&pair.session, SessionEnd::Exited, operate)
                    .unwrap();
            } else {
                workspace
                    .dispatch(
                        "hide",
                        None,
                        Action::DismissPopup(PopupTarget {
                            tab: "t1".into(),
                            instance: pair.id.clone(),
                        }),
                        prepared,
                        operate,
                    )
                    .unwrap();
            }
            let start = SessionOperation::Start {
                session: Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
                directory: "/start".into(),
                command: LaunchCommand::Pane,
                tab: None,
            };
            let tab = &workspace.snapshot().tabs[0];
            assert!(!tab.pending);
            assert_eq!(tab.directory, b"/start");
            assert_eq!(tab.panes().count(), 1);
            assert_eq!(tab.selected_popup, None);
            if exited {
                assert_eq!(operations, [start]);
                assert!(tab.popups.is_empty());
            } else {
                assert_eq!(
                    operations,
                    [start, SessionOperation::SettlePair(pair.session.clone())]
                );
                assert_eq!(tab.popups, [pair]);
            }
        }
    }

    /// Detachment dismisses every pending pair in every workspace and keeps
    /// each alive (EON-C18).
    #[test]
    fn detachment_dismisses_every_pending_pair_and_keeps_it() {
        let mut workspace = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        for (id, action) in [
            ("t2", WorkspaceAction::CreateTab),
            (
                "s2",
                WorkspaceAction::NewWorkspace {
                    name: None,
                    focus: false,
                },
            ),
        ] {
            workspace
                .dispatch(id, None, Action::Workspace(action), prepared, |_| Ok(()))
                .unwrap();
        }
        let mut operations = Vec::new();
        workspace
            .dismiss_pending_pairs(|operation| {
                operations.push(operation);
                Ok(())
            })
            .unwrap();
        let starts = operations
            .iter()
            .filter(|operation| matches!(operation, SessionOperation::Start { .. }))
            .count();
        let settled = operations
            .iter()
            .filter_map(|operation| match operation {
                SessionOperation::SettlePair(id) => Some(id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(starts, 3);
        assert_eq!(settled, ["terminal-2", "terminal-3", "terminal-4"]);
        assert!(
            !operations
                .iter()
                .any(|operation| matches!(operation, SessionOperation::Stop(_)))
        );
        for snapshot in workspace.snapshots() {
            for tab in &snapshot.tabs {
                assert!(!tab.pending);
                assert_eq!(tab.panes().count(), 1);
                assert_eq!(tab.popups.len(), 1);
                assert_eq!(tab.selected_popup, None);
            }
        }
    }

    /// Alt+Z and Alt+Shift+Y both reach a pending tab's one pair, other
    /// entries stay refused there, and Alt+Z on a tab without a pair opens
    /// Files on quick search (EON-C18).
    #[test]
    fn project_and_files_invoke_the_tabs_one_pair() {
        let mut workspace = WorkspaceSet::pending(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            catalog(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let pair = workspace.snapshot().tabs[0].popups[0].clone();
        assert_eq!(
            workspace.pair(None, "t1", "project"),
            Some((pair.session.clone(), true))
        );
        assert_eq!(
            workspace.pair(None, "t1", "files"),
            Some((pair.session.clone(), true))
        );
        assert_eq!(workspace.pair(None, "t1", "agent"), None);
        assert_eq!(workspace.pair(None, "t9", "files"), None);
        let invoke = |entry: &str, expected: Option<&str>, intent| Action::InvokePopup {
            tab: "t1".into(),
            entry: entry.into(),
            expected_instance: expected.map(Into::into),
            intent,
        };
        let refused = workspace
            .dispatch(
                "agent",
                None,
                invoke("agent", None, InvokeIntent::Toggle),
                prepared,
                |_| panic!("a pending tab opened a tool"),
            )
            .unwrap_err();
        assert_eq!(refused.code, "choose-directory-first");
        // Project's observed instance is the pair.
        assert_eq!(
            workspace
                .dispatch(
                    "stale",
                    None,
                    invoke("project", None, InvokeIntent::Focus),
                    prepared,
                    |_| Ok(()),
                )
                .unwrap_err()
                .code,
            "stale-popup"
        );
        let before = workspace.snapshot();
        for (id, entry) in [("focus-z", "project"), ("focus-y", "files")] {
            workspace
                .dispatch(
                    id,
                    None,
                    invoke(entry, Some(&pair.id), InvokeIntent::Focus),
                    prepared,
                    |_| panic!("focusing the pair started or stopped a terminal"),
                )
                .unwrap();
            assert_eq!(workspace.snapshot(), before);
        }
        // Toggling the shown pair closes it: dismissal.
        workspace
            .dispatch(
                "toggle-z",
                None,
                invoke("project", Some(&pair.id), InvokeIntent::Toggle),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let tab = &workspace.snapshot().tabs[0];
        assert!(!tab.pending);
        assert_eq!(tab.selected_popup, None);
        assert_eq!(
            workspace.pair(None, "t1", "project"),
            Some((pair.session, false))
        );

        // On an established tab without a pair, Alt+Z opens Files on quick search.
        let mut workspace = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        for (entry, quick) in [("project", true), ("files", false)] {
            let mut launches = Vec::new();
            workspace
                .dispatch(
                    entry,
                    None,
                    invoke(entry, None, InvokeIntent::Toggle),
                    prepared,
                    |operation| {
                        launches.push(operation);
                        Ok(())
                    },
                )
                .unwrap();
            assert!(
                matches!(
                    &launches[..],
                    [SessionOperation::Start {
                        command: LaunchCommand::Files { quick: started, pending: false, .. },
                        ..
                    }] if *started == quick
                ),
                "{entry}: {launches:?}"
            );
            let tab = &workspace.snapshot().tabs[0];
            assert_eq!(tab.popups.len(), 1);
            assert_eq!(tab.popups[0].entry, FILES_ENTRY);
            let session = tab.popups[0].session.clone();
            workspace
                .session_ended(&session, SessionEnd::Exited, |_| Ok(()))
                .unwrap();
        }
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
                    id: "terminal-1".into(),
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
                    id: "terminal-1".into(),
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
        let edit = || b"/src/\xff.rs".to_vec();

        // An unavailable editor changes nothing.
        let before = workspace.snapshot();
        let failure = workspace
            .edit(
                "missing",
                Some(&files.session),
                edit(),
                |_, _| Err("no editor".into()),
                |_, _| panic!("no reuse"),
                |_| Err("nothing starts".into()),
            )
            .unwrap_err();
        assert_eq!(failure.code, "edit-command");
        assert_eq!(workspace.snapshot(), before);

        let mut started = Vec::new();
        workspace
            .edit(
                "edit",
                Some(&files.session),
                edit(),
                |_, _| {
                    Ok(super::super::editor::EditCommand {
                        command: PreparedCommand {
                            argv: vec!["editor".into()],
                            listener: None,
                        },
                        popup: false,
                    })
                },
                |_, _| panic!("pane route"),
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
            &[
                "editor".into(),
                OsString::from_vec(b"/src/\xff.rs".to_vec())
            ]
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
                    id: "terminal-1".into(),
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
                Some("terminal-1"),
                command(),
                |_, _| Err("no claude".into()),
                |_| Err("nothing starts".into()),
            )
            .unwrap_err();
        assert_eq!(failure.code, "pane-command");
        assert_eq!(workspace.snapshot(), before);

        let mut started = Vec::new();
        workspace
            .dispatch(
                "run",
                Some("terminal-1"),
                command(),
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
    fn a_pick_parks_a_hidden_agent_as_the_first_pane_of_a_paneless_tab() {
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
                    id: "terminal-1".into(),
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
            .session_ended("terminal-1", SessionEnd::Exited, |_| Ok(()))
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
            .find(|popup| popup.entry == FILES_ENTRY)
            .unwrap()
            .clone();
        workspace
            .dispatch(
                "commit",
                None,
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: next.as_os_str().as_bytes().to_vec(),
                }),
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
            matches!(&launches[..], [SessionOperation::Start { directory, command: LaunchCommand::Files { quick: true, pending: true, .. }, .. }] if directory == &root)
        );

        let pair = workspace.snapshot().tabs[1].popups[0].session.clone();
        let error = workspace
            .session_ended(&pair, SessionEnd::Exited, |_| Err("start failed".into()))
            .unwrap_err();
        assert_eq!(error.code, "terminal-start");
        assert_eq!(workspace.snapshot(), initial);
        eon_workspace_protocol::v42::encode_response(
            &eon_workspace_protocol::v42::Response::Snapshot(workspace.snapshot()),
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
    fn a_keyless_project_still_leaves_a_pending_tab_its_pair() {
        let overridden = PopupKey::Overridden {
            default: Shortcut {
                modifiers: ALT,
                key: "KeyA".into(),
            },
            by: "agent".into(),
        };
        for (key, json_key) in [
            (
                PopupKey::Unbound,
                r#""keybinding":null,"overridden_by":null,"default_keybinding":null}"#,
            ),
            (
                overridden,
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
            assert_eq!(state.tabs[0].popups[0].entry, FILES_ENTRY);
            assert_eq!(state.entries[0].shortcut, key);
            assert!(human(&state).contains("entry=files label=Files key=Alt+Y "));
            assert!(
                json(&state).contains(&format!(r#"{{"id":"project","label":"Project",{json_key}"#))
            );
            assert!(json(&state).contains(
                r#"{"id":"agent","label":"Agent","keybinding":"Alt+A","overridden_by":null,"default_keybinding":null}"#
            ));
            eon_workspace_protocol::v42::encode_response(
                &eon_workspace_protocol::v42::Response::Snapshot(state),
            )
            .unwrap();
        }
    }

    #[test]
    fn pending_tabs_keep_independent_pairs_and_close_only_the_selected_tab() {
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
                    Action::Workspace(WorkspaceAction::SetTabDirectory {
                        tab: "t1".into(),
                        directory: b"/".to_vec(),
                    }),
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
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/".to_vec(),
                }),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert!(!workspace.snapshot().tabs[0].pending);
        assert_eq!(workspace.snapshot().tabs[0].panes().count(), 1);
    }

    #[test]
    fn background_pending_pair_exit_keeps_the_selected_tab() {
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
        let second_pair = workspace.snapshot().tabs[1].popups[0].session.clone();
        workspace
            .session_ended(&second_pair, SessionEnd::Exited, |_| Ok(()))
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

    #[test]
    fn background_tab_close_preserves_foreground_state() {
        let setup = || {
            let mut set = WorkspaceSet::with_recovered_sessions(
                "/runtime".into(),
                "/".into(),
                Some("/".into()),
                vec![(
                    1,
                    Session {
                        id: "terminal-1".into(),
                        endpoint: "/runtime/orbit.sock".into(),
                    },
                )],
                catalog(),
            )
            .unwrap();
            let apply = |set: &mut WorkspaceSet, id: &str, caller: Option<&str>, action| {
                set.dispatch(id, caller, action, prepared, |_| Ok(()))
                    .unwrap();
            };
            apply(
                &mut set,
                "t1-p2",
                Some("terminal-1"),
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
            );
            apply(&mut set, "t1-git", Some("terminal-1"), invoke("t1", "git"));
            apply(
                &mut set,
                "t2",
                Some("terminal-1"),
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: Vec::new(),
                    focus: true,
                },
            );
            apply(&mut set, "t2-col", None, Action::CreateColumn);
            apply(&mut set, "t2-agent", None, invoke("t2", "agent"));
            apply(
                &mut set,
                "t3",
                Some("terminal-1"),
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: Vec::new(),
                    focus: true,
                },
            );
            apply(&mut set, "t3-git", None, invoke("t3", "git"));
            apply(
                &mut set,
                "focus-t2",
                None,
                Action::Workspace(WorkspaceAction::FocusId("t2".into())),
            );
            set
        };

        for (target, surviving) in [("t1", ["t2", "t3"]), ("t3", ["t1", "t2"])] {
            let mut set = setup();
            let before = set.snapshot();
            assert_eq!(before.active_tab, "t2");
            let active_before = before
                .tabs
                .iter()
                .find(|tab| tab.id == "t2")
                .unwrap()
                .clone();
            assert!(active_before.selected_popup.is_some());
            let target_tab = before.tabs.iter().find(|tab| tab.id == target).unwrap();
            let expected_sessions = target_tab
                .panes()
                .map(|pane| pane.session.clone())
                .chain(target_tab.popups.iter().map(|popup| popup.session.clone()))
                .collect::<Vec<_>>();
            assert!(expected_sessions.len() >= 2);
            let sessions = set
                .prepare_close_tab(&format!("close-{target}"), None, target)
                .unwrap();
            assert_eq!(sessions, expected_sessions);
            assert_eq!(set.snapshot(), before);
            for session in &sessions {
                set.session_ended(session, SessionEnd::Closed, |operation| {
                    assert!(
                        !matches!(operation, SessionOperation::Start { .. }),
                        "closing a background tab must not start a terminal: {operation:?}"
                    );
                    Ok(())
                })
                .unwrap();
            }
            let after = set.snapshot();
            assert_eq!(after.active_tab, "t2");
            assert_eq!(
                after
                    .tabs
                    .iter()
                    .map(|tab| tab.id.as_str())
                    .collect::<Vec<_>>(),
                surviving
            );
            assert_eq!(
                after.tabs.iter().find(|tab| tab.id == "t2").unwrap(),
                &active_before
            );
        }

        let mut set = setup();
        set.dispatch(
            "t4-pending",
            None,
            Action::Workspace(WorkspaceAction::CreateTab),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        set.dispatch(
            "refocus-t2",
            None,
            Action::Workspace(WorkspaceAction::FocusId("t2".into())),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        let before = set.snapshot();
        assert_eq!(before.active_tab, "t2");
        let active_before = before
            .tabs
            .iter()
            .find(|tab| tab.id == "t2")
            .unwrap()
            .clone();
        let pending_tab = before.tabs.iter().find(|tab| tab.id == "t4").unwrap();
        assert!(pending_tab.pending);
        let pair = pending_tab.popups[0].session.clone();
        let sessions = set.prepare_close_tab("close-t4", None, "t4").unwrap();
        assert_eq!(sessions, vec![pair.clone()]);
        assert_eq!(set.snapshot(), before);
        set.session_ended(&pair, SessionEnd::Closed, |operation| {
            assert!(
                !matches!(operation, SessionOperation::Start { .. }),
                "closing a pending background tab must not start a replacement pane: {operation:?}"
            );
            Ok(())
        })
        .unwrap();
        let after = set.snapshot();
        assert_eq!(after.active_tab, "t2");
        assert!(!after.tabs.iter().any(|tab| tab.id == "t4"));
        assert_eq!(
            after.tabs.iter().find(|tab| tab.id == "t2").unwrap(),
            &active_before
        );
    }

    #[test]
    fn pane_close_resolves_any_tab_without_selecting_it() {
        let setup = || {
            let mut set = WorkspaceSet::with_recovered_sessions(
                "/runtime".into(),
                "/".into(),
                Some("/".into()),
                vec![(
                    1,
                    Session {
                        id: "terminal-1".into(),
                        endpoint: "/runtime/orbit.sock".into(),
                    },
                )],
                catalog(),
            )
            .unwrap();
            let caller = "terminal-1".to_string();
            let apply = |set: &mut WorkspaceSet, id: &str, action_caller: Option<&str>, action| {
                set.dispatch(id, action_caller, action, prepared, |_| Ok(()))
                    .unwrap();
            };
            apply(
                &mut set,
                "t2",
                Some(&caller),
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: Vec::new(),
                    focus: true,
                },
            );
            apply(&mut set, "t2-col", None, Action::CreateColumn);
            apply(&mut set, "t2-agent", None, invoke("t2", "agent"));
            apply(
                &mut set,
                "t3",
                Some(&caller),
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: Vec::new(),
                    focus: true,
                },
            );
            let t3_first_session = set
                .snapshot()
                .tabs
                .iter()
                .find(|tab| tab.id == "t3")
                .unwrap()
                .panes()
                .next()
                .unwrap()
                .session
                .clone();
            apply(&mut set, "t3-col2", None, Action::CreateColumn);
            apply(
                &mut set,
                "t3-p2",
                Some(&t3_first_session),
                Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
            );
            apply(
                &mut set,
                "focus-t2",
                None,
                Action::Workspace(WorkspaceAction::FocusId("t2".into())),
            );
            set.insert_pending(
                "/other".into(),
                Some("/other".into()),
                catalog(),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
            let pair = set.workspaces[1].tabs[0].popups[0].session.id.clone();
            set.shown = 1;
            set.session_ended(&pair, SessionEnd::Exited, |_| Ok(()))
                .unwrap();
            set.shown = 0;
            set.recent_workspaces = VecDeque::from(["s1".into(), "s2".into()]);
            (set, caller)
        };

        for (close_expanded, with_caller) in [(false, true), (true, true), (true, false)] {
            let (mut set, caller) = setup();
            let before = set.snapshot();
            let recency_before = set.recent_workspaces.clone();
            assert_eq!(before.workspace, "s1");
            assert_eq!(before.active_workspace, "s1");
            assert_eq!(before.active_tab, "t2");
            let t3 = before.tabs.iter().find(|tab| tab.id == "t3").unwrap();
            assert_eq!(t3.columns.len(), 2);
            assert_eq!(t3.columns[0].panes.len(), 2);
            assert_eq!(t3.columns[1].panes.len(), 1);
            let collapsed = &t3.columns[0].panes[0];
            let expanded = &t3.columns[0].panes[1];
            assert_eq!(t3.columns[0].expanded, expanded.id);
            let (target, survivor) = if close_expanded {
                (expanded.clone(), collapsed.clone())
            } else {
                (collapsed.clone(), expanded.clone())
            };
            let request_caller = with_caller.then_some(caller.as_str());
            let sessions = set
                .prepare_close_pane("close-pane", request_caller, &target.id)
                .unwrap();
            assert_eq!(sessions, vec![target.session.clone()]);
            assert_eq!(set.snapshot(), before);
            set.session_ended(&target.session, SessionEnd::Closed, |operation| {
                assert!(!matches!(operation, SessionOperation::Start { .. }));
                Ok(())
            })
            .unwrap();
            let after = set.snapshot();
            assert_eq!(after.workspace, "s1");
            assert_eq!(after.active_workspace, "s1");
            assert_eq!(set.recent_workspaces, recency_before);
            assert_eq!(after.active_tab, "t2");
            assert_eq!(after.tabs[0], before.tabs[0]);
            assert_eq!(after.tabs[1], before.tabs[1]);
            let after_t3 = after.tabs.iter().find(|tab| tab.id == "t3").unwrap();
            assert_eq!(after_t3.columns.len(), 2);
            assert_eq!(after_t3.columns[0].panes.len(), 1);
            assert_eq!(after_t3.columns[0].panes[0].id, survivor.id);
            assert_eq!(after_t3.columns[0].expanded, survivor.id);
            assert_eq!(
                after_t3.selected_pane.as_deref(),
                Some(survivor.id.as_str())
            );
        }
    }

    #[test]
    fn background_close_retains_popup_and_last_pane_guards() {
        let mut set = WorkspaceSet::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            Some("/".into()),
            vec![(
                1,
                Session {
                    id: "terminal-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
            catalog(),
        )
        .unwrap();
        let apply = |set: &mut WorkspaceSet, id: &str, caller: Option<&str>, action| {
            set.dispatch(id, caller, action, prepared, |_| Ok(()))
                .unwrap();
        };
        let new_pane = |set: &mut WorkspaceSet, id: &str| {
            apply(
                set,
                id,
                Some("terminal-1"),
                Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
            );
            set.snapshot().tabs[0].panes().last().unwrap().clone()
        };
        // t1 shows Git over p1, and the active t2 shows Agent over its pane.
        apply(&mut set, "t1-git", None, invoke("t1", "git"));
        apply(
            &mut set,
            "t2",
            None,
            Action::LaunchTab {
                directory: b"/".to_vec(),
                argv: Vec::new(),
                focus: true,
            },
        );
        apply(&mut set, "t2-agent", None, invoke("t2", "agent"));
        let snapshot = set.snapshot();
        assert_eq!(snapshot.active_tab, "t2");
        let pane_caller = snapshot.tabs[1].panes().next().unwrap().session.clone();
        let popup_caller = snapshot.tabs[1].popups[0].session.clone();
        let git = snapshot.tabs[0].selected_popup.clone().unwrap();

        // Under t1's popup, a validated pane or popup terminal of another tab
        // may close a pane that is not the last; Alt+W, with no caller, may not.
        let before = set.snapshot();
        for (id, caller) in [
            ("pane-caller", &pane_caller),
            ("popup-caller", &popup_caller),
        ] {
            let pane = new_pane(&mut set, &format!("{id}-pane"));
            let covered = set.snapshot();
            let refused = set
                .prepare_close_pane(&format!("{id}-alt-w"), None, &pane.id)
                .unwrap_err();
            assert_eq!(
                (refused.code.as_str(), refused.detail.as_str()),
                ("unavailable", "close the popup before closing a pane")
            );
            assert_eq!(set.snapshot(), covered);
            assert_eq!(
                set.prepare_close_pane(&format!("{id}-close"), Some(caller), &pane.id)
                    .unwrap(),
                std::slice::from_ref(&pane.session)
            );
            set.session_ended(&pane.session, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
            // The active tab, both popups and every selection are as they were.
            assert_eq!(set.snapshot(), before);
            assert_eq!(set.snapshot().tabs[0].selected_popup.as_ref(), Some(&git));
        }

        // Even a validated caller may not close the covered tab's last pane.
        for caller in [&pane_caller, &popup_caller] {
            let refused = set
                .prepare_close_pane("last-covered", Some(caller), "p1")
                .unwrap_err();
            assert_eq!(
                (refused.code.as_str(), refused.detail.as_str()),
                (
                    "unavailable",
                    "close the popup before closing the tab's last pane"
                )
            );
            assert_eq!(set.snapshot(), before);
        }

        // With Git hidden, t1's last pane closes and t1 stays with only its
        // keep-alive popup, while t2 stays active and unchanged. Only the
        // active tab's popup toggles, so t1 is shown for that and left again.
        let focus = |id: &str| Action::Workspace(WorkspaceAction::FocusId(id.into()));
        apply(&mut set, "show-t1", None, focus("t1"));
        apply(
            &mut set,
            "hide-git",
            None,
            Action::InvokePopup {
                tab: "t1".into(),
                entry: "git".into(),
                expected_instance: Some(git.clone()),
                intent: InvokeIntent::Toggle,
            },
        );
        apply(&mut set, "show-t2", None, focus("t2"));
        assert_eq!(set.snapshot().tabs[0].selected_popup, None);
        assert_eq!(set.snapshot().tabs[1], before.tabs[1]);
        assert_eq!(
            set.prepare_close_pane("t1-last", None, "p1").unwrap(),
            ["terminal-1".to_owned()]
        );
        set.session_ended("terminal-1", SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        let snapshot = set.snapshot();
        assert_eq!(snapshot.active_tab, "t2");
        assert_eq!(snapshot.tabs[1], before.tabs[1]);
        assert_eq!(snapshot.tabs[0].panes().count(), 0);
        assert_eq!(snapshot.tabs[0].popups, before.tabs[0].popups);

        // Once t2 is the sole tab, its last pane is a final close, which stops
        // its hidden popup with it; preparing it changes nothing.
        let popup_only = set.prepare_close_tab("close-t1", None, "t1").unwrap();
        assert_eq!(popup_only, [before.tabs[0].popups[0].session.clone()]);
        set.session_ended(&popup_only[0], SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        let agent = set.snapshot().tabs[0].popups[0].id.clone();
        apply(
            &mut set,
            "hide-agent",
            None,
            Action::InvokePopup {
                tab: "t2".into(),
                entry: "agent".into(),
                expected_instance: Some(agent),
                intent: InvokeIntent::Toggle,
            },
        );
        let sole = set.snapshot();
        let t2_pane = sole.tabs[0].panes().next().unwrap().clone();
        assert_eq!(
            set.prepare_close_pane("close-sole-pane", None, &t2_pane.id)
                .unwrap(),
            [t2_pane.session, popup_caller]
        );
        assert_eq!(set.snapshot(), sole);
    }

    #[test]
    fn tab_switch_selects_a_callers_tab_as_a_click_does_without_showing_its_session() {
        let mut set = two_workspace_set(|_| Ok(()));
        let apply = |set: &mut WorkspaceSet, id: &str, caller: Option<&str>, action| {
            set.dispatch(id, caller, action, prepared, |_| Ok(()))
        };
        let switch = |tab: &str| Action::Workspace(WorkspaceAction::FocusId(tab.into()));
        let callers = [0, 1].map(|index| {
            set.workspaces[index].tabs[0].columns[0].panes[0]
                .session
                .id
                .clone()
        });
        // Each session gets a second tab showing its Agent, then its first
        // tab is active again, with the caller's pane in it.
        for (index, caller) in callers.iter().enumerate() {
            let caller = Some(caller.as_str());
            let launch = Action::LaunchTab {
                directory: b"/".to_vec(),
                argv: Vec::new(),
                focus: true,
            };
            apply(&mut set, &format!("launch-{index}"), caller, launch).unwrap();
            let [first, second] = [0, 1].map(|tab| set.workspaces[index].tabs[tab].id.clone());
            apply(
                &mut set,
                &format!("agent-{index}"),
                caller,
                invoke(&second, "agent"),
            )
            .unwrap();
            apply(&mut set, &format!("back-{index}"), caller, switch(&first)).unwrap();
        }

        for (index, caller) in callers.iter().enumerate() {
            let before = set.clone();
            let second = before.workspaces[index].tabs[1].id.clone();
            let request = format!("switch-{index}");
            apply(&mut set, &request, Some(caller), switch(&second)).unwrap();
            // Only the caller's session's active tab moves: the Agent the
            // tab showed stays shown and the shown session stays shown.
            let mut expected = before.workspaces.clone();
            expected[index].active = 1;
            assert_eq!(set.workspaces, expected);
            assert_eq!(set.shown, 0);
            assert!(set.workspaces[index].tabs[1].selected_popup.is_some());
            if index == 0 {
                // A click on the shown session's tab has the same result.
                let mut clicked = before.clone();
                apply(&mut clicked, &request, None, switch(&second)).unwrap();
                assert_eq!(clicked.workspaces, set.workspaces);
            }
        }

        // The active tab is a harmless no-op; another session's tab or a
        // dead one fails as unknown-tab and changes nothing.
        let before = set.clone();
        let active = set.workspaces[0].tabs[1].id.clone();
        apply(&mut set, "again", Some(&callers[0]), switch(&active)).unwrap();
        assert_eq!(set.workspaces, before.workspaces);
        let foreign = set.workspaces[1].tabs[0].id.clone();
        for (id, tab) in [("foreign", foreign.as_str()), ("dead", "t99")] {
            let failure = apply(&mut set, id, Some(&callers[0]), switch(tab)).unwrap_err();
            assert_eq!(failure.code, "unknown-tab", "{tab}");
            assert_eq!(set.workspaces, before.workspaces);
            assert_eq!(set.shown, 0);
        }
    }

    #[test]
    fn hidden_callers_close_only_their_workspace_without_showing_it() {
        let mut set = two_workspace_set(|_| Ok(()));
        let s1_caller = set.workspaces[0].tabs[0].columns[0].panes[0]
            .session
            .id
            .clone();
        let s2_caller = set.workspaces[1].tabs[0].columns[0].panes[0]
            .session
            .id
            .clone();
        let apply = |set: &mut WorkspaceSet, id: &str, caller: Option<&str>, action| {
            set.dispatch(id, caller, action, prepared, |_| Ok(()))
                .unwrap();
        };
        // Give s2 a second tab (background) and a second pane on its first tab.
        apply(
            &mut set,
            "s2-p2",
            Some(&s2_caller),
            Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
        );
        apply(
            &mut set,
            "s2-bg-tab",
            Some(&s2_caller),
            Action::LaunchTab {
                directory: b"/".to_vec(),
                argv: Vec::new(),
                focus: false,
            },
        );
        // s2 keeps a hidden Agent; the shown s1 shows its own Agent.
        let s2_tab0 = set.workspaces[1].tabs[0].id.clone();
        apply(
            &mut set,
            "s2-agent",
            Some(&s2_caller),
            invoke(&s2_tab0, "agent"),
        );
        let s2_agent = set.workspaces[1].tabs[0].popups[0].clone();
        apply(
            &mut set,
            "s2-agent-hide",
            Some(&s2_caller),
            Action::InvokePopup {
                tab: s2_tab0.clone(),
                entry: "agent".into(),
                expected_instance: Some(s2_agent.id.clone()),
                intent: InvokeIntent::Toggle,
            },
        );
        set.shown = 0;
        let s1_tab0 = set.workspaces[0].tabs[0].id.clone();
        apply(&mut set, "s1-agent", None, invoke(&s1_tab0, "agent"));
        assert!(set.snapshot().tabs[0].selected_popup.is_some());
        let shown_before = set.workspaces[0].clone();
        let recency_before = set.recent_workspaces.clone();
        let s1_tab = set.workspaces[0].tabs[0].id.clone();
        let s1_pane = set.workspaces[0].tabs[0].columns[0].panes[0].id.clone();
        let s2_bg_tab = set.workspaces[1].tabs[1].id.clone();
        let s2_second_pane = set.workspaces[1].tabs[0].columns[0].panes[1].id.clone();
        let s2_second_pane_session = set.workspaces[1].tabs[0].columns[0].panes[1]
            .session
            .id
            .clone();

        // A caller in s1 (or None) attempting to close a tab or pane in s2 fails with unknown-tab / unknown-pane.
        let before = set.clone();
        for caller in [Some(s1_caller.as_str()), None] {
            assert_eq!(
                set.prepare_close_tab("s1-to-s2-tab", caller, &s2_bg_tab)
                    .unwrap_err()
                    .code,
                "unknown-tab"
            );
            assert_eq!(
                set.prepare_close_pane("s1-to-s2-pane", caller, &s2_second_pane)
                    .unwrap_err()
                    .code,
                "unknown-pane"
            );
            assert_eq!(set, before);
        }

        // A caller in s2 attempting to close a tab or pane in s1 fails with unknown-tab / unknown-pane.
        assert_eq!(
            set.prepare_close_tab("s2-to-s1-tab", Some(&s2_caller), &s1_tab)
                .unwrap_err()
                .code,
            "unknown-tab"
        );
        assert_eq!(
            set.prepare_close_pane("s2-to-s1-pane", Some(&s2_caller), &s1_pane)
                .unwrap_err()
                .code,
            "unknown-pane"
        );
        assert_eq!(set, before);

        // A caller in s2 closes a pane and a background tab in s2 without showing s2 or changing s1 or recency.
        let pane_sessions = set
            .prepare_close_pane("s2-close-pane", Some(&s2_caller), &s2_second_pane)
            .unwrap();
        assert_eq!(pane_sessions, vec![s2_second_pane_session.clone()]);
        set.session_ended(&s2_second_pane_session, SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(set.shown, 0);
        assert_eq!(set.snapshot().active_workspace, "s1");
        assert_eq!(set.snapshot().active_tab, s1_tab);
        assert_eq!(set.recent_workspaces, recency_before);
        assert_eq!(set.workspaces[1].tabs[0].panes().count(), 1);

        let tab_sessions = set
            .prepare_close_tab("s2-close-tab", Some(&s2_caller), &s2_bg_tab)
            .unwrap();
        assert_eq!(tab_sessions.len(), 1);
        set.session_ended(&tab_sessions[0], SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(set.shown, 0);
        assert_eq!(set.snapshot().active_workspace, "s1");
        assert_eq!(set.snapshot().active_tab, s1_tab);
        assert_eq!(set.recent_workspaces, recency_before);
        assert_eq!(set.workspaces[1].tabs.len(), 1);
        assert_eq!(set.workspaces[0], shown_before);

        // Its own caller closes the hidden s2's final tab, by the tab or by its
        // last pane, stopping its hidden popup too; only s2 goes, and the shown
        // s1 stays shown and unchanged.
        let s2_pane = set.workspaces[1].tabs[0].columns[0].panes[0].id.clone();
        let final_sessions = vec![s2_caller.clone(), s2_agent.session.id.clone()];
        let unchanged = set.clone();
        assert_eq!(
            set.prepare_close_tab("s2-final-tab", Some(&s2_caller), &s2_tab0)
                .unwrap(),
            final_sessions
        );
        assert_eq!(
            set.prepare_close_pane("s2-final-pane", Some(&s2_caller), &s2_pane)
                .unwrap(),
            final_sessions
        );
        assert_eq!(set.workspaces, unchanged.workspaces);
        for session in &final_sessions {
            set.session_ended(session, SessionEnd::Closed, |_| Ok(()))
                .unwrap();
        }
        assert_eq!(set.workspaces.len(), 1);
        assert_eq!(set.shown, 0);
        assert_eq!(set.workspaces[0], shown_before);
        assert_eq!(set.snapshot().active_workspace, "s1");
    }
}
