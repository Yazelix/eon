use super::{
    control::{
        ControlListener, ControlResponse, EndpointFailure, EndpointFailureKind, SocketIdentity,
        connect_control, failure, probe_launch_mode, probe_presentable_runtime, send_action_on,
        socket_identity,
    },
    generation::{current_generation, generation_directory},
    managed_environment::{self, configured_program, nonempty_environment_path},
    sessions::{
        RunningSession, recover_sessions, session_finished, start_orbit, stop_managed_sessions,
    },
    workspace::{self, Workspace},
};
use eon_workspace_protocol::v4::{
    Action, Availability, Failure, LifecycleResponse, Request, Response, Runtime, Snapshot,
    Stopped, VERSION,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::Write,
    os::fd::OwnedFd,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const MANIFEST: &str = include_str!("../../../components/eon-alpha-v3.json");
pub(super) const EON_ANSI_PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";
pub(super) const SESSION_START_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

pub(super) fn request_id() -> String {
    let sequence = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}-{sequence}", std::process::id())
}

#[cfg(test)]
static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(super) fn temporary_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "eon-test-{}-{}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

pub(super) fn probe_supervisor(
    socket: &Path,
    generation: &str,
) -> Result<(LaunchMode, SocketIdentity), EndpointFailure> {
    let identity = socket_identity(socket)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
        .ok_or_else(|| {
            EndpointFailure::new(
                EndpointFailureKind::Dead,
                format!("endpoint {} is missing", socket.display()),
            )
        })?;
    let result = (|| {
        let info = probe_presentable_runtime(socket)?;
        if !info.attach.available {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Incompatible,
                info.attach.reason,
            ));
        }
        validate_runtime(&info, generation)
            .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?;
        probe_launch_mode(socket)
    })();
    if socket_identity(socket)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
        != Some(identity)
    {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Dead,
            "supervisor endpoint changed while it was being validated",
        ));
    }
    result.map(|mode| (mode, identity))
}

