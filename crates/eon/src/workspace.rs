use eon_workspace_protocol::v4::{
    Action, Direction, DirectoryPicker as SnapshotDirectoryPicker, Failure, MAX_DIRECTORY_BYTES,
    MAX_PANES, MAX_TABS, Pane as SnapshotPane, Snapshot, Tab as SnapshotTab,
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const MAX_RECENT_REQUESTS: usize = 256;
pub(crate) const DIRECTORY_PICKER_SESSION: &str = "directory-picker";
pub(crate) const DIRECTORY_PICKER_ENDPOINT: &str = "pick.sock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) endpoint: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: Session,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tab {
    id: String,
    directory: PathBuf,
    panes: Vec<Pane>,
    selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryPicker {
    tab: String,
    previous_tab: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Workspace {
    runtime: PathBuf,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: usize,
    next_pane: usize,
    directory_picker: Option<DirectoryPicker>,
    recent_requests: VecDeque<String>,
}

impl Workspace {
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            active_tab: self.tabs[self.active].id.clone(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| SnapshotTab {
                    id: tab.id.clone(),
                    directory: tab.directory.as_os_str().as_bytes().to_vec(),
                    selected_pane: tab.selected.map(|selected| tab.panes[selected].id.clone()),
                    panes: tab
                        .panes
                        .iter()
                        .map(|pane| SnapshotPane {
                            id: pane.id.clone(),
                            session: pane.session.id.clone(),
                            endpoint: pane.session.endpoint.as_os_str().as_bytes().to_vec(),
                            live: true,
                        })
                        .collect(),
                })
                .collect(),
            directory_picker: self.directory_picker.as_ref().map(|picker| {
                SnapshotDirectoryPicker {
                    tab: picker.tab.clone(),
                    endpoint: self
                        .runtime
                        .join(DIRECTORY_PICKER_ENDPOINT)
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                }
            }),
        }
    }
}

pub(crate) fn human(snapshot: &Snapshot) -> String {
    let active = &snapshot.active_tab;
    let mut output = format!("active {active}\n");
    if let Some(picker) = &snapshot.directory_picker {
        output.push_str(&format!(
            "picker tab={} endpoint={}\n",
            picker.tab,
            String::from_utf8_lossy(&picker.endpoint).escape_debug()
        ));
    }
    for tab in &snapshot.tabs {
        output.push_str(&format!(
            "tab {} directory={} selected={} active={}\n",
            tab.id,
            String::from_utf8_lossy(&tab.directory).escape_debug(),
            tab.selected_pane.as_deref().unwrap_or("none"),
            tab.id == *active
        ));
        for pane in &tab.panes {
            output.push_str(&format!(
                "  pane {} session={} live={} endpoint={}\n",
                pane.id,
                pane.session,
                pane.live,
                String::from_utf8_lossy(&pane.endpoint).escape_debug()
            ));
        }
    }
    output
}

pub(crate) fn json(snapshot: &Snapshot) -> String {
    let mut output = format!(
        "{{\"active_tab\":\"{}\",\"tabs\":[",
        json_escape(&snapshot.active_tab)
    );
    for (tab_index, tab) in snapshot.tabs.iter().enumerate() {
        if tab_index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"directory\":[{}],\"selected_pane\":",
            json_escape(&tab.id),
            json_bytes(&tab.directory),
        ));
        match &tab.selected_pane {
            Some(selected) => output.push_str(&format!("\"{}\"", json_escape(selected))),
            None => output.push_str("null"),
        }
        output.push_str(",\"panes\":[");
        for (pane_index, pane) in tab.panes.iter().enumerate() {
            if pane_index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"session\":\"{}\",\"endpoint\":[{}],\"live\":{}}}",
                json_escape(&pane.id),
                json_escape(&pane.session),
                json_bytes(&pane.endpoint),
                pane.live
            ));
        }
        output.push_str("]}");
    }
    output.push_str("],\"directory_picker\":");
    match &snapshot.directory_picker {
        Some(picker) => output.push_str(&format!(
            "{{\"tab\":\"{}\",\"endpoint\":[{}]}}",
            json_escape(&picker.tab),
            json_bytes(&picker.endpoint)
        )),
        None => output.push_str("null"),
    }
    output.push_str("}\n");
    output
}

fn json_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
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

fn action_error(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        detail: detail.into(),
    }
}

