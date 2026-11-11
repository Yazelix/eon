use super::cli::Inputs;
use super::control::{
    CONTROL_TIMEOUT, ControlResponse, EndpointFailure, EndpointFailureKind, Error, StopAnswer,
    connect_control, failure, probe_generation_runtime, probe_generation_runtime_from,
    probe_generation_runtime_until, remove_socket_if_identity, report_failure,
    send_action_from_until, send_stop_on, socket_identity, socket_identity_from, write_stdout,
};
use super::sessions::terminal_session;
use super::supervisor::{
    base_runtime_directory, effective_uid, present_at, probe_supervisor, runtime_directory,
    supervisor_lock_path, try_lock_supervisor_lifecycle, validate_private_directory,
};
use super::windows::{
    DEFAULT_NOT_RUNNING, DEFAULT_WINDOW, WINDOW_RUNTIME, Window, default_namespace_id,
    runtime_window,
};
use super::workspace::{json_escape, terminal_text};
use eon_workspace_protocol::stop;
use eon_workspace_protocol::v40::{
    Action, Availability, BusyTerminals, Failure, MAX_RUNNING_WINDOWS, Response, WindowEntry,
    WindowState, WorkspaceAction,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::IsTerminal,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, symlink},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

/// This build's exact identity, `g1-…`, which `eon versions` reports and generations carry.
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
    value.len() == 35
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn generation_directory(root: &Path, generation: &str) -> PathBuf {
    root.join("generations").join(generation)
}

/// The link in a generation's directory to the `eon` of the build that started
/// it, which a later build runs to present a generation it cannot present.
const LAUNCHER: &str = "eon";
/// Set on a delegated attach to the generation it presents, and removed from
/// everything Eon starts.
pub(super) const DELEGATION_MARKER: &str = "EON_ATTACH_DELEGATED";

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
    Delegation {
        window: (!is_default_namespace(root)).then(|| root.to_owned()),
        generation: generation.to_owned(),
        arguments: vec!["window".into(), "attach".into(), window],
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

/// The generation a delegated attach names, else none: set, the marker must
/// name this build's own generation, since a build is only ever delegated its
/// own (EON-C11). It narrows the named window to that generation and never
/// selects another.
pub(super) fn delegated_generation(inputs: &Inputs) -> Result<Option<String>, Error> {
    let Some(value) = env::var_os(DELEGATION_MARKER) else {
        return Ok(None);
    };
    let current = current_generation(inputs)?;
    if value != current.as_str() {
        return Err(Error::usage(format!(
            "{DELEGATION_MARKER} does not name this Eon's generation; nothing was sent"
        )));
    }
    Ok(Some(current))
}

/// The failure for the marker on window `shown`, which does not run this
/// build's generation.
pub(super) fn delegated_elsewhere(shown: &str) -> Error {
    Error::usage(format!(
        "{DELEGATION_MARKER} names this Eon's generation, which window {shown} does not run; nothing was sent"
    ))
}

/// Set on a routed session command to the generation it was routed to, and
/// removed from everything Eon starts (EON-C11, Routing).
pub(super) const ROUTE_MARKER: &str = "EON_ROUTED";

/// Runs `command` as the build that started generation `generation` of
/// namespace `root`, through its recorded `eon`, in place of this `eon`
/// (EON-C11, Routing). It returns only when it does not run it: with nothing
/// to add when no `eon` is recorded, else with why. With the marker set, to
/// any value, it never runs it, so one command makes at most one hop.
fn route(root: &Path, generation: &str, command: &[OsString]) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    let eon = recorded_launcher(&generation_directory(root, generation))?;
    if let Some(routed) = env::var_os(ROUTE_MARKER) {
        return Some(format!(
            "{ROUTE_MARKER} is {:?}, so this `eon` was itself routed and does not run {}",
            routed.to_string_lossy(),
            eon.display()
        ));
    }
    if !eon.is_file() {
        return Some(format!(
            "the `eon` that started it, {}, no longer exists",
            eon.display()
        ));
    }
    super::diagnostics::bench_trace("exec", "generation-route");
    let error = Command::new(&eon)
        .args(command)
        .env(ROUTE_MARKER, generation)
        .exec();
    Some(format!("cannot run {}: {error}", eon.display()))
}

