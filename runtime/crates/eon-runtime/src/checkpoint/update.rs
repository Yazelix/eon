//! Update's required `update` section on the shared EONR core.
//!
//! The section carries the logical Runtime state an in-place Update keeps
//! beyond the core topology: transient popups and the pending pair, the
//! allocator, replay rings, recency, terminal extras and attention. It carries
//! no unsettled work: pending answers, helper samples, naming retries, queued
//! notices and in-flight requests settle before capture, and the attention
//! cursor, focus and viewing state are not kept.
//!
//! Stamps are unsigned distances from the capture anchor: a past fact is its
//! age before it and the next poll is its distance after it (0 when due), so a
//! future past fact and a second zero cannot be written.
//!
//! Schema admission (C22, release window): `SCHEMA` advances when the carried
//! representation or meaning changes, independently of EONW; a change to the
//! Orbit management ProgramRecord encoding, which attention records reuse,
//! counts. Update accepts every schema from `SCHEMA_FLOOR`, the schema of the
//! preceding release, through `SCHEMA`. Every field below arrived at schema 1;
//! a later field reads as `if schema >= N { read } else { neutral value }`,
//! with the neutral value justified beside it, and a change of meaning needs a
//! typed conversion. Each supported schema keeps one committed byte fixture. A
//! release moves the floor to its own schema and drops the older fixtures.
//! Crash recovery never opts into this window.

use super::{
    Checkpoint, Core, Decoder, Encoder, Error, Header, Popup, Result, Run, RunKind, Section, count,
    number, path, read_popup, text, unique, validate_popup, write_popup,
};
use crate::workspace::{IDS_PER_SUPERVISOR, MAX_RECENT_REQUESTS};
use eon_workspace_protocol::v47::{
    FILES_ENTRY, MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS, MAX_DETAIL_BYTES,
    MAX_DIRECTORY_BYTES, MAX_ENTRIES, MAX_SESSIONS, MAX_TABS, MAX_WORKSPACES, Sidebar,
};
use orbit_protocol::management::{
    self, Foreground, MAX_ID_BYTES, MAX_MESSAGE_BYTES, MAX_PROGRAM_RECORDS,
    MAX_TERMINAL_TITLE_BYTES, ProgramProcesses, ProgramRecord, ProgramStatus, ServerMessage,
};
use std::collections::{HashMap, HashSet};

pub(crate) const SECTION: &str = "update";
/// The carried-state schema this build writes.
pub(crate) const SCHEMA: u16 = 1;
/// The oldest schema this build admits: the preceding release's.
pub(crate) const SCHEMA_FLOOR: u16 = 1;
/// No age or due distance exceeds about 146 years, so a restore adding the
/// elapsed gap cannot overflow u64 nanoseconds.
pub(crate) const MAX_STAMP_NS: u64 = 1 << 62;
const RANGE_IDS: u64 = IDS_PER_SUPERVISOR as u64;
const MAX_TAB_EXTRAS: usize = MAX_WORKSPACES * MAX_TABS;

// A consuming owner's limit change must not silently change what schema 1
// admits.
const _: () = {
    assert!(IDS_PER_SUPERVISOR == 1 << 20 && MAX_RECENT_REQUESTS == 256);
    assert!(MAX_MESSAGE_BYTES == 4096 && MAX_PROGRAM_RECORDS == 256);
    assert!(MAX_TERMINAL_TITLE_BYTES == 64 && MAX_DETAIL_BYTES == 1024);
    assert!(MAX_COMMAND_ARGUMENTS == 256 && MAX_COMMAND_ARGUMENT_BYTES == 32 * 1024);
};

/// The independently held identity an Update target expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    pub header: Header,
    pub range: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Update {
    pub captured_monotonic_ns: u64,
    pub allocator: Allocator,
    pub registry_available: bool,
    pub session_extras: Vec<SessionExtra>,
    pub tab_extras: Vec<TabExtra>,
    pub recent_inspections: Vec<String>,
    pub recent_mutations: Vec<String>,
    pub recent_sessions: Vec<String>,
    pub terminal_extras: Vec<TerminalExtra>,
    pub attention: Option<Attention>,
    pub supervisor: Supervisor,
}

