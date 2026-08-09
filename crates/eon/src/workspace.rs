use std::{collections::VecDeque, path::PathBuf};

const MAX_TABS: usize = 64;
const MAX_PANES: usize = 256;
const MAX_RECENT_REQUESTS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) endpoint: PathBuf,
    live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: Session,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tab {
    id: String,
    panes: Vec<Pane>,
    selected: usize,
}

#[derive(Debug)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug)]
pub(crate) enum Action {
    Inspect,
    CreateTab,
    CreatePane,
    FocusId(String),
    Focus(Direction),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Workspace {
    runtime: PathBuf,
    tabs: Vec<Tab>,
    active: usize,
    recent_requests: VecDeque<String>,
}

impl Workspace {
    pub(crate) fn human(&self) -> String {
        let active = self.tabs.get(self.active).map_or("", |tab| tab.id.as_str());
        let mut output = format!("active {active}\n");
        for tab in &self.tabs {
            output.push_str(&format!(
                "tab {} selected={} active={}\n",
                tab.id,
                tab.panes[tab.selected].id,
                tab.id == active
            ));
            for pane in &tab.panes {
                output.push_str(&format!(
                    "  pane {} session={} live={} endpoint={}\n",
                    pane.id,
                    pane.session.id,
                    pane.session.live,
                    pane.session.endpoint.display()
                ));
            }
        }
        output
    }

    pub(crate) fn json(&self) -> String {
        let active = self.tabs.get(self.active).map_or("", |tab| tab.id.as_str());
        let mut output = format!("{{\"active_tab\":\"{}\",\"tabs\":[", json_escape(active));
        for (tab_index, tab) in self.tabs.iter().enumerate() {
            if tab_index != 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"selected_pane\":\"{}\",\"panes\":[",
                json_escape(&tab.id),
                json_escape(&tab.panes[tab.selected].id)
            ));
            for (pane_index, pane) in tab.panes.iter().enumerate() {
                if pane_index != 0 {
                    output.push(',');
                }
                output.push_str(&format!(
                    "{{\"id\":\"{}\",\"session\":\"{}\",\"endpoint\":\"{}\",\"live\":{}}}",
                    json_escape(&pane.id),
                    json_escape(&pane.session.id),
                    json_escape(&pane.session.endpoint.to_string_lossy()),
                    pane.session.live
                ));
            }
            output.push_str("]}");
        }
        output.push_str("]}\n");
        output
    }
}

#[derive(Debug)]
pub(crate) struct ActionError {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl ActionError {
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub(crate) fn human(&self) -> String {
        format!("error {}: {}\n", self.code, self.detail)
    }

    pub(crate) fn json(&self) -> String {
        format!(
            "{{\"error\":{{\"code\":\"{}\",\"detail\":\"{}\"}}}}\n",
            self.code,
            json_escape(&self.detail)
        )
    }
}

impl Workspace {
    pub(crate) fn empty(runtime: PathBuf) -> Self {
        Self {
            runtime,
            tabs: Vec::new(),
            active: 0,
            recent_requests: VecDeque::new(),
        }
    }

    pub(crate) fn with_initial_session(runtime: PathBuf, endpoint: PathBuf) -> Self {
        let mut workspace = Self::empty(runtime);
        let (tab, mut pane) = workspace.next_tab_and_pane();
        pane.session.endpoint = endpoint;
        workspace.tabs.push(Tab {
            id: tab,
            panes: vec![pane],
            selected: 0,
        });
        workspace
    }

    pub(crate) fn dispatch(
        &mut self,
        request_id: &str,
        action: Action,
        mut start: impl FnMut(&Session) -> Result<(), String>,
    ) -> Result<(), ActionError> {
        if self.recent_requests.iter().any(|seen| seen == request_id) {
            return Err(ActionError::new(
                "duplicate-request",
                format!("request {request_id} was already accepted"),
            ));
        }

        match action {
            Action::Inspect => {}
            Action::CreateTab => self.create_tab(&mut start)?,
            Action::CreatePane => self.create_pane(&mut start)?,
            Action::FocusId(id) => self.focus_id(&id)?,
            Action::Focus(direction) => self.focus_direction(direction)?,
        }
        self.remember(request_id);
        Ok(())
    }