fn validate_owned_private_directory(path: &Path, kind: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {kind} {}: {error}", path.display()))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "{kind} {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn validate_private_directory(path: &Path) -> Result<(), String> {
    validate_owned_private_directory(path, "directory")
}

pub(super) fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub(super) struct Programs {
    pub(super) orbit: PathBuf,
    pub(super) venus: PathBuf,
    pub(super) session_bin: Option<PathBuf>,
    pub(super) venus_decorations: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LaunchMode {
    Workspace,
    Terminal,
}

impl LaunchMode {
    fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Terminal => "EonTerm",
        }
    }
}

pub(super) fn launch_current(
    mode: LaunchMode,
    child: &[OsString],
    attach_existing: bool,
    decorations: bool,
    application_id: &str,
) -> Result<i32, String> {
    let generation = current_generation()?;
    let root = runtime_directory(if mode == LaunchMode::Terminal {
        "eonterm"
    } else {
        "eon"
    });
    prepare_runtime(&root)?;
    let lifecycle_lock_path = supervisor_lock_path(&root, &generation);
    let runtime = generation_directory(&root, &generation);
    let socket = runtime.join("eon.sock");
    let mut missing_deadline = None;
    let lifecycle_lock = loop {
        let endpoint_failure = match probe_supervisor(&socket, &generation) {
            Ok((active_mode, supervisor)) => {
                missing_deadline = None;
                if active_mode != mode {
                    return Err(format!(
                        "generation {generation} already has a live {} mode; requested {} mode",
                        active_mode.name(),
                        mode.name()
                    ));
                }
                if !attach_existing {
                    return Err(format!(
                        "generation {generation} already has a live Eon supervisor"
                    ));
                }
                let presentation_error = match present_at(&runtime, &generation, mode, supervisor) {
                    Ok(code) => return Ok(code),
                    Err(error) => error,
                };
                match probe_supervisor(&socket, &generation) {
                    Ok((_, current)) if current == supervisor => return Err(presentation_error),
                    Ok(_) => continue,
                    Err(error)
                        if matches!(
                            error.kind,
                            EndpointFailureKind::Dead | EndpointFailureKind::Unreachable
                        ) =>
                    {
                        error
                    }
                    Err(_) => return Err(presentation_error),
                }
            }
            Err(error)
                if matches!(
                    error.kind,
                    EndpointFailureKind::Dead | EndpointFailureKind::Unreachable
                ) =>
            {
                error
            }
            Err(error) => return Err(error.detail),
        };
        let lifecycle_lock = if endpoint_failure.kind == EndpointFailureKind::Dead {
            let Some(lock) = try_lock_supervisor_lifecycle(&lifecycle_lock_path)? else {
                let deadline = missing_deadline.get_or_insert_with(|| {
                    Instant::now() + SESSION_START_TIMEOUT.saturating_add(Duration::from_secs(1))
                });
                if Instant::now() >= *deadline {
                    return Err(format!(
                        "timed out waiting for Eon supervisor lifecycle {}",
                        lifecycle_lock_path.display()
                    ));
                }
                thread::sleep(Duration::from_millis(25));
                continue;
            };
            lock
        } else {
            lock_supervisor_lifecycle(&lifecycle_lock_path)?
        };
        match probe_supervisor(&socket, &generation) {
            Err(error) if error.kind == EndpointFailureKind::Dead => break lifecycle_lock,
            Ok(_) => {
                drop(lifecycle_lock);
                continue;
            }
            Err(error) => return Err(error.detail),
        }
    };
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_generation_runtime(&root, &generation)?;
    prepare_configuration(&config)?;
    let programs = programs(decorations);
    supervise(
        &programs,
        &config,
        terminal,
        lifecycle_lock,
        &runtime,
        child,
        &generation,
        mode,
        application_id,
    )
    .or_else(|error| {
        if attach_existing && path_exists(&socket) {
            attach_competing_supervisor(&socket, &runtime, &generation, mode, &error)
        } else {
            Err(error)
        }
    })
}

fn attach_competing_supervisor(
    socket: &Path,
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    launch_error: &str,
) -> Result<i32, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match probe_supervisor(socket, generation) {
            Ok((active_mode, supervisor)) => {
                if active_mode != mode {
                    return Err(format!(
                        "{launch_error}; competing supervisor started in {} mode, requested {} mode",
                        active_mode.name(),
                        mode.name()
                    ));
                }
                return present_at(runtime, generation, mode, supervisor);
            }
            Err(error)
                if matches!(
                    error.kind,
                    EndpointFailureKind::Dead | EndpointFailureKind::Unreachable
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(format!(
                    "{launch_error}; competing supervisor did not become attachable: {}",
                    error.detail
                ));
            }
        }
    }
}

fn validate_runtime(info: &Runtime, generation: &str) -> Result<(), String> {
    if info.generation != generation {
        return Err(format!(
            "runtime directory contains generation {}, expected {generation}",
            info.generation
        ));
    }
    if info.workspace_protocol != VERSION {
        return Err(format!(
            "supervisor uses EONW {}, expected EONW {VERSION}",
            info.workspace_protocol
        ));
    }
    let components = eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?;
    if info.component_report != components {
        return Err("supervisor reports a different component graph".into());
    }
    Ok(())
}

pub(super) fn attach_legacy(runtime: &Path) -> Result<i32, String> {
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_configuration(&config)?;
    let programs = programs(true);
    venus_command(
        &programs,
        &config,
        &runtime.join("eon.sock"),
        LaunchMode::Workspace,
        terminal,
        "eon",
    )
    .spawn()
    .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?
    .wait()
    .map(status_code)
    .map_err(|error| format!("cannot observe Eon Desktop: {error}"))
}

pub(super) fn present_at(
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    supervisor: SocketIdentity,
) -> Result<i32, String> {
    let control = runtime.join("eon.sock");
    let stream = connect_control(&control).map_err(|error| error.detail)?;
    if socket_identity(&control)? != Some(supervisor) {
        return Err("supervisor changed before Eon Desktop could present".into());
    }
    match send_action_on(
        stream,
        Action::Present {
            workspace: mode == LaunchMode::Workspace,
        },
    )
    .map_err(|error| error.detail)?
    {
        ControlResponse::Lifecycle(LifecycleResponse::Runtime(info)) => {
            validate_runtime(&info, generation)?;
            Ok(0)
        }
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure)) => {
            Err(format!("cannot present Eon Desktop: {}", failure.detail))
        }
        _ => Err("supervisor returned the wrong EONW result for presentation".into()),
    }
}

