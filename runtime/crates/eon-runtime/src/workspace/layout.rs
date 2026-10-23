//! The workspace's durable structure. Terminal contents and commands stay with Orbit.
use super::*;
use eon_workspace_protocol::v13::{Response, SessionRun, WorkspaceSnapshot, encode_response};
use std::{
    collections::HashSet,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

const FORMAT: u32 = 1;
const MAX_LAYOUT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    format: u32,
    workspace: String,
    stopped: bool,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: usize,
    next_pane: usize,
    next_session: usize,
    next_popup: usize,
}

pub(super) mod path_bytes {
    use super::*;
    pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().as_bytes().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(|bytes| OsString::from_vec(bytes).into())
    }
}

impl Workspace {
    pub(crate) fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.tabs.iter().flat_map(|tab| {
            tab.panes()
                .map(|pane| &pane.session)
                .chain(tab.popups.iter().map(|popup| &popup.session))
        })
    }

    pub(crate) fn reconcile_runs(&mut self, live: &[(String, String)]) {
        for tab in &mut self.tabs {
            for session in tab
                .columns
                .iter_mut()
                .flat_map(|column| &mut column.panes)
                .map(|pane| &mut pane.session)
                .chain(tab.popups.iter_mut().map(|popup| &mut popup.session))
            {
                session.live = live
                    .iter()
                    .any(|(id, run)| *id == session.id && *run == session.run);
            }
        }
    }

    pub(crate) fn named_snapshot(&self, workspace: &str) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            workspace: workspace.into(),
            snapshot: self.snapshot(),
            runs: self
                .sessions()
                .map(|session| SessionRun {
                    session: session.id.clone(),
                    run: session.run.clone(),
                    live: session.live,
                })
                .collect(),
        }
    }

    /// Recovery never replays a picker or an unfinished Start operation.
    pub(crate) fn retire_transient_layout(&mut self) {
        let persistent: HashSet<_> = self
            .catalog
            .entries
            .iter()
            .filter(|entry| entry.keep_alive)
            .map(|entry| entry.id.as_str())
            .collect();
        for tab in &mut self.tabs {
            // Keep the stopped slot until an explicit dismiss/reopen, including its unfinished
            // Ready intent. A child already spawned may still publish a record after recovery.
            for popup in &mut tab.popups {
                if !persistent.contains(popup.entry.as_str()) {
                    popup.session.live = false;
                }
            }
            tab.pending = false;
        }
    }

    pub(crate) fn save_layout(&self, path: &Path, workspace: &str) -> Result<(), String> {
        let value = Layout {
            format: FORMAT,
            workspace: workspace.into(),
            stopped: self.stopped,
            tabs: self.tabs.clone(),
            active: self.active,
            next_tab: self.next_tab,
            next_pane: self.next_pane,
            next_session: self.next_session,
            next_popup: self.next_popup,
        };
        value.validate(&self.runtime, &self.catalog)?;
        encode_response(&Response::Workspace(self.named_snapshot(workspace)))
            .map_err(|error| format!("unrepresentable workspace layout: {error}"))?;
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| format!("cannot encode workspace layout: {error}"))?;
        if bytes.len() > MAX_LAYOUT_BYTES {
            return Err("workspace layout exceeds its size limit".into());
        }
        let parent = path.parent().ok_or("layout has no state directory")?;
        super::super::supervisor::validate_private_directory(parent)?;
        let temporary = parent.join(format!(
            ".layout-{}",
            super::super::supervisor::request_id()
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
            .map_err(|error| format!("cannot commit workspace layout {}: {error}", path.display()))
    }

    pub(crate) fn persisted_stopped(path: &Path, workspace: &str) -> Result<bool, String> {
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Ok(false);
        }
        Ok(read_layout(path, workspace)?.stopped)
    }

    pub(crate) fn load_layout(
        path: &Path,
        workspace: &str,
        runtime: PathBuf,
        home: Option<PathBuf>,
        catalog: PopupCatalog,
    ) -> Result<Self, String> {
        let value = read_layout(path, workspace)?;
        value.validate(&runtime, &catalog)?;
        let restored = Self {
            runtime,
            home,
            catalog,
            tabs: value.tabs,
            active: value.active,
            next_tab: value.next_tab,
            next_pane: value.next_pane,
            next_session: value.next_session,
            next_popup: value.next_popup,
            recent_requests: VecDeque::new(),
            stopped: value.stopped,
        };
        encode_response(&Response::Workspace(restored.named_snapshot(workspace)))
            .map_err(|error| format!("invalid workspace layout: {error}"))?;
        Ok(restored)
    }
}