/// Records this build's `eon`, from its session bin, in the workspace
/// generation directory `runtime`.
pub(super) fn record_launcher(inputs: &Inputs, runtime: &Path) -> Result<(), String> {
    let Some(eon) = inputs.managed.shell_bin.as_ref().map(|bin| bin.join("eon")) else {
        return Ok(());
    };
    if !eon.is_absolute() || !eon.is_file() {
        return Ok(());
    }
    remove_launcher(runtime);
    let link = runtime.join(LAUNCHER);
    symlink(&eon, &link).map_err(|error| format!("cannot record {}: {error}", link.display()))
}

/// Removes a generation's launcher record so its directory can be removed.
pub(super) fn remove_launcher(runtime: &Path) {
    let link = runtime.join(LAUNCHER);
    if fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let _ = fs::remove_file(link);
    }
}

fn recorded_launcher(runtime: &Path) -> Option<PathBuf> {
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

const MAX_GENERATIONS: usize = 256;

#[derive(Clone, Default)]
pub(super) struct WorkspaceTarget {
    /// An EON-C23 window ID or prefix, as typed.
    pub(super) window: Option<String>,
    /// The arguments the build of a generation that speaks another EONW runs
    /// in this `eon`'s place, else empty: never routed (EON-C11, Routing).
    pub(super) command: Vec<OsString>,
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
    target: WorkspaceTarget,
    caller: Option<&str>,
) -> Result<ResolvedWorkspace, Failure> {
    let current =
        current_generation(inputs).map_err(|detail| failure("supervisor-unavailable", detail))?;
    // A default-namespace window names its exact generation; an independent
    // window names its namespace, where the window-only rule selects.
    let (root, exact, shown) = match &target.window {
        Some(id) => match super::windows::select_window(inputs, id)? {
            (Window::Default, shown) => {
                (base_runtime_directory("eon"), Some(current.clone()), shown)
            }
            (Window::Other(generation), shown) => {
                (base_runtime_directory("eon"), Some(generation), shown)
            }
            (Window::Independent { root, .. }, shown) => (root, None, shown),
        },
        None => (runtime_directory("eon"), None, String::new()),
    };
    let window_only = target.window.is_some() && exact.is_none();
    let window = super::windows::runtime_window(&root);
    let label = |generation: &str| {
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
    // command as the build that started it.
    let refused = |generation: &str, (kind, detail): (EndpointFailureKind, String)| {
        let routed = (kind == EndpointFailureKind::Incompatible)
            .then(|| route(&root, generation, &target.command))
            .flatten();
        unavailable(
            generation,
            match routed {
                Some(why) => format!("{detail}; {why}"),
                None => detail,
            },
        )
    };
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let probe = |id: &str| {
        let path = generation_directory(&root, id);
        validate_private_directory(&path)
            .map_err(|detail| (EndpointFailureKind::Corrupt, detail))?;
        let socket = path.join("eon.sock");
        match probe_generation_runtime_until(&socket, deadline) {
            Ok(runtime) if runtime.generation == id => Ok(()),
            Ok(runtime) => Err((
                EndpointFailureKind::Corrupt,
                format!("supervisor reports generation {}", runtime.generation),
            )),
            Err(error) => Err((error.kind, error.detail)),
        }
    };
    // A plain outside-Eon command keeps its current-generation default and existing failures.
    if caller.is_none() && target.window.is_none() {
        if !workspace_runtime_exists(&root, &current)
            .map_err(|detail| unavailable(&current, detail))?
        {
            return Err(failure(
                "missing-supervisor",
                match &window {
                    Some(id) => format!("Eon window {id} is not running; see `eon window ls`"),
                    None => DEFAULT_NOT_RUNNING.into(),
                },
            ));
        }
        probe(&current).map_err(|error| unavailable(&current, error.1))?;
        return Ok(selected(current.clone()));
    }

    let generation = if let Some(id) = exact {
        let present =
            workspace_runtime_exists(&root, &id).map_err(|detail| unavailable(&id, detail))?;
        if !present && id == current {
            return Err(failure("missing-supervisor", DEFAULT_NOT_RUNNING));
        }
        if !present {
            return Err(failure(
                "unknown-window",
                format!("Eon window {shown} was not found"),
            ));
        }
        probe(&id).map_err(|error| refused(&id, error))?;
        id
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
            match preferred_window_generation(&current, &live) {
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
                return Ok(selected(owner));
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
    // Messages name windows by their full EON-C23 ID; `current` is this
    // build's generation, the default namespace's `default`.
    let window = runtime_window(root);
    let label = |id: &str| {
        window
            .clone()
            .unwrap_or_else(|| default_namespace_id(id, current))
    };
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let mut owners = Vec::new();
    let mut live = Vec::new();
    let mut failures = Vec::new();
    for (id, path) in generation_candidates(root)? {
        let Some(id) = id.to_str().filter(|id| valid_generation(id)) else {
            continue;
        };
        if let Err(detail) = validate_private_directory(&path) {
            failures.push((id.to_owned(), format!("Eon window {}: {detail}", label(id))));
            continue;
        }
        let socket = path.join("eon.sock");
        let validated = probe_generation_runtime_until(&socket, deadline).and_then(|runtime| {
            if runtime.generation == id {
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
            Err(error) => failures.push((
                id.to_owned(),
                format!("Eon window {}: {}", label(id), error.detail),
            )),
        }
        live.push(id.to_owned());
    }
    match owners.as_slice() {
        [owner] => Ok(owner.clone()),
        [] => {
            // As `resolve_workspace`: the caller's Orbit log narrows its owner.
            let origin = super::diagnostics::orbit_run_generation(caller);
            failures.retain(|(id, _)| origin.as_ref().is_none_or(|origin| origin == id));
            Err(failures.into_iter().next().map_or_else(
                || stale_caller(caller, origin.map(|id| (label(&id), live.contains(&id)))),
                |(_, failure)| failure,
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
        "the calling terminal is not live in this Eon runtime namespace",
    )
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
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(Vec::new());
    }
    let parent = root.join("generations");
    let mut candidates = Vec::new();
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
            candidates.push((entry.file_name(), entry.path()));
            if candidates.len() > MAX_GENERATIONS {
                return Err(format!(
                    "generation directory {} exceeds the {MAX_GENERATIONS}-entry inspection limit",
                    parent.display()
                ));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(candidates)
}
#[derive(Debug)]
pub(super) struct GenerationRecord {
    pub(super) id: String,
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
    detail: String,
    /// The recorded `eon` of the other build that started it, which presents
    /// or stops it when this build cannot.
    launcher: Option<PathBuf>,
}

impl GenerationRecord {
    /// Whether this build started it.
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
    let component_report = inputs.component_report()?;
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(vec![unstarted_current_generation(root, current)]);
    }

    let candidates = generation_candidates(root)?;

    let current_path = generation_directory(root, current);
    let mut records = vec![inspect_generation(
        current,
        "current",
        current_path,
        &supervisor_lock_path(root, current),
        component_report,
        true,
        remove_dead,
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
            ));
        }
    }
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
                            inspect_generation(&id, "previous", runtime, &lock, "", false, false)
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
    )))
}

fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    lifecycle_lock: &Path,
    current_components: &str,
    cleanup: bool,
    remove_dead: bool,
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

    let socket = runtime.join("eon.sock");
    let probe = if cleanup {
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
                             the Eon build that started it can attach it, and any later \
                             Eon build can stop it"
                        .into(),
                };
            }
            GenerationRecord {
                id: id.into(),
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
                && let Ok(Some(_lifecycle_lock)) = try_lock_supervisor_lifecycle(lifecycle_lock)
            {
                remove_dead_socket(&socket);
                remove_launcher(&runtime);
                let _ = fs::remove_dir(&runtime);
            }
            record
        }
    };
    if record.launcher.is_some() {
        record.attach = Availability {
            available: true,
            reason: "attach runs the Eon build that started it".into(),
        };
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

fn remove_dead_socket(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return;
    }
    let identity = socket_identity_from(&metadata);
    if matches!(
        UnixStream::connect(path),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    ) {
        remove_socket_if_identity(path, identity);
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
        output.push_str("],\"busy\":");
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
        "],\"venus\":{},\"busy\":",
        json_option(record.venus.as_deref())
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
    // SAFETY: tm is plain data that localtime_r fills; it reports failure as null.
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
        return "-".into();
    }
    let mut buffer = [0u8; 32];
    // SAFETY: strftime writes at most buffer.len() bytes and returns how many.
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%Y-%m-%d %H:%M".as_ptr(),
            &tm,
        )
    };
    if length == 0 {
        return "-".into();
    }
    String::from_utf8_lossy(&buffer[..length]).into_owned()
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