    pub(crate) fn session_exited(&mut self, session_id: &str) -> Result<(), ActionError> {
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.session.id == session_id {
                    pane.session.live = false;
                    return Ok(());
                }
            }
        }
        Err(ActionError::new(
            "unknown-session",
            format!("session {session_id} is not in the workspace"),
        ))
    }

    fn create_tab(
        &mut self,
        start: &mut impl FnMut(&Session) -> Result<(), String>,
    ) -> Result<(), ActionError> {
        if self.tabs.len() >= MAX_TABS {
            return Err(ActionError::new(
                "capacity",
                format!("workspace is limited to {MAX_TABS} tabs"),
            ));
        }
        self.check_pane_capacity()?;
        let (tab_id, pane) = self.next_tab_and_pane();
        start(&pane.session).map_err(|detail| ActionError::new("session-start", detail))?;
        self.tabs.push(Tab {
            id: tab_id,
            panes: vec![pane],
            selected: 0,
        });
        self.active = self.tabs.len() - 1;
        Ok(())
    }

    fn create_pane(
        &mut self,
        start: &mut impl FnMut(&Session) -> Result<(), String>,
    ) -> Result<(), ActionError> {
        if self.tabs.is_empty() {
            return Err(ActionError::new(
                "invalid-transition",
                "a pane requires an active tab",
            ));
        }
        self.check_pane_capacity()?;
        let pane = self.next_pane();
        start(&pane.session).map_err(|detail| ActionError::new("session-start", detail))?;
        let active = &mut self.tabs[self.active];
        active.panes.push(pane);
        active.selected = active.panes.len() - 1;
        Ok(())
    }

    fn focus_id(&mut self, id: &str) -> Result<(), ActionError> {
        if let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == id) {
            self.ensure_selected_session_live(tab_index)?;
            if tab_index == self.active {
                return Err(ActionError::new(
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
                if !self.tabs[tab_index].panes[pane_index].session.live {
                    return Err(ActionError::new(
                        "missing-session",
                        format!("pane {id} has no live Orbit session"),
                    ));
                }
                if tab_index == self.active && pane_index == self.tabs[tab_index].selected {
                    return Err(ActionError::new(
                        "already-focused",
                        format!("pane {id} is already selected"),
                    ));
                }
                self.active = tab_index;
                self.tabs[tab_index].selected = pane_index;
                return Ok(());
            }
        }

        Err(ActionError::new(
            "unknown-id",
            format!("{id} is not a live tab or pane identity"),
        ))
    }

    fn focus_direction(&mut self, direction: Direction) -> Result<(), ActionError> {
        if self.tabs.is_empty() {
            return Err(ActionError::new(
                "invalid-transition",
                "the workspace has no active tab",
            ));
        }
        match direction {
            Direction::Left => {
                let target = self
                    .active
                    .checked_sub(1)
                    .ok_or_else(|| ActionError::new("unavailable", "no tab exists to the left"))?;
                self.ensure_selected_session_live(target)?;
                self.active = target;
            }
            Direction::Right => {
                let target = self.active + 1;
                if target >= self.tabs.len() {
                    return Err(ActionError::new(
                        "unavailable",
                        "no tab exists to the right",
                    ));
                }
                self.ensure_selected_session_live(target)?;
                self.active = target;
            }
            Direction::Up => {
                let target = self.tabs[self.active]
                    .selected
                    .checked_sub(1)
                    .ok_or_else(|| ActionError::new("unavailable", "no pane exists above"))?;
                self.ensure_pane_live(self.active, target)?;
                self.tabs[self.active].selected = target;
            }
            Direction::Down => {
                let target = self.tabs[self.active].selected + 1;
                if target >= self.tabs[self.active].panes.len() {
                    return Err(ActionError::new("unavailable", "no pane exists below"));
                }
                self.ensure_pane_live(self.active, target)?;
                self.tabs[self.active].selected = target;
            }
        }
        Ok(())
    }

    fn check_pane_capacity(&self) -> Result<(), ActionError> {
        if self.pane_count() >= MAX_PANES {
            return Err(ActionError::new(
                "capacity",
                format!("workspace is limited to {MAX_PANES} panes"),
            ));
        }
        Ok(())
    }

    fn ensure_selected_session_live(&self, tab: usize) -> Result<(), ActionError> {
        self.ensure_pane_live(tab, self.tabs[tab].selected)
    }

    fn ensure_pane_live(&self, tab: usize, pane: usize) -> Result<(), ActionError> {
        let pane = &self.tabs[tab].panes[pane];
        if pane.session.live {
            Ok(())
        } else {
            Err(ActionError::new(
                "missing-session",
                format!("pane {} has no live Orbit session", pane.id),
            ))
        }
    }

    fn next_tab_and_pane(&self) -> (String, Pane) {
        let tab = format!("tab-{}", self.tabs.len() + 1);
        (tab, self.next_pane())
    }

    fn next_pane(&self) -> Pane {
        let next = self.pane_count() + 1;
        let pane_id = format!("pane-{next}");
        let session_id = format!("session-{next}");
        let endpoint = if next == 1 {
            self.runtime.join("orbit.sock")
        } else {
            self.runtime.join(format!("{session_id}.sock"))
        };
        Pane {
            id: pane_id,
            session: Session {
                id: session_id,
                endpoint,
                live: true,
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

fn json_escape(value: &str) -> String {
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
    use std::{cell::RefCell, path::PathBuf};

    #[test]
    fn ordered_topology_rejects_invalid_actions_without_mutation() {
        let mut workspace = Workspace::empty(PathBuf::from("/runtime"));
        let empty = workspace.clone();
        assert_eq!(
            workspace
                .dispatch("request-1", Action::CreateTab, |_| Err("offline".into()))
                .unwrap_err()
                .code,
            "session-start"
        );
        assert_eq!(workspace, empty);
        let started = RefCell::new(Vec::new());
        let mut start = |session: &Session| {
            started.borrow_mut().push(session.clone());
            Ok(())
        };

        workspace
            .dispatch("request-1", Action::CreateTab, &mut start)
            .unwrap();
        workspace
            .dispatch("request-2", Action::CreatePane, &mut start)
            .unwrap();
        workspace
            .dispatch("request-3", Action::CreateTab, &mut start)
            .unwrap();

        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.tabs[workspace.active].id, "tab-2");
        assert_eq!(workspace.tabs[0].id, "tab-1");
        assert_eq!(
            workspace.tabs[0].panes[workspace.tabs[0].selected].id,
            "pane-2"
        );
        assert_eq!(workspace.tabs[1].id, "tab-2");
        assert_eq!(
            workspace.tabs[1].panes[workspace.tabs[1].selected].id,
            "pane-3"
        );
        assert_eq!(started.borrow().len(), 3);
        assert!(
            started
                .borrow()
                .windows(2)
                .all(|pair| pair[0].endpoint != pair[1].endpoint)
        );

        workspace
            .dispatch("request-4", Action::FocusId("pane-1".into()), &mut start)
            .unwrap();
        workspace
            .dispatch("request-5", Action::Focus(Direction::Down), &mut start)
            .unwrap();
        workspace
            .dispatch("request-6", Action::Focus(Direction::Right), &mut start)
            .unwrap();
        workspace
            .dispatch("request-7", Action::Focus(Direction::Left), &mut start)
            .unwrap();
        workspace
            .dispatch("request-8", Action::Focus(Direction::Up), &mut start)
            .unwrap();
        assert_eq!(
            workspace.tabs[0].panes[workspace.tabs[0].selected].id,
            "pane-1"
        );

        for (request, action, code) in [
            ("request-9", Action::Focus(Direction::Up), "unavailable"),
            ("request-10", Action::Focus(Direction::Left), "unavailable"),
            (
                "request-11",
                Action::FocusId("pane-1".into()),
                "already-focused",
            ),
            (
                "request-12",
                Action::FocusId("pane-stale".into()),
                "unknown-id",
            ),
            ("request-8", Action::Inspect, "duplicate-request"),
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

        workspace
            .dispatch("request-13", Action::FocusId("pane-2".into()), &mut start)
            .unwrap();
        workspace.session_exited("session-2").unwrap();
        for (request, id) in [("request-14", "pane-2"), ("request-15", "tab-1")] {
            let before = workspace.clone();
            assert_eq!(
                workspace
                    .dispatch(request, Action::FocusId(id.into()), &mut start)
                    .unwrap_err()
                    .code,
                "missing-session"
            );
            assert_eq!(workspace, before);
        }
    }
}