fn read_layout(path: &Path, workspace: &str) -> Result<Layout, String> {
    let parent = path.parent().ok_or("layout has no state directory")?;
    super::super::supervisor::validate_private_directory(parent)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("cannot read workspace layout {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != super::super::supervisor::effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > MAX_LAYOUT_BYTES as u64
    {
        return Err(
            "workspace layout must be an owned mode-0600 file within the size limit".into(),
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_LAYOUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_LAYOUT_BYTES {
        return Err("workspace layout exceeds its size limit".into());
    }
    let value: Layout = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid workspace layout: {error}"))?;
    if value.format != FORMAT || value.workspace != workspace {
        return Err(
            "unknown workspace layout format or wrong workspace identity; tasks were left running"
                .into(),
        );
    }
    Ok(value)
}

fn counter(id: &str, prefix: &str, next: usize) -> Result<(), String> {
    if id
        .strip_prefix(prefix)
        .and_then(canonical_number)
        .is_none_or(|number| number >= next)
    {
        return Err(format!("invalid or reused workspace identity {id}"));
    }
    Ok(())
}

fn directory(path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute() || bytes.len() > MAX_DIRECTORY_BYTES || bytes.contains(&0) {
        return Err("invalid directory in workspace layout".into());
    }
    Ok(())
}

impl Layout {
    fn validate(&self, runtime: &Path, catalog: &PopupCatalog) -> Result<(), String> {
        if self.tabs.is_empty()
            || self.tabs.len() > MAX_TABS
            || self.active >= self.tabs.len()
            || [
                self.next_tab,
                self.next_pane,
                self.next_session,
                self.next_popup,
            ]
            .contains(&0)
        {
            return Err("invalid workspace layout bounds".into());
        }
        let mut identities = HashSet::new();
        let mut sessions = HashSet::new();
        let mut runs = HashSet::new();
        let mut check_session = |session: &Session| -> Result<(), String> {
            let picker = is_directory_picker_session(&session.id);
            counter(
                &session.id,
                if picker {
                    "directory-picker-"
                } else {
                    "session-"
                },
                if picker {
                    self.next_popup
                } else {
                    self.next_session
                },
            )?;
            let filename = if picker {
                format!(
                    "k{}.sock",
                    session.id.strip_prefix("directory-picker-").unwrap()
                )
            } else if session.id == "session-1" {
                "orbit.sock".into()
            } else {
                format!("{}.sock", session.id)
            };
            if session.endpoint != runtime.join(filename)
                || !sessions.insert(session.id.clone())
                || session.run.is_empty()
                || session.run.len() > 128
                || !runs.insert(session.run.clone())
                || !session
                    .run
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            {
                return Err("invalid, duplicate or misplaced terminal identity in layout".into());
            }
            Ok(())
        };
        for tab in &self.tabs {
            counter(&tab.id, "t", self.next_tab)?;
            if !identities.insert(&tab.id)
                || tab.columns.len() > MAX_COLUMNS
                || match tab.selected_column {
                    Some(index) => index >= tab.columns.len(),
                    None => !tab.columns.is_empty(),
                }
            {
                return Err("invalid tab selection in workspace layout".into());
            }
            directory(&tab.directory)?;
            for column in &tab.columns {
                if column.panes.is_empty() || column.expanded >= column.panes.len() {
                    return Err("invalid column selection in workspace layout".into());
                }
                for pane in &column.panes {
                    counter(&pane.id, "p", self.next_pane)?;
                    if !identities.insert(&pane.id) {
                        return Err("duplicate pane identity".into());
                    }
                    check_session(&pane.session)?;
                }
            }
            for popup in &tab.popups {
                counter(&popup.id, "u", self.next_popup)?;
                if !identities.insert(&popup.id)
                    || !catalog.entries.iter().any(|entry| entry.id == popup.entry)
                {
                    return Err("duplicate popup identity or unavailable popup definition".into());
                }
                directory(&popup.launch_directory)?;
                check_session(&popup.session)?;
            }
            if tab
                .selected_popup
                .as_ref()
                .is_some_and(|id| !tab.popups.iter().any(|popup| &popup.id == id))
            {
                return Err("invalid popup selection in workspace layout".into());
            }
        }
        if sessions.len() > MAX_SESSIONS {
            return Err("workspace layout exceeds the session limit".into());
        }
        Ok(())
    }
}
