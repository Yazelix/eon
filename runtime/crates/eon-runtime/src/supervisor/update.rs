//! Update's two halves: a full-Eon supervisor's checkpoint, taken only at a
//! settled boundary (`export`), and the supervisor rebuilt from it in the
//! target build (`import`, checked read-only by `validate_target`).
// The test-gated Update exchange validates the target and exercises import after exec.
#![cfg_attr(not(test), allow(dead_code))]

use super::{
    Carried, ControlListener, DiagnosticLog, Inputs, LaunchMode, Registry, RunningSession,
    SupervisorState, WORKSPACE_APPLICATION_ID, WORKSPACE_DECORATIONS, WindowSettings,
    current_generation, generation_directory,
};
use crate::{
    attention::Attention,
    checkpoint::{
        Core, Error, Header, Result,
        update::{self, Scope, Supervisor, Update, model},
    },
    editor::Sockets,
    pair,
    running::Running,
    scrollback::Snapshots,
    session_registry::Worker,
    sessions::FailedStarts,
    start_pump::StartPump,
    windows::runtime_window,
    workspace::WorkspaceSet,
};
use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

mod handoff;
mod preflight;
pub(super) use handoff::{InheritedPresentation, Owner};
pub(crate) use handoff::{RESUME_UPDATE, resume_update};
pub(super) use preflight::PreflightState;
pub(crate) use preflight::{VALIDATE_UPDATE_TARGET, validate_update_target};

/// What the target build supplies to an import: its own inputs, settings,
/// registry and log, and the lifetime its own launch says it serves. None of
/// it comes from the exchange.
pub(super) struct Target<'a> {
    pub(super) inputs: &'a Inputs,
    /// Read by `WindowSettings::read`, as a launch reads them.
    pub(super) settings: &'a WindowSettings,
    /// The namespace root.
    pub(super) root: &'a Path,
    /// The window's lifetime.
    pub(super) generation: &'a str,
    /// The base runtime directory, which the running-window scan reads.
    pub(super) base: &'a Path,
    pub(super) registry: Option<Registry>,
    pub(super) log: Option<&'a Arc<DiagnosticLog>>,
}

impl Target<'_> {
    fn runtime(&self) -> PathBuf {
        generation_directory(self.root, self.generation)
    }
}

/// A checkpoint decoded for its target, before any row or registry is read.
struct Decoded {
    header: Header,
    core: Core,
    update: Update,
    active_build: String,
    elapsed: Duration,
}

const ANOTHER_WINDOW: Error = Error::Unrestorable("the checkpoint is for another window");

/// The checks before rows or the registry: the scope names the target's own
/// lifetime, the target build has an identity, the bytes decode against the
/// scope, the window is full Eon and was captured at or before `boot`.
fn decoded(target: &Target, bytes: &[u8], scope: &Scope, boot: Duration) -> Result<Decoded> {
    if Path::new(&scope.header.namespace) != target.root
        || scope.header.generation != target.generation
    {
        return Err(ANOTHER_WINDOW);
    }
    let active_build = current_generation(target.inputs)
        .map_err(|_| Error::Unrestorable("the target build's identity cannot be read"))?;
    let (checkpoint, update) = update::decode(bytes, scope)?;
    if update.attention.is_none() {
        return Err(Error::Unrestorable(
            "only a full Eon window can be restored",
        ));
    }
    let elapsed = boot
        .checked_sub(Duration::from_nanos(update.captured_monotonic_ns))
        .ok_or(Error::Unrestorable(
            "the checkpoint was captured after the restore's clock",
        ))?;
    Ok(Decoded {
        header: checkpoint.header,
        core: checkpoint.core,
        update,
        active_build,
        elapsed,
    })
}