fn venus_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: managed_environment::TerminalConfig,
    application_id: &str,
) -> Command {
    let mut command = Command::new(&programs.venus);
    if !programs.venus_decorations {
        command.arg("--no-decorations");
    }
    command
        .arg("--application-id")
        .arg(application_id)
        .arg("--background-opacity")
        .arg(terminal.background_opacity.to_string());
    if terminal.background_blur {
        command.arg("--background-blur");
    }
    if mode == LaunchMode::Workspace {
        command.arg("--workspace");
    }
    command.arg(socket);
    command.env("XDG_CONFIG_HOME", config);
    command
}

struct PresentationProcess {
    child: Child,
    control: UnixStream,
}

impl PresentationProcess {
    fn start(mut command: Command) -> Result<Self, String> {
        let (control, input) = UnixStream::pair()
            .map_err(|error| format!("cannot create Eon Desktop presentation control: {error}"))?;
        control
            .set_write_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| format!("cannot bound Eon Desktop presentation control: {error}"))?;
        command
            .env("EON_VENUS_PRESENTATION_CONTROL", "stdin")
            .stdin(Stdio::from(OwnedFd::from(input)));
        let child = command
            .spawn()
            .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?;
        Ok(Self { child, control })
    }

    fn present(&self) -> Result<(), String> {
        (&self.control)
            .write_all(b"present\n")
            .map_err(|error| format!("cannot signal Eon Desktop presentation: {error}"))
    }
}

fn close_presentation(process: PresentationProcess) {
    let PresentationProcess { mut child, control } = process;
    drop(control);
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    stop(&mut child);
}

fn programs(venus_decorations: bool) -> Programs {
    Programs {
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        session_bin: nonempty_environment_path("EON_SESSION_BIN"),
        venus_decorations,
    }
}

pub(super) fn configuration_directory() -> Result<PathBuf, String> {
    let path = nonempty_environment_path("EON_CONFIG_HOME")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_CONFIG_HOME")).map(|path| path.join("eon"))
        })
        .or_else(|| nonempty_environment_path("HOME").map(|path| path.join(".config/eon")))
        .ok_or_else(|| "HOME, XDG_CONFIG_HOME, and EON_CONFIG_HOME are unset".to_string())?;
    std::path::absolute(path)
        .map_err(|error| format!("cannot resolve Eon configuration root: {error}"))
}

pub(super) fn prepare_configuration(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

pub(super) fn runtime_directory(product: &str) -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join(product))
        })
        .unwrap_or_else(|| {
            eprintln!(
                "{product}: warning: XDG_RUNTIME_DIR is unset; using a private temporary runtime root"
            );
            env::temp_dir().join(format!("{product}-{}", effective_uid()))
        })
}

pub(super) fn prepare_generation_runtime(root: &Path, generation: &str) -> Result<PathBuf, String> {
    prepare_runtime(root)?;
    prepare_runtime(&root.join("generations"))?;
    let runtime = generation_directory(root, generation);
    prepare_runtime(&runtime)?;
    Ok(runtime)
}

fn xdg_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| path.is_absolute())
}

fn prepare_runtime(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| {
            format!(
                "cannot create runtime directory {}: {error}",
                path.display()
            )
        })?;
    validate_owned_private_directory(path, "runtime directory")
}

pub(super) fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and no failure state.
    unsafe { libc::geteuid() }
}