impl Workspace {
    pub(crate) fn pending(
        runtime: PathBuf,
        directory: PathBuf,
        mut start: impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut workspace = Self {
            runtime,
            tabs: vec![Tab {
                id: "t1".into(),
                directory,
                panes: Vec::new(),
                selected: None,
            }],
            active: 0,
            next_tab: 2,
            next_pane: 1,
            directory_picker: None,
            recent_requests: VecDeque::new(),
        };
        if workspace.create_directory_picker(&mut start).is_err() {
            let pane = workspace.next_pane();
            start(&pane.session, &workspace.tabs[0].directory, None)?;
            workspace.next_pane += 1;
            workspace.tabs[0].panes.push(pane);
            workspace.tabs[0].selected = Some(0);
        }
        Ok(workspace)
    }

    pub(crate) fn with_recovered_sessions(
        runtime: PathBuf,
        directory: PathBuf,
        sessions: Vec<(usize, Session)>,
    ) -> Result<Self, String> {
        let next_pane = sessions
            .last()
            .ok_or("cannot recover an empty workspace")?
            .0
            .checked_add(1)
            .ok_or("recovered Session identity leaves no next pane identity")?;
        Ok(Self {
            runtime,
            tabs: vec![Tab {
                id: "t1".into(),
                directory,
                panes: sessions
                    .into_iter()
                    .map(|(number, session)| Pane {
                        id: format!("p{number}"),
                        session,
                    })
                    .collect(),
                selected: Some(0),
            }],
            active: 0,
            next_tab: 2,
            next_pane,
            directory_picker: None,
            recent_requests: VecDeque::new(),
        })
    }

    pub(crate) fn dispatch(
        &mut self,
        request_id: &str,
        action: Action,
        mut start: impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if self.recent_requests.iter().any(|seen| seen == request_id) {
            return Err(action_error(
                "duplicate-request",
                format!("request {request_id} was already accepted"),
            ));
        }

        if let Some(picker) = &self.directory_picker {
            match &action {
                Action::Inspect => {}
                Action::SetTabDirectory { tab, .. } if tab == &picker.tab => {}
                _ => {
                    return Err(action_error(
                        "picker-active",
                        format!("tab {} already has an active directory picker", picker.tab),
                    ));
                }
            }
        }

        match action {
            Action::Inspect => {}
            Action::InspectRuntime
            | Action::InspectPresentation
            | Action::Present { .. }
            | Action::Stop { .. } => {
                return Err(action_error(
                    "unavailable",
                    "supervisor lifecycle actions do not mutate workspace topology",
                ));
            }
            Action::CreateTab => self.create_tab(&mut start)?,
            Action::CreatePane => self.create_pane(&mut start)?,
            Action::FocusId(id) => self.focus_id(&id)?,
            Action::Focus(direction) => self.focus_direction(direction)?,
            Action::SetTabDirectory { tab, directory } => {
                self.set_tab_directory(&tab, directory, &mut start)?;
            }
            Action::PickTabDirectory => self.create_directory_picker(&mut start)?,
        }
        self.remember(request_id);
        Ok(())
    }

    pub(crate) fn session_exited(
        &mut self,
        session_id: &str,
        mut start: impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<bool, Failure> {
        if session_id == DIRECTORY_PICKER_SESSION && self.directory_picker.is_some() {
            let picker = self.directory_picker.take().unwrap();
            let tab_index = self
                .tabs
                .iter()
                .position(|tab| tab.id == picker.tab)
                .ok_or_else(|| action_error("unknown-tab", "directory picker tab is not live"))?;
            if !self.tabs[tab_index].panes.is_empty() {
                return Ok(false);
            }
            if let Some(previous_tab) = picker.previous_tab {
                self.tabs.remove(tab_index);
                self.active = self
                    .tabs
                    .iter()
                    .position(|tab| tab.id == previous_tab)
                    .unwrap_or_else(|| tab_index.min(self.tabs.len().saturating_sub(1)));
                return Ok(false);
            }
            self.check_pane_capacity()?;
            let pane = self.next_pane();
            let directory = self.tabs[tab_index].directory.clone();
            start(&pane.session, &directory, None)
                .map_err(|detail| action_error("session-start", detail))?;
            self.next_pane += 1;
            self.tabs[tab_index].panes.push(pane);
            self.tabs[tab_index].selected = Some(0);
            return Ok(false);
        }
        let (tab_index, pane_index) = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(tab_index, tab)| {
                tab.panes
                    .iter()
                    .position(|pane| pane.session.id == session_id)
                    .map(|pane_index| (tab_index, pane_index))
            })
            .ok_or_else(|| {
                action_error(
                    "unknown-session",
                    format!("session {session_id} is not in the workspace"),
                )
            })?;

        let tab = &mut self.tabs[tab_index];
        tab.panes.remove(pane_index);
        tab.selected = Some(index_after_removal(
            tab.selected.expect("durable tab has a selected pane"),
            pane_index,
            tab.panes.len(),
        ));
        if !tab.panes.is_empty() {
            return Ok(false);
        }

        let removed_tab = tab.id.clone();
        self.tabs.remove(tab_index);
        self.active = index_after_removal(self.active, tab_index, self.tabs.len());
        Ok(self
            .directory_picker
            .take_if(|picker| picker.tab == removed_tab)
            .is_some())
    }

