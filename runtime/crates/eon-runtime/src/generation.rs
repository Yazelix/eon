use super::cli::Inputs;
use super::control::{
    CONTROL_TIMEOUT, ControlResponse, EndpointFailure, EndpointFailureKind, Error, StopAnswer,
    connect_control, failure, probe_generation_runtime, probe_generation_runtime_from,
    probe_generation_runtime_until, remove_stale_socket, report_failure, send_action_from_until,
    send_stop_on, socket_identity, socket_identity_from, write_stdout,
};
use super::sessions::terminal_session;
use super::supervisor::{
    base_runtime_directory, effective_uid, present_at, probe_supervisor, runtime_directory,
    supervisor_lock_path, try_lock_supervisor_lifecycle, validate_private_directory,
};
use super::windows::{
    DEFAULT_WINDOW, WINDOW_RUNTIME, Window, default_namespace_id, missing_default, runtime_window,
};
use super::workspace::{json_escape, terminal_text};
use eon_workspace_protocol::stop;
use eon_workspace_protocol::v48::{
    Action, Availability, BusyTerminals, Failure, MAX_RUNNING_WINDOWS, Response, WindowEntry,
    WindowState, WorkspaceAction,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::IsTerminal,
    os::unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

mod identity;
pub(super) use identity::Identity;
mod launch;
pub(super) use launch::NamespaceLock;

/// This build's exact digest, `g1-…`; it is not a lifetime lookup.
pub fn current_generation(inputs: &Inputs) -> Result<String, String> {
    inputs.component_report()?;
    Ok(generation_id(inputs.assembly))
}

fn generation_id(input: &[u8]) -> String {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut hash = OFFSET;
    for byte in input {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("g1-{hash:032x}")
}

pub(super) fn valid_generation(value: &str) -> bool {
    eon_workspace_protocol::v48::valid_generation(value)
}

pub(super) fn generation_directory(root: &Path, generation: &str) -> PathBuf {
    root.join("generations").join(generation)
}

/// Projection of the active launcher; recordless generations use the link directly.
const LAUNCHER: &str = "eon";
/// Set on a delegated attach to the generation it presents, and removed from
/// everything Eon starts.
pub(super) use super::launch_environment::DELEGATION_MARKER;

/// How this build runs another build's recorded `eon` to present one of that
/// build's generations (EON-C11, Delegation).
#[derive(Debug, PartialEq)]
struct Delegation {
    /// The generation's independent window root, else none for the default
    /// namespace.
    window: Option<PathBuf>,
    generation: String,
    arguments: Vec<String>,
}

/// The delegated attach of `generation` in namespace `root`.
fn delegation(root: &Path, generation: &str) -> Delegation {
    let window = runtime_window(root).unwrap_or_else(|| DEFAULT_WINDOW.into());
    let argument = if is_default_namespace(root)
        && Identity::read(&generation_directory(root, generation))
            .is_ok_and(|record| record.is_some())
    {
        generation[3..].to_owned()
    } else {
        window
    };
    Delegation {
        window: (!is_default_namespace(root)).then(|| root.to_owned()),
        generation: generation.to_owned(),
        arguments: vec!["window".into(), "attach".into(), argument],
    }
}

impl Delegation {
    /// `eon` run as this delegation.
    fn command(&self, eon: &Path) -> Command {
        let mut command = Command::new(eon);
        command
            .args(&self.arguments)
            .env(DELEGATION_MARKER, &self.generation);
        match &self.window {
            Some(root) => command.env(WINDOW_RUNTIME, root),
            None => command.env_remove(WINDOW_RUNTIME),
        };
        command
    }
}

/// The delegated lifetime, whose active build must be this executable's build.
/// The marker narrows the named window and never selects another (EON-C11).
pub(super) fn delegated_generation(inputs: &Inputs) -> Result<Option<String>, Error> {
    let Some(value) = env::var_os(DELEGATION_MARKER) else {
        return Ok(None);
    };
    let generation = value.to_str().filter(|value| valid_generation(value));
    let current = current_generation(inputs)?;
    if !generation.is_some_and(|generation| {
        active_build(&runtime_directory("eon"), generation).is_ok_and(|build| build == current)
    }) {
        return Err(Error::usage(format!(
            "{DELEGATION_MARKER} does not name a lifetime controlled by this Eon's build; nothing was sent"
        )));
    }
    Ok(generation.map(str::to_owned))
}

/// The failure for a marker whose exact lifetime is absent from window `shown`.
pub(super) fn delegated_elsewhere(shown: &str) -> Error {
    Error::usage(format!(
        "{DELEGATION_MARKER} names a lifetime which window {shown} does not run; nothing was sent"
    ))
}

/// Set on a routed session command to the generation it was routed to, and
/// removed from everything Eon starts (EON-C11, Routing).
pub(super) use super::launch_environment::ROUTE_MARKER;

/// A routing hop narrows its namespace to the generation already selected.
pub(super) fn routed_generation(
    root: &Path,
    selected: Option<&str>,
) -> Result<Option<String>, Failure> {
    let Some(routed) = env::var_os(ROUTE_MARKER) else {
        return Ok(None);
    };
    let unavailable = |detail: String| {
        failure(
            "supervisor-unavailable",
            format!(
                "{ROUTE_MARKER} is {:?}: {detail}; no action sent",
                routed.to_string_lossy()
            ),
        )
    };
    let generation = routed
        .to_str()
        .filter(|generation| valid_generation(generation))
        .ok_or_else(|| unavailable("not a valid generation".into()))?;
    if let Some(selected) = selected
        && selected != generation
    {
        return Err(unavailable(format!(
            "cannot retarget generation {generation} to {selected}"
        )));
    }
    if !workspace_runtime_exists(root, generation).map_err(unavailable)? {
        return Err(unavailable(format!(
            "generation {generation} is not running; inspect targets with `eon window ls`"
        )));
    }
    Ok(Some(generation.into()))
}

/// Runs `command` as the active controlling build of lifetime `generation` in
/// namespace `root`, through its recorded `eon`, in place of this `eon`
/// (EON-C11, Routing). It returns only when it does not run it: with nothing
/// to add for a command that cannot route, else with why. With the marker set, to
/// any value, it never runs it, so one command makes at most one hop.
pub(super) fn route(root: &Path, generation: &str, command: &[OsString]) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    if let Some(routed) = env::var_os(ROUTE_MARKER) {
        return Some(format!(
            "{ROUTE_MARKER} is {:?}, so this `eon` was itself routed and does not route generation {generation} again",
            routed.to_string_lossy(),
        ));
    }
    let recovery = "open a new Eon window with the installed build";
    let Some(eon) = recorded_launcher(&generation_directory(root, generation)) else {
        return Some(format!(
            "generation {generation}: the `eon` that started it is not recorded; {recovery}"
        ));
    };
    if !eon.is_file() {
        return Some(format!(
            "generation {generation}: the `eon` that started it, {}, no longer exists; {recovery}",
            eon.display()
        ));
    }
    super::diagnostics::bench_trace("exec", "generation-route");
    let error = Command::new(&eon)
        .args(command)
        .env(ROUTE_MARKER, generation)
        .exec();
    Some(format!(
        "generation {generation}: cannot run {}: {error}; {recovery}",
        eon.display()
    ))
}

pub(super) fn installed_launcher(inputs: &Inputs) -> Option<PathBuf> {
    inputs
        .managed
        .shell_bin
        .as_ref()
        .map(|bin| bin.join("eon"))
        .filter(|path| path.is_absolute() && path.is_file())
}

/// Records this build's `eon`, from its session bin, in the workspace
/// generation directory `runtime`.
pub(super) fn record_launcher(inputs: &Inputs, runtime: &Path) -> Result<Identity, String> {
    let build = current_generation(inputs)?;
    if let Some(identity) = Identity::read(runtime)? {
        identity.require_build(&build)?;
        return Ok(identity);
    }
    let launcher = installed_launcher(inputs);
    let generation = runtime
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|id| valid_generation(id))
        .ok_or("invalid lifetime directory")?;
    let identity = Identity::initial(generation, &build, launcher);
    identity.publish(runtime)?;
    Ok(identity)
}

/// A lifetime directory's supervisor endpoints, which cleanup removes only
/// when dead (EON-C11).
const ENDPOINTS: [&str; 2] = ["eon.sock", "update.sock"];

