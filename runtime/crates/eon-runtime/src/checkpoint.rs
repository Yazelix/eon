//! Shared EONR v1 topology; bytes grant no process, filesystem or registry authority.
// Preparatory owner for recovery and Update; its tests and the owner adapters'
// tests consume the codec until S4a (eon-t3fyd.7) calls the supervisor's
// export and S4b-1 (eon-t3fyd.8) its import.
#![cfg_attr(not(test), allow(dead_code))]

use eon_workspace_protocol::v51::{
    MAX_COLUMNS, MAX_DIRECTORY_BYTES, MAX_ENTRIES, MAX_ENTRY_ID_BYTES, MAX_LABEL_BYTES,
    MAX_PANE_NAME_BYTES, MAX_PANE_NAME_CHARS, MAX_SESSIONS, MAX_TABS, MAX_WORKSPACES,
    validate_pane_name,
};
use std::collections::HashSet;

pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
/// The EONR envelope version this build reads and writes.
pub(crate) const VERSION: u16 = 1;
const MAX_SECTIONS: usize = 32;
const MAX_ID_BYTES: usize = orbit_protocol::management::MAX_ID_BYTES;

// A consuming protocol's limit change must not silently change EONR version 1.
const _: () = {
    assert!(MAX_WORKSPACES == 32 && MAX_TABS == 64 && MAX_COLUMNS == 4);
    assert!(MAX_SESSIONS == 256 && MAX_ENTRIES == 32);
    assert!(MAX_DIRECTORY_BYTES == 4096 && MAX_ID_BYTES == 128);
    assert!(MAX_ENTRY_ID_BYTES == 64 && MAX_LABEL_BYTES == 128);
    assert!(MAX_PANE_NAME_BYTES == 320 && MAX_PANE_NAME_CHARS == 80);
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Header {
    pub namespace: String,
    pub generation: String,
    pub created_build: String,
    pub active_build: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Checkpoint {
    pub header: Header,
    pub core: Core,
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Section {
    pub name: String,
    pub required: bool,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Core {
    pub revision: u64,
    pub sessions: Vec<Session>,
    pub shown_session: Option<String>,
    pub terminal_id_high_water: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Session {
    pub id: String,
    pub persistent_id: Option<String>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tab {
    pub id: String,
    // EONR v1 has only the fixed launch-directory policy (wire tag 0).
    pub directory: Vec<u8>,
    pub columns: Vec<Column>,
    pub popups: Vec<Popup>,
    pub selected_pane: Option<String>,
    pub selected_popup: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Column {
    pub panes: Vec<Pane>,
    pub expanded: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pane {
    pub id: String,
    pub pinned_name: Option<String>,
    pub tag: Option<String>,
    pub run: Run,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Popup {
    pub id: String,
    pub entry: String,
    pub label: String,
    pub launch_directory: Vec<u8>,
    pub run: Run,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub kind: RunKind,
    pub terminal_id: String,
    pub run_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunKind {
    StartIntent,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid(&'static str),
    UnsupportedVersion(u16),
    UnknownRequiredSection(String),
    ScopeMismatch(&'static str),
    UnsupportedUpdateSchema {
        found: u16,
        floor: u16,
        current: u16,
    },
    /// Capture refused: the live state is not one an Update carries.
    Unsettled(&'static str),
    /// Restore refused before anything changed.
    Unrestorable(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(field) => write!(f, "invalid checkpoint {field}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported EONR checkpoint version {version}")
            }
            Self::UnknownRequiredSection(name) => {
                write!(f, "unknown required checkpoint section {name:?}")
            }
            Self::ScopeMismatch(field) => {
                write!(f, "checkpoint {field} does not match expected scope")
            }
            Self::UnsupportedUpdateSchema {
                found,
                floor,
                current,
            } => write!(
                f,
                "unsupported Update checkpoint schema {found}: this build accepts schemas \
                 {floor} through {current}; move the window's sessions to a new window by hand"
            ),
            Self::Unsettled(what) => write!(f, "cannot checkpoint: {what}"),
            Self::Unrestorable(what) => write!(f, "cannot restore checkpoint: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl Header {
    fn validate(&self) -> Result<()> {
        path("namespace", self.namespace.as_bytes())?;
        for (field, value) in [
            ("generation", &self.generation),
            ("created_build", &self.created_build),
            ("active_build", &self.active_build),
        ] {
            if !crate::generation::valid_generation(value) {
                return Err(Error::Invalid(field));
            }
        }
        Ok(())
    }

    fn require_scope(&self, expected: &Self) -> Result<()> {
        expected.validate()?;
        // Recovery distinguishes active-build refusal from damage of its own file.
        for (field, actual, expected) in [
            ("active_build", &self.active_build, &expected.active_build),
            ("namespace", &self.namespace, &expected.namespace),
            ("generation", &self.generation, &expected.generation),
            (
                "created_build",
                &self.created_build,
                &expected.created_build,
            ),
        ] {
            if actual != expected {
                return Err(Error::ScopeMismatch(field));
            }
        }
        Ok(())
    }
}

impl Checkpoint {
    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Encoder(Vec::new());
        self.write(&mut out)?;
        Ok(out.0)
    }

    pub(super) fn decode(bytes: &[u8], expected: &Header, understood: &[&str]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Invalid("size"));
        }
        let mut input = Decoder::new(bytes);
        if input.take(4)? != b"EONR" {
            return Err(Error::Invalid("magic"));
        }
        let version = input.u16()?;
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let header = Header {
            namespace: input.text("namespace", MAX_DIRECTORY_BYTES)?,
            generation: input.fixed_identity()?,
            created_build: input.fixed_identity()?,
            active_build: input.fixed_identity()?,
        };
        header.validate()?;
        header.require_scope(expected)?;
        let core_bytes = input.blob()?;
        let sections = input.list("sections", MAX_SECTIONS, |input| {
            Ok(Section {
                name: input.text("section name", MAX_ID_BYTES)?,
                required: input.boolean()?,
                payload: input.blob()?.to_vec(),
            })
        })?;
        input.finish()?;
        validate_sections(&sections)?;
        for section in &sections {
            if section.required && !understood.contains(&section.name.as_str()) {
                return Err(Error::UnknownRequiredSection(section.name.clone()));
            }
        }
        let mut input = Decoder::new(core_bytes);
        let core = Core {
            revision: input.u64()?,
            sessions: input.list("sessions", MAX_WORKSPACES, read_session)?,
            shown_session: input.optional_text("shown session", MAX_ID_BYTES)?,
            terminal_id_high_water: input.u64()?,
        };
        input.finish()?;
        let checkpoint = Self {
            header,
            core,
            sections,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    fn validate(&self) -> Result<()> {
        self.header.validate()?;
        validate_sections(&self.sections)?;
        count("sessions", self.core.sessions.len(), MAX_WORKSPACES)?;
        let mut ids = HashSet::new();
        let mut persistent_ids = HashSet::new();
        let mut terminals = HashSet::new();
        for session in &self.core.sessions {
            local_id(&mut ids, &session.id, "s", "session id")?;
            if let Some(id) = &session.persistent_id {
                crate::session_registry::validate_id(id)
                    .map_err(|_| Error::Invalid("persistent id"))?;
                unique(&mut persistent_ids, id, "duplicate persistent id")?;
            }
            count("tabs", session.tabs.len(), MAX_TABS)?;
            reference("active tab", session.active_tab.as_deref(), |id| {
                session.tabs.iter().any(|tab| tab.id == id)
            })?;
            for tab in &session.tabs {
                local_id(&mut ids, &tab.id, "t", "tab id")?;
                path("directory", &tab.directory)?;
                count("columns", tab.columns.len(), MAX_COLUMNS)?;
                for column in &tab.columns {
                    count("panes", column.panes.len(), MAX_SESSIONS)?;
                    reference("expanded pane", Some(&column.expanded), |id| {
                        column.panes.iter().any(|pane| pane.id == id)
                    })?;
                    for pane in &column.panes {
                        local_id(&mut ids, &pane.id, "p", "pane id")?;
                        for value in [&pane.pinned_name, &pane.tag].into_iter().flatten() {
                            validate_pane_name(value).map_err(|_| Error::Invalid("pin/tag"))?;
                            if value.is_empty() || value.trim() != value {
                                return Err(Error::Invalid("pin/tag"));
                            }
                        }
                        validate_run(&pane.run, self.core.terminal_id_high_water, &mut terminals)?;
                    }
                }
                count("popups", tab.popups.len(), MAX_ENTRIES)?;
                let mut entries = HashSet::new();
                for popup in &tab.popups {
                    validate_popup(
                        popup,
                        self.core.terminal_id_high_water,
                        &mut ids,
                        &mut entries,
                        &mut terminals,
                    )?;
                }
                reference("selected pane", tab.selected_pane.as_deref(), |id| {
                    tab.columns.iter().any(|column| column.expanded == id)
                })?;
                reference("selected popup", tab.selected_popup.as_deref(), |id| {
                    tab.popups.iter().any(|popup| popup.id == id)
                })?;
            }
        }
        reference("shown session", self.core.shown_session.as_deref(), |id| {
            self.core.sessions.iter().any(|session| session.id == id)
        })
    }

    fn write(&self, out: &mut Encoder) -> Result<()> {
        out.bytes(b"EONR")?;
        out.u16(VERSION)?;
        out.text(&self.header.namespace)?;
        for id in [
            &self.header.generation,
            &self.header.created_build,
            &self.header.active_build,
        ] {
            out.bytes(id.as_bytes())?;
        }
        let length_at = out.0.len();
        out.u32(0)?;
        let core_at = out.0.len();
        out.u64(self.core.revision)?;
        out.list(&self.core.sessions, |out, session| {
            out.text(&session.id)?;
            out.optional_text(session.persistent_id.as_deref())?;
            out.list(&session.tabs, |out, tab| {
                out.text(&tab.id)?;
                out.u8(0)?;
                out.raw(&tab.directory)?;
                out.list(&tab.columns, |out, column| {
                    out.list(&column.panes, |out, pane| {
                        out.text(&pane.id)?;
                        out.optional_text(pane.pinned_name.as_deref())?;
                        out.optional_text(pane.tag.as_deref())?;
                        write_run(out, &pane.run)
                    })?;
                    out.text(&column.expanded)
                })?;
                out.list(&tab.popups, write_popup)?;
                out.optional_text(tab.selected_pane.as_deref())?;
                out.optional_text(tab.selected_popup.as_deref())
            })?;
            out.optional_text(session.active_tab.as_deref())
        })?;
        out.optional_text(self.core.shown_session.as_deref())?;
        out.u64(self.core.terminal_id_high_water)?;
        let length =
            u32::try_from(out.0.len() - core_at).map_err(|_| Error::Invalid("core size"))?;
        out.0[length_at..core_at].copy_from_slice(&length.to_le_bytes());
        out.list(&self.sections, |out, section| {
            out.text(&section.name)?;
            out.u8(u8::from(section.required))?;
            out.blob(&section.payload)
        })
    }
}

fn text(field: &'static str, value: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

pub(crate) fn path(field: &'static str, bytes: &[u8]) -> Result<()> {
    if !bytes.starts_with(b"/") || bytes.len() > MAX_DIRECTORY_BYTES || bytes.contains(&0) {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

fn count(field: &'static str, length: usize, maximum: usize) -> Result<()> {
    if length > maximum {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

fn unique<'a>(seen: &mut HashSet<&'a str>, value: &'a str, field: &'static str) -> Result<()> {
    if !seen.insert(value) {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

fn number(value: &str, prefix: &str, field: &'static str) -> Result<u64> {
    let suffix = value.strip_prefix(prefix).ok_or(Error::Invalid(field))?;
    if !matches!(suffix.as_bytes().first(), Some(b'1'..=b'9'))
        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::Invalid(field));
    }
    suffix.parse().map_err(|_| Error::Invalid(field))
}

fn local_id<'a>(
    seen: &mut HashSet<&'a str>,
    id: &'a str,
    prefix: &str,
    field: &'static str,
) -> Result<()> {
    number(id, prefix, field)?;
    unique(seen, id, "duplicate local id")
}

fn reference(
    field: &'static str,
    value: Option<&str>,
    belongs: impl FnOnce(&str) -> bool,
) -> Result<()> {
    if let Some(value) = value
        && !belongs(value)
    {
        return Err(Error::Invalid(field));
    }
    Ok(())
}

fn validate_run<'a>(run: &'a Run, high_water: u64, seen: &mut HashSet<&'a str>) -> Result<()> {
    let terminal = number(&run.terminal_id, "terminal-", "terminal id")?;
    if terminal > high_water {
        return Err(Error::Invalid("terminal high-water"));
    }
    text("run id", &run.run_id, MAX_ID_BYTES)?;
    unique(seen, &run.terminal_id, "duplicate terminal/run ownership")?;
    count("terminal slots", seen.len(), MAX_SESSIONS)
}

/// One popup slot's shared-record rules, for a core popup and for every
/// popup an extension section carries beside the core.
fn validate_popup<'a>(
    popup: &'a Popup,
    high_water: u64,
    ids: &mut HashSet<&'a str>,
    entries: &mut HashSet<&'a str>,
    terminals: &mut HashSet<&'a str>,
) -> Result<()> {
    local_id(ids, &popup.id, "u", "popup id")?;
    crate::managed_environment::validate_popup_id(&popup.entry)
        .map_err(|_| Error::Invalid("popup entry"))?;
    unique(entries, &popup.entry, "duplicate popup entry")?;
    text("popup label", &popup.label, MAX_LABEL_BYTES)?;
    if popup.label.chars().any(char::is_control) {
        return Err(Error::Invalid("popup label"));
    }
    path("popup directory", &popup.launch_directory)?;
    validate_run(&popup.run, high_water, terminals)
}

fn validate_sections(sections: &[Section]) -> Result<()> {
    count("sections", sections.len(), MAX_SECTIONS)?;
    let mut names = HashSet::new();
    for section in sections {
        text("section name", &section.name, MAX_ID_BYTES)?;
        unique(&mut names, &section.name, "duplicate section name")?;
        count("section size", section.payload.len(), MAX_BYTES)?;
    }
    Ok(())
}

fn write_run(out: &mut Encoder, run: &Run) -> Result<()> {
    out.u8(match run.kind {
        RunKind::StartIntent => 0,
        RunKind::Confirmed => 1,
    })?;
    out.text(&run.terminal_id)?;
    out.text(&run.run_id)
}

fn read_run(input: &mut Decoder<'_>) -> Result<Run> {
    Ok(Run {
        kind: match input.u8()? {
            0 => RunKind::StartIntent,
            1 => RunKind::Confirmed,
            _ => return Err(Error::Invalid("run kind")),
        },
        terminal_id: input.text("terminal id", MAX_ID_BYTES)?,
        run_id: input.text("run id", MAX_ID_BYTES)?,
    })
}

fn write_popup(out: &mut Encoder, popup: &Popup) -> Result<()> {
    out.text(&popup.id)?;
    out.text(&popup.entry)?;
    out.text(&popup.label)?;
    out.raw(&popup.launch_directory)?;
    write_run(out, &popup.run)
}

fn read_popup(input: &mut Decoder<'_>) -> Result<Popup> {
    Ok(Popup {
        id: input.text("popup id", MAX_ID_BYTES)?,
        entry: input.text("popup entry", MAX_ENTRY_ID_BYTES)?,
        label: input.text("popup label", MAX_LABEL_BYTES)?,
        launch_directory: input.raw(MAX_DIRECTORY_BYTES)?.to_vec(),
        run: read_run(input)?,
    })
}

fn read_session(input: &mut Decoder<'_>) -> Result<Session> {
    Ok(Session {
        id: input.text("session id", MAX_ID_BYTES)?,
        persistent_id: input.optional_text("persistent id", MAX_ID_BYTES)?,
        tabs: input.list("tabs", MAX_TABS, read_tab)?,
        active_tab: input.optional_text("active tab", MAX_ID_BYTES)?,
    })
}

fn read_tab(input: &mut Decoder<'_>) -> Result<Tab> {
    let id = input.text("tab id", MAX_ID_BYTES)?;
    if input.u8()? != 0 {
        return Err(Error::Invalid("launch-directory policy"));
    }
    let directory = input.raw(MAX_DIRECTORY_BYTES)?.to_vec();
    let columns = input.list("columns", MAX_COLUMNS, |input| {
        let length = input.slot_count("panes", MAX_SESSIONS)?;
        Ok(Column {
            panes: input.items(length, |input| {
                Ok(Pane {
                    id: input.text("pane id", MAX_ID_BYTES)?,
                    pinned_name: input.optional_text("pane name", MAX_PANE_NAME_BYTES)?,
                    tag: input.optional_text("pane tag", MAX_PANE_NAME_BYTES)?,
                    run: read_run(input)?,
                })
            })?,
            expanded: input.text("expanded pane", MAX_ID_BYTES)?,
        })
    })?;
    let length = input.slot_count("popups", MAX_ENTRIES)?;
    let popups = input.items(length, read_popup)?;
    Ok(Tab {
        id,
        directory,
        columns,
        popups,
        selected_pane: input.optional_text("selected pane", MAX_ID_BYTES)?,
        selected_popup: input.optional_text("selected popup", MAX_ID_BYTES)?,
    })
}

struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_BYTES - self.0.len() {
            return Err(Error::Invalid("size"));
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn u8(&mut self, value: u8) -> Result<()> {
        self.bytes(&[value])
    }
    fn u16(&mut self, value: u16) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }
    fn u32(&mut self, value: u32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }
    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.u16(u16::try_from(bytes.len()).map_err(|_| Error::Invalid("string size"))?)?;
        self.bytes(bytes)
    }
    fn text(&mut self, value: &str) -> Result<()> {
        self.raw(value.as_bytes())
    }
    fn blob(&mut self, bytes: &[u8]) -> Result<()> {
        self.u32(u32::try_from(bytes.len()).map_err(|_| Error::Invalid("blob size"))?)?;
        self.bytes(bytes)
    }
    fn optional_raw(&mut self, value: Option<&[u8]>) -> Result<()> {
        self.u8(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.raw(value)?;
        }
        Ok(())
    }
    fn optional_text(&mut self, value: Option<&str>) -> Result<()> {
        self.u8(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.text(value)?;
        }
        Ok(())
    }
    fn list<T>(
        &mut self,
        values: &[T],
        mut write: impl FnMut(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        self.u16(u16::try_from(values.len()).map_err(|_| Error::Invalid("array size"))?)?;
        for value in values {
            write(self, value)?;
        }
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    slots: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, slots: 0 }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let bytes = self
            .bytes
            .get(..length)
            .ok_or(Error::Invalid("truncated data"))?;
        self.bytes = &self.bytes[length..];
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid("presence/boolean tag")),
        }
    }
    fn raw(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = usize::from(self.u16()?);
        count("string size", length, maximum)?;
        self.take(length)
    }
    fn text(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        let value = std::str::from_utf8(self.raw(maximum)?).map_err(|_| Error::Invalid(field))?;
        text(field, value, maximum)?;
        Ok(value.into())
    }
    fn optional_raw(&mut self, maximum: usize) -> Result<Option<Vec<u8>>> {
        if self.boolean()? {
            Ok(Some(self.raw(maximum)?.to_vec()))
        } else {
            Ok(None)
        }
    }
    fn optional_text(&mut self, field: &'static str, maximum: usize) -> Result<Option<String>> {
        if self.boolean()? {
            Ok(Some(self.text(field, maximum)?))
        } else {
            Ok(None)
        }
    }
    fn fixed_identity(&mut self) -> Result<String> {
        std::str::from_utf8(self.take(35)?)
            .map(str::to_owned)
            .map_err(|_| Error::Invalid("build/lifetime identity"))
    }
    fn blob(&mut self) -> Result<&'a [u8]> {
        let length = usize::try_from(self.u32()?).map_err(|_| Error::Invalid("blob size"))?;
        count("blob size", length, MAX_BYTES)?;
        self.take(length)
    }
    fn count(&mut self, field: &'static str, maximum: usize) -> Result<usize> {
        let length = usize::from(self.u16()?);
        count(field, length, maximum.min(self.bytes.len()))?;
        Ok(length)
    }
    fn slot_count(&mut self, field: &'static str, maximum: usize) -> Result<usize> {
        let length = self.count(field, maximum)?;
        self.slots = self
            .slots
            .checked_add(length)
            .ok_or(Error::Invalid("terminal slots"))?;
        count("terminal slots", self.slots, MAX_SESSIONS)?;
        Ok(length)
    }
    fn items<T>(
        &mut self,
        length: usize,
        mut read: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::with_capacity(length);
        for _ in 0..length {
            items.push(read(self)?);
        }
        Ok(items)
    }
    fn list<T>(
        &mut self,
        field: &'static str,
        maximum: usize,
        read: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let length = self.count(field, maximum)?;
        self.items(length, read)
    }
    fn finish(self) -> Result<()> {
        if !self.bytes.is_empty() {
            return Err(Error::Invalid("trailing bytes"));
        }
        Ok(())
    }
}

pub(super) mod recovery;
pub(crate) mod update;

#[cfg(test)]
mod tests;