    pub(crate) fn clear_directory_picker(&mut self) {
        self.directory_picker = None;
    }

    fn create_tab(
        &mut self,
        start: &mut impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<(), Failure> {
        if self.tabs.len() >= MAX_TABS {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_TABS} tabs"),
            ));
        }
        if self.next_tab == usize::MAX {
            return Err(action_error(
                "capacity",
                "workspace has exhausted tab identities",
            ));
        }
        self.check_pane_capacity()?;
        let tab_id = format!("t{}", self.next_tab);
        let directory = self.tabs[self.active].directory.clone();
        let session = Session {
            id: DIRECTORY_PICKER_SESSION.into(),
            endpoint: self.runtime.join(DIRECTORY_PICKER_ENDPOINT),
        };
        start(&session, &directory, Some(&tab_id))
            .map_err(|detail| action_error("picker-start", detail))?;
        let previous_tab = self.tabs[self.active].id.clone();
        self.next_tab += 1;
        self.tabs.push(Tab {
            id: tab_id.clone(),
            directory,
            panes: Vec::new(),
            selected: None,
        });
        self.active = self.tabs.len() - 1;
        self.directory_picker = Some(DirectoryPicker {
            tab: tab_id,
            previous_tab: Some(previous_tab),
        });
        Ok(())
    }

    fn create_pane(
        &mut self,
        start: &mut impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<(), Failure> {
        self.check_pane_capacity()?;
        let pane = self.next_pane();
        let directory = self.tabs[self.active].directory.clone();
        start(&pane.session, &directory, None)
            .map_err(|detail| action_error("session-start", detail))?;
        self.next_pane += 1;
        let active = &mut self.tabs[self.active];
        active.panes.push(pane);
        active.selected = Some(active.panes.len() - 1);
        Ok(())
    }

    fn create_directory_picker(
        &mut self,
        start: &mut impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
    ) -> Result<(), Failure> {
        let tab = &self.tabs[self.active];
        let session = Session {
            id: DIRECTORY_PICKER_SESSION.into(),
            endpoint: self.runtime.join(DIRECTORY_PICKER_ENDPOINT),
        };
        start(&session, &tab.directory, Some(&tab.id))
            .map_err(|detail| action_error("picker-start", detail))?;
        self.directory_picker = Some(DirectoryPicker {
            tab: tab.id.clone(),
            previous_tab: None,
        });
        Ok(())
    }

    fn set_tab_directory(
        &mut self,
        id: &str,
        directory: Vec<u8>,
        start: &mut impl FnMut(&Session, &Path, Option<&str>) -> Result<(), String>,
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
        let directory = directory_path(directory)?;
        let metadata = fs::metadata(&directory).map_err(|_| {
            action_error("invalid-directory", "tab launch directory is unavailable")
        })?;
        if !metadata.is_dir() {
            return Err(action_error(
                "invalid-directory",
                "tab launch directory is not a directory",
            ));
        }
        if self.tabs[tab_index].panes.is_empty() {
            self.check_pane_capacity()?;
            let pane = self.next_pane();
            start(&pane.session, &directory, None)
                .map_err(|detail| action_error("session-start", detail))?;
            self.next_pane += 1;
            self.tabs[tab_index].panes.push(pane);
            self.tabs[tab_index].selected = Some(0);
        }
        self.tabs[tab_index].directory = directory;
        Ok(())
    }

    fn focus_id(&mut self, id: &str) -> Result<(), Failure> {
        if let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == id) {
            if tab_index == self.active {
                return Err(action_error(
                    "already-focused",
                    format!("tab {id} is already active"),
                ));
            }
            self.active = tab_index;
            return Ok(());
        }

        for tab_index in 0..self.tabs.len() {
            if let Some(pane_index) = self.tabs[tab_index]
                .panes
                .iter()
                .position(|pane| pane.id == id)
            {
                if tab_index == self.active && Some(pane_index) == self.tabs[tab_index].selected {
                    return Err(action_error(
                        "already-focused",
                        format!("pane {id} is already selected"),
                    ));
                }
                self.active = tab_index;
                self.tabs[tab_index].selected = Some(pane_index);
                return Ok(());
            }
        }

        Err(action_error(
            "unknown-id",
            format!("{id} is not a live tab or pane identity"),
        ))
    }

    fn focus_direction(&mut self, direction: Direction) -> Result<(), Failure> {
        match direction {
            Direction::Left => {
                if self.tabs.len() == 1 {
                    return Err(action_error("unavailable", "no tab exists to the left"));
                }
                self.active = self.active.checked_sub(1).unwrap_or(self.tabs.len() - 1);
            }
            Direction::Right => {
                if self.tabs.len() == 1 {
                    return Err(action_error("unavailable", "no tab exists to the right"));
                }
                self.active = (self.active + 1) % self.tabs.len();
            }
            Direction::Up => {
                let tab = &mut self.tabs[self.active];
                let selected = tab
                    .selected
                    .ok_or_else(|| action_error("unavailable", "active tab has no pane"))?;
                if tab.panes.len() == 1 {
                    return Err(action_error("unavailable", "no pane exists above"));
                }
                tab.selected = Some(selected.checked_sub(1).unwrap_or(tab.panes.len() - 1));
            }
            Direction::Down => {
                let tab = &mut self.tabs[self.active];
                let selected = tab
                    .selected
                    .ok_or_else(|| action_error("unavailable", "active tab has no pane"))?;
                if tab.panes.len() == 1 {
                    return Err(action_error("unavailable", "no pane exists below"));
                }
                tab.selected = Some((selected + 1) % tab.panes.len());
            }
        }
        Ok(())
    }

    fn check_pane_capacity(&self) -> Result<(), Failure> {
        if self.next_pane == usize::MAX {
            return Err(action_error(
                "capacity",
                "workspace has exhausted pane identities",
            ));
        }
        if self.pane_count() >= MAX_PANES {
            return Err(action_error(
                "capacity",
                format!("workspace is limited to {MAX_PANES} panes"),
            ));
        }
        Ok(())
    }

    fn next_pane(&self) -> Pane {
        let next = self.next_pane;
        let pane_id = format!("p{next}");
        let session_id = format!("session-{next}");
        let endpoint = self.runtime.join(if next == 1 {
            "orbit.sock".into()
        } else {
            format!("{session_id}.sock")
        });
        Pane {
            id: pane_id,
            session: Session {
                id: session_id,
                endpoint,
            },
        }
    }

    fn pane_count(&self) -> usize {
        self.tabs.iter().map(|tab| tab.panes.len()).sum()
    }

    fn remember(&mut self, request_id: &str) {
        if self.recent_requests.len() == MAX_RECENT_REQUESTS {
            self.recent_requests.pop_front();
        }
        self.recent_requests.push_back(request_id.into());
    }
}