/// Under the lifecycle lock, remove metadata only after every host owner settled.
pub(super) fn cleanup_metadata(runtime: &Path) -> Result<(), String> {
    validate_private_directory(runtime)?;
    let parent = crate::private_file::object(runtime)?;
    for socket in ENDPOINTS {
        let _ = remove_stale_socket(&runtime.join(socket), "Eon supervisor");
    }
    let mut metadata = Vec::with_capacity(4);
    for entry in fs::read_dir(runtime)
        .map_err(|error| format!("cannot list {}: {error}", runtime.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect {}: {error}", runtime.display()))?;
        let name = entry.file_name();
        if !matches!(
            name.to_str(),
            Some("eon.sock" | "update.sock" | "eon" | "identity")
        ) {
            return Err(format!(
                "retaining metadata in {}: residue {}",
                runtime.display(),
                name.to_string_lossy()
            ));
        }
        let path = entry.path();
        let object = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let safe = object.uid() == effective_uid()
            && match name.to_str() {
                Some("eon") => object.file_type().is_symlink(),
                Some("identity") => object.is_file() && object.mode() & 0o7777 == 0o600,
                // A remaining endpoint was not safely removed as dead above.
                Some("eon.sock" | "update.sock") => false,
                _ => unreachable!(),
            };
        if !safe {
            return Err(format!(
                "retaining metadata: unsafe or active object {}",
                path.display()
            ));
        }
        metadata.push((path, socket_identity_from(&object)));
    }
    validate_private_directory(runtime)?;
    if crate::private_file::object(runtime)?.map(|id| (id.0, id.1)) != parent.map(|id| (id.0, id.1))
    {
        return Err(format!(
            "generation directory {} changed before cleanup",
            runtime.display()
        ));
    }
    for (path, identity) in &metadata {
        crate::private_file::revalidate(path, Some(*identity))?;
    }
    for (path, identity) in metadata {
        crate::private_file::revalidate(&path, Some(identity))?;
        fs::remove_file(&path)
            .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
    }
    fs::remove_dir(runtime).map_err(|error| format!("cannot remove {}: {error}", runtime.display()))
}

fn recorded_launcher(runtime: &Path) -> Option<PathBuf> {
    match Identity::read(runtime) {
        Ok(Some(identity)) => return identity.active_launcher,
        Err(error) => {
            eprintln!(
                "cannot read private identity in {}: {error}",
                runtime.display()
            );
            return None;
        }
        Ok(None) => {}
    }
    let link = runtime.join(LAUNCHER);
    let metadata = fs::symlink_metadata(&link).ok()?;
    if !metadata.file_type().is_symlink() || metadata.uid() != effective_uid() {
        return None;
    }
    fs::read_link(link).ok().filter(|eon| eon.is_absolute())
}

fn is_default_namespace(root: &Path) -> bool {
    root == base_runtime_directory("eon")
}

pub(super) const MAX_GENERATIONS: usize = 256;

#[cfg(test)]
mod identity_tests;

#[derive(Clone, Default)]
pub(super) struct WorkspaceTarget {
    /// An EON-C23 window ID or prefix, as typed.
    pub(super) window: Option<String>,
    /// The arguments the build of a generation that speaks another EONW runs
    /// in this `eon`'s place, else empty: never routed (EON-C11, Routing).
    pub(super) command: Vec<OsString>,
    /// One budget for native quota discovery and inspection.
    pub(super) deadline: Option<Instant>,
}

pub(super) struct ResolvedWorkspace {
    pub(super) generation: String,
    pub(super) window: Option<String>,
    /// The window's full EON-C23 ID, for human text.
    pub(super) label: String,
    pub(super) socket: PathBuf,
}

pub(super) fn resolve_workspace(
    inputs: &Inputs,
    mut target: WorkspaceTarget,
    caller: Option<&str>,
) -> Result<ResolvedWorkspace, Failure> {
    let current =
        current_generation(inputs).map_err(|detail| failure("supervisor-unavailable", detail))?;
    if target.window.is_none()
        && caller.is_none()
        && is_default_namespace(&runtime_directory("eon"))
    {
        target.window = Some(DEFAULT_WINDOW.into());
    }
    // A default-namespace window names its exact generation; an independent
    // window names its namespace, where the window-only rule selects.
    let (root, exact, shown) = match &target.window {
        Some(id) => match super::windows::select_routed_window(inputs, id, target.deadline)? {
            (Window::Default(generation), shown) => {
                (base_runtime_directory("eon"), Some(generation), shown)
            }
            (Window::Other(generation), shown) => {
                (base_runtime_directory("eon"), Some(generation), shown)
            }
            (Window::Independent { root, .. }, shown) => (root, None, shown),
        },
        None => {
            let root = runtime_directory("eon");
            let shown = runtime_window(&root).unwrap_or_default();
            (root, None, shown)
        }
    };
    let exact = routed_generation(&root, exact.as_deref())?.or(exact);
    let window = super::windows::runtime_window(&root);
    let window_only =
        exact.is_none() && (target.window.is_some() || (caller.is_none() && window.is_some()));
    let label = |generation: &str| {
        if shown == DEFAULT_WINDOW && exact.as_deref() == Some(generation) {
            return DEFAULT_WINDOW.to_owned();
        }
        window
            .clone()
            .unwrap_or_else(|| default_namespace_id(generation, &current))
    };
    let selected = |generation: String| ResolvedWorkspace {
        socket: generation_directory(&root, &generation).join("eon.sock"),
        label: label(&generation),
        generation,
        window: window.clone(),
    };
    let unavailable = |generation: &str, detail: String| {
        failure(
            "supervisor-unavailable",
            format!(
                "Eon window {}: {detail}; use the terminal's own `eon` from its PATH",
                label(generation)
            ),
        )
    };
    // The selected generation's failure; one of another EONW first runs the
    // command as its active controlling build.
    let refused = |generation: &str, (kind, detail): (EndpointFailureKind, String)| {
        let routed = (kind == EndpointFailureKind::Incompatible)
            .then(|| route(&root, generation, &target.command))
            .flatten();
        match routed {
            Some(why) => failure(
                "supervisor-unavailable",
                format!("Eon window {}: {detail}; {why}", label(generation)),
            ),
            None => unavailable(generation, detail),
        }
    };
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let deadline = target
        .deadline
        .map_or(deadline, |limit| limit.min(deadline));
    let probe = |id: &str| {
        probe_workspace_generation(&root, id, deadline).map_err(|error| (error.kind, error.detail))
    };
    let generation = if let Some(id) = exact.as_ref() {
        let present =
            workspace_runtime_exists(&root, id).map_err(|detail| unavailable(id, detail))?;
        if !present && env::var_os(ROUTE_MARKER).is_some() {
            return Err(unavailable(
                id,
                format!("{ROUTE_MARKER} target generation {id} ended; no action sent"),
            ));
        }
        if !present && *id == current {
            return Err(missing_default(inputs, target.deadline));
        }
        if !present {
            return Err(failure(
                "unknown-window",
                format!("Eon window {shown} was not found"),
            ));
        }
        probe(id).map_err(|error| refused(id, error))?;
        id.clone()
    } else {
        let candidates = generation_candidates(&root)
            .map_err(|detail| failure("supervisor-unavailable", detail))?;
        let mut live = Vec::new();
        let mut failures = Vec::new();
        let mut owner = None;
        for (id, path) in candidates {
            let id = id
                .to_str()
                .filter(|id| valid_generation(id))
                .ok_or_else(|| {
                    failure(
                        "supervisor-unavailable",
                        format!("invalid generation directory {}", path.display()),
                    )
                })?;
            let verified = match probe(id) {
                Ok(_) => true,
                Err((EndpointFailureKind::Dead, _)) => continue,
                Err(error) => {
                    failures.push((id.to_owned(), error));
                    false
                }
            };
            live.push(id.to_owned());
            if verified
                && !window_only
                && let Some(caller) = caller
            {
                match validates_caller(&path.join("eon.sock"), caller, deadline) {
                    Ok(true) if owner.is_some() => {
                        return Err(unavailable(
                            id,
                            "more than one supervisor validates the caller".into(),
                        ));
                    }
                    Ok(true) => owner = Some(id.to_owned()),
                    Ok(false) => {}
                    Err(error) => failures.push((id.to_owned(), (error.kind, error.detail))),
                }
            }
        }
        if window_only {
            match preferred_window_generation(&root, &current, &live)
                .map_err(|detail| failure("supervisor-unavailable", detail))?
            {
                Some(id) => {
                    if let Some(index) = failures.iter().position(|(failed, _)| failed == id) {
                        let (id, error) = failures.swap_remove(index);
                        return Err(refused(&id, error));
                    }
                    id.to_owned()
                }
                None if live.is_empty() => {
                    return Err(failure(
                        "supervisor-unavailable",
                        "the selected Eon window has ended; no supervisor runs in it",
                    ));
                }
                None => {
                    return Err(failure(
                        "ambiguous-window",
                        format!(
                            "Eon window {shown} runs generations of {} Eon builds and none is this build's; run the command from a terminal of the one to target",
                            live.len()
                        ),
                    ));
                }
            }
        } else {
            if Instant::now() >= deadline {
                return Err(failure(
                    "supervisor-unavailable",
                    "caller discovery exceeded the supervisor inspection deadline; no action sent",
                ));
            }
            // A validated complete identity is authoritative; another generation's failure is not its owner.
            if let Some(owner) = owner {
                let mut resolved = selected(owner);
                if window.is_none() {
                    resolved.label = default_namespace_id(
                        &resolved.generation,
                        preferred_window_generation(&root, &current, &live)
                            .ok()
                            .flatten()
                            .unwrap_or(""),
                    );
                }
                return Ok(resolved);
            }
            // The caller's Orbit log, when it remains, names the one
            // generation that can own it.
            let caller = caller.unwrap_or_default();
            let origin = super::diagnostics::orbit_run_generation(caller);
            failures.retain(|(id, _)| origin.as_ref().is_none_or(|origin| origin == id));
            return Err(match failures.len() {
                0 => failure(
                    "unknown-caller",
                    stale_caller(caller, origin.map(|id| (label(&id), live.contains(&id)))),
                ),
                1 => {
                    let (id, error) = failures.swap_remove(0);
                    refused(&id, error)
                }
                _ => {
                    let (id, (_, detail)) = failures.swap_remove(0);
                    unavailable(&id, detail)
                }
            });
        }
    };
    let resolved = selected(generation);
    if let Some(caller) = caller
        && !validates_caller(&resolved.socket, caller, deadline)
            .map_err(|error| unavailable(&resolved.generation, error.detail))?
    {
        return Err(unknown_caller());
    }
    Ok(resolved)
}

/// What an `unknown-caller` failure says of a caller, an `EON_TERMINAL`
/// value, that no running window runs: its terminal and, when its Orbit log
/// names it, the window it ran in and whether that window still runs.
fn stale_caller(caller: &str, origin: Option<(String, bool)>) -> String {
    let terminal = terminal_text(terminal_session(caller).unwrap_or(caller));
    let place = match origin {
        Some((window, false)) => format!(" of Eon window {window}, which has ended"),
        Some((window, true)) => format!(" of Eon window {window}, which no longer runs it"),
        None => ", which no running Eon window here runs".into(),
    };
    format!(
        "EON_TERMINAL names terminal {terminal}{place}; a resumed or restored shell can keep an \
         old value. Run `eon` from a live Eon terminal's own shell; a shell outside Eon carries \
         neither EON_TERMINAL nor EON_TAB"
    )
}

/// The generation in namespace `root` whose supervisor validates `caller`.
/// Unlike `resolve_workspace`, an entry that names no generation does not
/// stop the search: callers that report damaged entries themselves still
/// find the caller's window beside them.
pub(super) fn caller_generation(
    root: &Path,
    current: &str,
    caller: &str,
) -> Result<String, String> {
    // Messages use EON-C23's active-build preference after discovery completes.
    let window = runtime_window(root);
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let mut owners = Vec::new();
    let mut live = Vec::new();
    let mut failures = Vec::new();
    for (id, path) in generation_candidates(root)? {
        let Some(id) = id.to_str().filter(|id| valid_generation(id)) else {
            continue;
        };
        if let Err(detail) = validate_private_directory(&path) {
            failures.push((id.to_owned(), detail));
            continue;
        }
        let socket = path.join("eon.sock");
        let validated = probe_generation_runtime_until(&socket, deadline).and_then(|runtime| {
            if runtime.generation == id {
                validate_recorded_runtime(root, id, &runtime)?;
                validates_caller(&socket, caller, deadline)
            } else {
                Err(EndpointFailure::new(
                    EndpointFailureKind::Corrupt,
                    format!("supervisor reports generation {}", runtime.generation),
                ))
            }
        });
        match validated {
            Ok(true) => owners.push(id.to_owned()),
            Ok(false) => {}
            Err(error) if matches!(error.kind, EndpointFailureKind::Dead) => continue,
            Err(error) => failures.push((id.to_owned(), error.detail)),
        }
        live.push(id.to_owned());
    }
    let preferred = preferred_window_generation(root, current, &live)
        .ok()
        .flatten()
        .unwrap_or(current);
    let label = |id: &str| {
        window
            .clone()
            .unwrap_or_else(|| default_namespace_id(id, preferred))
    };
    match owners.as_slice() {
        [owner] => Ok(owner.clone()),
        [] => {
            // As `resolve_workspace`: the caller's Orbit log narrows its owner.
            let origin = super::diagnostics::orbit_run_generation(caller);
            failures.retain(|(id, _)| origin.as_ref().is_none_or(|origin| origin == id));
            Err(failures.into_iter().next().map_or_else(
                || stale_caller(caller, origin.map(|id| (label(&id), live.contains(&id)))),
                |(id, detail)| format!("Eon window {}: {detail}", label(&id)),
            ))
        }
        _ => {
            let mut windows: Vec<String> = owners.iter().map(|id| label(id)).collect();
            windows.dedup();
            Err(format!(
                "more than one supervisor validates the calling terminal, in Eon window{} {}",
                if windows.len() == 1 { "" } else { "s" },
                windows.join(", ")
            ))
        }
    }
}

fn unknown_caller() -> Failure {
    failure(
        "unknown-caller",
        "the calling terminal is not live in this Eon runtime namespace; to address another window, clear EON_TERMINAL and EON_TAB, or run outside Eon",
    )
}

/// Validates the private generation and its exact owner before control or routing.
pub(super) fn probe_workspace_generation(
    root: &Path,
    generation: &str,
    deadline: Instant,
) -> Result<(), EndpointFailure> {
    let socket = generation_directory(root, generation).join("eon.sock");
    if !workspace_runtime_exists(root, generation)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
    {
        return Err(super::control::missing_endpoint(&socket));
    }
    let runtime = probe_generation_runtime_until(&socket, deadline)?;
    if runtime.generation != generation {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            format!("supervisor reports generation {}", runtime.generation),
        ));
    }
    validate_recorded_runtime(root, generation, &runtime)?;
    Ok(())
}

