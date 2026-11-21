//! The window topology's part of an Update checkpoint: sessions, tabs, panes,
//! popups, identity counters, replay rings and recency.
// Consumed by `checkpoint::update::model`, whose capture and restore halves
// the supervisor's export and import use once S4a (eon-t3fyd.7) and S4b-1
// (eon-t3fyd.8) call them; until then their tests consume it.
#![cfg_attr(not(test), allow(dead_code))]

use super::{
    Column, IDS_PER_SUPERVISOR, IdAllocator, Pane, Popup, Session, Tab, Workspace, WorkspaceSet,
    terminal_endpoint,
};
use crate::checkpoint::{
    self, Core, Error, Result, Run, RunKind,
    update::{Allocator, ExtraPopup, RegistrationError, Role, SessionExtra, TabExtra, Update},
};
use crate::managed_environment::{PopupCatalog, PopupDefinition};
use crate::session_registry::Registry;
use eon_workspace_protocol::v47::Failure;
use std::{
    collections::VecDeque,
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

impl WorkspaceSet {
    /// The core, stamped `revision`, and the topology's part of the Update
    /// section; `terminal_extras` and `attention` stay empty for
    /// `checkpoint::update::model::capture` to fill, and `supervisor` for the
    /// supervisor's export. `run` names the live run
    /// of a terminal ID. Refuses state an Update does not carry.
    pub(crate) fn checkpoint(
        &self,
        revision: u64,
        run: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(Core, Update)> {
        if self.in_flight.is_some() {
            return Err(Error::Unsettled("a session request is in flight"));
        }
        let slot = |terminal: &Session| -> Result<Run> {
            Ok(Run {
                kind: RunKind::Confirmed,
                terminal_id: terminal.id.clone(),
                run_id: run(&terminal.id).ok_or(Error::Unsettled("a terminal has no live run"))?,
            })
        };
        let mut sessions = Vec::with_capacity(self.workspaces.len());
        let mut session_extras = Vec::with_capacity(self.workspaces.len());
        let mut tab_extras = Vec::new();
        for workspace in &self.workspaces {
            // Only a recovered session waits for its registration this way;
            // `registered_recovery` always ends with an ID or an error.
            if self.registry.is_some()
                && workspace.persistent_id.is_none()
                && workspace.registration_error.is_none()
            {
                return Err(Error::Unsettled(
                    "a session's registration is still being recovered",
                ));
            }
            let mut tabs = Vec::with_capacity(workspace.tabs.len());
            for tab in &workspace.tabs {
                if tab.pending && tab.popups.is_empty() {
                    return Err(Error::Unsettled(
                        "the pending tab has no Files pair; choose its directory",
                    ));
                }
                let columns = tab
                    .columns
                    .iter()
                    .map(|column| {
                        Ok(checkpoint::Column {
                            panes: column
                                .panes
                                .iter()
                                .map(|pane| {
                                    Ok(checkpoint::Pane {
                                        id: pane.id.clone(),
                                        pinned_name: pane.pinned_name.clone(),
                                        tag: pane.tag.clone(),
                                        run: slot(&pane.session)?,
                                    })
                                })
                                .collect::<Result<_>>()?,
                            expanded: column.panes[column.expanded].id.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut popups = Vec::new();
                let mut extra_popups = Vec::new();
                for popup in &tab.popups {
                    let definition = definition(&workspace.catalog, &popup.entry).ok_or(
                        Error::Unsettled("a popup's entry is not in its session's popup catalog"),
                    )?;
                    let record = checkpoint::Popup {
                        id: popup.id.clone(),
                        entry: popup.entry.clone(),
                        label: definition.label.clone(),
                        launch_directory: popup.launch_directory.as_os_str().as_bytes().to_vec(),
                        run: slot(&popup.session)?,
                    };
                    match placement(tab.pending, definition.keep_alive) {
                        None => popups.push(record),
                        Some(role) => extra_popups.push(ExtraPopup {
                            role,
                            popup: record,
                        }),
                    }
                }
                let durable = |id: &&String| popups.iter().any(|popup| &popup.id == *id);
                let selected_popup = tab.selected_popup.as_ref().filter(durable).cloned();
                tab_extras.push(TabExtra {
                    tab: tab.id.clone(),
                    pending: tab.pending,
                    popup_order: tab.popups.iter().map(|popup| popup.id.clone()).collect(),
                    extra_popups,
                    selected_popup: tab.selected_popup.clone(),
                    // The owner keeps a cover whose popup has ended and drops
                    // it on use; the one wire form for it is none.
                    covered_popup: tab
                        .covered_popup
                        .clone()
                        .filter(|covered| tab.popups.iter().any(|popup| &popup.id == covered)),
                });
                tabs.push(checkpoint::Tab {
                    id: tab.id.clone(),
                    directory: tab.directory.as_os_str().as_bytes().to_vec(),
                    selected_pane: tab.selected_column.map(|index| {
                        let column = &tab.columns[index];
                        column.panes[column.expanded].id.clone()
                    }),
                    columns,
                    popups,
                    selected_popup,
                });
            }
            sessions.push(checkpoint::Session {
                id: workspace.id.clone(),
                persistent_id: workspace.persistent_id.clone(),
                active_tab: workspace
                    .tabs
                    .get(workspace.active)
                    .map(|tab| tab.id.clone()),
                tabs,
            });
            session_extras.push(SessionExtra {
                session: workspace.id.clone(),
                // The owner checks home only on use, so a form the wire refuses
                // is none, which every use refuses the same way. Home is never
                // judged by the filesystem here: it may return.
                home: workspace
                    .home
                    .as_ref()
                    .map(|home| home.as_os_str().as_bytes().to_vec())
                    .filter(|home| checkpoint::path("home", home).is_ok()),
                registration_error: workspace.registration_error.as_ref().map(|error| {
                    RegistrationError {
                        code: error.code.clone(),
                        detail: error.detail.clone(),
                    }
                }),
            });
        }
        let ids = &self.ids;
        let counter = |value: usize| value as u64;
        let core = Core {
            revision,
            sessions,
            shown_session: self
                .workspaces
                .get(self.shown)
                .map(|shown| shown.id.clone()),
            terminal_id_high_water: counter(ids.next_session - 1),
        };
        let update = Update {
            captured_monotonic_ns: 0,
            allocator: Allocator {
                range: counter((ids.limit - IDS_PER_SUPERVISOR - 1) / IDS_PER_SUPERVISOR),
                next_session: counter(ids.next_workspace),
                next_tab: counter(ids.next_tab),
                next_pane: counter(ids.next_pane),
                next_terminal: counter(ids.next_session),
                next_popup: counter(ids.next_popup),
                first_reserved: ids.first_reserved,
            },
            registry_available: self.registry.is_some(),
            session_extras,
            tab_extras,
            recent_inspections: self.recent_inspections.iter().cloned().collect(),
            recent_mutations: self.recent_mutations.iter().cloned().collect(),
            recent_sessions: self.recent_workspaces.iter().cloned().collect(),
            terminal_extras: Vec::new(),
            attention: None,
            supervisor: Default::default(),
        };
        Ok((core, update))
    }

    /// Rebuilds the set from a decoded, validated checkpoint under `runtime`,
    /// with the target's `registry` and popup `catalog`. Session names come
    /// only from `registry`. It only reads the registry and starts nothing.
    pub(crate) fn restore(
        core: &Core,
        update: &Update,
        runtime: PathBuf,
        registry: Option<Registry>,
        catalog: PopupCatalog,
    ) -> Result<Self> {
        if registry.is_some() != update.registry_available {
            return Err(Error::Unrestorable(
                "the session registry availability differs",
            ));
        }
        // One listing, retried while busy, names every registered session.
        let registrations = match &registry {
            Some(registry)
                if core
                    .sessions
                    .iter()
                    .any(|session| session.persistent_id.is_some()) =>
            {
                registry
                    .retry_busy(|| registry.list())
                    .map_err(|_| Error::Unrestorable("the session registry cannot be read"))?
            }
            _ => Vec::new(),
        };
        let mut tab_extras = update.tab_extras.iter();
        let mut workspaces = Vec::with_capacity(core.sessions.len());
        for (session, extra) in core.sessions.iter().zip(&update.session_extras) {
            let name = match (&session.persistent_id, &registry) {
                (Some(id), Some(_)) => registrations
                    .iter()
                    .find(|registration| &registration.id == id)
                    .ok_or(Error::Unrestorable("a session's registration is missing"))?
                    .name
                    .clone(),
                _ => None,
            };
            let mut tabs = Vec::with_capacity(session.tabs.len());
            for tab in &session.tabs {
                let extra = tab_extras.next().ok_or(Error::Invalid("tab extras"))?;
                tabs.push(restore_tab(tab, extra, &runtime, &catalog)?);
            }
            workspaces.push(Workspace {
                id: session.id.clone(),
                persistent_id: session.persistent_id.clone(),
                registration_error: extra.registration_error.as_ref().map(|error| Failure {
                    code: error.code.clone(),
                    detail: error.detail.clone(),
                }),
                name,
                catalog: catalog.clone(),
                home: extra.home.clone().map(path_buf),
                active: index(&session.tabs, session.active_tab.as_deref(), |tab| &tab.id)?,
                tabs,
            });
        }
        let allocator = &update.allocator;
        let counter = |value: u64| {
            usize::try_from(value)
                .map_err(|_| Error::Unrestorable("an identity counter does not fit this platform"))
        };
        let mut ids = IdAllocator::new(runtime, counter(allocator.range)?)
            .map_err(|_| Error::Unrestorable("an identity counter does not fit this platform"))?;
        ids.next_workspace = counter(allocator.next_session)?;
        ids.next_tab = counter(allocator.next_tab)?;
        ids.next_pane = counter(allocator.next_pane)?;
        ids.next_session = counter(allocator.next_terminal)?;
        ids.next_popup = counter(allocator.next_popup)?;
        ids.first_reserved = allocator.first_reserved;
        Ok(Self {
            registry,
            shown: index(&core.sessions, core.shown_session.as_deref(), |session| {
                &session.id
            })?,
            workspaces,
            ids,
            recent_inspections: VecDeque::from(update.recent_inspections.clone()),
            recent_mutations: VecDeque::from(update.recent_mutations.clone()),
            in_flight: None,
            recent_workspaces: VecDeque::from(update.recent_sessions.clone()),
        })
    }
}

/// Where a popup of a tab goes: the core holds durable popups of committed
/// tabs; a pending tab's popup is its Files pair, and a popup that is not
/// kept alive is transient.
fn placement(pending: bool, keep_alive: bool) -> Option<Role> {
    if pending {
        Some(Role::PendingPair)
    } else if keep_alive {
        None
    } else {
        Some(Role::Transient)
    }
}

fn definition<'a>(catalog: &'a PopupCatalog, entry: &str) -> Option<&'a PopupDefinition> {
    catalog
        .entries
        .iter()
        .find(|definition| definition.id == entry)
}

fn restore_tab(
    tab: &checkpoint::Tab,
    extra: &TabExtra,
    runtime: &Path,
    catalog: &PopupCatalog,
) -> Result<Tab> {
    let columns = tab
        .columns
        .iter()
        .map(|column| {
            Ok(Column {
                panes: column
                    .panes
                    .iter()
                    .map(|pane| {
                        Ok(Pane {
                            id: pane.id.clone(),
                            session: session(runtime, &pane.run)?,
                            pinned_name: pane.pinned_name.clone(),
                            tag: pane.tag.clone(),
                        })
                    })
                    .collect::<Result<_>>()?,
                expanded: index(&column.panes, Some(&column.expanded), |pane| &pane.id)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let durable = tab.popups.iter().map(|popup| (popup, None));
    let extension = extra
        .extra_popups
        .iter()
        .map(|extra| (&extra.popup, Some(extra.role)));
    let records: Vec<_> = durable.chain(extension).collect();
    let mut popups = Vec::with_capacity(records.len());
    for id in &extra.popup_order {
        let (popup, role) = records
            .iter()
            .find(|(popup, _)| &popup.id == id)
            .ok_or(Error::Invalid("popup order"))?;
        let definition = definition(catalog, &popup.entry).ok_or(Error::Unrestorable(
            "a popup's entry is not in the target popup catalog",
        ))?;
        if placement(extra.pending, definition.keep_alive) != *role
            || (*role == Some(Role::PendingPair) && !definition.keep_alive)
        {
            return Err(Error::Unrestorable(
                "a popup's keep_alive differs in the target popup catalog",
            ));
        }
        popups.push(Popup {
            id: popup.id.clone(),
            entry: popup.entry.clone(),
            session: session(runtime, &popup.run)?,
            launch_directory: path_buf(popup.launch_directory.clone()),
        });
    }
    Ok(Tab {
        id: tab.id.clone(),
        directory: path_buf(tab.directory.clone()),
        pending: extra.pending,
        selected_column: match &tab.selected_pane {
            Some(selected) => Some(
                tab.columns
                    .iter()
                    .position(|column| &column.expanded == selected)
                    .ok_or(Error::Invalid("selected pane"))?,
            ),
            None => None,
        },
        columns,
        popups,
        selected_popup: extra.selected_popup.clone(),
        covered_popup: extra.covered_popup.clone(),
    })
}

/// A slot's terminal and its derived endpoint.
fn session(runtime: &Path, run: &Run) -> Result<Session> {
    let number = run
        .terminal_id
        .strip_prefix("terminal-")
        .and_then(|number| number.parse().ok())
        .ok_or(Error::Invalid("terminal id"))?;
    Ok(Session {
        id: run.terminal_id.clone(),
        endpoint: terminal_endpoint(runtime, number),
    })
}

fn path_buf(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

/// The position of the item `id` names; validated input always has it.
fn index<T>(items: &[T], id: Option<&str>, key: impl Fn(&T) -> &String) -> Result<usize> {
    items
        .iter()
        .position(|item| Some(key(item).as_str()) == id)
        .ok_or(Error::Invalid("reference"))
}

#[cfg(test)]
pub(crate) mod tests;