#[allow(clippy::too_many_arguments)]
fn supervise(
    programs: &Programs,
    config: &Path,
    terminal: managed_environment::TerminalConfig,
    lifecycle_lock: fs::File,
    runtime: &Path,
    child: &[OsString],
    generation: &str,
    mode: LaunchMode,
    application_id: &str,
) -> Result<i32, String> {
    let _lifecycle_lock = lifecycle_lock;
    let launch_directory = env::current_dir()
        .map_err(|error| format!("cannot resolve Eon launch directory: {error}"))?;
    if mode == LaunchMode::Workspace {
        workspace::validate_initial_directory(&launch_directory)?;
    }
    let component_generation =
        eon_manifest::component_revision(MANIFEST, "orbit").map_err(|error| error.to_string())?;
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    let (mut sessions, recovered_picker) =
        recover_sessions(runtime, mode, &component_generation, deadline)?;
    let control_listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    let mut initial_child = if sessions.is_empty() {
        child.to_vec()
    } else {
        Vec::new()
    };
    if sessions.is_empty() && (mode == LaunchMode::Terminal || recovered_picker) {
        sessions.push(start_orbit(
            programs,
            config,
            &runtime.join("orbit.sock"),
            "session-1",
            &component_generation,
            &launch_directory,
            &initial_child,
            deadline,
        )?);
        initial_child.clear();
    }
    let mut directory_picker = None;
    let workspace = if mode == LaunchMode::Workspace {
        Some(if sessions.is_empty() {
            Workspace::pending(
                runtime.to_path_buf(),
                launch_directory.clone(),
                |session, directory, picker_tab| {
                    start_workspace_session(
                        programs,
                        config,
                        &component_generation,
                        &mut sessions,
                        &mut directory_picker,
                        &mut initial_child,
                        session,
                        directory,
                        picker_tab,
                    )
                },
            )?
        } else {
            Workspace::with_recovered_sessions(
                runtime.to_path_buf(),
                launch_directory,
                sessions
                    .iter()
                    .map(|session| {
                        Ok((
                            session
                                .number
                                .ok_or("recovered a transient Session as a durable pane")?,
                            workspace::Session {
                                id: session.id.clone(),
                                endpoint: session.endpoint.clone(),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            )?
        })
    } else {
        None
    };
    let presentation_socket = if mode == LaunchMode::Workspace {
        runtime.join("eon.sock")
    } else {
        sessions
            .first()
            .ok_or("EonTerm supervisor has no live Session")?
            .endpoint
            .clone()
    };
    let command = venus_command(
        programs,
        config,
        &presentation_socket,
        mode,
        terminal,
        application_id,
    );
    let mut state = SupervisorState {
        component_generation,
        application_id: application_id.into(),
        workspace,
        sessions,
        directory_picker,
        venus: None,
        initial_child,
        initial_status: None,
    };
    state.venus = match PresentationProcess::start(command) {
        Ok(venus) => Some(venus),
        Err(error) => {
            let picker_cleanup = stop_directory_picker(&mut state);
            let session_cleanup = stop_managed_sessions(&mut state.sessions, SESSION_START_TIMEOUT);
            drop(control_listener);
            if picker_cleanup.is_ok() && session_cleanup.is_ok() {
                let _ = fs::remove_dir(runtime);
            }
            let mut errors = vec![error];
            if let Err(cleanup) = picker_cleanup {
                errors.push(format!("cannot roll back directory picker: {cleanup}"));
            }
            if let Err(cleanup) = session_cleanup {
                errors.push(format!("cannot roll back Sessions: {cleanup}"));
            }
            return Err(errors.join("; "));
        }
    };

    let status = (|| {
        loop {
            reap_finished_sessions(&mut state, programs, config)?;

            if state.sessions.is_empty() && state.directory_picker.is_none() {
                return Ok(state.initial_status.unwrap_or(0));
            }

            if reap_presentation(&mut state, programs, config)? {
                match mode {
                    LaunchMode::Workspace => eprintln!(
                        "Eon Desktop exited; Sessions remains active. Run `eon attach {generation}` to reconnect."
                    ),
                    LaunchMode::Terminal => eprintln!(
                        "Eon Desktop exited; Session remains active. Run `eonterm attach {generation}` to reconnect."
                    ),
                }
            }

            if control_listener.accept(|request| {
                dispatch_control_request(request, &mut state, programs, config, runtime, generation)
            })? {
                return Ok(state.initial_status.unwrap_or(0));
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    drop(control_listener);
    if let Some(process) = state.venus.take() {
        close_presentation(process);
    }
    let picker_cleanup = stop_directory_picker(&mut state);
    let _ = fs::remove_dir(runtime);
    match (status, picker_cleanup) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!(
            "{error}; cannot clean up directory picker: {cleanup}"
        )),
    }
}

struct SupervisorState {
    component_generation: String,
    application_id: String,
    workspace: Option<Workspace>,
    sessions: Vec<RunningSession>,
    directory_picker: Option<RunningSession>,
    venus: Option<PresentationProcess>,
    initial_child: Vec<OsString>,
    initial_status: Option<i32>,
}

fn reap_finished_sessions(
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
) -> Result<(), String> {
    let picker_finished = match state.directory_picker.as_mut() {
        Some(picker) => session_finished(picker)?,
        None => None,
    };
    if picker_finished.is_some() {
        let picker = state
            .directory_picker
            .take()
            .ok_or("directory picker disappeared while it was reaped")?;
        let SupervisorState {
            component_generation,
            workspace,
            sessions,
            directory_picker,
            initial_child,
            ..
        } = state;
        workspace
            .as_mut()
            .ok_or("directory picker has no workspace owner")?
            .session_exited(&picker.id, |session, directory, picker_tab| {
                start_workspace_session(
                    programs,
                    config,
                    component_generation,
                    sessions,
                    directory_picker,
                    initial_child,
                    session,
                    directory,
                    picker_tab,
                )
            })
            .map_err(|error| error.detail)?;
    }

    let mut index = 0;
    while index < state.sessions.len() {
        if let Some(code) = session_finished(&mut state.sessions[index])? {
            let session = state.sessions.remove(index);
            if let Some(workspace) = &mut state.workspace {
                let stop_picker = workspace
                    .session_exited(&session.id, |_, _, _| {
                        unreachable!("durable Session exit cannot start a Session")
                    })
                    .map_err(|error| error.detail)?;
                if stop_picker {
                    stop_directory_picker_session(&mut state.directory_picker)?;
                }
            }
            if session.id == "session-1" {
                state.initial_status = Some(code);
            }
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn take_directory_picker(state: &mut SupervisorState) -> Option<RunningSession> {
    if let Some(workspace) = state.workspace.as_mut() {
        workspace.clear_directory_picker();
    }
    state.directory_picker.take()
}

fn stop_directory_picker(state: &mut SupervisorState) -> Result<(), String> {
    if let Some(workspace) = state.workspace.as_mut() {
        workspace.clear_directory_picker();
    }
    stop_directory_picker_session(&mut state.directory_picker)
}

fn stop_directory_picker_session(running: &mut Option<RunningSession>) -> Result<(), String> {
    let Some(session) = running.as_mut() else {
        return Ok(());
    };
    stop_workspace_session(session)?;
    running.take();
    Ok(())
}

fn stop_workspace_session(session: &mut RunningSession) -> Result<Option<i32>, String> {
    if let Some(status) = session_finished(session)? {
        return Ok(Some(status));
    }
    match stop_managed_sessions(std::slice::from_mut(session), SESSION_START_TIMEOUT) {
        Ok(_) => Ok(None),
        Err(stop_error) => match session_finished(session) {
            Ok(Some(status)) => Ok(Some(status)),
            Ok(None) => Err(stop_error),
            Err(reconcile_error) => Err(format!(
                "{stop_error}; cannot reconcile Session after failed stop: {reconcile_error}"
            )),
        },
    }
}

fn close_workspace_tab(
    request_id: &str,
    tab: &str,
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
) -> Result<Snapshot, Failure> {
    reap_finished_sessions(state, programs, config)
        .map_err(|detail| failure("tab-close-failed", detail))?;
    let sessions = state
        .workspace
        .as_mut()
        .ok_or_else(|| failure("workspace-unavailable", "EonTerm has no Eon workspace"))?
        .prepare_close_tab(request_id, tab)?;

    if let Some(sessions) = sessions {
        for id in sessions {
            let index = state
                .sessions
                .iter()
                .position(|session| session.id == id)
                .ok_or_else(|| {
                    failure(
                        "tab-close-failed",
                        format!("Session {id} is missing from the supervisor"),
                    )
                })?;
            let natural_status = stop_workspace_session(&mut state.sessions[index])
                .map_err(|detail| failure("tab-close-failed", format!("{id}: {detail}")))?;
            let session = state.sessions.remove(index);
            state
                .workspace
                .as_mut()
                .expect("workspace close retains its owner")
                .session_exited(&session.id, |_, _, _| {
                    unreachable!("closing a durable tab cannot start a Session")
                })?;
            if session.id == "session-1" && natural_status.is_some() {
                state.initial_status = natural_status;
            }
        }
    } else {
        stop_directory_picker_session(&mut state.directory_picker)
            .map_err(|detail| failure("tab-close-failed", detail))?;
        state
            .workspace
            .as_mut()
            .expect("pending-tab close retains its workspace")
            .session_exited(workspace::DIRECTORY_PICKER_SESSION, |_, _, _| {
                unreachable!("closing a non-final pending tab cannot start a Session")
            })?;
    }

    Ok(state
        .workspace
        .as_ref()
        .expect("tab close retains a non-final workspace")
        .snapshot())
}

fn directory_picker_command(
    programs: &Programs,
    runtime: &Path,
    tab: &str,
) -> Result<Vec<OsString>, String> {
    let session_bin = programs
        .session_bin
        .as_ref()
        .ok_or("the installed Eon package has no directory picker")?;
    Ok(vec![
        session_bin.join("eon-directory-picker").into_os_string(),
        "__directory-picker".into(),
        runtime.join("eon.sock").into_os_string(),
        tab.into(),
    ])
}

#[allow(clippy::too_many_arguments)]
fn start_workspace_session(
    programs: &Programs,
    config: &Path,
    component_generation: &str,
    sessions: &mut Vec<RunningSession>,
    directory_picker: &mut Option<RunningSession>,
    initial_child: &mut Vec<OsString>,
    session: &workspace::Session,
    directory: &Path,
    picker_tab: Option<&str>,
) -> Result<(), String> {
    if picker_tab.is_some() && directory_picker.is_some() {
        return Err("a directory picker is already running".into());
    }
    let picker_child = picker_tab
        .map(|tab| {
            directory_picker_command(
                programs,
                session
                    .endpoint
                    .parent()
                    .ok_or("directory picker endpoint has no runtime directory")?,
                tab,
            )
        })
        .transpose()?;
    let child = if let Some(picker_child) = picker_child.as_deref() {
        picker_child
    } else if session.id == "session-1" {
        initial_child.as_slice()
    } else {
        &[]
    };
    let running = start_orbit(
        programs,
        config,
        &session.endpoint,
        &session.id,
        component_generation,
        directory,
        child,
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    if picker_tab.is_some() {
        *directory_picker = Some(running);
    } else {
        sessions.push(running);
        if session.id == "session-1" {
            initial_child.clear();
        }
    }
    Ok(())
}

fn cancel_directory_picker(
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
) -> Result<(), String> {
    if state.directory_picker.is_none() {
        return Ok(());
    }
    stop_directory_picker_session(&mut state.directory_picker)?;
    let SupervisorState {
        component_generation,
        workspace,
        sessions,
        directory_picker,
        initial_child,
        ..
    } = state;
    workspace
        .as_mut()
        .ok_or("directory picker has no workspace owner")?
        .session_exited(
            workspace::DIRECTORY_PICKER_SESSION,
            |session, directory, picker_tab| {
                start_workspace_session(
                    programs,
                    config,
                    component_generation,
                    sessions,
                    directory_picker,
                    initial_child,
                    session,
                    directory,
                    picker_tab,
                )
            },
        )
        .map(|_| ())
        .map_err(|error| error.detail)
}

fn reap_presentation(
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
) -> Result<bool, String> {
    let exited = match state.venus.as_mut() {
        Some(process) => process
            .child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
            .is_some(),
        None => false,
    };
    if exited {
        state.venus = None;
        cancel_directory_picker(state, programs, config)?;
    }
    Ok(exited)
}

pub(super) fn supervisor_lock_path(root: &Path, generation: &str) -> PathBuf {
    root.join(format!("supervisor-{generation}.lock"))
}

fn open_supervisor_lifecycle_lock(path: &Path) -> Result<fs::File, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("cannot open Eon lifecycle lock {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "cannot inspect Eon lifecycle lock {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != effective_uid() {
        return Err(format!(
            "Eon lifecycle lock {} must be an owned regular file",
            path.display()
        ));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "cannot protect Eon lifecycle lock {}: {error}",
                path.display()
            )
        })?;
    Ok(file)
}

pub(super) fn lock_supervisor_lifecycle(path: &Path) -> Result<fs::File, String> {
    let file = open_supervisor_lifecycle_lock(path)?;
    let deadline = Instant::now() + SESSION_START_TIMEOUT.saturating_add(Duration::from_secs(1));
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "timed out waiting for Eon supervisor lifecycle {}",
                    path.display()
                ));
            }
            Err(fs::TryLockError::Error(error))
                if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "cannot lock Eon supervisor lifecycle {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

pub(super) fn try_lock_supervisor_lifecycle(path: &Path) -> Result<Option<fs::File>, String> {
    let file = open_supervisor_lifecycle_lock(path)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) => return Ok(None),
            Err(fs::TryLockError::Error(error))
                if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "cannot lock Eon supervisor lifecycle {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

fn dispatch_control_request(
    request: Request,
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> (ControlResponse, bool) {
    let mode = if state.workspace.is_some() {
        LaunchMode::Workspace
    } else {
        LaunchMode::Terminal
    };
    match request {
        Request {
            action: Action::InspectRuntime | Action::InspectPresentation,
            ..
        } => match runtime_status(generation, &state.sessions, mode) {
            Ok(runtime) => (
                ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)),
                false,
            ),
            Err(detail) => (
                ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                    "unrepresentable-state",
                    detail,
                ))),
                false,
            ),
        },
        Request {
            action: Action::Present {
                workspace: expected,
            },
            ..
        } => {
            if expected != (mode == LaunchMode::Workspace) {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "launch-mode-mismatch",
                        format!("supervisor owns {} mode", mode.name()),
                    ))),
                    false,
                );
            }
            if let Err(detail) = reap_finished_sessions(state, programs, config) {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "presentation-unavailable",
                        detail,
                    ))),
                    false,
                );
            }
            if state.sessions.is_empty() && state.directory_picker.is_none() {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "generation-ending",
                        "the last Session exited while Eon Desktop was reopening",
                    ))),
                    true,
                );
            }
            let result = reap_presentation(state, programs, config).and_then(|_| {
                if let Some(venus) = &state.venus {
                    return venus.present();
                }
                let socket = if mode == LaunchMode::Workspace {
                    runtime.join("eon.sock")
                } else {
                    state
                        .sessions
                        .first()
                        .ok_or("EonTerm has no live Session")?
                        .endpoint
                        .clone()
                };
                let terminal = managed_environment::terminal_presentation(config)?;
                let command = venus_command(
                    programs,
                    config,
                    &socket,
                    mode,
                    terminal,
                    &state.application_id,
                );
                state.venus = Some(PresentationProcess::start(command)?);
                Ok(())
            });
            match result.and_then(|()| runtime_status(generation, &state.sessions, mode)) {
                Ok(runtime) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)),
                    false,
                ),
                Err(detail) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "presentation-unavailable",
                        detail,
                    ))),
                    false,
                ),
            }
        }
        Request {
            action: Action::Stop { generation: target },
            ..
        } => {
            if target != generation {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "generation-mismatch",
                        format!("supervisor owns generation {generation}, not {target}"),
                    ))),
                    false,
                );
            }
            if let Some(picker) = take_directory_picker(state) {
                state.sessions.push(picker);
            }
            let response = match stop_managed_sessions(&mut state.sessions, SESSION_START_TIMEOUT) {
                Ok(mut sessions) => {
                    sessions.retain(|session| session != workspace::DIRECTORY_PICKER_SESSION);
                    LifecycleResponse::Stopped(Stopped {
                        generation: generation.into(),
                        sessions,
                    })
                }
                Err(detail) => LifecycleResponse::Failure(failure("stop-failed", detail)),
            };
            (ControlResponse::Lifecycle(response), true)
        }
        Request {
            id,
            action: Action::CloseTab { tab },
        } => match close_workspace_tab(&id, &tab, state, programs, config) {
            Ok(snapshot) => (
                ControlResponse::Workspace(Response::Snapshot(snapshot)),
                false,
            ),
            Err(error) => (ControlResponse::Workspace(Response::Failure(error)), false),
        },
        request => match &mut state.workspace {
            Some(workspace) => {
                let component_generation = &state.component_generation;
                let sessions = &mut state.sessions;
                let directory_picker = &mut state.directory_picker;
                let initial_child = &mut state.initial_child;
                match workspace.dispatch(
                    &request.id,
                    request.action,
                    |session, directory, picker_tab| {
                        start_workspace_session(
                            programs,
                            config,
                            component_generation,
                            sessions,
                            directory_picker,
                            initial_child,
                            session,
                            directory,
                            picker_tab,
                        )
                    },
                ) {
                    Ok(()) => (
                        ControlResponse::Workspace(Response::Snapshot(workspace.snapshot())),
                        false,
                    ),
                    Err(error) => (
                        ControlResponse::Workspace(Response::Failure(failure(
                            error.code,
                            error.detail,
                        ))),
                        false,
                    ),
                }
            }
            None => (
                ControlResponse::Workspace(Response::Failure(failure(
                    "workspace-unavailable",
                    "EonTerm has no Eon workspace",
                ))),
                false,
            ),
        },
    }
}