/// R1-R7 against the target's registry and catalog, at `now`.
fn validated(
    target: &Target,
    decoded: &Decoded,
    now: Instant,
    preflight: bool,
) -> Result<(WorkspaceSet, Attention)> {
    let (workspaces, attention) = model::validate(
        &decoded.core,
        &decoded.update,
        target.runtime(),
        target.registry.clone(),
        target.settings.popup_catalog(target.inputs),
        now,
        decoded.elapsed,
        preflight,
    )?;
    Ok((
        workspaces,
        attention.expect("decoded refuses a checkpoint without attention"),
    ))
}

/// Whether `import` would accept `bytes` before it needs rows or a
/// listener. Reads only: no rows, lease, bind, start or write.
pub(super) fn validate_target(
    target: &Target,
    bytes: &[u8],
    scope: &Scope,
    now: Instant,
    boot: Duration,
) -> Result<()> {
    let decoded = decoded(target, bytes, scope, boot)?;
    validated(target, &decoded, now, true).map(drop)
}

pub(super) fn validate_handoff_target(
    target: &Target,
    bytes: &[u8],
    scope: &Scope,
    hosts: &[eon_workspace_protocol::update::Host],
    now: Instant,
    boot: Duration,
) -> Result<()> {
    let decoded = decoded(target, bytes, scope, boot)?;
    let slots = decoded.update.slots(&decoded.core)?;
    if slots.len() != hosts.len()
        || slots.iter().any(|slot| {
            hosts
                .iter()
                .filter(|host| host.terminal == slot.run.terminal_id && host.run == slot.run.run_id)
                .count()
                != 1
        })
    {
        return Err(Error::Unrestorable(
            "the retained hosts differ from the checkpoint",
        ));
    }
    validated(target, &decoded, now, true).map(drop)
}

/// The window's supervisor rebuilt in the target build around `sessions`,
/// the runs S4b-1 re-adopts, with `listener` bound at the lifetime's control
/// socket; `now` and `boot` are the restore's monotonic and boot clocks.
/// Every check runs before the first effect: a refusal returns the rows as
/// they came, sends them nothing, and starts, writes and removes nothing.
/// The result is detached: no presentation, no detach notice.
pub(super) fn import(
    target: &Target,
    bytes: &[u8],
    scope: &Scope,
    mut sessions: Vec<RunningSession>,
    listener: &ControlListener,
    now: Instant,
    boot: Duration,
) -> std::result::Result<SupervisorState, (Error, Vec<RunningSession>)> {
    let runtime = target.runtime();
    let Some(log) = target.log else {
        return Err((
            Error::Unrestorable("the restored supervisor needs a diagnostic log"),
            sessions,
        ));
    };
    let checked = (|| {
        if listener.path() != runtime.join("eon.sock") {
            return Err(ANOTHER_WINDOW);
        }
        let decoded = decoded(target, bytes, scope, boot)?;
        let order = model::match_rows(&decoded.core, &decoded.update, &runtime, &sessions)?;
        let (workspaces, attention) = validated(target, &decoded, now, false)?;
        // The only fallible effects, in order; a failure drops what started.
        let registry_worker = target.registry.clone().map(Worker::start).transpose();
        let running = registry_worker
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                Running::start(
                    target.base.to_path_buf(),
                    target.root,
                    target.generation,
                    Arc::clone(log),
                )
                .map(|running| (worker, running))
            });
        let (registry_worker, running) = running.map_err(|error| {
            log.note(&format!("cannot restore the window's supervisor: {error}"));
            Error::Unrestorable("the restored supervisor cannot start its workers")
        })?;
        Ok((
            decoded,
            order,
            workspaces,
            attention,
            registry_worker,
            running,
        ))
    })();
    let (decoded, order, workspaces, attention, registry_worker, running) = match checked {
        Ok(checked) => checked,
        Err(error) => return Err((error, sessions)),
    };
    let Decoded {
        header,
        update,
        active_build,
        ..
    } = decoded;
    model::apply_extras(&mut sessions, &order, &update);
    let log = Arc::clone(log);
    let scrollback = Snapshots::adopt(
        &runtime,
        sessions.iter().map(|session| session.id.as_str()),
        &log,
    );
    let editor_sockets = Sockets::recover(&runtime, &sessions, &log);
    let start_pump = StartPump::new(listener.try_clone_listener(), Arc::clone(&log));
    let Supervisor {
        started_at_ms,
        initial_command,
        initial_status,
    } = update.supervisor;
    Ok(SupervisorState::assemble(
        Carried {
            registry_worker,
            registry_pending: None,
            started_at: started_at_ms,
            window: runtime_window(target.root),
            running: Some(running),
            component_generation: target.generation.to_owned(),
            created_build: header.created_build,
            active_build,
            application_id: WORKSPACE_APPLICATION_ID.to_owned(),
            decorations: WORKSPACE_DECORATIONS,
            workspaces: Some(workspaces),
            sessions,
            failed_starts: FailedStarts::default(),
            start_pump,
            initial_child: initial_command
                .into_iter()
                .map(OsString::from_vec)
                .collect(),
            initial_status: initial_status.map(i32::from),
            stopping: None,
            attention: Some(attention),
            scrollback,
            editor_sockets,
            log,
        },
        target.settings,
        LaunchMode::Workspace,
        target.inputs,
    ))
}