fn validate_recorded_runtime(
    root: &Path,
    generation: &str,
    info: &eon_workspace_protocol::v48::Runtime,
) -> Result<(), EndpointFailure> {
    let record = Identity::read(&generation_directory(root, generation))
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?;
    if record.is_some_and(|record| {
        !record.matches(&info.generation, &info.created_build, &info.active_build)
    }) {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor and committed identity disagree",
        ));
    }
    Ok(())
}

fn workspace_runtime_exists(root: &Path, id: &str) -> Result<bool, String> {
    Ok(private_directory_exists(root, "runtime directory")?
        && private_directory_exists(&root.join("generations"), "generation directory")?
        && private_directory_exists(
            &generation_directory(root, id),
            "generation runtime directory",
        )?)
}

fn validates_caller(
    socket: &Path,
    caller: &str,
    deadline: Instant,
) -> Result<bool, EndpointFailure> {
    match send_action_from_until(
        socket,
        Some(caller.into()),
        Action::Workspace(WorkspaceAction::Inspect),
        deadline,
    )? {
        ControlResponse::Workspace(Response::Snapshot(_)) => Ok(true),
        ControlResponse::Workspace(Response::Failure(error)) if error.code == "unknown-caller" => {
            Ok(false)
        }
        ControlResponse::Workspace(Response::Failure(error)) => Err(EndpointFailure::new(
            EndpointFailureKind::Unreachable,
            format!(
                "supervisor rejected caller inspection: {}: {}",
                error.code, error.detail
            ),
        )),
        _ => Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor returned the wrong result for caller inspection",
        )),
    }
}

fn generation_candidates(root: &Path) -> Result<Vec<(OsString, PathBuf)>, String> {
    generation_inventory(root)?.bounded(root)
}

pub(super) struct GenerationInventory {
    pub(super) entries: Vec<(OsString, PathBuf)>,
    pub(super) count: usize,
}

impl GenerationInventory {
    pub(super) fn bounded(self, root: &Path) -> Result<Vec<(OsString, PathBuf)>, String> {
        if self.count > MAX_GENERATIONS {
            return Err(format!(
                "generation directory {} contains {} entries and exceeds the {MAX_GENERATIONS}-entry inspection limit; to intentionally stop a known lifetime use `eon window stop <full-id>` (all 32 hex digits of its lifetime directory), then run `eon window ls`",
                root.join("generations").display(),
                self.count
            ));
        }
        Ok(self.entries)
    }
}

pub(super) fn generation_inventory(root: &Path) -> Result<GenerationInventory, String> {
    generation_inventory_until(root, None)
}

pub(super) fn generation_inventory_until(
    root: &Path,
    deadline: Option<Instant>,
) -> Result<GenerationInventory, String> {
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(GenerationInventory {
            entries: Vec::new(),
            count: 0,
        });
    }
    let parent = root.join("generations");
    let mut candidates = Vec::new();
    let mut count = 0;
    if private_directory_exists(&parent, "generation directory")? {
        for entry in fs::read_dir(&parent).map_err(|error| {
            format!(
                "cannot list generation directory {}: {error}",
                parent.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot read generation entry in {}: {error}",
                    parent.display()
                )
            })?;
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                return Err("generation discovery exceeded the inspection deadline".into());
            }
            count += 1;
            // Count names without retaining or probing an unbounded inventory.
            if candidates.len() <= MAX_GENERATIONS {
                candidates.push((entry.file_name(), entry.path()));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(GenerationInventory {
        entries: candidates,
        count,
    })
}
#[derive(Debug)]
pub(super) struct GenerationRecord {
    pub(super) id: String,
    created_build: Option<String>,
    active_build: Option<String>,
    terminal_inventory: Vec<eon_workspace_protocol::v48::TerminalInventory>,
    kind: &'static str,
    pub(super) state: &'static str,
    runtime: PathBuf,
    pub(super) name: Option<String>,
    started_at: Option<u64>,
    eon_version: Option<String>,
    workspace_protocol: Option<u16>,
    component_report: Option<String>,
    venus: Option<String>,
    pub(super) sessions: Vec<String>,
    /// Its busy terminals (EON-C11), known for a live full-Eon window.
    busy: Option<BusyTerminals>,
    attach: Availability,
    pub(super) stop: Availability,
    pub(super) detail: String,
    /// The recorded active `eon`, which presents it when this build cannot.
    launcher: Option<PathBuf>,
}

impl GenerationRecord {
    pub(super) fn caller_build(&self, caller: &str) -> Option<&str> {
        let (terminal, run) = caller.split_once('.')?;
        if self.state == "live"
            && self
                .terminal_inventory
                .iter()
                .any(|entry| entry.terminal == terminal && entry.run == run)
        {
            self.active_build.as_deref()
        } else {
            None
        }
    }

    /// Whether this build controls it.
    pub(super) fn current(&self) -> bool {
        self.kind == "current"
    }

    /// Its name and Eon version, as a window listing names it.
    pub(super) fn label(&self) -> String {
        format!(
            "{}, Eon {}",
            terminal_text(self.name.as_deref().unwrap_or("unnamed")),
            terminal_text(self.eon_version.as_deref().unwrap_or("unknown"))
        )
    }

    /// Whether this `eon` presents and controls it directly.
    fn compatible(&self) -> bool {
        self.state == "live" && self.launcher.is_none() && self.attach.available
    }
}

/// Every generation of the window at `root`, current first. With
/// `remove_dead`, a dead generation's socket, launcher record and directory
/// are removed once its lifecycle lock is free.
fn discover_generations(
    inputs: &Inputs,
    root: &Path,
    current: &str,
    remove_dead: bool,
) -> Result<Vec<GenerationRecord>, String> {
    discover_generations_until(inputs, root, current, remove_dead, None)
}

fn discover_generations_until(
    inputs: &Inputs,
    root: &Path,
    current: &str,
    remove_dead: bool,
    deadline: Option<Instant>,
) -> Result<Vec<GenerationRecord>, String> {
    let component_report = inputs.component_report()?;
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(vec![unstarted_current_generation(root, current)]);
    }

    let inventory = generation_inventory_until(root, deadline)?;
    let candidates = if remove_dead && deadline.is_none() && inventory.count > MAX_GENERATIONS {
        NamespaceLock::acquire(root)?.inventory()?
    } else {
        inventory.bounded(root)?
    };

    let current_path = generation_directory(root, current);
    let mut records = vec![inspect_generation(
        current,
        "current",
        current_path,
        &supervisor_lock_path(root, current),
        component_report,
        true,
        remove_dead,
        deadline,
    )];
    for (id, path) in candidates {
        let id = id.to_string_lossy();
        if id != current {
            records.push(inspect_generation(
                &id,
                "previous",
                path,
                &supervisor_lock_path(root, &id),
                component_report,
                true,
                remove_dead,
                deadline,
            ));
        }
    }
    for record in &mut records {
        record.kind = if record.active_build.as_deref().unwrap_or(&record.id) == current {
            "current"
        } else {
            "previous"
        };
    }
    records.sort_by_key(|record| !record.current());
    Ok(records)
}

pub(super) struct SupervisorInventory {
    pub entries: Vec<WindowEntry>,
    /// Include dormant generations: recovery can reuse their existing directory.
    pub observed: Vec<(PathBuf, String)>,
}

pub(super) const RUNNING_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Read-only, off-loop inventory with at most 32 concurrent peer probes.
pub(super) fn live_supervisors(
    base: &Path,
    self_root: &Path,
    generation: &str,
    log: &super::diagnostics::DiagnosticLog,
) -> Result<(SupervisorInventory, super::running::Fingerprint), String> {
    let (roots, _) = super::running::window_roots(base, Some(log))?;
    let mut candidates = Vec::new();
    let mut observed = Vec::new();
    for root in roots {
        let generations = match generation_candidates(&root) {
            Ok(generations) => generations,
            Err(error) => {
                log.note_once(&format!(
                    "skipping running Eon namespace {}: {error}",
                    root.display()
                ));
                continue;
            }
        };
        for (id, runtime) in generations {
            let Some(id) = id.to_str().filter(|id| valid_generation(id)) else {
                continue;
            };
            if root == self_root && id == generation {
                continue;
            }
            observed.push((root.clone(), id.to_owned()));
            let lock = supervisor_lock_path(&root, id);
            let held = match super::running::supervisor_held(&lock) {
                Ok(held) => held,
                Err(error) => {
                    log.note_once(&format!("skipping running Eon generation: {error}"));
                    continue;
                }
            };
            if held {
                let window = (root != base)
                    .then(|| root.file_name().unwrap().to_string_lossy().into_owned());
                candidates.push((
                    WindowEntry {
                        created_build: None,
                        active_build: None,
                        window,
                        generation: id.into(),
                        name: None,
                        started_at: None,
                        live_terminals: None,
                        state: WindowState::Unreachable,
                    },
                    runtime,
                    lock,
                ));
            }
        }
    }
    let fingerprint = super::running::Fingerprint::read(base, &observed)?;
    let mut candidates = candidates.into_iter();
    let mut entries = Vec::new();
    loop {
        let batch: Vec<_> = candidates.by_ref().take(MAX_RUNNING_WINDOWS).collect();
        if batch.is_empty() {
            break;
        }
        let inspected = thread::scope(|scope| {
            let probes: Vec<_> = batch
                .into_iter()
                .map(|(entry, runtime, lock)| {
                    let id = entry.generation.clone();
                    let probe = thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            inspect_generation(
                                &id, "previous", runtime, &lock, "", false, false, None,
                            )
                        })
                        .map_err(|error| {
                            format!("cannot start running Eon window inspection: {error}")
                        })?;
                    Ok((entry, probe))
                })
                .collect::<Result<_, String>>()?;
            probes
                .into_iter()
                .map(|(mut entry, probe)| {
                    let record = probe
                        .join()
                        .map_err(|_| "running Eon window inspection panicked".to_string())?;
                    match record.state {
                        // Full Eon always supplies its derived workspace name; eonbase has none.
                        "live" if record.name.is_none() => return Ok(None),
                        "live" => {
                            entry.created_build = record.created_build;
                            entry.active_build = record.active_build;
                            entry.name = record.name;
                            entry.started_at = record.started_at;
                            entry.live_terminals = Some(record.sessions.len() as u16);
                            entry.state = WindowState::Live;
                        }
                        "incompatible" => entry.state = WindowState::Incompatible,
                        _ => {}
                    }
                    Ok(Some(entry))
                })
                .filter_map(Result::transpose)
                .collect::<Result<Vec<_>, String>>()
        })?;
        entries.extend(inspected);
    }
    Ok((SupervisorInventory { entries, observed }, fingerprint))
}

