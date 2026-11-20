//! Update's export: a full-Eon supervisor's checkpoint, taken only at a
//! settled boundary. Restoring it is eon-8bfip's (S3b-2).
// S4a (eon-t3fyd.7) calls `export` from the Update exchange; until then its
// tests consume it.
#![cfg_attr(not(test), allow(dead_code))]

use super::SupervisorState;
use crate::{
    checkpoint::{
        Error, Header, Result,
        update::{self, Scope, Supervisor, model},
    },
    pair,
};
use std::{
    os::unix::ffi::OsStrExt,
    path::Path,
    time::{Duration, Instant},
};

/// A checkpoint and the scope its target must expect.
pub(super) struct Export {
    pub(super) bytes: Vec<u8>,
    pub(super) scope: Scope,
}

impl SupervisorState {
    /// The window's checkpoint at `now`, whose boot-clock reading is `boot`,
    /// or the first reason it is not settled. `root` is the namespace
    /// directory. Call it between turns, after `tend` and the detach notice's
    /// answer and before the next `accept`, where the turn has just pruned
    /// ended work. It checks and captures and writes nothing: a refusal
    /// leaves the window as it was, and the caller waits by calling again at
    /// a later turn. A refusal is never an error of the loop.
    pub(super) fn export(&mut self, root: &Path, now: Instant, boot: Duration) -> Result<Export> {
        if let Some(what) = self.unsettled() {
            return Err(Error::Unsettled(what));
        }
        let (Some(workspaces), Some(attention)) = (&self.workspaces, &self.attention) else {
            unreachable!("unsettled refuses a window without workspaces or attention");
        };
        let captured = u64::try_from(boot.as_nanos())
            .map_err(|_| Error::Unsettled("the boot clock exceeds the checkpoint"))?;
        let (core, mut update) = model::capture(
            workspaces,
            &self.sessions,
            Some(attention),
            1,
            now,
            captured,
        )?;
        update.supervisor =
            Supervisor {
                started_at_ms: self.started_at,
                initial_command: self
                    .initial_child
                    .iter()
                    .map(|argument| argument.as_bytes().to_vec())
                    .collect(),
                initial_status: self.initial_status.map(u8::try_from).transpose().map_err(
                    |_| Error::Unsettled("terminal-1's exit status does not fit the checkpoint"),
                )?,
            };
        update.supervisor.validate().map_err(|_| {
            Error::Unsettled("the window's initial command exceeds the checkpoint bounds")
        })?;
        let header = Header {
            namespace: root.to_str().ok_or(Error::Invalid("namespace"))?.to_owned(),
            generation: self.component_generation.clone(),
            created_build: self.created_build.clone(),
            active_build: self.active_build.clone(),
        };
        let scope = Scope {
            header: header.clone(),
            range: update.allocator.range,
        };
        let bytes = update::encode(header, core, &update, Vec::new())?;
        Ok(Export { bytes, scope })
    }

    /// The first supervisor-level work in flight, as "cannot checkpoint:"
    /// completes it; the owners' own refusals follow in `model::capture`. A
    /// new owner with work in flight joins this list.
    fn unsettled(&mut self) -> Option<&'static str> {
        let (Some(workspaces), Some(_)) = (&self.workspaces, &self.attention) else {
            return Some("only a full Eon window can be updated");
        };
        let runtime = workspaces.runtime_directory().to_owned();
        let checks: [(bool, &'static str); 10] = [
            (self.stopping.is_some(), "the window is stopping"),
            (
                self.reconciliation.is_some(),
                "a terminal end has an unreported error",
            ),
            (
                self.registry_pending.is_some()
                    || self
                        .registry_worker
                        .as_ref()
                        .is_some_and(|worker| !worker.is_idle()),
                "a session registration is in progress",
            ),
            (
                !self.start_pump.is_idle(),
                "a control client is waiting for a terminal start",
            ),
            (
                !self.failed_starts.is_empty(),
                "a failed terminal start is still being cleaned up",
            ),
            (
                !self.pair_dispatches.is_empty(),
                "a Files pair dispatch is running",
            ),
            (
                pair::any_pending_in(&runtime),
                "a Files pair is starting or leaving",
            ),
            (
                !self.scrollback.is_settled(),
                "a scrollback snapshot is not settled",
            ),
            (
                !self.editor_sockets.is_settled(&self.sessions),
                "an Editor socket is not settled",
            ),
            (self.sessions.is_empty(), "the window has no terminal left"),
        ];
        if let Some((_, what)) = checks.into_iter().find(|(unsettled, _)| *unsettled) {
            return Some(what);
        }
        match self.venus.as_mut().map(|venus| venus.has_exited()) {
            Some(Ok(true)) => Some("the presentation exited and is not yet reaped"),
            Some(Err(_)) => Some("the presentation's state cannot be read"),
            Some(Ok(false)) | None => None,
        }
    }
}

#[cfg(test)]
mod tests;
