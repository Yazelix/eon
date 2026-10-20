use super::managed_environment::{PopupCatalog, PopupCommand, PopupDefinition};
use eon_workspace_protocol::v12::{
    Action, Column as SnapshotColumn, Direction, Failure, InvokeIntent, MAX_COLUMNS,
    MAX_DIRECTORY_BYTES, MAX_SESSIONS, MAX_TABS, Pane as SnapshotPane, Popup as SnapshotPopup,
    PopupEntry as SnapshotEntry, PopupTarget, Side, Snapshot, Tab as SnapshotTab, WorkspaceAction,
    check_popup_action,
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const MAX_RECENT_REQUESTS: usize = 256;

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
    },
    Stop(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: Session,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Popup {
    id: String,
    entry: String,
    session: Session,
    launch_directory: PathBuf,
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
pub(crate) struct Workspace {
    runtime: PathBuf,
    catalog: PopupCatalog,
    home: Option<PathBuf>,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: usize,
    next_pane: usize,
    next_session: usize,
    next_popup: usize,
    recent_requests: VecDeque<String>,
}

impl Workspace {
    pub(crate) fn pending(
        runtime: PathBuf,
        directory: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut workspace = Self {
            runtime,
            catalog,
            home,
            tabs: vec![Tab {
                id: "t1".into(),
                directory,
                pending: true,
                columns: Vec::new(),
                selected_column: None,
                popups: Vec::new(),
                selected_popup: None,
            }],
            active: 0,
            next_tab: 2,
            next_pane: 1,
            next_session: 1,
            next_popup: 1,
            recent_requests: VecDeque::new(),
        };
        if workspace
            .open_popup(0, "project", &mut prepare, &mut operate, true)
            .is_err()
        {
            workspace.tabs[0].pending = false;
            workspace.start_pane(0, &mut operate)?;
        }
        Ok(workspace)
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
        let panes = sessions
            .into_iter()
            .map(|(number, session)| Pane {
                id: format!("p{number}"),
                session,
            })
            .collect::<Vec<_>>();
        Ok(Self {
            runtime,
            catalog,
            home,
            tabs: vec![Tab {
                id: "t1".into(),
                directory,
                pending: false,
                columns: vec![Column { panes, expanded: 0 }],
                selected_column: Some(0),
                popups: Vec::new(),
                selected_popup: None,
            }],
            active: 0,
            next_tab: 2,
            next_pane: next_session,
            next_session,
            next_popup: 1,
            recent_requests: VecDeque::new(),
        })
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
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
                                    id: pane.id.clone(),
                                    session: pane.session.id.clone(),
                                    endpoint: pane.session.endpoint.as_os_str().as_bytes().to_vec(),
                                    live: true,
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
                        })
                        .collect(),
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
        }
    }

    pub(crate) fn dispatch(
        &mut self,
        request_id: &str,
        action: Action,
        mut prepare: impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.reject_duplicate(request_id)?;
        check_popup_action(&self.snapshot(), &action)
            .map_err(|error| action_error("stale-popup", error.to_string()))?;
        match action {
            Action::Workspace(action) => {
                self.dispatch_workspace(action, &mut prepare, &mut operate)?
            }
            Action::ClosePane { .. } => {
                return Err(action_error(
                    "unavailable",
                    "the supervisor stops a closed pane's terminal before pruning it",
                ));
            }
            Action::InvokePopup {
                tab, entry, intent, ..
            } => self.invoke_popup(&tab, &entry, intent, &mut prepare, &mut operate)?,
            Action::DismissPopup(target) => self.dismiss_popup(&target, &mut operate)?,
            Action::CommitDirectory { target, directory } => {
                self.commit_directory(&target, directory, &mut operate)?
            }
            // A tab without columns gains its first one exactly as a new pane would.
            Action::CreateColumn if self.tabs[self.active].columns.is_empty() => {
                self.dispatch_workspace(WorkspaceAction::CreatePane, &mut prepare, &mut operate)?
            }
            Action::CreateColumn => {
                self.require_established(self.active)?;
                if self.tabs[self.active].columns.len() >= MAX_COLUMNS {
                    return Err(action_error(
                        "unavailable",
                        format!("a tab holds at most {MAX_COLUMNS} columns"),
                    ));
                }
                self.dismiss_for_pane_action(self.active, &mut operate)?;
                self.start_column(self.active, &mut operate)
                    .map_err(|detail| action_error("session-start", detail))?;
            }
            Action::MovePaneSideways(side) => self.move_pane_sideways(side)?,
        }
        self.remember(request_id);
        Ok(())
    }

    fn dispatch_workspace(
        &mut self,
        action: WorkspaceAction,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match action {
            WorkspaceAction::Inspect => {}
            WorkspaceAction::InspectRuntime
            | WorkspaceAction::InspectPresentation
            | WorkspaceAction::Present { .. }
            | WorkspaceAction::CloseTab { .. }
            | WorkspaceAction::Stop { .. } => {
                return Err(action_error(
                    "unavailable",
                    "supervisor lifecycle actions do not mutate workspace topology",
                ));
            }
            WorkspaceAction::CreateTab => self.create_tab(prepare, operate)?,
            WorkspaceAction::CreatePane => {
                self.require_established(self.active)?;
                self.dismiss_for_pane_action(self.active, operate)?;
                self.start_pane(self.active, operate)
                    .map_err(|detail| action_error("session-start", detail))?;
            }
            WorkspaceAction::FocusId(id) => self.focus_id(&id, operate)?,
            WorkspaceAction::Focus(direction) => self.focus_direction(direction, operate)?,
            WorkspaceAction::Move(direction) => self.move_direction(direction, operate)?,
            WorkspaceAction::SetTabDirectory { tab, directory } => {
                self.set_tab_directory(&tab, directory)?;
            }
            WorkspaceAction::PickTabDirectory => {
                return Err(action_error(
                    "unavailable",
                    "the EONW v8 Project entry replaces the retired picker action",
                ));
            }
        }
        Ok(())
    }

    fn create_tab(
        &mut self,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if self.tabs.len() >= MAX_TABS || self.next_tab == usize::MAX {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_TABS} tabs"),
            ));
        }
        let id = format!("t{}", self.next_tab);
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
        if let Err(error) = self.open_popup(index, "project", prepare, operate, true) {
            self.tabs.pop();
            return Err(error);
        }
        self.next_tab += 1;
        self.active = index;
        Ok(())
    }

    fn invoke_popup(
        &mut self,
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
            if popup.launch_directory != self.tabs[tab_index].directory {
                let candidate = self.popup_candidate(tab_index, &definition, true)?;
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
                self.commit_popup(tab_index, candidate);
                return Ok(());
            }
            let selected = self.tabs[tab_index].selected_popup.as_deref() == Some(&popup.id);
            if intent == InvokeIntent::Toggle && selected {
                return self.dismiss_popup(
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
        self.open_popup(tab_index, entry, prepare, operate, false)
    }

    fn open_popup(
        &mut self,
        tab_index: usize,
        entry: &str,
        prepare: &mut impl FnMut(&PopupCommand, &Path) -> Result<Vec<OsString>, String>,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
        pending: bool,
    ) -> Result<(), Failure> {
        let definition = self.definition(entry)?.clone();
        let displaced = self.displaced_transient(tab_index, None)?;
        let stops_selected = displaced.is_some();
        let candidate = self.popup_candidate(tab_index, &definition, stops_selected)?;
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
        self.commit_popup(tab_index, candidate);
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
        tab_index: usize,
        definition: &PopupDefinition,
        replacing: bool,
    ) -> Result<Popup, Failure> {
        if !replacing && self.session_count() >= MAX_SESSIONS {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_SESSIONS} terminals"),
            ));
        }
        if self.next_popup == usize::MAX || self.next_session == usize::MAX {
            return Err(action_error(
                "capacity",
                "workspace exhausted terminal identities",
            ));
        }
        let project = matches!(definition.command, PopupCommand::Project);
        let number = if project {
            self.next_popup
        } else {
            self.next_session
        };
        let session_id = if project {
            format!("directory-picker-{number}")
        } else {
            format!("session-{number}")
        };
        Ok(Popup {
            id: format!("u{}", self.next_popup),
            entry: definition.id.clone(),
            session: Session {
                id: session_id.clone(),
                endpoint: self.runtime.join(if project {
                    format!("k{number}.sock")
                } else {
                    format!("{session_id}.sock")
                }),
            },
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
        })
    }

    fn commit_popup(&mut self, tab_index: usize, popup: Popup) {
        let project = is_directory_picker_session(&popup.session.id);
        self.tabs[tab_index].selected_popup = Some(popup.id.clone());
        self.tabs[tab_index].popups.push(popup);
        self.next_popup += 1;
        if !project {
            self.next_session += 1;
        }
    }

    fn dismiss_popup(
        &mut self,
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
            self.finish_pending_tab(tab_index, operate)?;
        } else {
            self.remove_empty_tab(tab_index);
        }
        Ok(())
    }

    fn commit_directory(
        &mut self,
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
            self.start_pane_at(tab_index, &directory, operate)
                .map_err(|detail| action_error("session-start", detail))?;
            self.tabs[tab_index].directory = directory;
            self.tabs[tab_index].pending = false;
        } else {
            self.tabs[tab_index].directory = directory;
        }
        Ok(())
    }

    pub(crate) fn session_ended(
        &mut self,
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
                self.finish_pending_tab(tab_index, &mut operate)?;
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

    pub(crate) fn prepare_close_tab(
        &mut self,
        request_id: &str,
        id: &str,
    ) -> Result<Vec<String>, Failure> {
        self.reject_duplicate(request_id)?;
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
        if self.tabs.len() == 1 {
            return Err(action_error(
                "unavailable",
                "the final tab cannot be closed",
            ));
        }
        let sessions = self.tabs[index]
            .panes()
            .map(|pane| pane.session.id.clone())
            .chain(
                self.tabs[index]
                    .popups
                    .iter()
                    .map(|popup| popup.session.id.clone()),
            )
            .collect();
        self.remember(request_id);
        Ok(sessions)
    }

    /// Validates closing one pane of the active tab and names its terminal.
    pub(crate) fn prepare_close_pane(
        &mut self,
        request_id: &str,
        id: &str,
    ) -> Result<String, Failure> {
        self.reject_duplicate(request_id)?;
        let tab = &self.tabs[self.active];
        let Some((column, pane)) = tab.find_pane(|pane| pane.id == id) else {
            return Err(
                if self
                    .tabs
                    .iter()
                    .any(|tab| tab.find_pane(|pane| pane.id == id).is_some())
                {
                    action_error("not-active", format!("pane {id} is not in the active tab"))
                } else {
                    action_error("unknown-pane", format!("pane {id} is not live"))
                },
            );
        };
        if tab.selected_popup.is_some() {
            return Err(action_error(
                "unavailable",
                "close the popup before closing a pane",
            ));
        }
        if self.tabs.len() == 1 && tab.panes().count() == 1 {
            return Err(action_error(
                "unavailable",
                "the final pane cannot be closed",
            ));
        }
        let session = tab.columns[column].panes[pane].session.id.clone();
        self.remember(request_id);
        Ok(session)
    }

    fn start_pane(
        &mut self,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let directory = self.tabs[tab_index].directory.clone();
        self.start_pane_at(tab_index, &directory, operate)
    }

    fn start_column(
        &mut self,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        let directory = self.tabs[tab_index].directory.clone();
        self.start_pane_in(tab_index, &directory, true, operate)
    }

    fn start_pane_at(
        &mut self,
        tab_index: usize,
        directory: &Path,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        self.start_pane_in(tab_index, directory, false, operate)
    }

    /// Starts a pane in the selected column, or in a new column right of it.
    fn start_pane_in(
        &mut self,
        tab_index: usize,
        directory: &Path,
        new_column: bool,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check_session_capacity()?;
        if self.next_pane == usize::MAX || self.next_session == usize::MAX {
            return Err("workspace exhausted pane or terminal identities".into());
        }
        let pane = Pane {
            id: format!("p{}", self.next_pane),
            session: Session {
                id: format!("session-{}", self.next_session),
                endpoint: self.runtime.join(if self.next_session == 1 {
                    "orbit.sock".into()
                } else {
                    format!("session-{}.sock", self.next_session)
                }),
            },
        };
        operate(SessionOperation::Start {
            session: pane.session.clone(),
            directory: directory.to_path_buf(),
            command: LaunchCommand::Pane,
        })?;
        self.next_pane += 1;
        self.next_session += 1;
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
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.tabs[tab_index].pending = false;
        if let Err(detail) = self.start_pane(tab_index, operate) {
            self.remove_empty_tab(tab_index);
            return Err(action_error("session-start", detail));
        }
        Ok(())
    }

    fn dismiss_for_pane_action(
        &mut self,
        tab_index: usize,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let Some(instance) = self.tabs[tab_index].selected_popup.clone() else {
            return Ok(());
        };
        self.dismiss_popup(
            &PopupTarget {
                tab: self.tabs[tab_index].id.clone(),
                instance,
            },
            operate,
        )
    }

    fn focus_id(
        &mut self,
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
                self.dismiss_for_pane_action(tab_index, operate)?;
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

    fn focus_direction(
        &mut self,
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
                self.dismiss_for_pane_action(self.active, operate)?;
                let tab = &mut self.tabs[self.active];
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

    fn move_direction(
        &mut self,
        direction: Direction,
        operate: &mut impl FnMut(SessionOperation) -> Result<(), String>,
    ) -> Result<(), Failure> {
        match direction {
            Direction::Left if self.active > 0 => {
                self.tabs.swap(self.active, self.active - 1);
                self.active -= 1;
            }
            Direction::Right if self.active + 1 < self.tabs.len() => {
                self.tabs.swap(self.active, self.active + 1);
                self.active += 1;
            }
            Direction::Up | Direction::Down => {
                self.require_established(self.active)?;
                self.dismiss_for_pane_action(self.active, operate)?;
                let tab = &mut self.tabs[self.active];
                let column = tab
                    .selected_column
                    .ok_or_else(|| action_error("unavailable", "active tab has no pane"))?;
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
            Direction::Left | Direction::Right => {}
        }
        Ok(())
    }

    /// Moves the selected pane into the neighbouring column's stack, below its
    /// expanded pane; past the outer column a stacked pane opens a new column.
    fn move_pane_sideways(&mut self, side: Side) -> Result<(), Failure> {
        self.require_established(self.active)?;
        let tab = &mut self.tabs[self.active];
        if tab.selected_popup.is_some() {
            return Err(action_error(
                "unavailable",
                "close the popup before moving a pane sideways",
            ));
        }
        let source = tab
            .selected_column
            .ok_or_else(|| action_error("unavailable", "active tab has no pane"))?;
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

    fn set_tab_directory(&mut self, id: &str, directory: Vec<u8>) -> Result<(), Failure> {
        if tab_number(id).is_none() {
            return Err(action_error(
                "invalid-tab",
                "tab directory targets require a canonical tN identity",
            ));
        }
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .ok_or_else(|| action_error("unknown-tab", format!("tab {id} is not live")))?;
        if tab.pending {
            return Err(action_error(
                "choose-directory-first",
                "the pending tab requires its exact Project chooser result",
            ));
        }
        tab.directory = validated_directory(directory)?;
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

    fn check_session_capacity(&self) -> Result<(), String> {
        if self.session_count() >= MAX_SESSIONS {
            Err(format!("workspace is limited to {MAX_SESSIONS} terminals"))
        } else {
            Ok(())
        }
    }

    fn session_count(&self) -> usize {
        self.tabs
            .iter()
            .map(|tab| tab.panes().count() + tab.popups.len())
            .sum()
    }

    fn reject_duplicate(&self, request_id: &str) -> Result<(), Failure> {
        if self.recent_requests.iter().any(|seen| seen == request_id) {
            Err(action_error(
                "duplicate-request",
                format!("request {request_id} was already accepted"),
            ))
        } else {
            Ok(())
        }
    }

    fn remember(&mut self, request_id: &str) {
        if self.recent_requests.len() == MAX_RECENT_REQUESTS {
            self.recent_requests.pop_front();
        }
        self.recent_requests.push_back(request_id.into());
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

pub(crate) fn human(snapshot: &Snapshot) -> String {
    let mut output = format!("active {}\n", snapshot.active_tab);
    for tab in &snapshot.tabs {
        output.push_str(&format!(
            "tab {} directory={} selected={} popup={} pending={} active={}\n",
            tab.id,
            String::from_utf8_lossy(&tab.directory).escape_debug(),
            tab.selected_pane.as_deref().unwrap_or("none"),
            tab.selected_popup.as_deref().unwrap_or("none"),
            tab.pending,
            tab.id == snapshot.active_tab,
        ));
        for (column, expanded, pane) in column_panes(tab) {
            output.push_str(&format!(
                "  pane {} column={} expanded={} terminal={} live={} endpoint={}\n",
                pane.id,
                column + 1,
                expanded,
                pane.session,
                pane.live,
                String::from_utf8_lossy(&pane.endpoint).escape_debug()
            ));
        }
        for popup in &tab.popups {
            let entry = snapshot
                .entries
                .iter()
                .find(|entry| entry.id == popup.entry);
            output.push_str(&format!(
                "  popup {} entry={} label={} key={} terminal={} chosen={} endpoint={}\n",
                popup.id,
                popup.entry,
                entry.map_or("?", |entry| entry.label.as_str()),
                entry.map_or_else(|| "?".into(), |entry| shortcut_text(&entry.shortcut)),
                popup.session,
                tab.selected_popup.as_deref() == Some(&popup.id),
                String::from_utf8_lossy(&popup.endpoint).escape_debug(),
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
            "{{\"id\":\"{}\",\"label\":\"{}\",\"keybinding\":\"{}\"}}",
            json_escape(&entry.id),
            json_escape(&entry.label),
            json_escape(&shortcut_text(&entry.shortcut)),
        ));
    }
    output.push_str("],\"tabs\":[");
    for (tab_index, tab) in snapshot.tabs.iter().enumerate() {
        if tab_index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"directory\":[{}],\"pending\":{},\"selected_pane\":",
            json_escape(&tab.id),
            json_bytes(&tab.directory),
            tab.pending
        ));
        json_option(&mut output, tab.selected_pane.as_deref());
        output.push_str(",\"selected_popup\":");
        json_option(&mut output, tab.selected_popup.as_deref());
        output.push_str(",\"panes\":[");
        for (index, (column, expanded, pane)) in column_panes(tab).enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"column\":{},\"expanded\":{},\"session\":\"{}\",\"endpoint\":[{}],\"live\":{}}}",
                json_escape(&pane.id),
                column + 1,
                expanded,
                json_escape(&pane.session),
                json_bytes(&pane.endpoint),
                pane.live
            ));
        }
        output.push_str("],\"popups\":[");
        for (index, popup) in tab.popups.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"entry\":\"{}\",\"session\":\"{}\",\"endpoint\":[{}],\"chosen\":{}}}",
                json_escape(&popup.id), json_escape(&popup.entry), json_escape(&popup.session),
                json_bytes(&popup.endpoint), tab.selected_popup.as_deref() == Some(&popup.id)
            ));
        }
        output.push_str("]}");
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

fn action_error(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        detail: detail.into(),
    }
}

fn shortcut_text(shortcut: &eon_workspace_protocol::v12::Shortcut) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [
        (eon_workspace_protocol::v12::CTRL, "Ctrl"),
        (eon_workspace_protocol::v12::ALT, "Alt"),
        (eon_workspace_protocol::v12::SHIFT, "Shift"),
        (eon_workspace_protocol::v12::SUPER, "Super"),
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

fn json_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
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
    use eon_workspace_protocol::v12::{ALT, PopupGeometry, Shortcut};

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
                    shortcut: Shortcut {
                        modifiers: ALT,
                        key: "KeyZ".into(),
                    },
                    command: PopupCommand::Project,
                    keep_alive: false,
                },
                PopupDefinition {
                    id: "agent".into(),
                    label: "Agent".into(),
                    shortcut: Shortcut {
                        modifiers: ALT,
                        key: "KeyA".into(),
                    },
                    command: PopupCommand::Argv(vec!["agent".into()]),
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
        let mut workspace = Workspace::with_recovered_sessions(
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
                .dispatch(id, Action::CreateColumn, prepared, ok)
                .unwrap();
        }
        workspace
            .dispatch(
                "t2",
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
            .dispatch("open", project("t2"), prepared, ok)
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
            .dispatch("c4", Action::CreateColumn, prepared, ok)
            .unwrap();
        let after = workspace.snapshot();
        assert_eq!(after.tabs.len(), 1);
        assert_eq!(after.tabs[0].columns.len(), MAX_COLUMNS);
        eon_workspace_protocol::v12::encode_response(
            &eon_workspace_protocol::v12::Response::Snapshot(after),
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn columns_hold_accordions_traverse_into_tabs_and_collapse_on_exit() {
        let root = std::env::temp_dir().join(format!("eon-columns-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = Workspace::with_recovered_sessions(
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
        let request = |workspace: &mut Workspace, id: &str, action| {
            workspace.dispatch(id, action, prepared, ok)
        };
        let shape = |workspace: &Workspace| {
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
            Action::Workspace(WorkspaceAction::CreatePane),
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
        let mut workspace = Workspace::with_recovered_sessions(
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
        let request = |workspace: &mut Workspace, id: &str, action| {
            workspace.dispatch(id, action, prepared, ok).unwrap()
        };
        let close = |workspace: &mut Workspace, id: &str, pane: &str| {
            let session = workspace.prepare_close_pane(id, pane)?;
            workspace.session_ended(&session, SessionEnd::Closed, ok)?;
            Ok::<_, Failure>(session)
        };
        let refused = |workspace: &mut Workspace, id: &str, pane: &str| {
            let before = workspace.snapshot();
            let failure = close(workspace, id, pane).unwrap_err();
            assert_eq!(workspace.snapshot(), before);
            failure
        };

        assert_eq!(
            refused(&mut workspace, "x1", "p1").detail,
            "the final pane cannot be closed"
        );
        request(&mut workspace, "c1", Action::CreateColumn);
        request(
            &mut workspace,
            "c2",
            Action::Workspace(WorkspaceAction::CreatePane),
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
    }

    #[test]
    fn sideways_pane_moves_join_neighbor_stacks_split_at_edges_and_drop_emptied_columns() {
        let mut workspace = Workspace::with_recovered_sessions(
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
        let request = |workspace: &mut Workspace, id: &str, action| {
            workspace.dispatch(id, action, prepared, ok)
        };
        let workspace_action = |workspace: &mut Workspace, id: &str, action| {
            request(workspace, id, Action::Workspace(action))
        };
        let sideways = |workspace: &mut Workspace, id: &str, side| {
            request(workspace, id, Action::MovePaneSideways(side))
        };
        // Each column's panes with the expanded one marked, then the selected pane.
        let layout = |workspace: &Workspace| {
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
        workspace_action(&mut workspace, "p", WorkspaceAction::CreatePane).unwrap();

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
        workspace_action(&mut workspace, "p3", WorkspaceAction::CreatePane).unwrap();
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
    fn sideways_pane_moves_refuse_covering_popups_without_changing_tabs() {
        let mut workspace = Workspace::with_recovered_sessions(
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
            ("stack", WorkspaceAction::CreatePane),
            ("other-tab", WorkspaceAction::CreateTab),
            ("select", WorkspaceAction::FocusId("t1".into())),
        ] {
            workspace
                .dispatch(id, Action::Workspace(action), prepared, |_| Ok(()))
                .unwrap();
        }
        for entry in ["agent", "project"] {
            for with_panes in [true, false] {
                let mut covered = workspace.clone();
                covered
                    .dispatch(
                        "open",
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
                for side in [Side::Left, Side::Right] {
                    assert_eq!(
                        covered
                            .dispatch("move", Action::MovePaneSideways(side), prepared, |_| {
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
        let mut workspace = Workspace::with_recovered_sessions(
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
        let request = |workspace: &mut Workspace, id: &str, action| {
            workspace.dispatch(id, action, prepared, ok)
        };
        let workspace_action = |workspace: &mut Workspace, id: &str, action| {
            request(workspace, id, Action::Workspace(action))
        };
        let panes = |workspace: &Workspace| {
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
        workspace_action(&mut workspace, "p", WorkspaceAction::CreatePane).unwrap();

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
    fn popup_identity_reuses_hides_and_replaces_only_after_retarget() {
        let root = std::env::temp_dir().join(format!("eon-workspace-{}", std::process::id()));
        let old = root.join("old");
        let new = root.join("new");
        let newest = root.join("newest");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::create_dir_all(&newest).unwrap();
        let mut operations = Vec::new();
        let mut workspace = Workspace::with_recovered_sessions(
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
        workspace
            .dispatch(
                "open",
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "agent".into(),
                    expected_instance: None,
                    intent: InvokeIntent::Toggle,
                },
                prepared,
                &mut operate,
            )
            .unwrap();
        let first = workspace.snapshot().tabs[0].popups[0].clone();
        workspace
            .dispatch(
                "hide",
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "agent".into(),
                    expected_instance: Some(first.id.clone()),
                    intent: InvokeIntent::Toggle,
                },
                prepared,
                &mut operate,
            )
            .unwrap();
        assert_eq!(workspace.snapshot().tabs[0].popups[0], first);
        workspace
            .dispatch(
                "retarget",
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: new.as_os_str().as_bytes().to_vec(),
                }),
                prepared,
                &mut operate,
            )
            .unwrap();
        assert_eq!(workspace.snapshot().tabs[0].popups[0], first);
        workspace
            .dispatch(
                "replace",
                Action::InvokePopup {
                    tab: "t1".into(),
                    entry: "agent".into(),
                    expected_instance: Some(first.id.clone()),
                    intent: InvokeIntent::Toggle,
                },
                prepared,
                &mut operate,
            )
            .unwrap();
        let replacement = &workspace.snapshot().tabs[0].popups[0];
        assert_ne!(replacement.id, first.id);
        assert_eq!(
            operations
                .iter()
                .filter_map(|operation| match operation {
                    SessionOperation::Stop(id) => Some(id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [first.session]
        );
        assert_eq!(
            operations
                .iter()
                .filter_map(|operation| match operation {
                    SessionOperation::Start { directory, .. } => Some(directory),
                    SessionOperation::Stop(_) => None,
                })
                .collect::<Vec<_>>(),
            [&root.join("old"), &new]
        );

        let live = replacement.clone();
        workspace
            .dispatch(
                "retarget-again",
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t1".into(),
                    directory: newest.as_os_str().as_bytes().to_vec(),
                }),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let replace = || Action::InvokePopup {
            tab: "t1".into(),
            entry: "agent".into(),
            expected_instance: Some(live.id.clone()),
            intent: InvokeIntent::Toggle,
        };
        assert_eq!(
            workspace
                .dispatch(
                    "stop-fails",
                    replace(),
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
            workspace.snapshot().tabs[0].popups.as_slice(),
            std::slice::from_ref(&live)
        );
        assert_eq!(
            workspace
                .dispatch(
                    "start-fails",
                    replace(),
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
        let mut workspace = Workspace::with_recovered_sessions(
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
            .dispatch("agent-open", invoke("agent", None), prepared, &mut operate)
            .unwrap();
        let agent = workspace.snapshot().tabs[0].popups[0].clone();
        workspace
            .dispatch(
                "agent-hide",
                invoke("agent", Some(agent.id.clone())),
                prepared,
                &mut operate,
            )
            .unwrap();
        workspace
            .dispatch(
                "project-open",
                invoke("project", None),
                prepared,
                &mut operate,
            )
            .unwrap();
        workspace
            .dispatch(
                "agent-return",
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

        workspace.tabs.push(Tab {
            id: "t2".into(),
            directory: "/".into(),
            pending: false,
            columns: vec![Column {
                panes: vec![Pane {
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
            .dispatch("project-again", invoke("project", None), prepared, |_| {
                Ok(())
            })
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
        let mut workspace = Workspace::pending(
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
                .dispatch("commit-fails", commit(), prepared, |_| {
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
            .dispatch("commit", commit(), prepared, |_| Ok(()))
            .unwrap();
        assert!(!workspace.snapshot().tabs[0].pending);
        assert!(
            workspace
                .dispatch(
                    "stale",
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
    fn new_tabs_start_at_home_and_reject_unavailable_home_without_mutation() {
        let root = std::env::temp_dir().join(format!("eon-home-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut workspace = Workspace::pending(
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
        eon_workspace_protocol::v12::encode_response(
            &eon_workspace_protocol::v12::Response::Snapshot(workspace.snapshot()),
        )
        .unwrap();

        for home in [None, Some("relative".into()), Some(root.join("missing"))] {
            workspace.home = home;
            let before = workspace.snapshot();
            let error = workspace
                .dispatch(
                    "invalid",
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
    fn pending_tabs_keep_independent_pickers_and_close_only_the_selected_tab() {
        let mut workspace = Workspace::pending(
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
                Action::Workspace(WorkspaceAction::FocusId("t1".into())),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            workspace.prepare_close_tab("close-t1", "t1").unwrap(),
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
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        let third = workspace.snapshot().tabs[1].popups[0].clone();
        assert_eq!(
            workspace.prepare_close_tab("close-t3", "t3").unwrap(),
            std::slice::from_ref(&third.session)
        );
        workspace
            .session_ended(&third.session, SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot(), before_failed_launch);

        workspace
            .dispatch(
                "accept-t2",
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
        let mut workspace = Workspace::pending(
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