/// The raw names of the entries under `root`'s generation directory that name
/// no generation.
pub(super) fn damaged_generations(root: &Path) -> Result<Vec<String>, String> {
    Ok(generation_candidates(root)?
        .into_iter()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| !valid_generation(name))
        .collect())
}

/// Whether generation `target` of namespace `root` has a supervisor that has
/// not ended, from its own inspection alone.
pub(super) fn generation_running(
    inputs: &Inputs,
    root: &Path,
    target: &str,
) -> Result<bool, String> {
    Ok(
        inspect_selected_generation(inputs, root, &current_generation(inputs)?, target)?
            .is_some_and(|record| record.state != "dead"),
    )
}

/// The generations of the window at `root` whose supervisor has not ended,
/// current first, removing what dead ones left when `remove_dead`.
pub(super) fn running_generations(
    inputs: &Inputs,
    root: &Path,
    remove_dead: bool,
) -> Result<Vec<GenerationRecord>, String> {
    let mut records =
        discover_generations(inputs, root, &current_generation(inputs)?, remove_dead)?;
    records.retain(|record| record.state != "dead");
    Ok(records)
}

/// The generation `eon window attach` presents: current unless it has ended,
/// else the window's one other running generation.
pub(super) fn window_generation(inputs: &Inputs, id: &str, root: &Path) -> Result<String, Error> {
    let records = running_generations(inputs, root, true)?;
    if let Some(selected) = preferred_window_generation(
        &current_generation(inputs)?,
        &records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
    ) {
        return Ok(selected.to_owned());
    }
    match records.as_slice() {
        [] => Err(Error::unreachable(format!(
            "Eon window {id} has ended; no supervisor runs in it"
        ))),
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

fn preferred_window_generation<'a>(current: &str, live: &'a [String]) -> Option<&'a str> {
    live.iter()
        .find(|id| *id == current)
        .or_else(|| (live.len() == 1).then(|| &live[0]))
        .map(String::as_str)
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
        let mut byte = 0u8;
        // SAFETY: read writes at most one byte into the valid local buffer.
        let result = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        match result {
            0 => break,
            1 if byte == b'\n' => break,
            1 => answer.push(byte),
            _ => {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(format!("cannot read stop confirmation: {error}"));
                }
            }
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
    use super::{
        BusyTerminals, Confirmation, confirmation_status, current_generation, discover_generations,
        generation_directory, generation_id, valid_generation,
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
    fn read_only_discovery_excludes_terminal_only_supervisors() {
        use eon_workspace_protocol::v40::{
            Availability, HEADER_BYTES, LifecycleResponse, Runtime, declared_message_len,
            encode_lifecycle_response,
        };
        use std::io::{Read, Write};
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
                venus: None,
                generation: id.into(),
                name: None,
                started_at: 123,
                eon_version: "fixture".into(),
                workspace_protocol: eon_workspace_protocol::v40::VERSION,
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
        let log = crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
        let entries = super::live_supervisors(&base, &base.join("w/1234abcd"), id, &log)
            .unwrap()
            .0;
        server.join().unwrap();
        held.unlock().unwrap();
        assert!(
            entries.entries.is_empty(),
            "eonbase appeared as a window: {:?}",
            entries.entries
        );
        assert_eq!(entries.observed.len(), 1);
        fs::remove_dir_all(base).unwrap();
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
        use eon_workspace_protocol::v40::{BusyTerminal, TargetId, TerminalTarget};
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
        use eon_workspace_protocol::v40::{BusyTerminal, TargetId, TerminalTarget};
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
}