fn runtime_status(
    generation: &str,
    sessions: &[RunningSession],
    mode: LaunchMode,
) -> Result<Runtime, String> {
    if mode == LaunchMode::Terminal && sessions.is_empty() {
        return Err("supervisor has no live Sessions".into());
    }
    Ok(Runtime {
        generation: generation.into(),
        eon_version: env!("CARGO_PKG_VERSION").into(),
        workspace_protocol: VERSION,
        component_report: eon_manifest::version_report(MANIFEST)
            .map_err(|error| error.to_string())?,
        sessions: sessions.iter().map(|session| session.id.clone()).collect(),
        attach: Availability {
            available: true,
            reason: match mode {
                LaunchMode::Workspace => "supervisor accepts EONW v4 presentation requests",
                LaunchMode::Terminal => "supervisor owns one EonTerm Session",
            }
            .into(),
        },
        stop: Availability {
            available: true,
            reason: "generation-aware supervisor owns these Sessions".into(),
        },
    })
}

pub(super) fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(super) fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{
        LaunchMode, Programs, effective_uid, prepare_configuration, prepare_runtime,
        temporary_directory, venus_command, xdg_path,
    };
    use std::{
        ffi::OsString,
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    fn terminal_presentation(
        background_opacity: f32,
        background_blur: bool,
    ) -> crate::managed_environment::TerminalConfig {
        crate::managed_environment::TerminalConfig {
            background_opacity,
            background_blur,
        }
    }

    #[test]
    fn venus_receives_workspace_endpoint_only_in_workspace_mode() {
        let mut programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: None,
            venus_decorations: true,
        };
        let config = Path::new("/config/eon");
        let socket = Path::new("/runtime/eon.sock");

        let workspace = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Workspace,
            terminal_presentation(0.88, true),
            "eon",
        );
        assert_eq!(
            workspace.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--application-id",
                "eon",
                "--background-opacity",
                "0.88",
                "--background-blur",
                "--workspace",
                "/runtime/eon.sock",
            ]
            .map(OsString::from)
        );

        let terminal = venus_command(
            &programs,
            config,
            Path::new("/runtime/orbit.sock"),
            LaunchMode::Terminal,
            terminal_presentation(1.0, false),
            "eonterm",
        );
        assert_eq!(
            terminal.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--application-id",
                "eonterm",
                "--background-opacity",
                "1",
                "/runtime/orbit.sock",
            ]
            .map(OsString::from)
        );

        programs.venus_decorations = false;
        let undecorated = venus_command(
            &programs,
            config,
            Path::new("/runtime/orbit.sock"),
            LaunchMode::Terminal,
            terminal_presentation(0.0, true),
            "eonova",
        );
        assert_eq!(
            undecorated
                .get_args()
                .map(OsString::from)
                .collect::<Vec<_>>(),
            [
                "--no-decorations",
                "--application-id",
                "eonova",
                "--background-opacity",
                "0",
                "--background-blur",
                "/runtime/orbit.sock",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn runtime_directory_is_private() {
        let root = temporary_directory();
        let runtime = root.join("runtime");

        prepare_runtime(&runtime).unwrap();

        let metadata = fs::metadata(&runtime).unwrap();
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(metadata.mode() & 0o777, 0o700);

        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            prepare_runtime(&runtime).unwrap_err(),
            format!(
                "runtime directory {} must be an owned private directory",
                runtime.display()
            )
        );
        assert_eq!(fs::metadata(&runtime).unwrap().mode() & 0o777, 0o777);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_directory_is_private_only_when_created() {
        let root = temporary_directory();
        let config = root.join("config");

        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o700);

        fs::set_permissions(&config, fs::Permissions::from_mode(0o755)).unwrap();
        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o755);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xdg_paths_must_be_absolute() {
        assert_eq!(xdg_path(Some("relative".into())), None);

        let absolute = PathBuf::from("/absolute");
        assert_eq!(xdg_path(Some(absolute.clone())), Some(absolute));
    }
}