// EONU's own bounds must cover what a window can hold; an owner raising its
// bound breaks this build and forces an EONU decision (eon-t3fyd.7).
const _: () = {
    use eon_workspace_protocol::{update as eonu, v49};
    assert!(eonu::MAX_HOSTS >= v49::MAX_SESSIONS);
    assert!(eonu::MAX_ID_BYTES >= orbit_protocol::management::MAX_ID_BYTES);
    assert!(eonu::MAX_PATH_BYTES >= v49::MAX_DIRECTORY_BYTES);
    assert!(eonu::MAX_CHECKPOINT_BYTES >= crate::checkpoint::MAX_BYTES);
};

impl SupervisorState {
    /// INSPECTED for `request`: who this window is, for a client of any EONW.
    /// `runtime` is the lifetime directory, `<namespace>/generations/<id>`.
    /// Reads only; an unreadable identity record is an error.
    pub(super) fn inspected(
        &self,
        request: eon_workspace_protocol::update::RequestId,
        runtime: &Path,
    ) -> std::result::Result<eon_workspace_protocol::update::Inspected, String> {
        use eon_workspace_protocol::update::{Inspected, Presentation};
        let namespace = runtime.parent().and_then(Path::parent).unwrap_or(runtime);
        // An unreadable record is not "no launcher": it refuses.
        let launcher = super::super::generation::Identity::read(runtime)?
            .and_then(|identity| identity.active_launcher)
            .map(|path| path.into_os_string().into_vec());
        Ok(Inspected {
            request,
            pid: std::process::id(),
            namespace: namespace.as_os_str().as_bytes().to_vec(),
            lifetime: self.component_generation.clone(),
            created_build: self.created_build.clone(),
            active_build: self.active_build.clone(),
            active_launcher: launcher,
            // The one place EONU learns the workspace version: as a number.
            eonw: eon_workspace_protocol::v49::VERSION,
            eonr: crate::checkpoint::VERSION,
            update_schema: update::SCHEMA,
            presentation: if self.venus.is_some() {
                Presentation::Attached
            } else {
                Presentation::Detached
            },
            hosts: self.update_hosts(),
            last: None,
        })
    }

    fn update_hosts(&self) -> Vec<eon_workspace_protocol::update::Host> {
        self.sessions
            .iter()
            .map(|session| {
                let inventory = session.inventory();
                eon_workspace_protocol::update::Host {
                    terminal: inventory.terminal,
                    run: inventory.run,
                    origin: inventory.origin_build,
                    management: inventory.management_version,
                    record_generation: inventory.record_generation,
                    orbs: inventory.orbs,
                    orbf: inventory.orbf,
                }
            })
            .collect()
    }
}

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