fn tab_number(value: &str) -> Option<usize> {
    let number = value.strip_prefix('t')?;
    if number.is_empty()
        || number.starts_with('0')
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    number.parse().ok().filter(|number| *number > 0)
}

fn directory_path(bytes: Vec<u8>) -> Result<PathBuf, Failure> {
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(action_error(
            "invalid-directory",
            "tab launch directory is empty or contains NUL",
        ));
    }
    if bytes.len() > MAX_DIRECTORY_BYTES {
        return Err(action_error(
            "invalid-directory",
            format!("tab launch directory exceeds {MAX_DIRECTORY_BYTES} bytes"),
        ));
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    if !path.is_absolute() {
        return Err(action_error(
            "invalid-directory",
            "tab launch directory must be absolute",
        ));
    }
    Ok(path)
}

pub(crate) fn validate_initial_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.as_os_str().as_bytes().len() > MAX_DIRECTORY_BYTES {
        return Err("initial tab launch directory must be a bounded absolute path".into());
    }
    if !path.is_dir() {
        return Err("initial tab launch directory is unavailable".into());
    }
    Ok(())
}

fn index_after_removal(selected: usize, removed: usize, remaining: usize) -> usize {
    if removed < selected {
        selected - 1
    } else {
        selected.min(remaining.saturating_sub(1))
    }
}

