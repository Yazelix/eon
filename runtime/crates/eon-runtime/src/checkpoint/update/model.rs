//! The complete logical Runtime state of an Update: the three owners' parts
//! composed into one checkpoint, and back. The supervisor assembly (settling,
//! the anchor and the elapsed gap) is S3b's (eon-t3fyd.6).
#![cfg_attr(not(test), allow(dead_code))]

use super::Update;
use crate::attention::Attention;
use crate::checkpoint::{Core, Error, Result};
use crate::managed_environment::PopupCatalog;
use crate::session_registry::Registry;
use crate::sessions::RunningSession;
use crate::workspace::{WorkspaceSet, terminal_endpoint};
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

/// The checkpoint of a settled window: its topology, its terminal rows (one
/// per slot, no other) and its attention at `anchor`, whose monotonic clock
/// reading is `captured_monotonic_ns`. Callers encode it with
/// `update::encode`, which validates it.
pub(crate) fn capture(
    workspaces: &WorkspaceSet,
    sessions: &[RunningSession],
    attention: Option<&Attention>,
    revision: u64,
    anchor: Instant,
    captured_monotonic_ns: u64,
) -> Result<(Core, Update)> {
    // Every row first, so an unsettled row reports itself rather than its
    // slot's missing run.
    let extras = sessions
        .iter()
        .map(RunningSession::checkpoint_extra)
        .collect::<Result<Vec<_>>>()?;
    let mut terminals = HashSet::new();
    if !sessions
        .iter()
        .all(|session| terminals.insert(session.id.as_str()))
    {
        return Err(Error::Unsettled("two terminal rows share a terminal"));
    }
    let run = |terminal: &str| {
        sessions
            .iter()
            .find(|session| session.id == terminal)
            .map(|session| session.run_id().to_owned())
    };
    let (core, mut update) = workspaces.checkpoint(revision, &run)?;
    let slots = update.slots(&core)?;
    if slots.len() != sessions.len() {
        // Each slot found its own row, so some row has no slot: a start that
        // timed out may leave its terminal running.
        return Err(Error::Unsettled("a terminal has no place in the window"));
    }
    let terminal_extras = slots
        .iter()
        .map(|slot| {
            extras
                .iter()
                .find(|extra| extra.terminal == slot.run.terminal_id)
                .cloned()
                .ok_or(Error::Unsettled("a terminal has no live run"))
        })
        .collect::<Result<Vec<_>>>()?;
    let attention = attention
        .map(|attention| attention.checkpoint(anchor, &slots))
        .transpose()?;
    drop(slots);
    update.captured_monotonic_ns = captured_monotonic_ns;
    update.terminal_extras = terminal_extras;
    update.attention = attention;
    Ok((core, update))
}

/// Restores a decoded checkpoint under `runtime` with the target's `registry`
/// and `catalog`, `elapsed` after its capture. `sessions` are the runs S4
/// re-adopted, in any order: exactly one per slot, each at its derived
/// endpoint. Every check runs before the rows change, so a refusal leaves
/// them, and the registry, untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn restore(
    core: &Core,
    update: &Update,
    runtime: PathBuf,
    registry: Option<Registry>,
    catalog: PopupCatalog,
    sessions: &mut [RunningSession],
    now: Instant,
    elapsed: Duration,
) -> Result<(WorkspaceSet, Option<Attention>)> {
    let readopted = Error::Unrestorable("the re-adopted terminals differ from the checkpoint");
    let slots = update.slots(core)?;
    let mut order = Vec::with_capacity(slots.len());
    for (slot, extra) in slots.iter().zip(&update.terminal_extras) {
        let number = slot
            .run
            .terminal_id
            .strip_prefix("terminal-")
            .and_then(|number| number.parse().ok())
            .ok_or(Error::Invalid("terminal id"))?;
        let mut rows = sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| session.id == slot.run.terminal_id);
        match (rows.next(), rows.next()) {
            (Some((index, session)), None)
                if !session.is_finished()
                    && session.run_id() == slot.run.run_id
                    && session.endpoint == terminal_endpoint(&runtime, number) =>
            {
                order.push((index, extra));
            }
            _ => return Err(readopted),
        }
    }
    if order.len() != sessions.len() {
        return Err(readopted);
    }
    let attention = update
        .attention
        .as_ref()
        .map(|attention| Attention::restore(attention, now, elapsed))
        .transpose()?;
    let workspaces = WorkspaceSet::restore(core, update, runtime, registry, catalog)?;
    for (index, extra) in order {
        sessions[index].restore_extra(extra);
    }
    Ok((workspaces, attention))
}

#[cfg(test)]
mod tests;