/// The supervisor's own facts, which the target cannot derive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Supervisor {
    /// The lifetime's start in Unix milliseconds, as inspection reports it.
    pub started_at_ms: u64,
    /// The launch command (`eon run -- CMD`) a pending first tab keeps for
    /// its first pane; empty once a pane used it. Arguments may be empty.
    pub initial_command: Vec<Vec<u8>>,
    /// terminal-1's natural exit status, the window's eventual exit status.
    pub initial_status: Option<u8>,
}

/// `next_session` is Runtime's `next_workspace`; `next_terminal` its
/// `next_session`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Allocator {
    pub range: u64,
    pub next_session: u64,
    pub next_tab: u64,
    pub next_pane: u64,
    pub next_terminal: u64,
    pub next_popup: u64,
    pub first_reserved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionExtra {
    pub session: String,
    pub home: Option<Vec<u8>>,
    pub registration_error: Option<RegistrationError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegistrationError {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabExtra {
    pub tab: String,
    pub pending: bool,
    /// Every popup of the tab, core and extension, in the tab's order.
    pub popup_order: Vec<String>,
    pub extra_popups: Vec<ExtraPopup>,
    /// The tab's full selection; the core keeps only a durable one.
    pub selected_popup: Option<String>,
    pub covered_popup: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtraPopup {
    pub role: Role,
    pub popup: Popup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Transient,
    PendingPair,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalExtra {
    pub terminal: String,
    pub default_shell: bool,
    pub named: Option<Named>,
}

/// An acknowledged `AttachAllow` name; `process_start` is an opaque same-boot
/// token from the platform owner, never reinterpreted here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Named {
    pub process_id: u32,
    pub process_start: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attention {
    pub terminals: Vec<AttentionTerminal>,
    pub recovered: Vec<Recovered>,
    pub next_poll_due_ns: u64,
    pub sidebar: Sidebar,
    pub epoch_age_ns: u64,
    pub epoch_wall_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recovered {
    pub run_id: String,
    pub baseline: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttentionTerminal {
    pub terminal: String,
    pub run_id: String,
    pub update: u64,
    pub count: u16,
    pub background: u8,
    pub foreground: Foreground,
    /// The terminal title, empty when it has none.
    pub name: String,
    pub records: Vec<AttentionRecord>,
    pub omitted: Vec<AttentionRecord>,
    pub replied_age_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttentionRecord {
    pub record: ProgramRecord,
    pub changed_age_ns: u64,
    pub raise: Option<Raise>,
    pub blocked_notified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Raise {
    pub age_ns: u64,
    pub changed: u64,
    pub seen: bool,
    pub notified: bool,
}

/// Encodes a complete Update checkpoint: the core, the required `update`
/// section, then `optional` sections, which must not be required or `update`.
pub(crate) fn encode(
    header: Header,
    core: Core,
    update: &Update,
    optional: Vec<Section>,
) -> Result<Vec<u8>> {
    if optional
        .iter()
        .any(|section| section.required || section.name == SECTION)
    {
        return Err(Error::Invalid("optional sections"));
    }
    let mut checkpoint = Checkpoint {
        header,
        core,
        sections: Vec::with_capacity(optional.len() + 1),
    };
    checkpoint.sections.push(Section {
        name: SECTION.into(),
        required: true,
        payload: Vec::new(),
    });
    checkpoint.sections.extend(optional);
    checkpoint.validate()?;
    update.validate(&checkpoint.core)?;
    checkpoint.sections[0].payload = update.write()?;
    checkpoint.encode()
}

/// Decodes and validates a complete Update checkpoint for `scope`, admitting
/// every schema in the release window.
pub(crate) fn decode(bytes: &[u8], scope: &Scope) -> Result<(Checkpoint, Update)> {
    let checkpoint = Checkpoint::decode(bytes, &scope.header, &[SECTION])?;
    let mut sections = checkpoint
        .sections
        .iter()
        .filter(|section| section.name == SECTION);
    let section = sections.next().ok_or(Error::Invalid("update section"))?;
    if !section.required {
        return Err(Error::Invalid("update section"));
    }
    let update = Update::read(&section.payload)?;
    update.validate(&checkpoint.core)?;
    if update.allocator.range != scope.range {
        return Err(Error::ScopeMismatch("identity range"));
    }
    Ok((checkpoint, update))
}

impl Allocator {
    /// The partition's first local number and its exclusive limit.
    fn bounds(&self) -> Result<(u64, u64)> {
        let first = self
            .range
            .checked_mul(RANGE_IDS)
            .and_then(|base| base.checked_add(1))
            .ok_or(Error::Invalid("identity range"))?;
        let limit = first
            .checked_add(RANGE_IDS)
            .filter(|limit| usize::try_from(*limit).is_ok())
            .ok_or(Error::Invalid("identity range"))?;
        Ok((first, limit))
    }
}

/// One pane or popup terminal, core or extension, in window order.
pub(crate) struct Slot<'a> {
    pub run: &'a Run,
    /// Its tab is pending (only the tab's Files pair).
    pub pending: bool,
}

impl Update {
    /// Every slot of the window in its one order: session by session, tab by
    /// tab, each tab's panes column by column, then its popups in
    /// `popup_order`. Terminal extras follow this order.
    pub(crate) fn slots<'a>(&'a self, core: &'a Core) -> Result<Vec<Slot<'a>>> {
        let tabs = core.sessions.iter().flat_map(|session| &session.tabs);
        let mut slots = Vec::new();
        for (tab, extra) in tabs.zip(&self.tab_extras) {
            if extra.tab != tab.id {
                return Err(Error::Invalid("tab extras"));
            }
            let pending = extra.pending;
            for pane in tab.columns.iter().flat_map(|column| &column.panes) {
                slots.push(Slot {
                    run: &pane.run,
                    pending,
                });
            }
            for id in &extra.popup_order {
                let run = tab
                    .popups
                    .iter()
                    .chain(extra.extra_popups.iter().map(|extra| &extra.popup))
                    .find(|popup| &popup.id == id)
                    .map(|popup| &popup.run)
                    .ok_or(Error::Invalid("popup order"))?;
                slots.push(Slot { run, pending });
            }
        }
        Ok(slots)
    }

    /// Every rule beyond the core's, over the combined model. `core` must
    /// already be valid.
    pub(crate) fn validate(&self, core: &Core) -> Result<()> {
        let high_water = core.terminal_id_high_water;
        let allocator = &self.allocator;
        let (first, limit) = allocator.bounds()?;
        for counter in [
            allocator.next_session,
            allocator.next_tab,
            allocator.next_pane,
            allocator.next_popup,
        ] {
            if counter < first || counter > limit {
                return Err(Error::Invalid("allocator counter"));
            }
        }
        if allocator.next_terminal.checked_sub(1) != Some(high_water) {
            return Err(Error::Invalid("terminal counter"));
        }
        let below = |id: &str, prefix: &str, next: u64, field: &'static str| -> Result<()> {
            if number(id, prefix, field)? >= next {
                return Err(Error::Invalid(field));
            }
            Ok(())
        };

        if core.sessions.is_empty() || core.shown_session.is_none() {
            return Err(Error::Invalid("shown session"));
        }
        count("session extras", self.session_extras.len(), MAX_WORKSPACES)?;
        if self.session_extras.len() != core.sessions.len() {
            return Err(Error::Invalid("session extras"));
        }
        let tabs: usize = core.sessions.iter().map(|session| session.tabs.len()).sum();
        count("tab extras", self.tab_extras.len(), MAX_TAB_EXTRAS)?;
        if self.tab_extras.len() != tabs {
            return Err(Error::Invalid("tab extras"));
        }

        // The core's local IDs and terminals, which extension popups join.
        let mut ids = HashSet::new();
        let mut terminals = HashSet::new();
        for (session, extra) in core.sessions.iter().zip(&self.session_extras) {
            if extra.session != session.id {
                return Err(Error::Invalid("session extras"));
            }
            if session.tabs.is_empty() || session.active_tab.is_none() {
                return Err(Error::Invalid("active tab"));
            }
            below(&session.id, "s", allocator.next_session, "session id")?;
            ids.insert(session.id.as_str());
            if let Some(home) = &extra.home {
                path("home", home)?;
            }
            if let Some(error) = &extra.registration_error {
                text("registration error code", &error.code, MAX_ID_BYTES)?;
                text("registration error detail", &error.detail, MAX_DETAIL_BYTES)?;
                if session.persistent_id.is_some() {
                    return Err(Error::Invalid("registration error"));
                }
            }
            if session.persistent_id.is_some() && !self.registry_available {
                return Err(Error::Invalid("registry availability"));
            }
            for tab in &session.tabs {
                below(&tab.id, "t", allocator.next_tab, "tab id")?;
                ids.insert(tab.id.as_str());
                for pane in tab.columns.iter().flat_map(|column| &column.panes) {
                    below(&pane.id, "p", allocator.next_pane, "pane id")?;
                    ids.insert(pane.id.as_str());
                    terminals.insert(pane.run.terminal_id.as_str());
                }
                for popup in &tab.popups {
                    below(&popup.id, "u", allocator.next_popup, "popup id")?;
                    ids.insert(popup.id.as_str());
                    terminals.insert(popup.run.terminal_id.as_str());
                }
            }
        }

        let tabs = core.sessions.iter().flat_map(|session| &session.tabs);
        for (tab, extra) in tabs.zip(&self.tab_extras) {
            if extra.tab != tab.id {
                return Err(Error::Invalid("tab extras"));
            }
            if tab.selected_pane.is_some() == tab.columns.is_empty() {
                return Err(Error::Invalid("selected pane"));
            }
            count(
                "popups",
                tab.popups.len() + extra.extra_popups.len(),
                MAX_ENTRIES,
            )?;
            let mut entries = tab
                .popups
                .iter()
                .map(|popup| popup.entry.as_str())
                .collect();
            for extra_popup in &extra.extra_popups {
                validate_popup(
                    &extra_popup.popup,
                    high_water,
                    &mut ids,
                    &mut entries,
                    &mut terminals,
                )?;
                below(&extra_popup.popup.id, "u", allocator.next_popup, "popup id")?;
            }
            validate_pending(tab, extra)?;
            validate_popup_order(&tab.popups, extra)?;
            validate_selection(&tab.popups, extra, tab.selected_popup.as_deref())?;
        }

        let slots = self.slots(core)?;
        let mut runs = HashSet::new();
        for slot in &slots {
            if slot.run.kind != RunKind::Confirmed {
                return Err(Error::Invalid("start intent"));
            }
            unique(&mut runs, &slot.run.run_id, "duplicate run id")?;
        }
        // The owner reserves terminal-1 only while moving the next terminal
        // from 1 to 2 (`IdAllocator::popup_session`); a window without slots
        // must not hand terminal-1 out twice.
        if allocator.first_reserved
            && (allocator.next_terminal < 2 || terminals.contains("terminal-1"))
        {
            return Err(Error::Invalid("reserved terminal"));
        }

        count("terminal extras", self.terminal_extras.len(), MAX_SESSIONS)?;
        if self.terminal_extras.len() != slots.len() {
            return Err(Error::Invalid("terminal extras"));
        }
        for (extra, slot) in self.terminal_extras.iter().zip(&slots) {
            if extra.terminal != slot.run.terminal_id {
                return Err(Error::Invalid("terminal extras"));
            }
            if extra
                .named
                .is_some_and(|named| named.process_id == 0 || named.process_start == 0)
            {
                return Err(Error::Invalid("attach name"));
            }
        }

        self.validate_recent(core)?;
        if let Some(attention) = &self.attention {
            attention.validate(&slots)?;
        }
        self.supervisor.validate()
    }

    fn validate_recent(&self, core: &Core) -> Result<()> {
        let mut requests = HashSet::new();
        for ring in [&self.recent_inspections, &self.recent_mutations] {
            count("recent requests", ring.len(), MAX_RECENT_REQUESTS)?;
            for id in ring {
                text("request id", id, MAX_ID_BYTES)?;
                unique(&mut requests, id, "duplicate request id")?;
            }
        }
        let mut recent = HashSet::new();
        for id in &self.recent_sessions {
            if !core.sessions.iter().any(|session| &session.id == id) {
                return Err(Error::Invalid("recent sessions"));
            }
            unique(&mut recent, id, "recent sessions")?;
        }
        if recent.len() != core.sessions.len()
            || self.recent_sessions.first() != core.shown_session.as_ref()
        {
            return Err(Error::Invalid("recent sessions"));
        }
        Ok(())
    }

    /// The section payload, unchecked; `encode` validates first.
    fn write(&self) -> Result<Vec<u8>> {
        let mut out = Encoder(Vec::new());
        out.u16(SCHEMA)?;
        out.u64(self.captured_monotonic_ns)?;
        let allocator = &self.allocator;
        for value in [
            allocator.range,
            allocator.next_session,
            allocator.next_tab,
            allocator.next_pane,
            allocator.next_terminal,
            allocator.next_popup,
        ] {
            out.u64(value)?;
        }
        out.u8(u8::from(allocator.first_reserved))?;
        out.u8(u8::from(self.registry_available))?;
        out.list(&self.session_extras, |out, extra| {
            out.text(&extra.session)?;
            out.optional_raw(extra.home.as_deref())?;
            out.u8(u8::from(extra.registration_error.is_some()))?;
            if let Some(error) = &extra.registration_error {
                out.text(&error.code)?;
                out.text(&error.detail)?;
            }
            Ok(())
        })?;
        out.list(&self.tab_extras, |out, extra| {
            out.text(&extra.tab)?;
            out.u8(u8::from(extra.pending))?;
            out.list(&extra.popup_order, |out, id| out.text(id))?;
            out.list(&extra.extra_popups, |out, extra| {
                out.u8(match extra.role {
                    Role::Transient => 0,
                    Role::PendingPair => 1,
                })?;
                write_popup(out, &extra.popup)
            })?;
            out.optional_text(extra.selected_popup.as_deref())?;
            out.optional_text(extra.covered_popup.as_deref())
        })?;
        for ring in [
            &self.recent_inspections,
            &self.recent_mutations,
            &self.recent_sessions,
        ] {
            out.list(ring, |out, id| out.text(id))?;
        }
        out.list(&self.terminal_extras, |out, extra| {
            out.text(&extra.terminal)?;
            out.u8(u8::from(extra.default_shell))?;
            out.u8(u8::from(extra.named.is_some()))?;
            if let Some(named) = extra.named {
                out.u32(named.process_id)?;
                out.u64(named.process_start)?;
            }
            Ok(())
        })?;
        out.u8(u8::from(self.attention.is_some()))?;
        if let Some(attention) = &self.attention {
            attention.write(&mut out)?;
        }
        self.supervisor.write(&mut out)?;
        Ok(out.0)
    }

    fn read(payload: &[u8]) -> Result<Self> {
        let mut input = Decoder::new(payload);
        let schema = input.u16()?;
        if !(SCHEMA_FLOOR..=SCHEMA).contains(&schema) {
            return Err(Error::UnsupportedUpdateSchema {
                found: schema,
                floor: SCHEMA_FLOOR,
                current: SCHEMA,
            });
        }
        // Every field below arrived at schema 1.
        let captured_monotonic_ns = input.u64()?;
        let allocator = Allocator {
            range: input.u64()?,
            next_session: input.u64()?,
            next_tab: input.u64()?,
            next_pane: input.u64()?,
            next_terminal: input.u64()?,
            next_popup: input.u64()?,
            first_reserved: input.boolean()?,
        };
        let registry_available = input.boolean()?;
        let session_extras = input.list("session extras", MAX_WORKSPACES, |input| {
            Ok(SessionExtra {
                session: input.text("session id", MAX_ID_BYTES)?,
                home: input.optional_raw(MAX_DIRECTORY_BYTES)?,
                registration_error: if input.boolean()? {
                    Some(RegistrationError {
                        code: input.text("registration error code", MAX_ID_BYTES)?,
                        detail: input.text("registration error detail", MAX_DETAIL_BYTES)?,
                    })
                } else {
                    None
                },
            })
        })?;
        let tab_extras = input.list("tab extras", MAX_TAB_EXTRAS, |input| {
            Ok(TabExtra {
                tab: input.text("tab id", MAX_ID_BYTES)?,
                pending: input.boolean()?,
                popup_order: input.list("popup order", MAX_ENTRIES, |input| {
                    input.text("popup id", MAX_ID_BYTES)
                })?,
                extra_popups: input.list("extra popups", MAX_ENTRIES, |input| {
                    Ok(ExtraPopup {
                        role: match input.u8()? {
                            0 => Role::Transient,
                            1 => Role::PendingPair,
                            _ => return Err(Error::Invalid("popup role")),
                        },
                        popup: read_popup(input)?,
                    })
                })?,
                selected_popup: input.optional_text("selected popup", MAX_ID_BYTES)?,
                covered_popup: input.optional_text("covered popup", MAX_ID_BYTES)?,
            })
        })?;
        let mut ring = |maximum| {
            input.list("recent requests", maximum, |input| {
                input.text("request id", MAX_ID_BYTES)
            })
        };
        let recent_inspections = ring(MAX_RECENT_REQUESTS)?;
        let recent_mutations = ring(MAX_RECENT_REQUESTS)?;
        let recent_sessions = input.list("recent sessions", MAX_WORKSPACES, |input| {
            input.text("recent sessions", MAX_ID_BYTES)
        })?;
        let terminal_extras = input.list("terminal extras", MAX_SESSIONS, |input| {
            Ok(TerminalExtra {
                terminal: input.text("terminal id", MAX_ID_BYTES)?,
                default_shell: input.boolean()?,
                named: if input.boolean()? {
                    Some(Named {
                        process_id: input.u32()?,
                        process_start: input.u64()?,
                    })
                } else {
                    None
                },
            })
        })?;
        let attention = if input.boolean()? {
            Some(Attention::read(&mut input)?)
        } else {
            None
        };
        let supervisor = Supervisor::read(&mut input)?;
        input.finish()?;
        Ok(Self {
            captured_monotonic_ns,
            allocator,
            registry_available,
            session_extras,
            tab_extras,
            recent_inspections,
            recent_mutations,
            recent_sessions,
            terminal_extras,
            attention,
            supervisor,
        })
    }
}

/// A pending tab is pane-free and shows exactly its Files pair; only a
/// pending tab holds a pair.
fn validate_pending(tab: &super::Tab, extra: &TabExtra) -> Result<()> {
    if !extra.pending {
        if extra
            .extra_popups
            .iter()
            .any(|popup| popup.role == Role::PendingPair)
        {
            return Err(Error::Invalid("pending pair"));
        }
        return Ok(());
    }
    match extra.extra_popups.as_slice() {
        [pair]
            if tab.columns.is_empty()
                && tab.popups.is_empty()
                && pair.role == Role::PendingPair
                && pair.popup.entry == FILES_ENTRY =>
        {
            if extra.selected_popup.as_deref() != Some(pair.popup.id.as_str()) {
                return Err(Error::Invalid("pending tab selection"));
            }
            Ok(())
        }
        _ => Err(Error::Invalid("pending tab")),
    }
}

/// `popup_order` merges core and extension popups, each in its own order.
fn validate_popup_order(core: &[Popup], extra: &TabExtra) -> Result<()> {
    let mut core = core.iter().peekable();
    let mut extension = extra.extra_popups.iter().peekable();
    for id in &extra.popup_order {
        if core.next_if(|popup| &popup.id == id).is_none()
            && extension.next_if(|popup| &popup.popup.id == id).is_none()
        {
            return Err(Error::Invalid("popup order"));
        }
    }
    if core.next().is_some() || extension.next().is_some() {
        return Err(Error::Invalid("popup order"));
    }
    Ok(())
}

/// The full selection agrees with the core's durable one, a transient exists
/// only while it is shown (the owner stops it before selecting anything else,
/// `displaced_transient`), and a cover is a durable popup beneath it.
fn validate_selection(core: &[Popup], extra: &TabExtra, core_selected: Option<&str>) -> Result<()> {
    let durable = |id: &str| core.iter().any(|popup| popup.id == id);
    let transient = |id: &str| {
        extra
            .extra_popups
            .iter()
            .any(|popup| popup.role == Role::Transient && popup.popup.id == id)
    };
    let selected = extra.selected_popup.as_deref();
    if let Some(id) = selected
        && !durable(id)
        && !extra.extra_popups.iter().any(|popup| popup.popup.id == id)
    {
        return Err(Error::Invalid("selected popup"));
    }
    if core_selected != selected.filter(|id| durable(id)) {
        return Err(Error::Invalid("selected popup"));
    }
    if extra
        .extra_popups
        .iter()
        .any(|popup| popup.role == Role::Transient && selected != Some(popup.popup.id.as_str()))
    {
        return Err(Error::Invalid("hidden transient"));
    }
    if let Some(covered) = extra.covered_popup.as_deref()
        && (!durable(covered) || !selected.is_some_and(transient))
    {
        return Err(Error::Invalid("covered popup"));
    }
    Ok(())
}

impl Attention {
    fn validate(&self, slots: &[Slot<'_>]) -> Result<()> {
        let mut live = HashMap::new();
        for slot in slots {
            live.insert(slot.run.terminal_id.as_str(), slot);
        }
        stamp(self.next_poll_due_ns)?;
        stamp(self.epoch_age_ns)?;
        count("attention terminals", self.terminals.len(), MAX_SESSIONS)?;
        let mut seen = HashSet::new();
        for terminal in &self.terminals {
            match live.get(terminal.terminal.as_str()) {
                Some(slot) if slot.run.run_id == terminal.run_id && !slot.pending => {}
                _ => return Err(Error::Invalid("attention terminal")),
            }
            unique(&mut seen, &terminal.terminal, "attention terminal")?;
            terminal.validate()?;
        }
        count("recovered runs", self.recovered.len(), MAX_SESSIONS)?;
        let mut recovered = HashSet::new();
        for run in &self.recovered {
            if !slots.iter().any(|slot| slot.run.run_id == run.run_id) {
                return Err(Error::Invalid("recovered run"));
            }
            unique(&mut recovered, &run.run_id, "recovered run")?;
        }
        Ok(())
    }

    fn write(&self, out: &mut Encoder) -> Result<()> {
        out.list(&self.terminals, |out, terminal| terminal.write(out))?;
        out.list(&self.recovered, |out, run| {
            out.text(&run.run_id)?;
            out.u8(u8::from(run.baseline.is_some()))?;
            if let Some(baseline) = run.baseline {
                out.u64(baseline)?;
            }
            Ok(())
        })?;
        out.u64(self.next_poll_due_ns)?;
        out.u8(match self.sidebar {
            Sidebar::Open => 0,
            Sidebar::Closed => 1,
            Sidebar::Off => 2,
        })?;
        out.u64(self.epoch_age_ns)?;
        out.u64(self.epoch_wall_ms)
    }

    fn read(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            terminals: input.list("attention terminals", MAX_SESSIONS, AttentionTerminal::read)?,
            recovered: input.list("recovered runs", MAX_SESSIONS, |input| {
                Ok(Recovered {
                    run_id: input.text("run id", MAX_ID_BYTES)?,
                    baseline: if input.boolean()? {
                        Some(input.u64()?)
                    } else {
                        None
                    },
                })
            })?,
            next_poll_due_ns: input.u64()?,
            sidebar: match input.u8()? {
                0 => Sidebar::Open,
                1 => Sidebar::Closed,
                2 => Sidebar::Off,
                _ => return Err(Error::Invalid("sidebar")),
            },
            epoch_age_ns: input.u64()?,
            epoch_wall_ms: input.u64()?,
        })
    }
}

impl AttentionTerminal {
    fn validate(&self) -> Result<()> {
        if self.count > MAX_PROGRAM_RECORDS
            || self.records.len() + self.omitted.len() > usize::from(self.count)
            || (self.count == 0 && self.background != 0)
        {
            return Err(Error::Invalid("attention record counts"));
        }
        count("terminal title", self.name.len(), MAX_TERMINAL_TITLE_BYTES)?;
        stamp(self.replied_age_ns)?;
        let mut created = HashSet::new();
        for record in self.records.iter().chain(&self.omitted) {
            if !created.insert(record.record.created) {
                return Err(Error::Invalid("duplicate attention record"));
            }
            if read_record_frame(&record_frame(&record.record)?)? != record.record {
                return Err(Error::Invalid("attention record"));
            }
            stamp(record.changed_age_ns)?;
            if let Some(raise) = record.raise {
                stamp(raise.age_ns)?;
            }
        }
        Ok(())
    }

    fn write(&self, out: &mut Encoder) -> Result<()> {
        out.text(&self.terminal)?;
        out.text(&self.run_id)?;
        out.u64(self.update)?;
        out.u16(self.count)?;
        out.u8(self.background)?;
        out.u8(match self.foreground {
            Foreground::Unknown => 0,
            Foreground::Child => 1,
            Foreground::Other => 2,
        })?;
        out.text(&self.name)?;
        for records in [&self.records, &self.omitted] {
            out.list(records, |out, record| {
                out.blob(&record_frame(&record.record)?)?;
                out.u64(record.changed_age_ns)?;
                out.u8(u8::from(record.raise.is_some()))?;
                if let Some(raise) = record.raise {
                    out.u64(raise.age_ns)?;
                    out.u64(raise.changed)?;
                    out.u8(u8::from(raise.seen))?;
                    out.u8(u8::from(raise.notified))?;
                }
                out.u8(u8::from(record.blocked_notified))
            })?;
        }
        out.u64(self.replied_age_ns)
    }

    fn read(input: &mut Decoder<'_>) -> Result<Self> {
        let terminal = input.text("terminal id", MAX_ID_BYTES)?;
        let run_id = input.text("run id", MAX_ID_BYTES)?;
        let update = input.u64()?;
        let count = input.u16()?;
        let background = input.u8()?;
        let foreground = match input.u8()? {
            0 => Foreground::Unknown,
            1 => Foreground::Child,
            2 => Foreground::Other,
            _ => return Err(Error::Invalid("foreground")),
        };
        let name = std::str::from_utf8(input.raw(MAX_TERMINAL_TITLE_BYTES)?)
            .map_err(|_| Error::Invalid("terminal title"))?
            .to_owned();
        let records = input.list("attention records", MAX_PROGRAM_RECORDS.into(), read_record)?;
        let omitted = input.list("omitted records", MAX_PROGRAM_RECORDS.into(), read_record)?;
        Ok(Self {
            terminal,
            run_id,
            update,
            count,
            background,
            foreground,
            name,
            records,
            omitted,
            replied_age_ns: input.u64()?,
        })
    }
}

fn read_record(input: &mut Decoder<'_>) -> Result<AttentionRecord> {
    Ok(AttentionRecord {
        record: read_record_frame(input.blob()?)?,
        changed_age_ns: input.u64()?,
        raise: if input.boolean()? {
            Some(Raise {
                age_ns: input.u64()?,
                changed: input.u64()?,
                seen: input.boolean()?,
                notified: input.boolean()?,
            })
        } else {
            None
        },
        blocked_notified: input.boolean()?,
    })
}

/// The one canonical Orbit frame holding `record`: a ProgramStatus whose
/// other fields are fixed and neutral, so Orbit's own decoder validates it.
fn record_frame(record: &ProgramRecord) -> Result<Vec<u8>> {
    management::encode_server_message(&ServerMessage::ProgramStatus(ProgramStatus {
        count: 1,
        background: 0,
        processes: ProgramProcesses {
            background: Vec::new(),
            complete: false,
            foreground: None,
        },
        foreground: Foreground::Unknown,
        update: 0,
        terminal_title: String::new(),
        omitted: Vec::new(),
        records: vec![record.clone()],
    }))
    .map_err(|_| Error::Invalid("attention record"))
}

fn read_record_frame(bytes: &[u8]) -> Result<ProgramRecord> {
    count("attention record size", bytes.len(), MAX_MESSAGE_BYTES)?;
    let Ok(ServerMessage::ProgramStatus(mut status)) = management::decode_server_message(bytes)
    else {
        return Err(Error::Invalid("attention record"));
    };
    let record = status
        .records
        .pop()
        .filter(|_| status.records.is_empty())
        .ok_or(Error::Invalid("attention record"))?;
    if record_frame(&record)? != bytes {
        return Err(Error::Invalid("attention record frame"));
    }
    Ok(record)
}

fn stamp(value: u64) -> Result<()> {
    if value > MAX_STAMP_NS {
        return Err(Error::Invalid("stamp"));
    }
    Ok(())
}

pub(crate) mod model;

impl Supervisor {
    pub(crate) fn validate(&self) -> Result<()> {
        count(
            "initial command",
            self.initial_command.len(),
            MAX_COMMAND_ARGUMENTS,
        )?;
        for argument in &self.initial_command {
            if argument.len() > MAX_COMMAND_ARGUMENT_BYTES || argument.contains(&0) {
                return Err(Error::Invalid("initial command"));
            }
        }
        Ok(())
    }

    fn write(&self, out: &mut Encoder) -> Result<()> {
        out.u64(self.started_at_ms)?;
        out.list(&self.initial_command, |out, argument| out.raw(argument))?;
        out.u8(u8::from(self.initial_status.is_some()))?;
        if let Some(status) = self.initial_status {
            out.u8(status)?;
        }
        Ok(())
    }

    fn read(input: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            started_at_ms: input.u64()?,
            initial_command: input.list("initial command", MAX_COMMAND_ARGUMENTS, |input| {
                Ok(input.raw(MAX_COMMAND_ARGUMENT_BYTES)?.to_vec())
            })?,
            initial_status: if input.boolean()? {
                Some(input.u8()?)
            } else {
                None
            },
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