fn inspect_selected_generation(
    inputs: &Inputs,
    root: &Path,
    current: &str,
    target: &str,
) -> Result<Option<GenerationRecord>, String> {
    inspect_selected_generation_until(inputs, root, current, target, None)
}

fn inspect_selected_generation_until(
    inputs: &Inputs,
    root: &Path,
    current: &str,
    target: &str,
    deadline: Option<Instant>,
) -> Result<Option<GenerationRecord>, String> {
    let missing_current =
        || (target == current).then(|| unstarted_current_generation(root, current));
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(missing_current());
    }
    let parent = root.join("generations");
    if !private_directory_exists(&parent, "generation directory")? {
        return Ok(missing_current());
    }
    let runtime = generation_directory(root, target);
    if target != current
        && matches!(
            fs::symlink_metadata(&runtime),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    {
        return Ok(None);
    }
    let component_report = inputs.component_report()?;
    Ok(Some(inspect_generation(
        target,
        if target == current {
            "current"
        } else {
            "previous"
        },
        runtime,
        &supervisor_lock_path(root, target),
        component_report,
        true,
        true,
        deadline,
    )))
}

#[allow(clippy::too_many_arguments)]
fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    lifecycle_lock: &Path,
    current_components: &str,
    cleanup: bool,
    remove_dead: bool,
    deadline: Option<Instant>,
) -> GenerationRecord {
    if !valid_generation(id) {
        return failed_generation(
            id,
            kind,
            runtime,
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "generation directory name is not a valid Eon identity",
            ),
        );
    }
    if let Err(detail) = validate_private_directory(&runtime) {
        return if matches!(
            fs::symlink_metadata(&runtime),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ) {
            dead_generation(id, kind, runtime, "generation has not started")
        } else {
            failed_generation(
                id,
                kind,
                runtime,
                EndpointFailure::new(EndpointFailureKind::Corrupt, detail),
            )
        };
    }

    let identity = Identity::read(&runtime);
    let socket = runtime.join("eon.sock");
    let probe = if let Some(deadline) = deadline {
        probe_generation_runtime_until(&socket, deadline.min(Instant::now() + CONTROL_TIMEOUT))
    } else if cleanup {
        probe_generation_runtime(&socket)
    } else {
        probe_generation_runtime_until(&socket, Instant::now() + RUNNING_PROBE_TIMEOUT)
    };
    let mut record = match probe {
        Ok(info) if info.generation != id => failed_generation(
            id,
            kind,
            runtime,
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!(
                    "supervisor reports generation {} from directory {id}",
                    info.generation
                ),
            ),
        ),
        Ok(mut info) => {
            // A supervisor that refuses presentation itself keeps its reason.
            let other_build =
                cleanup && info.component_report != current_components && info.attach.available;
            if other_build {
                info.attach = Availability {
                    available: false,
                    reason: "component graph differs from the current Eon build; \
                             its active Eon build can attach it, and any later \
                             Eon build can stop it"
                        .into(),
                };
            }
            GenerationRecord {
                id: id.into(),
                created_build: Some(info.created_build),
                active_build: Some(info.active_build),
                terminal_inventory: info.terminal_inventory,
                kind,
                state: "live",
                name: info.name,
                started_at: Some(info.started_at),
                eon_version: Some(info.eon_version),
                workspace_protocol: Some(info.workspace_protocol),
                component_report: Some(info.component_report),
                venus: info.venus,
                sessions: info.sessions,
                busy: info.busy,
                attach: info.attach,
                stop: info.stop,
                detail: "live supervisor validated its generation identity".into(),
                launcher: other_build.then(|| recorded_launcher(&runtime)).flatten(),
                runtime,
            }
        }
        Err(error) => {
            let dead = error.kind == EndpointFailureKind::Dead;
            let mut record = failed_generation(id, kind, runtime.clone(), error);
            if cleanup && record.state == "incompatible" {
                record.launcher = recorded_launcher(&runtime);
                // From the first Reserved Stop on, every supervisor answers it,
                // whatever its EONW, so no build is needed to stop it (EON-C11).
                record.stop = match record.workspace_protocol {
                    Some(version) if version < stop::FIRST_VERSION => Availability {
                        available: false,
                        reason: format!(
                            "supervisor uses EONW {version}, which predates the reserved Stop; \
                             {QUIT_FROM_INSIDE}"
                        ),
                    },
                    _ => Availability {
                        available: true,
                        reason: "stop sends the reserved Stop".into(),
                    },
                };
            }
            if remove_dead
                && dead
                && identity.is_ok()
                && let Ok(Some(_lifecycle_lock)) = try_lock_supervisor_lifecycle(lifecycle_lock)
                && let Err(error) = cleanup_metadata(&runtime)
            {
                eprintln!("{error}");
            }
            record
        }
    };
    if record.launcher.is_some() {
        record.attach = Availability {
            available: true,
            reason: "attach runs its active Eon build".into(),
        };
    }
    match identity {
        Ok(Some(identity)) => {
            if record.state == "live"
                && !identity.matches(
                    &record.id,
                    record.created_build.as_deref().unwrap_or(""),
                    record.active_build.as_deref().unwrap_or(""),
                )
            {
                record.state = "corrupt";
                record.attach = Availability {
                    available: false,
                    reason: "supervisor and committed identity disagree".into(),
                };
                record.detail = record.attach.reason.clone();
                record.launcher = None;
            }
            record.created_build = Some(identity.created_build);
            record.active_build = Some(identity.active_build);
        }
        Err(error) => {
            // Stop remains independent of metadata; execution never uses a bad record.
            record.state = "corrupt";
            record.attach = Availability {
                available: false,
                reason: error.clone(),
            };
            record.detail = error;
            record.launcher = None;
        }
        Ok(None) => {}
    }
    record
}

fn dead_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    detail: impl Into<String>,
) -> GenerationRecord {
    failed_generation(
        id,
        kind,
        runtime,
        EndpointFailure::new(EndpointFailureKind::Dead, detail),
    )
}

fn unstarted_current_generation(root: &Path, current: &str) -> GenerationRecord {
    dead_generation(
        current,
        "current",
        generation_directory(root, current),
        "current generation has not started",
    )
}

fn failed_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    error: EndpointFailure,
) -> GenerationRecord {
    let state = match error.kind {
        EndpointFailureKind::Dead => "dead",
        EndpointFailureKind::Incompatible => "incompatible",
        EndpointFailureKind::InvalidAction | EndpointFailureKind::Corrupt => "corrupt",
        EndpointFailureKind::Unreachable | EndpointFailureKind::Starting => "unreachable",
    };
    GenerationRecord {
        id: id.into(),
        created_build: None,
        active_build: None,
        terminal_inventory: Vec::new(),
        kind,
        state,
        runtime,
        name: None,
        started_at: None,
        eon_version: None,
        workspace_protocol: error.protocol,
        component_report: None,
        venus: None,
        sessions: Vec::new(),
        busy: None,
        attach: Availability {
            available: false,
            reason: error.detail.clone(),
        },
        stop: Availability {
            available: false,
            reason: error.detail.clone(),
        },
        detail: error.detail,
        launcher: None,
    }
}

pub(super) fn private_directory_exists(path: &Path, description: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_private_directory(path).map(|()| true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        )),
    }
}

pub(super) fn generations_command(
    inputs: &Inputs,
    json: bool,
    product: &str,
) -> Result<i32, String> {
    let records = discover_generations(
        inputs,
        &runtime_directory(product),
        &current_generation(inputs)?,
        true,
    )?;
    write_stdout(if json {
        generations_json(&records)
    } else {
        generations_human(&records)
    })?;
    Ok(0)
}

fn generations_human(records: &[GenerationRecord]) -> String {
    let mut output = String::new();
    for record in records {
        output.push_str(&format!(
            "{} {} {} terminals={}\n",
            record.kind,
            record.id.escape_debug(),
            record.state,
            record.sessions.len()
        ));
        output.push_str(&format!(
            "  attach={} {}\n  stop={} {}\n  {}\n",
            record.attach.available,
            record.attach.reason.escape_debug(),
            record.stop.available,
            terminal_text(&record.stop.reason),
            record.detail.escape_debug()
        ));
        if let (Some(version), Some(protocol)) = (&record.eon_version, record.workspace_protocol) {
            output.push_str(&format!(
                "  eon={} eonw={protocol}\n",
                version.escape_debug()
            ));
        }
        output.push_str(&format!(
            "  created_build={} active_build={}\n",
            record.created_build.as_deref().unwrap_or("unknown"),
            record.active_build.as_deref().unwrap_or("unknown")
        ));
        if let Some(eon) = &record.launcher {
            output.push_str(&format!("  executable {}\n", eon.display()));
        }
        if let Some(started_at) = record.started_at {
            output.push_str(&format!(
                "  name={} started_at={started_at}\n",
                record.name.as_deref().unwrap_or("unknown").escape_debug()
            ));
        }
        for session in &record.sessions {
            output.push_str(&format!("  terminal {session}\n"));
        }
        for line in super::workspace::human_busy(record.busy.as_ref()).lines() {
            output.push_str(&format!("  {line}\n"));
        }
        if let Some(report) = &record.component_report {
            for line in report.lines() {
                output.push_str(&format!("  component {}\n", line.escape_debug()));
            }
        }
    }
    output
}