pub(crate) fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
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
    use crate::supervisor::temporary_directory;

    #[test]
    fn tab_directories_inherit_retarget_and_preserve_failed_spawn_state() {
        let root = temporary_directory();
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let mut workspace = Workspace::with_recovered_sessions(
            "/runtime".into(),
            first.clone(),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
        )
        .unwrap();
        let mut launches = Vec::new();
        workspace
            .dispatch("request-1", Action::CreateTab, |_, directory, _| {
                launches.push(directory.to_path_buf());
                Ok(())
            })
            .unwrap();
        workspace
            .dispatch(
                "request-1b",
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: first.as_os_str().as_bytes().to_vec(),
                },
                |_, directory, _| {
                    launches.push(directory.to_path_buf());
                    Ok(())
                },
            )
            .unwrap();
        workspace
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();
        assert_eq!(launches, [first.clone(), first.clone()]);
        assert_eq!(
            workspace.snapshot().tabs[1].directory,
            launches[0].as_os_str().as_bytes()
        );

        workspace
            .dispatch(
                "request-2",
                Action::SetTabDirectory {
                    tab: "t1".into(),
                    directory: second.as_os_str().as_bytes().to_vec(),
                },
                |_, _, _| unreachable!(),
            )
            .unwrap();
        workspace
            .dispatch(
                "request-3",
                Action::FocusId("t1".into()),
                |_, _, _| unreachable!(),
            )
            .unwrap();
        workspace
            .dispatch("request-4", Action::CreatePane, |_, directory, _| {
                assert_eq!(directory, second);
                Ok(())
            })
            .unwrap();

        let before = workspace.clone();
        assert_eq!(
            workspace
                .dispatch(
                    "request-5",
                    Action::SetTabDirectory {
                        tab: "t9".into(),
                        directory: second.as_os_str().as_bytes().to_vec(),
                    },
                    |_, _, _| unreachable!(),
                )
                .unwrap_err()
                .code,
            "unknown-tab"
        );
        assert_eq!(workspace, before);
        for id in ["tab-1", "t0", "t01", "t"] {
            let before = workspace.clone();
            assert_eq!(
                workspace
                    .dispatch(
                        "invalid-tab",
                        Action::SetTabDirectory {
                            tab: id.into(),
                            directory: second.as_os_str().as_bytes().to_vec(),
                        },
                        |_, _, _| unreachable!(),
                    )
                    .unwrap_err()
                    .code,
                "invalid-tab"
            );
            assert_eq!(workspace, before);
        }

        let file = root.join("file");
        std::fs::write(&file, "not a directory").unwrap();
        for (request, directory) in [
            ("invalid-empty", Vec::new()),
            ("invalid-relative", b"relative".to_vec()),
            ("invalid-nul", b"/tmp/eon\0bad".to_vec()),
            ("invalid-oversized", vec![b'x'; MAX_DIRECTORY_BYTES + 1]),
            (
                "invalid-missing",
                root.join("missing").as_os_str().as_bytes().to_vec(),
            ),
            ("invalid-file", file.as_os_str().as_bytes().to_vec()),
        ] {
            let before = workspace.clone();
            assert_eq!(
                workspace
                    .dispatch(
                        request,
                        Action::SetTabDirectory {
                            tab: "t1".into(),
                            directory,
                        },
                        |_, _, _| unreachable!(),
                    )
                    .unwrap_err()
                    .code,
                "invalid-directory"
            );
            assert_eq!(workspace, before);
        }

        std::fs::remove_dir(&second).unwrap();
        let before_panes = workspace.tabs[0].panes.len();
        assert_eq!(
            workspace
                .dispatch("request-6", Action::CreatePane, |_, directory, _| {
                    directory
                        .is_dir()
                        .then_some(())
                        .ok_or_else(|| "launch directory is unavailable".into())
                })
                .unwrap_err()
                .code,
            "session-start"
        );
        assert_eq!(workspace.tabs[0].panes.len(), before_panes);
        assert_eq!(workspace.tabs[0].directory, second);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn initial_workspace() -> Workspace {
        Workspace::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            vec![(
                1,
                Session {
                    id: "session-1".into(),
                    endpoint: "/runtime/orbit.sock".into(),
                },
            )],
        )
        .unwrap()
    }

    #[test]
    fn pending_tabs_start_picker_before_their_first_session_and_cancel_cleanly() {
        let mut fallback_attempts = Vec::new();
        let fallback =
            Workspace::pending("/runtime".into(), "/".into(), |session, _, picker_tab| {
                fallback_attempts.push((session.id.clone(), picker_tab.map(str::to_owned)));
                picker_tab
                    .is_none()
                    .then_some(())
                    .ok_or("picker unavailable".into())
            })
            .unwrap();
        assert_eq!(
            fallback_attempts,
            [
                ("directory-picker".into(), Some("t1".into())),
                ("session-1".into(), None),
            ]
        );
        assert_eq!(
            fallback.snapshot().tabs[0].selected_pane.as_deref(),
            Some("p1")
        );

        let mut launches = Vec::new();
        let mut workspace = Workspace::pending(
            "/runtime".into(),
            "/".into(),
            |session, directory, picker_tab| {
                launches.push((
                    session.id.clone(),
                    directory.to_path_buf(),
                    picker_tab.map(str::to_owned),
                ));
                Ok(())
            },
        )
        .unwrap();
        let initial = workspace.snapshot();
        assert_eq!(initial.active_tab, "t1");
        assert!(initial.tabs[0].panes.is_empty());
        assert!(initial.tabs[0].selected_pane.is_none());
        assert_eq!(
            launches,
            [("directory-picker".into(), "/".into(), Some("t1".into()))]
        );

        workspace
            .session_exited(DIRECTORY_PICKER_SESSION, |session, _, picker_tab| {
                assert!(picker_tab.is_none());
                launches.push((session.id.clone(), "/".into(), None));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            workspace.snapshot().tabs[0].selected_pane.as_deref(),
            Some("p1")
        );

        workspace
            .dispatch(
                "new-tab",
                Action::CreateTab,
                |session, directory, picker_tab| {
                    launches.push((
                        session.id.clone(),
                        directory.to_path_buf(),
                        picker_tab.map(str::to_owned),
                    ));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t2");
        assert!(workspace.snapshot().tabs[1].panes.is_empty());
        workspace
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();
        let cancelled = workspace.snapshot();
        assert_eq!(cancelled.active_tab, "t1");
        assert_eq!(cancelled.tabs.len(), 1);

        workspace
            .dispatch("new-tab-again", Action::CreateTab, |_, _, _| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t3");

        let mut shifted = initial_workspace();
        shifted
            .dispatch("shift-t2", Action::CreateTab, |_, _, _| Ok(()))
            .unwrap();
        shifted
            .dispatch(
                "commit-t2",
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/".to_vec(),
                },
                |_, _, _| Ok(()),
            )
            .unwrap();
        shifted
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();
        shifted
            .dispatch("shift-t3", Action::CreateTab, |_, _, _| Ok(()))
            .unwrap();
        shifted
            .session_exited("session-1", |_, _, _| unreachable!())
            .unwrap();
        shifted
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();
        let shifted = shifted.snapshot();
        assert_eq!(shifted.active_tab, "t2");
        assert_eq!(shifted.tabs.len(), 1);
    }

    #[test]
    fn ordered_topology_wraps_focus_and_rejects_invalid_actions_without_mutation() {
        let mut workspace = initial_workspace();
        let initial = workspace.clone();
        assert_eq!(
            workspace
                .dispatch("request-1", Action::CreatePane, |_, _, _| {
                    Err("offline".into())
                })
                .unwrap_err()
                .code,
            "session-start"
        );
        assert_eq!(workspace, initial);
        let mut start = |_: &Session, _: &Path, _: Option<&str>| Ok(());

        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let before = workspace.clone();
            assert_eq!(
                workspace
                    .dispatch("singleton", Action::Focus(direction), &mut start)
                    .unwrap_err()
                    .code,
                "unavailable"
            );
            assert_eq!(workspace, before);
        }

        workspace
            .dispatch("request-1", Action::CreatePane, &mut start)
            .unwrap();
        workspace
            .dispatch("request-2", Action::CreateTab, &mut start)
            .unwrap();
        workspace
            .dispatch(
                "request-2b",
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/".to_vec(),
                },
                &mut start,
            )
            .unwrap();
        workspace
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();

        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.tabs[workspace.active].id, "t2");
        assert_eq!(workspace.tabs[0].id, "t1");
        assert_eq!(
            workspace.tabs[0].panes[workspace.tabs[0].selected.unwrap()].id,
            "p2"
        );
        assert_eq!(workspace.tabs[1].id, "t2");
        assert_eq!(
            workspace.tabs[1].panes[workspace.tabs[1].selected.unwrap()].id,
            "p3"
        );
        let mut projection = workspace.snapshot();
        projection.tabs[0].panes[0].endpoint = b"/runtime/\x1b[2J\n.sock".to_vec();
        assert!(human(&projection).contains("endpoint=/runtime/\\u{1b}[2J\\n.sock\n"));

        workspace
            .dispatch("request-3", Action::FocusId("p1".into()), &mut start)
            .unwrap();
        for (request, direction, expected_tab, expected_pane) in [
            ("request-4", Direction::Down, "t1", "p2"),
            ("request-5", Direction::Up, "t1", "p1"),
            ("request-6", Direction::Up, "t1", "p2"),
            ("request-7", Direction::Down, "t1", "p1"),
            ("request-8", Direction::Left, "t2", "p3"),
            ("request-9", Direction::Left, "t1", "p1"),
            ("request-10", Direction::Right, "t2", "p3"),
            ("request-11", Direction::Right, "t1", "p1"),
        ] {
            workspace
                .dispatch(request, Action::Focus(direction), &mut start)
                .unwrap();
            let active = &workspace.tabs[workspace.active];
            assert_eq!(active.id, expected_tab);
            assert_eq!(active.panes[active.selected.unwrap()].id, expected_pane);
        }

        for (request, action, code) in [
            (
                "request-12",
                Action::FocusId("p1".into()),
                "already-focused",
            ),
            (
                "request-13",
                Action::FocusId("pane-stale".into()),
                "unknown-id",
            ),
            ("request-11", Action::Inspect, "duplicate-request"),
        ] {
            let before = workspace.clone();
            assert_eq!(
                workspace
                    .dispatch(request, action, &mut start)
                    .unwrap_err()
                    .code,
                code
            );
            assert_eq!(workspace, before);
        }

        let before = workspace.clone();
        let error = workspace
            .dispatch(
                "request-14",
                Action::FocusId("pane\nforged".into()),
                &mut start,
            )
            .unwrap_err();
        assert_eq!(error.code, "unknown-id");
        assert_eq!(
            error.detail,
            "pane\nforged is not a live tab or pane identity"
        );
        assert_eq!(
            failure_human(&error),
            "error unknown-id: pane\\nforged is not a live tab or pane identity\n"
        );
        assert_eq!(workspace, before);
    }

    #[test]
    fn session_exit_removes_panes_and_empty_tabs_without_reusing_ids() {
        let mut workspace = initial_workspace();
        let mut start = |_: &Session, _: &Path, _: Option<&str>| Ok(());

        workspace
            .dispatch("request-1", Action::CreatePane, &mut start)
            .unwrap();
        workspace
            .dispatch("request-2", Action::CreatePane, &mut start)
            .unwrap();
        workspace
            .dispatch("request-3", Action::FocusId("p2".into()), &mut start)
            .unwrap();
        workspace
            .session_exited("session-2", |_, _, _| Ok(()))
            .unwrap();

        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p3"));
        assert_eq!(
            snapshot.tabs[0]
                .panes
                .iter()
                .map(|pane| pane.id.as_str())
                .collect::<Vec<_>>(),
            ["p1", "p3"]
        );

        workspace
            .dispatch("request-4", Action::CreateTab, &mut start)
            .unwrap();
        workspace
            .dispatch(
                "request-4b",
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/".to_vec(),
                },
                &mut start,
            )
            .unwrap();
        workspace
            .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
            .unwrap();
        workspace
            .dispatch("request-5", Action::CreatePane, &mut start)
            .unwrap();
        workspace
            .session_exited("session-5", |_, _, _| Ok(()))
            .unwrap();
        assert_eq!(
            workspace.snapshot().tabs[1].selected_pane.as_deref(),
            Some("p4")
        );
        workspace
            .session_exited("session-4", |_, _, _| Ok(()))
            .unwrap();
        assert_eq!(workspace.snapshot().active_tab, "t1");

        let before = workspace.clone();
        assert_eq!(
            workspace
                .session_exited("session-4", |_, _, _| Ok(()))
                .unwrap_err()
                .code,
            "unknown-session"
        );
        assert_eq!(workspace, before);

        workspace
            .dispatch("request-6", Action::CreateTab, &mut start)
            .unwrap();
        workspace
            .dispatch(
                "request-6b",
                Action::SetTabDirectory {
                    tab: "t3".into(),
                    directory: b"/".to_vec(),
                },
                &mut start,
            )
            .unwrap();
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.active_tab, "t3");
        assert_eq!(snapshot.tabs[1].panes[0].id, "p6");
    }

    #[test]
    fn directory_picker_is_one_tab_bound_non_pane_session() {
        let mut workspace = initial_workspace();
        let mut launches = Vec::new();

        workspace
            .dispatch(
                "picker-1",
                Action::PickTabDirectory,
                |session, directory, picker_tab| {
                    launches.push((
                        session.clone(),
                        directory.to_path_buf(),
                        picker_tab.map(str::to_owned),
                    ));
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(
            launches,
            [(
                Session {
                    id: DIRECTORY_PICKER_SESSION.into(),
                    endpoint: "/runtime/pick.sock".into(),
                },
                "/".into(),
                Some("t1".into()),
            )]
        );
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.tabs[0].panes.len(), 1);
        assert_eq!(snapshot.directory_picker.unwrap().tab, "t1");

        let before = workspace.clone();
        assert_eq!(
            workspace
                .dispatch("picker-2", Action::PickTabDirectory, |_, _, _| Ok(()))
                .unwrap_err()
                .code,
            "picker-active"
        );
        assert_eq!(workspace, before);
        assert_eq!(
            workspace
                .dispatch(
                    "focus-while-picker",
                    Action::Focus(Direction::Right),
                    |_, _, _| Ok(()),
                )
                .unwrap_err()
                .code,
            "picker-active"
        );

        assert!(
            !workspace
                .session_exited(DIRECTORY_PICKER_SESSION, |_, _, _| unreachable!())
                .unwrap()
        );
        assert!(workspace.snapshot().directory_picker.is_none());

        workspace
            .dispatch("picker-3", Action::PickTabDirectory, |_, _, _| Ok(()))
            .unwrap();
        assert!(
            workspace
                .session_exited("session-1", |_, _, _| unreachable!())
                .unwrap()
        );
    }

    #[test]
    fn recovered_sessions_have_one_numeric_projection_without_reusing_ids() {
        let sessions = vec![
            (
                2,
                Session {
                    id: "session-2".into(),
                    endpoint: "/runtime/session-2.sock".into(),
                },
            ),
            (
                9,
                Session {
                    id: "session-9".into(),
                    endpoint: "/runtime/session-9.sock".into(),
                },
            ),
        ];
        let mut workspace =
            Workspace::with_recovered_sessions("/runtime".into(), "/".into(), sessions).unwrap();

        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.active_tab, "t1");
        assert_eq!(snapshot.tabs[0].selected_pane.as_deref(), Some("p2"));
        assert_eq!(
            snapshot.tabs[0]
                .panes
                .iter()
                .map(|pane| (pane.id.as_str(), pane.session.as_str()))
                .collect::<Vec<_>>(),
            [("p2", "session-2"), ("p9", "session-9")]
        );

        workspace
            .dispatch("request-1", Action::CreatePane, |_, _, _| Ok(()))
            .unwrap();
        workspace
            .dispatch("request-2", Action::CreateTab, |_, _, _| Ok(()))
            .unwrap();
        workspace
            .dispatch(
                "request-2b",
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/".to_vec(),
                },
                |_, _, _| Ok(()),
            )
            .unwrap();
        let snapshot = workspace.snapshot();
        assert_eq!(snapshot.tabs[0].panes[2].id, "p10");
        assert_eq!(snapshot.tabs[1].id, "t2");
        assert_eq!(snapshot.tabs[1].panes[0].id, "p11");

        assert!(
            Workspace::with_recovered_sessions(
                "/runtime".into(),
                "/".into(),
                vec![(
                    usize::MAX,
                    Session {
                        id: format!("session-{}", usize::MAX),
                        endpoint: "/runtime/overflow.sock".into(),
                    },
                )],
            )
            .is_err()
        );

        let mut exhausted = Workspace::with_recovered_sessions(
            "/runtime".into(),
            "/".into(),
            vec![(
                usize::MAX - 1,
                Session {
                    id: format!("session-{}", usize::MAX - 1),
                    endpoint: "/runtime/exhausted.sock".into(),
                },
            )],
        )
        .unwrap();
        let failure = exhausted
            .dispatch("request-1", Action::CreatePane, |_, _, _| {
                unreachable!("capacity check must prevent Session start")
            })
            .unwrap_err();
        assert_eq!(failure.code, "capacity");
    }

    #[test]
    fn json_preserves_opaque_endpoint_bytes() {
        let snapshot = Snapshot {
            active_tab: "t1".into(),
            tabs: vec![SnapshotTab {
                id: "t1".into(),
                directory: b"/tmp/eon-\xff".to_vec(),
                selected_pane: Some("pane-1".into()),
                panes: vec![
                    SnapshotPane {
                        id: "pane-1".into(),
                        session: "session-1".into(),
                        endpoint: b"/a".to_vec(),
                        live: true,
                    },
                    SnapshotPane {
                        id: "pane-2".into(),
                        session: "session-2".into(),
                        endpoint: b"/\xff".to_vec(),
                        live: false,
                    },
                ],
            }],
            directory_picker: None,
        };

        assert_eq!(
            json(&snapshot),
            "{\"active_tab\":\"t1\",\"tabs\":[{\"id\":\"t1\",\"directory\":[47,116,109,112,47,101,111,110,45,255],\"selected_pane\":\"pane-1\",\"panes\":[{\"id\":\"pane-1\",\"session\":\"session-1\",\"endpoint\":[47,97],\"live\":true},{\"id\":\"pane-2\",\"session\":\"session-2\",\"endpoint\":[47,255],\"live\":false}]}],\"directory_picker\":null}\n"
        );
    }
}