fn generations_json(records: &[GenerationRecord]) -> String {
    let mut output = String::from("{\"generations\":[");
    for (index, record) in records.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"kind\":\"{}\",\"state\":\"{}\",\"name\":{},\"started_at\":{},\"eon_version\":{},\"workspace_protocol\":{},\"component_report\":{},\"executable\":{},\"terminals\":[",
            json_escape(&record.id),
            record.kind,
            record.state,
            json_option(record.name.as_deref()),
            record.started_at.map_or_else(|| "null".into(), |value| value.to_string()),
            json_option(record.eon_version.as_deref()),
            record.workspace_protocol.map_or_else(|| "null".into(), |value| value.to_string()),
            json_option(record.component_report.as_deref()),
            json_option(
                record
                    .launcher
                    .as_ref()
                    .map(|eon| eon.to_string_lossy())
                    .as_deref()
            ),
        ));
        for (session_index, session) in record.sessions.iter().enumerate() {
            if session_index != 0 {
                output.push(',');
            }
            output.push_str(&format!("\"{}\"", json_escape(session)));
        }
        output.push_str(&format!(
            "],\"created_build\":{},\"active_build\":{},\"terminal_inventory\":{},\"busy\":",
            json_option(record.created_build.as_deref()),
            json_option(record.active_build.as_deref()),
            super::workspace::json_inventory(&record.terminal_inventory)
        ));

        super::workspace::json_busy(&mut output, record.busy.as_ref(), true);
        output.push_str(&format!(
            ",\"attach\":{{\"available\":{},\"reason\":\"{}\"}},\"stop\":{{\"available\":{},\"reason\":\"{}\"}},\"detail\":\"{}\"}}",
            record.attach.available,
            json_escape(&record.attach.reason),
            record.stop.available,
            json_escape(&record.stop.reason),
            json_escape(&record.detail),
        ));
    }
    output.push_str("]}\n");
    output
}

fn json_option(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".into(),
        |value| format!("\"{}\"", json_escape(value)),
    )
}

/// One `eon window ls --json` window (EON-C23), with no trailing newline.
/// Without `labels`, the window's name is null and each busy terminal leaves
/// out its name: both are labels a person sees, which can come from a title or
/// directory.
pub(super) fn window_json(
    record: &GenerationRecord,
    id: &str,
    short_id: &str,
    labels: bool,
) -> String {
    let mut output = format!(
        "{{\"id\":\"{}\",\"short_id\":\"{}\",\"name\":{},\"current\":{},\"generation\":\"{}\",\"state\":\"{}\",\"compatible\":{},\"eon_version\":{},\"workspace_protocol\":{},\"started_at\":{},\"terminals\":[",
        json_escape(id),
        json_escape(short_id),
        json_option(record.name.as_deref().filter(|_| labels)),
        record.current(),
        json_escape(&record.id),
        record.state,
        record.compatible(),
        json_option(record.eon_version.as_deref()),
        record
            .workspace_protocol
            .map_or_else(|| "null".into(), |value| value.to_string()),
        record
            .started_at
            .map_or_else(|| "null".into(), |value| value.to_string()),
    );
    for (index, session) in record.sessions.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!("\"{}\"", json_escape(session)));
    }
    output.push_str(&format!(
        "],\"venus\":{},\"created_build\":{},\"active_build\":{},\"terminal_inventory\":{},\"busy\":",
        json_option(record.venus.as_deref()),
        json_option(record.created_build.as_deref()),
        json_option(record.active_build.as_deref()),
        super::workspace::json_inventory(&record.terminal_inventory)
    ));
    super::workspace::json_busy(&mut output, record.busy.as_ref(), labels);
    output.push_str(&format!(
        ",\"attach\":{{\"available\":{},\"reason\":\"{}\"}},\"stop\":{{\"available\":{},\"reason\":\"{}\"}},\"executable\":{},\"detail\":\"{}\",\"component_report\":{}}}",
        record.attach.available,
        json_escape(&record.attach.reason),
        record.stop.available,
        json_escape(&record.stop.reason),
        json_option(
            record
                .launcher
                .as_ref()
                .map(|eon| eon.to_string_lossy())
                .as_deref()
        ),
        json_escape(&record.detail),
        json_option(record.component_report.as_deref()),
    ));
    output
}

/// A window's `eon window ls` cells after its ID: NAME, VERSION, STARTED,
/// TERMINALS and COMPATIBLE.
pub(super) fn window_cells(record: &GenerationRecord, current_components: &str) -> [String; 5] {
    const NAME_WIDTH: usize = 24;
    let name = record.name.as_deref().map_or_else(
        || "-".into(),
        |name| {
            let name = terminal_text(name);
            if name.chars().count() > NAME_WIDTH {
                name.chars().take(NAME_WIDTH).chain(['…']).collect()
            } else {
                name
            }
        },
    );
    let live = record.state == "live";
    let compatible = match record.state {
        _ if record.compatible() => "yes",
        "live" if record.component_report.as_deref() != Some(current_components) => {
            "no (other build)"
        }
        "live" => "no",
        "incompatible" => "no (other EONW)",
        "unreachable" => "no (not answering)",
        _ => "no (corrupt)",
    };
    [
        name,
        record
            .eon_version
            .as_deref()
            .map_or_else(|| "-".into(), terminal_text),
        record.started_at.map_or_else(|| "-".into(), local_minute),
        if live {
            record.sessions.len().to_string()
        } else {
            "-".into()
        },
        compatible.into(),
    ]
}

/// A Unix millisecond time as a local `%Y-%m-%d %H:%M`, or `-`.
fn local_minute(unix_ms: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(unix_ms / 1000) else {
        return "-".into();
    };
    crate::platform::LocalTime::from_seconds(seconds)
        .map(|time| time.minute())
        .unwrap_or_else(|| "-".into())
}

/// Presents generation `target`, the current one without it. Messages name
/// `window`, as `eon window attach` shows it, else the generation (eonbase).
pub(super) fn attach_generation(
    inputs: &Inputs,
    root: &Path,
    target: Option<&str>,
    window: Option<&str>,
) -> Result<i32, Error> {
    let current = current_generation(inputs)?;
    let target = target.unwrap_or(&current);
    let subject = window.map_or_else(
        || format!("generation {target}"),
        |window| format!("Eon window {window}"),
    );
    let record = inspect_selected_generation(inputs, root, &current, target)?
        .ok_or_else(|| Error::missing(format!("{subject} was not found")))?;
    if record.state == "dead"
        && let Some(identity) = Identity::read(&record.runtime)?
    {
        if identity.active_build != current {
            let launcher = identity.active_launcher.ok_or_else(|| {
                Error::unreachable(format!("{subject}: active launcher is not recorded"))
            })?;
            if env::var_os(DELEGATION_MARKER).is_some() {
                return Err(Error::unreachable(format!(
                    "{subject}: attach cannot delegate again"
                )));
            }
            let error = delegation(root, target).command(&launcher).exec();
            return Err(format!("cannot run {}: {error}", launcher.display()).into());
        }
        // Recovery becomes a normal supervisor child; the attaching CLI still returns.
        let mut command = inputs.supervisor_command()?;
        command
            .arg("run")
            .env(DELEGATION_MARKER, target)
            .env_remove(ROUTE_MARKER)
            .env(WINDOW_RUNTIME, root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot recover {subject}: {error}"))?;
        let deadline =
            Instant::now() + super::supervisor::SESSION_CLEANUP_TIMEOUT.saturating_mul(4);
        loop {
            if probe_supervisor(inputs, &record.runtime.join("eon.sock"), target).is_ok() {
                return Ok(0);
            }
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("cannot recover {subject}: supervisor exited {status}").into());
            }
            if Instant::now() >= deadline {
                return Err(Error::unreachable(format!(
                    "{subject}: recovery is not ready; supervisor remains for inspection"
                )));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    if !record.attach.available {
        let versions = match (&record.eon_version, record.workspace_protocol) {
            (Some(version), Some(protocol)) => format!(" (Eon {version}, EONW {protocol})"),
            _ => String::new(),
        };
        return Err(format!(
            "{subject}{versions} is not attachable: {}",
            record.attach.reason
        )
        .into());
    }
    // That build presents its own generation with its own components. The
    // marker stops a record that names this build from delegating again.
    if let Some(eon) = &record.launcher
        && env::var_os(DELEGATION_MARKER).is_none_or(|delegated| delegated != target)
    {
        super::diagnostics::bench_trace("exec", "window-delegation");
        let error = delegation(root, target).command(eon).exec();
        return Err(format!("cannot run {}: {error}", eon.display()).into());
    }
    let (mode, supervisor) = probe_supervisor(inputs, &record.runtime.join("eon.sock"), target)
        .map_err(|error| error.detail)?;
    present_at(inputs, &record.runtime, target, mode, supervisor).map_err(Error::from)
}

/// Whether generation `target` of namespace `root` has a supervisor that has
/// not ended, from its own inspection alone.
pub(super) fn generation_running(
    inputs: &Inputs,
    root: &Path,
    target: &str,
) -> Result<bool, String> {
    generation_running_until(inputs, root, target, None)
}

pub(super) fn generation_running_until(
    inputs: &Inputs,
    root: &Path,
    target: &str,
    deadline: Option<Instant>,
) -> Result<bool, String> {
    Ok(inspect_selected_generation_until(
        inputs,
        root,
        &current_generation(inputs)?,
        target,
        deadline,
    )?
    .is_some_and(|record| record.state != "dead"))
}

/// The generations of the window at `root` whose supervisor has not ended,
/// current first, removing what dead ones left when `remove_dead`.
pub(super) fn running_generations(
    inputs: &Inputs,
    root: &Path,
    remove_dead: bool,
) -> Result<Vec<GenerationRecord>, String> {
    running_generations_until(inputs, root, remove_dead, None)
}

pub(super) fn running_generations_until(
    inputs: &Inputs,
    root: &Path,
    remove_dead: bool,
    deadline: Option<Instant>,
) -> Result<Vec<GenerationRecord>, String> {
    let mut records = discover_generations_until(
        inputs,
        root,
        &current_generation(inputs)?,
        remove_dead,
        deadline,
    )?;
    records.retain(|record| record.state != "dead");
    Ok(records)
}

/// The generation `eon window attach` presents: current unless it has ended,
/// else the window's one other running generation.
pub(super) fn window_generation(inputs: &Inputs, id: &str, root: &Path) -> Result<String, Error> {
    let records = running_generations(inputs, root, true)?;
    if let Some(selected) = preferred_window_generation(
        root,
        &current_generation(inputs)?,
        &records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
    )? {
        return Ok(selected.to_owned());
    }
    match records.as_slice() {
        [] => {
            let mut retained = Vec::new();
            for (_, runtime) in generation_candidates(root)? {
                validate_private_directory(&runtime)?;
                if let Some(identity) = Identity::read(&runtime)? {
                    retained.push(identity.generation);
                }
            }
            match retained.as_slice() {
                [only] => Ok(only.clone()),
                [] => Err(Error::unreachable(format!(
                    "Eon window {id} has ended; no supervisor runs in it"
                ))),
                _ => Err(format!("Eon window {id} has ambiguous recovery records").into()),
            }
        }
        records => Err(format!(
            "Eon window {id} runs generations of {} Eon builds and none is this build's; run `eon window attach {id}` with the `eon` of the one to present: {}",
            records.len(),
            records
                .iter()
                .map(|record| {
                    let version = terminal_text(record.eon_version.as_deref().unwrap_or("unknown"));
                    match &record.launcher {
                        Some(eon) => format!("Eon {version} (its eon: {})", eon.display()),
                        None => format!("Eon {version} (its eon is not recorded)"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

/// A committed record classifies a lifetime; absent records retain build-derived identity.
pub(super) fn active_build(root: &Path, generation: &str) -> Result<String, String> {
    let runtime = generation_directory(root, generation);
    validate_private_directory(&runtime)?;
    Ok(Identity::read(&runtime)?
        .map_or_else(|| generation.into(), |identity| identity.active_build))
}

fn preferred_generation<'a>(current: &str, candidates: &[(&'a str, &str)]) -> Option<&'a str> {
    let active: Vec<_> = candidates
        .iter()
        .filter(|(_, build)| *build == current)
        .collect();
    match active.as_slice() {
        [only] => Some(only.0),
        [] if candidates.len() == 1 => Some(candidates[0].0),
        _ => None,
    }
}

pub(super) fn preferred_window_generation<'a>(
    root: &Path,
    current: &str,
    live: &'a [String],
) -> Result<Option<&'a str>, String> {
    let builds = live
        .iter()
        .map(|id| {
            if valid_generation(id) {
                active_build(root, id)
            } else {
                Ok(id.clone())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = live
        .iter()
        .zip(&builds)
        .map(|(id, build)| (id.as_str(), build.as_str()))
        .collect::<Vec<_>>();
    Ok(preferred_generation(current, &candidates))
}

/// How a human-mode Stop asks first, and how its messages name the target.
pub(super) enum Ask<'a> {
    /// Always, naming the generation and its terminals (`eonbase stop`).
    Stop,
    /// Always, naming the window by its ID (EON-C23) and its terminals.
    StopWindow(&'a str),
    /// Only while a terminal other than `caller`, the terminal running the
    /// command, is busy (EON-C11), naming the window: "this window" when
    /// `this_window`, else its name, else `window`, its ID.
    Quit {
        caller: Option<String>,
        this_window: bool,
        window: &'a str,
    },
}

impl Ask<'_> {
    /// What this command's messages call generation `target`.
    fn subject(&self, target: &str) -> String {
        match self {
            Self::Stop => format!("generation {target}"),
            Self::StopWindow(window) | Self::Quit { window, .. } => format!("window {window}"),
        }
    }
}

/// Quit's question and its decline, or none while nothing is busy. A
/// window whose busy terminals are unknown names every live terminal.
fn quit_question(
    record: &GenerationRecord,
    sessions: &[String],
    busy: Option<&BusyTerminals>,
    this_window: bool,
    window: &str,
) -> Option<(String, String)> {
    let (names, omitted) = match busy {
        Some(busy) if busy.terminals.is_empty() && busy.omitted == 0 => return None,
        Some(busy) => (
            busy.terminals
                .iter()
                .map(|terminal| terminal.name.clone())
                .collect::<Vec<_>>(),
            usize::from(busy.omitted),
        ),
        None => (sessions.to_vec(), 0),
    };
    let window = if this_window {
        "this window".to_owned()
    } else {
        // EONW carries names without control characters.
        format!("window {}", record.name.as_deref().unwrap_or(window))
    };
    let plural = |count: usize| if count == 1 { "" } else { "s" };
    let running = names.len() + omitted;
    let mut question = format!(
        "Quit {window}? It stops {} terminal{}; {running} still running: {}",
        sessions.len(),
        plural(sessions.len()),
        names.join(", ")
    );
    if omitted > 0 {
        question.push_str(&format!(" and {omitted} more"));
    }
    question.push_str(". [y/N] ");
    Some((question, format!("cancelled; {window} keeps running\n")))
}

pub(super) fn stop_generation(
    inputs: &Inputs,
    root: &Path,
    target: &str,
    json: bool,
    ask: &Ask,
) -> Result<i32, String> {
    let current = current_generation(inputs)?;
    let subject = ask.subject(target);
    let control = generation_directory(root, target).join("eon.sock");
    let observed_supervisor = socket_identity(&control);
    let Some(record) = inspect_selected_generation(inputs, root, &current, target)? else {
        return report_failure(
            &failure("unknown-generation", format!("{subject} was not found")),
            json,
        );
    };
    if !record.stop.available {
        return report_failure(
            &failure(
                "stop-unavailable",
                format!("{subject}: {}", record.stop.reason),
            ),
            json,
        );
    }
    let supervisor = match observed_supervisor {
        Ok(Some(supervisor))
            if socket_identity(&control).is_ok_and(|current| current == Some(supervisor)) =>
        {
            supervisor
        }
        _ => {
            return report_failure(
                &failure(
                    "stop-failed",
                    "supervisor endpoint changed while stop was being validated",
                ),
                json,
            );
        }
    };
    let question = match ask {
        _ if json => None,
        _ if record.state == "incompatible" => Some(unknown_terminals_question(&record, ask)),
        Ask::Stop | Ask::StopWindow(_) => Some((
            format!(
                "Stop {subject} and {} live terminal{} [{}]? [y/N] ",
                record.sessions.len(),
                if record.sessions.len() == 1 { "" } else { "s" },
                record.sessions.join(", ")
            ),
            format!("cancelled; no Stop sent for {subject}\n"),
        )),
        Ask::Quit {
            caller,
            this_window,
            window,
        } => {
            // Inspect again as the caller, whose own terminal is not one to
            // ask about; a failed inspection leaves every terminal busy.
            let runtime = probe_generation_runtime_from(&control, caller.clone())
                .ok()
                .filter(|runtime| runtime.generation == target);
            let (sessions, busy) = runtime.map_or((record.sessions.clone(), None), |runtime| {
                (runtime.sessions, runtime.busy)
            });
            quit_question(&record, &sessions, busy.as_ref(), *this_window, window)
        }
    };
    if let Some((question, cancelled)) = question
        && let Some(status) = confirm(&question, &cancelled)?
    {
        return Ok(status);
    }

    let stream = match connect_control(&control) {
        Ok(stream) => stream,
        Err(error) => return report_failure(&failure("stop-failed", error.detail), json),
    };
    if !socket_identity(&control).is_ok_and(|current| current == Some(supervisor)) {
        return report_failure(
            &failure(
                "stop-failed",
                "supervisor endpoint changed before stop could be sent",
            ),
            json,
        );
    }
    match send_stop_on(stream, target) {
        Ok(StopAnswer::Reply(stop::Reply::Stopped {
            generation,
            terminals,
        })) if generation == target => report_stopped(&generation, &terminals, json, &subject),
        Ok(StopAnswer::Reply(stop::Reply::Failure { code, detail })) => {
            report_failure(&Failure { code, detail }, json)
        }
        Ok(StopAnswer::Reply(stop::Reply::Stopped { .. })) => report_failure(
            &failure(
                "stop-failed",
                "supervisor returned the wrong EONW result for stop",
            ),
            json,
        ),
        Ok(StopAnswer::Predates(version)) => report_failure(
            &failure(
                "stop-unavailable",
                format!(
                    "{subject} runs EONW {version}, which predates the reserved Stop; {}",
                    predates_advice(ask)
                ),
            ),
            json,
        ),
        Err(error) => report_failure(&failure("stop-failed", error.detail), json),
    }
}

/// How to end a window whose supervisor predates the Reserved Stop: from
/// inside it, since only its own build reaches it and an older `eon` takes no
/// `window attach ID`.
const QUIT_FROM_INSIDE: &str =
    "quit it from inside its window: Alt+Shift+Q, or `eon quit` in one of its terminals";

/// What to do with a generation whose supervisor predates the Reserved Stop.
fn predates_advice(ask: &Ask) -> &'static str {
    match ask {
        Ask::Stop => "stop it with the build that started it",
        Ask::StopWindow(_) | Ask::Quit { .. } => QUIT_FROM_INSIDE,
    }
}

/// Prints that `generation` stopped `terminals`, calling it `subject` in
/// human mode.
fn report_stopped(
    generation: &str,
    terminals: &[String],
    json: bool,
    subject: &str,
) -> Result<i32, String> {
    if json {
        write_stdout(stopped_json(generation, terminals))?;
    } else {
        write_stdout(format!("stopped {subject}: {}\n", terminals.join(", ")))?;
    }
    Ok(0)
}

/// The question and decline for a Stop of another build's generation: its
/// terminals are unknown here, so it always asks.
fn unknown_terminals_question(record: &GenerationRecord, ask: &Ask) -> (String, String) {
    const UNKNOWN: &str = "It runs another Eon build, so its terminals are unknown. [y/N] ";
    let subject = ask.subject(&record.id);
    match ask {
        Ask::Stop | Ask::StopWindow(_) => (
            format!("Stop {subject}? {UNKNOWN}"),
            format!("cancelled; no Stop sent for {subject}\n"),
        ),
        Ask::Quit { this_window, .. } => {
            let window = if *this_window {
                "this window".to_owned()
            } else {
                subject
            };
            (
                format!("Quit {window}? {UNKNOWN}"),
                format!("cancelled; {window} keeps running\n"),
            )
        }
    }
}

pub(super) fn confirm(question: &str, cancelled: &str) -> Result<Option<i32>, String> {
    eprint!("{question}");
    let mut answer = Vec::new();
    loop {
        let mut byte = [0u8];
        let result = rustix::io::read(std::io::stdin(), &mut byte);
        match result {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => answer.push(byte[0]),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(format!("cannot read stop confirmation: {error}")),
        }
    }
    match confirmation_status(&answer, std::io::stdin().is_terminal()) {
        Confirmation::Confirmed => Ok(None),
        Confirmation::Cancelled => {
            write_stdout(cancelled)?;
            Ok(Some(0))
        }
        Confirmation::Refused => report_failure(
            &failure(
                "not-confirmed",
                format!(
                    "{}; no yes was read and stdin is not a terminal",
                    cancelled.trim_end().trim_start_matches("cancelled; ")
                ),
            ),
            false,
        )
        .map(Some),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Confirmation {
    Confirmed,
    Cancelled,
    Refused,
}

fn confirmation_status(answer: &[u8], terminal: bool) -> Confirmation {
    match answer.trim_ascii() {
        b"y" | b"Y" | b"yes" | b"YES" => Confirmation::Confirmed,
        _ if terminal => Confirmation::Cancelled,
        _ => Confirmation::Refused,
    }
}

/// Stop of every running generation of namespace `root`, current last.
pub(super) fn stop_generations(
    inputs: &Inputs,
    root: &Path,
    json: bool,
    ask: &Ask,
) -> Result<i32, String> {
    let mut records = discover_generations(inputs, root, &current_generation(inputs)?, true)?;
    // The CLI may itself be running inside current; stop that generation last.
    records.sort_by_key(|record| record.kind == "current");
    if json {
        write_stdout("[")?;
    }
    let mut separator = "";
    let mut status = 0;
    for record in records {
        if record.state == "dead" {
            continue;
        }
        if json {
            write_stdout(separator)?;
            separator = ",";
        }
        let result = match stop_generation(inputs, root, &record.id, json, ask) {
            Ok(code) => code,
            Err(error) => report_failure(
                &failure(
                    "stop-failed",
                    format!("{}: {error}", ask.subject(&record.id)),
                ),
                json,
            )?,
        };
        status = status.max(result);
    }
    if json {
        write_stdout("]\n")?;
    }
    Ok(status)
}

fn stopped_json(generation: &str, terminals: &[String]) -> String {
    let mut output = format!(
        "{{\"stopped\":{{\"generation\":\"{}\",\"terminals\":[",
        json_escape(generation)
    );
    for (index, session) in terminals.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!("\"{}\"", json_escape(session)));
    }
    output.push_str("]}}\n");
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_minute_keeps_numeric_timezone_layouts() {
        for (zone, milliseconds, expected, timestamp) in [
            (
                "UTC0",
                0_u64,
                "1970-01-01 00:00",
                "1970-01-01T00:00:00.007+0000",
            ),
            (
                "UTC0",
                253_402_300_800_000,
                "10000-01-01 00:00",
                "10000-01-01T00:00:00.007+0000",
            ),
            (
                "NST3:30",
                0,
                "1969-12-31 20:30",
                "1969-12-31T20:30:00.007-0330",
            ),
            (
                "NPT-5:45",
                0,
                "1970-01-01 05:45",
                "1970-01-01T05:45:00.007+0545",
            ),
            (
                "XST0:00:30",
                0,
                "1969-12-31 23:59",
                "1969-12-31T23:59:30.007-0000",
            ),
            (
                "EST5EDT,M3.2.0/2,M11.1.0/2",
                1_710_053_999_000,
                "2024-03-10 01:59",
                "2024-03-10T01:59:59.007-0500",
            ),
            (
                "EST5EDT,M3.2.0/2,M11.1.0/2",
                1_710_054_000_000,
                "2024-03-10 03:00",
                "2024-03-10T03:00:00.007-0400",
            ),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "generation::tests::local_minute_child",
                    "--nocapture",
                ])
                .env("TZ", zone)
                .env("EON_TEST_LOCAL_MS", milliseconds.to_string())
                .env("EON_TEST_LOCAL_MINUTE", expected)
                .env("EON_TEST_LOCAL_TIMESTAMP", timestamp)
                .output()
                .unwrap();
            assert!(output.status.success(), "{zone}: {output:?}");
        }
    }

    #[test]
    fn local_minute_child() {
        let Ok(expected) = std::env::var("EON_TEST_LOCAL_MINUTE") else {
            return;
        };
        let milliseconds = std::env::var("EON_TEST_LOCAL_MS").unwrap().parse().unwrap();
        assert_eq!(super::local_minute(milliseconds), expected);
        let time = crate::platform::LocalTime::from_seconds(
            libc::time_t::try_from(milliseconds / 1000).unwrap(),
        )
        .unwrap();
        assert_eq!(
            time.timestamp(7),
            std::env::var("EON_TEST_LOCAL_TIMESTAMP").unwrap()
        );
        assert!(crate::platform::LocalTime::from_seconds(libc::time_t::MAX).is_none());
    }

    use super::{
        BusyTerminals, Confirmation, ENDPOINTS, cleanup_metadata, confirmation_status,
        current_generation, discover_generations, generation_directory, generation_id,
        valid_generation,
    };
    use crate::supervisor::{lock_supervisor_lifecycle, supervisor_lock_path, temporary_directory};
    use std::{
        fs,
        io::ErrorKind,
        os::unix::{
            fs::{PermissionsExt, symlink},
            net::{UnixListener, UnixStream},
        },
        path::Path,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn read_only_discovery_skips_bad_entries() {
        use std::os::unix::fs::DirBuilderExt;
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let log = crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
        let bad_id = "g1-11111111111111111111111111111111";
        let good_id = "g1-22222222222222222222222222222222";
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(generation_directory(&base, bad_id))
            .unwrap();
        let bad_lock = supervisor_lock_path(&base, bad_id);
        fs::create_dir(&bad_lock).unwrap();
        let good_root = base.join("w/00000001");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(generation_directory(&good_root, good_id))
            .unwrap();
        let held = lock_supervisor_lifecycle(&supervisor_lock_path(&good_root, good_id)).unwrap();
        let over_limit = base.join("w/00000002");
        for index in 0..=super::MAX_GENERATIONS {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(generation_directory(
                    &over_limit,
                    &format!("g1-{index:032x}"),
                ))
                .unwrap();
        }
        for _ in 0..2 {
            let (inventory, fingerprint) =
                super::live_supervisors(&base, &base, "self", &log).unwrap();
            assert_eq!(
                fingerprint,
                super::super::running::Fingerprint::read(&base, &inventory.observed).unwrap()
            );
            assert_eq!(inventory.entries.len(), 1);
            assert_eq!(inventory.entries[0].generation, good_id);
        }
        let notes = fs::read_dir(base.join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert_eq!(
            notes
                .lines()
                .filter(|line| line.contains(bad_lock.to_str().unwrap()))
                .count(),
            1
        );
        assert_eq!(
            notes
                .lines()
                .filter(|line| line.contains(over_limit.to_str().unwrap()))
                .count(),
            1
        );
        held.unlock().unwrap();
        drop(log);
        fs::remove_dir_all(base).unwrap();
    }

    /// A caller search that cannot check a generation names its window, as
    /// `eon window ls` does, not the generation.
    #[test]
    fn caller_search_failures_name_the_window() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let id = "g1-11111111111111111111111111111111";
        let directory = generation_directory(&base, id);
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(base.join("generations"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        for (current, window) in [
            (id, "default"),
            (
                "g1-22222222222222222222222222222222",
                "11111111111111111111111111111111",
            ),
        ] {
            let error = super::caller_generation(&base, current, "terminal-1.x").unwrap_err();
            assert!(
                error.starts_with(&format!("Eon window {window}: ")),
                "{error}"
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_only_inventory_preserves_build_roles_and_excludes_eonbase() {
        use eon_workspace_protocol::v48::{
            Availability, HEADER_BYTES, LifecycleResponse, Runtime, declared_message_len,
            encode_lifecycle_response,
        };
        use std::io::{Read, Write};
        for full in [false, true] {
            let base = temporary_directory();
            fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
            let id = "g1-11111111111111111111111111111111";
            let directory = generation_directory(&base, id);
            fs::create_dir_all(&directory).unwrap();
            for path in [base.join("generations"), directory.clone()] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let socket = directory.join("eon.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
            let held = lock_supervisor_lifecycle(&supervisor_lock_path(&base, id)).unwrap();
            if full {
                super::Identity {
                    generation: id.into(),
                    created_build: "g1-22222222222222222222222222222222".into(),
                    active_build: "g1-33333333333333333333333333333333".into(),
                    active_launcher: None,
                }
                .publish(&directory)
                .unwrap();
            }
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = vec![0; HEADER_BYTES];
                stream.read_exact(&mut request).unwrap();
                request.resize(declared_message_len(&request).unwrap(), 0);
                stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
                let available = Availability {
                    available: true,
                    reason: "terminal-only supervisor".into(),
                };
                let response = LifecycleResponse::Runtime(Runtime {
                    created_build: if full {
                        "g1-22222222222222222222222222222222"
                    } else {
                        id
                    }
                    .into(),
                    active_build: if full {
                        "g1-33333333333333333333333333333333"
                    } else {
                        id
                    }
                    .into(),
                    terminal_inventory: vec![eon_workspace_protocol::v48::TerminalInventory {
                        terminal: "terminal-1".into(),
                        run: "fixture-run".into(),
                        origin_build: None,
                        management_version: 8,
                        record_generation: 1,
                        orbs: None,
                        orbf: None,
                    }],
                    venus: None,
                    generation: id.into(),
                    name: full.then(|| "peer".into()),
                    started_at: 123,
                    eon_version: "fixture".into(),
                    workspace_protocol: eon_workspace_protocol::v48::VERSION,
                    component_report: "fixture".into(),
                    sessions: vec!["terminal-1".into()],
                    attach: available.clone(),
                    stop: available,
                    busy: None,
                });
                stream
                    .write_all(&encode_lifecycle_response(&response).unwrap())
                    .unwrap();
            });
            let log =
                crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
            let entries = super::live_supervisors(&base, &base.join("w/1234abcd"), id, &log)
                .unwrap()
                .0;
            server.join().unwrap();
            held.unlock().unwrap();
            if full {
                let [entry] = entries.entries.as_slice() else {
                    panic!("{:?}", entries.entries);
                };
                assert_eq!(entry.generation, id);
                assert_eq!(
                    entry.created_build.as_deref(),
                    Some("g1-22222222222222222222222222222222")
                );
                assert_eq!(
                    entry.active_build.as_deref(),
                    Some("g1-33333333333333333333333333333333")
                );
            } else {
                assert!(
                    entries.entries.is_empty(),
                    "eonbase appeared as a window: {:?}",
                    entries.entries
                );
            }
            assert_eq!(entries.observed.len(), 1);
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn read_only_inspection_keeps_dead_generation_residue() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let id = "g1-11111111111111111111111111111111";
        let directory = generation_directory(&base, id);
        fs::create_dir_all(&directory).unwrap();
        for path in [base.join("generations"), directory.clone()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let socket = directory.join("eon.sock");
        drop(UnixListener::bind(&socket).unwrap());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(&socket).is_ok() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        symlink("/bin/false", directory.join("eon")).unwrap();
        let lock = supervisor_lock_path(&base, id);
        let record = super::inspect_generation(
            id,
            "previous",
            directory.clone(),
            &lock,
            "test",
            false,
            false,
            None,
        );
        assert_eq!(record.state, "dead");
        assert!(
            super::live_supervisors(
                &base,
                &base,
                "other",
                &crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test")
            )
            .unwrap()
            .0
            .entries
            .is_empty()
        );
        assert!(!lock.exists(), "read-only discovery created a lock");
        assert!(fs::symlink_metadata(socket).is_ok());
        assert_eq!(
            fs::read_link(directory.join("eon")).unwrap(),
            Path::new("/bin/false")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn generations_json_carries_name_and_started_at() {
        let mut record = super::dead_generation("g1", "previous", "/runtime".into(), "ended");
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(std::slice::from_ref(&record))).unwrap();
        assert_eq!(
            value["generations"][0].get("name"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            value["generations"][0].get("started_at"),
            Some(&serde_json::Value::Null)
        );
        record.state = "live";
        record.name = Some("quoted \"name\"".into());
        record.started_at = Some(123);
        record.busy = Some(BusyTerminals {
            terminals: Vec::new(),
            omitted: 0,
        });
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(std::slice::from_ref(&record))).unwrap();
        assert_eq!(value["generations"][0]["name"], "quoted \"name\"");
        assert_eq!(value["generations"][0]["started_at"], 123);
        assert_eq!(
            value["generations"][0]["busy"],
            serde_json::json!({"terminals": [], "omitted": 0})
        );
        record.busy = None;
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(&[record])).unwrap();
        assert!(value["generations"][0]["busy"].is_null());
    }

    #[test]
    fn quit_asks_only_while_busy_in_window_language() {
        use eon_workspace_protocol::v48::{BusyTerminal, TargetId, TerminalTarget};
        let mut record = super::dead_generation("g1", "current", "/runtime".into(), "ended");
        let sessions = ["terminal-1".to_owned(), "terminal-2".to_owned()];
        let busy = |names: &[&str], omitted| BusyTerminals {
            terminals: names
                .iter()
                .enumerate()
                .map(|(index, name)| BusyTerminal {
                    target: TerminalTarget {
                        workspace: "s1".into(),
                        tab: "t1".into(),
                        id: TargetId::Pane(format!("p{index}")),
                        terminal: format!("terminal-{index}"),
                    },
                    name: (*name).into(),
                })
                .collect(),
            omitted,
        };
        assert_eq!(
            super::quit_question(&record, &sessions, Some(&busy(&[], 0)), true, "default"),
            None
        );
        assert_eq!(
            super::quit_question(
                &record,
                &sessions,
                Some(&busy(&["p2  vim", "Files"], 0)),
                true,
                "default"
            ),
            Some((
                "Quit this window? It stops 2 terminals; 2 still running: p2  vim, Files. [y/N] "
                    .into(),
                "cancelled; this window keeps running\n".into(),
            ))
        );
        // Unknown busy terminals name every live one; outside Eon the
        // question names the window, else its ID.
        assert_eq!(
            super::quit_question(&record, &sessions[..1], None, false, "0a1b"),
            Some((
                "Quit window 0a1b? It stops 1 terminal; 1 still running: terminal-1. [y/N] ".into(),
                "cancelled; window 0a1b keeps running\n".into(),
            ))
        );
        record.name = Some("eon".into());
        assert_eq!(
            super::quit_question(
                &record,
                &sessions,
                Some(&busy(&["Agent"], 4)),
                false,
                "0a1b"
            )
            .unwrap()
            .0,
            "Quit window eon? It stops 2 terminals; 5 still running: Agent and 4 more. [y/N] "
        );
    }

    #[test]
    fn window_json_leaves_out_labels_only_when_asked() {
        use eon_workspace_protocol::v48::{BusyTerminal, TargetId, TerminalTarget};
        let mut record = super::dead_generation("g1", "current", "/runtime".into(), "ended");
        record.name = Some("client-acme-window".into());
        record.busy = Some(BusyTerminals {
            terminals: vec![BusyTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p3".into()),
                    terminal: "terminal-3".into(),
                },
                name: "p3  ~/client-acme".into(),
            }],
            omitted: 2,
        });
        let listed: serde_json::Value =
            serde_json::from_str(&super::window_json(&record, "default", "default", true)).unwrap();
        assert_eq!(listed["name"], "client-acme-window");
        assert_eq!(listed["busy"]["terminals"][0]["name"], "p3  ~/client-acme");
        let reported = super::window_json(&record, "default", "default", false);
        assert!(!reported.contains("client-acme"), "{reported}");
        let reported: serde_json::Value = serde_json::from_str(&reported).unwrap();
        assert_eq!(reported["name"], serde_json::Value::Null);
        assert_eq!(
            reported["busy"],
            serde_json::json!({
                "terminals": [{
                    "session": "s1",
                    "tab": "t1",
                    "pane": "p3",
                    "popup": null,
                    "terminal": "terminal-3",
                }],
                "omitted": 2,
            })
        );
    }

    #[test]
    fn another_builds_question_says_its_terminals_are_unknown() {
        let record = super::dead_generation("g1", "previous", "/runtime".into(), "ended");
        assert_eq!(
            super::unknown_terminals_question(&record, &super::Ask::Stop),
            (
                "Stop generation g1? It runs another Eon build, so its terminals are unknown. [y/N] "
                    .into(),
                "cancelled; no Stop sent for generation g1\n".into(),
            )
        );
        // The window commands name the window by its ID, never the generation.
        assert_eq!(
            super::unknown_terminals_question(&record, &super::Ask::StopWindow("3f9a")),
            (
                "Stop window 3f9a? It runs another Eon build, so its terminals are unknown. [y/N] "
                    .into(),
                "cancelled; no Stop sent for window 3f9a\n".into(),
            )
        );
        assert_eq!(
            super::unknown_terminals_question(
                &record,
                &super::Ask::Quit {
                    caller: None,
                    this_window: false,
                    window: "3f9a",
                }
            ),
            (
                "Quit window 3f9a? It runs another Eon build, so its terminals are unknown. [y/N] "
                    .into(),
                "cancelled; window 3f9a keeps running\n".into(),
            )
        );
    }

    #[test]
    fn delegation_attaches_the_window_by_its_id() {
        use super::{Delegation, delegation};
        let default = crate::supervisor::base_runtime_directory("eon");
        let window = default.join("w").join("0000abcd");
        let generation = "g1-0123456789abcdef0123456789abcdef";
        let expected = |window: Option<&Path>, id: &str| Delegation {
            window: window.map(Path::to_owned),
            generation: generation.into(),
            arguments: ["window", "attach", id].map(str::to_owned).to_vec(),
        };
        assert_eq!(delegation(&default, generation), expected(None, "default"));
        assert_eq!(
            delegation(&window, generation),
            expected(Some(&window), "0000abcd")
        );
    }

    #[test]
    fn confirmation_requires_yes_and_only_terminal_declines_succeed() {
        for terminal in [false, true] {
            for answer in [b"y".as_slice(), b"Y", b"yes", b"YES", b" \tyes\r\n"] {
                assert_eq!(
                    confirmation_status(answer, terminal),
                    Confirmation::Confirmed
                );
            }
            for answer in [
                b"".as_slice(),
                b"n",
                b"no",
                b"Yes",
                b" yes please ",
                b"\xff",
                b"y\xff",
            ] {
                assert_eq!(
                    confirmation_status(answer, terminal),
                    if terminal {
                        Confirmation::Cancelled
                    } else {
                        Confirmation::Refused
                    }
                );
            }
        }
    }

    #[test]
    fn generation_identity_is_stable_bounded_and_namespaced() {
        let first = generation_id(b"abc");
        assert_eq!(first, generation_id(b"abc"));
        assert_ne!(first, generation_id(b"abd"));
        assert!(valid_generation(&first));
        assert!(!valid_generation("legacy"));
        assert!(!valid_generation("g1-0123456789ABCDEF0123456789ABCDEF"));
        assert_eq!(
            generation_directory(Path::new("/runtime/eon"), &first),
            Path::new("/runtime/eon/generations").join(first)
        );
    }

    #[test]
    fn current_generation_consumes_supplied_assembly() {
        let mut inputs = crate::fixtures::inputs();
        let first = current_generation(&inputs).unwrap();
        assert_eq!(first, current_generation(&inputs).unwrap());
        inputs.assembly = b"changed-assembly";
        assert_ne!(first, current_generation(&inputs).unwrap());
    }

    #[test]
    fn discovery_bounds_dead_cleanup_and_rejects_unsafe_metadata() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let generations = root.join("generations");
        fs::create_dir(&generations).unwrap();
        fs::set_permissions(&generations, fs::Permissions::from_mode(0o700)).unwrap();

        let outside = temporary_directory();
        let linked_id = "g1-00000000000000000000000000000000";
        symlink(&outside, generations.join(linked_id)).unwrap();

        let dead_id = "g1-11111111111111111111111111111111";
        let dead = generations.join(dead_id);
        fs::create_dir(&dead).unwrap();
        fs::set_permissions(&dead, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(dead.join("eon.sock")).unwrap();
        fs::set_permissions(dead.join("eon.sock"), fs::Permissions::from_mode(0o600)).unwrap();
        drop(listener);
        // Until it execs, a child forked by a concurrent test shares this
        // test's open files: the dropped listener can briefly keep accepting.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(
            UnixStream::connect(dead.join("eon.sock")),
            Err(error) if error.kind() == ErrorKind::ConnectionRefused
        ) {
            assert!(Instant::now() < deadline, "dead socket kept accepting");
            thread::sleep(Duration::from_millis(1));
        }

        let held = lock_supervisor_lifecycle(&supervisor_lock_path(&root, dead_id)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let inspection_root = root.clone();
        let inspection = thread::spawn(move || {
            sender
                .send(discover_generations(
                    &crate::fixtures::inputs(),
                    &inspection_root,
                    &current_generation(&crate::fixtures::inputs()).unwrap(),
                    true,
                ))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        let preserved = dead.exists();
        // Unlock rather than only drop: a child forked meanwhile shares the
        // lock until it execs, and only an explicit unlock releases it there.
        held.unlock().unwrap();
        drop(held);
        inspection.join().unwrap();

        let records = result
            .expect("generation inspection waited for startup")
            .unwrap();
        assert!(records.iter().any(|record| {
            record.id == linked_id && record.kind == "previous" && record.state == "corrupt"
        }));
        assert!(records.iter().any(|record| {
            record.id == dead_id && record.kind == "previous" && record.state == "dead"
        }));
        assert!(
            preserved,
            "contended inspection removed dead control metadata"
        );

        discover_generations(
            &crate::fixtures::inputs(),
            &root,
            &current_generation(&crate::fixtures::inputs()).unwrap(),
            true,
        )
        .unwrap();
        assert!(!dead.exists());
        assert!(outside.is_dir());

        fs::remove_file(generations.join(linked_id)).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    // EON-C11's cleanup rule: a dead update.sock is removed like eon.sock,
    // a live one keeps the directory.
    #[test]
    fn dead_generation_cleanup_accepts_update_socket() {
        use std::os::unix::net::UnixListener;
        for live in [false, true] {
            let runtime = temporary_directory();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            for name in ENDPOINTS {
                drop(UnixListener::bind(runtime.join(name)).unwrap());
            }
            let listener = live.then(|| {
                fs::remove_file(runtime.join("update.sock")).unwrap();
                UnixListener::bind(runtime.join("update.sock")).unwrap()
            });
            let result = cleanup_metadata(&runtime);
            if live {
                let error = result.unwrap_err();
                assert!(error.contains("unsafe or active object"), "{error}");
                assert!(runtime.join("update.sock").exists());
                assert!(!runtime.join("eon.sock").exists());
                drop(listener);
                fs::remove_dir_all(&runtime).unwrap();
            } else {
                result.unwrap();
                assert!(!runtime.exists());
            }
        }
    }

    #[test]
    fn a_busy_listener_neither_stalls_nor_loses_its_socket() {
        let runtime = temporary_directory();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        drop(std::os::unix::net::UnixListener::bind(runtime.join("eon.sock")).unwrap());
        let busy = runtime.join("update.sock");
        let (_listener, _queued) = crate::fixtures::busy_listener(&busy);
        let directory = runtime.clone();
        let path = busy.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let probe = thread::spawn(move || {
            let cleanup = cleanup_metadata(&directory);
            let bind = crate::control::PrivateListener::bind(&path, "Eon Update").map(drop);
            sender.send((cleanup, bind)).unwrap();
        });
        let (cleanup, bind) = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the socket probe blocked on a busy listener");
        probe.join().unwrap();
        let error = cleanup.unwrap_err();
        assert!(error.contains("unsafe or active object"), "{error}");
        let error = bind.unwrap_err();
        assert!(error.contains("already active"), "{error}");
        assert!(busy.exists(), "a busy socket is kept");
        fs::remove_dir_all(&runtime).unwrap();
    }
}
