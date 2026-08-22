mod control;
mod generation;
mod managed_environment;
mod sessions;
mod workspace;

use control::{
    ControlListener, ControlResponse, EndpointFailure, EndpointFailureKind, SocketIdentity,
    connect_control, failure, probe_launch_mode, probe_presentable_runtime, report_failure,
    send_action, send_action_on, socket_identity,
};
use eon_workspace_protocol::{
    Action, Availability, Direction, LifecycleResponse, Request, Response, Runtime, Stopped,
    VERSION,
};
use generation::{
    attach_generation, current_generation, generation_directory, generations_command,
    stop_generation, valid_generation,
};
use sessions::{
    RunningSession, recover_sessions, session_finished, start_orbit, stop_managed_sessions,
};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    os::fd::OwnedFd,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workspace::{Workspace, human as human_output, json as json_output};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v3.json");
const EON_ANSI_PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";
const EON_USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach [GENERATION] | generations [--json] | stop GENERATION [--json] | workspace [--json] | tab create [--json] | pane create [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
const EONTERM_USAGE: &str = "usage: eonterm [--no-decorations] -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json]";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

const SESSION_START_TIMEOUT: Duration = Duration::from_secs(5);

fn probe_supervisor(
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
    let info = probe_presentable_runtime(socket)?;
    if !info.attach.available {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Incompatible,
            info.attach.reason,
        ));
    }
    validate_runtime(&info, generation)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?;
    let mode = probe_launch_mode(socket)?;
    if socket_identity(socket)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
        != Some(identity)
    {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Dead,
            "supervisor endpoint changed while it was being validated",
        ));
    }
    Ok((mode, identity))
}

fn validate_private_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "directory {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn write_stdout(output: impl AsRef<[u8]>) -> Result<(), String> {
    match std::io::stdout().lock().write_all(output.as_ref()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("cannot write stdout: {error}")),
    }
}

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
    session_bin: Option<PathBuf>,
    venus_decorations: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchMode {
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

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments = arguments.collect();
    let eonterm = Path::new(&invocation).file_name() == Some(OsStr::new("eonterm"));
    let result = if eonterm {
        execute_eonterm(arguments)
    } else {
        match managed_environment::tool(&invocation) {
            Some(tool) => launch_managed(tool, arguments),
            None => execute(arguments),
        }
    };
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{}: {error}", if eonterm { "eonterm" } else { "eon" });
            ExitCode::FAILURE
        }
    }
}

fn launch_managed(
    tool: managed_environment::Tool,
    arguments: Vec<OsString>,
) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut command = managed_environment::command(tool, &config, &arguments)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eon")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [] => launch_current(LaunchMode::Workspace, &[], true, true),
        [command] if command == "run" => launch_current(LaunchMode::Workspace, &[], false, true),
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Workspace, child, false, true)
        }
        [command, ..]
            if command == "workspace"
                || command == "tab"
                || command == "pane"
                || command == "focus" =>
        {
            control(&arguments)
        }
        [command] if command == "versions" => {
            write_stdout(format!(
                "eon {} {}\neonw {}\n{}\n",
                env!("CARGO_PKG_VERSION"),
                current_generation()?,
                VERSION,
                eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?
            ))?;
            Ok(0)
        }
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            write_stdout(format!("{}\n", path.display()))?;
            Ok(0)
        }
        _ => Err(EON_USAGE.into()),
    }
}

fn execute_eonterm(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eonterm")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [separator, child @ ..] if separator == "--" && !child.is_empty() => {
            launch_current(LaunchMode::Terminal, child, true, true)
        }
        [flag, separator, child @ ..]
            if flag == "--no-decorations" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Terminal, child, true, false)
        }
        _ => Err(EONTERM_USAGE.into()),
    }
}

fn lifecycle_command(arguments: &[OsString], product: &str) -> Result<Option<i32>, String> {
    match arguments {
        [command] if command == "attach" => attach_generation(None, product),
        [command, generation] if command == "attach" => {
            attach_generation(Some(generation_argument(generation)?), product)
        }
        [command] if command == "generations" => generations_command(false, product),
        [command, flag] if command == "generations" && flag == "--json" => {
            generations_command(true, product)
        }
        [command, generation] if command == "stop" => {
            stop_generation(generation_argument(generation)?, false, product)
        }
        [command, generation, flag] | [command, flag, generation]
            if command == "stop" && flag == "--json" =>
        {
            stop_generation(generation_argument(generation)?, true, product)
        }
        _ => return Ok(None),
    }
    .map(Some)
}

fn generation_argument(argument: &OsStr) -> Result<&str, String> {
    let generation = argument
        .to_str()
        .ok_or_else(|| "Eon generation identities must be UTF-8".to_string())?;
    if generation != "legacy" && !valid_generation(generation) {
        return Err(format!("invalid Eon generation identity {generation:?}"));
    }
    Ok(generation)
}

fn launch_current(
    mode: LaunchMode,
    child: &[OsString],
    attach_existing: bool,
    decorations: bool,
) -> Result<i32, String> {
    let generation = current_generation()?;
    let root = runtime_directory(if mode == LaunchMode::Terminal {
        "eonterm"
    } else {
        "eon"
    });
    prepare_runtime(&root)?;
    // ponytail: one root-wide startup lock; partition if cross-generation starts contend.
    let startup_lock = lock_supervisor_startup(&root.join("startup.lock"))?;
    let runtime = generation_directory(&root, &generation);
    let socket = runtime.join("eon.sock");
    match probe_supervisor(&socket, &generation) {
        Ok((active_mode, supervisor)) => {
            if active_mode != mode {
                return Err(format!(
                    "generation {generation} already has a live {} mode; requested {} mode",
                    active_mode.name(),
                    mode.name()
                ));
            }
            drop(startup_lock);
            return if attach_existing {
                present_at(&runtime, &generation, mode, supervisor)
            } else {
                Err(format!(
                    "generation {generation} already has a live Eon supervisor"
                ))
            };
        }
        Err(error) if error.kind == EndpointFailureKind::Dead => {}
        Err(error) => return Err(error.detail),
    }
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_generation_runtime(&root, &generation)?;
    prepare_configuration(&config)?;
    let programs = programs(decorations);
    match supervise(
        &programs,
        &config,
        terminal,
        startup_lock,
        &runtime.join("orbit.sock"),
        child,
        &generation,
        mode,
    ) {
        Ok(code) => Ok(code),
        Err(error) => {
            if attach_existing && path_exists(&socket) {
                attach_competing_supervisor(&socket, &runtime, &generation, mode, error)
            } else {
                Err(error)
            }
        }
    }
}

fn attach_competing_supervisor(
    socket: &Path,
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    launch_error: String,
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

fn control(arguments: &[OsString]) -> Result<i32, String> {
    let (action, json) = parse_control_arguments(arguments)?;
    let generation = current_generation()?;
    let root = runtime_directory("eon");
    let runtime = prepare_generation_runtime(&root, &generation)?;
    let socket = runtime.join("eon.sock");
    let response = match send_action(&socket, action) {
        Ok(response) => response,
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            return report_failure(&failure("malformed-action", error.detail), json);
        }
        Err(error) => {
            let code = if error.kind == EndpointFailureKind::Dead {
                "missing-supervisor"
            } else {
                "supervisor-unavailable"
            };
            return report_failure(
                &failure(code, format!("{}; run `eon` first", error.detail)),
                json,
            );
        }
    };
    match response {
        ControlResponse::Workspace(Response::Snapshot(snapshot)) => {
            write_stdout(if json {
                json_output(&snapshot)
            } else {
                human_output(&snapshot)
            })?;
            Ok(0)
        }
        ControlResponse::Workspace(Response::Failure(failure)) => report_failure(&failure, json),
        ControlResponse::Lifecycle(_) => {
            Err("Eon supervisor returned a lifecycle result for a workspace action".into())
        }
    }
}

fn parse_control_arguments(arguments: &[OsString]) -> Result<(Action, bool), String> {
    let mut json = false;
    let mut values = Vec::new();
    for argument in arguments {
        let argument = argument
            .to_str()
            .ok_or_else(|| "Eon workspace actions require UTF-8 arguments".to_string())?;
        if argument == "--json" {
            if json {
                return Err(EON_USAGE.into());
            }
            json = true;
        } else {
            values.push(argument);
        }
    }
    let action = match values.as_slice() {
        ["workspace"] => Action::Inspect,
        ["tab", "create"] => Action::CreateTab,
        ["pane", "create"] => Action::CreatePane,
        ["focus", "left"] => Action::Focus(Direction::Left),
        ["focus", "right"] => Action::Focus(Direction::Right),
        ["focus", "up"] => Action::Focus(Direction::Up),
        ["focus", "down"] => Action::Focus(Direction::Down),
        ["focus", id] => Action::FocusId((*id).into()),
        _ => return Err(EON_USAGE.into()),
    };
    Ok((action, json))
}

fn request_id() -> String {
    let sequence = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}-{sequence}", std::process::id())
}

fn attach_legacy(runtime: &Path) -> Result<i32, String> {
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_configuration(&config)?;
    let programs = programs(true);
    venus_command(
        &programs,
        &config,
        &runtime.join("orbit.sock"),
        LaunchMode::Workspace,
        terminal,
    )
    .spawn()
    .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?
    .wait()
    .map(status_code)
    .map_err(|error| format!("cannot observe Eon Desktop: {error}"))
}

fn present_at(
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
) -> Command {
    let mut command = Command::new(&programs.venus);
    if !programs.venus_decorations {
        command.arg("--no-decorations");
    }
    command
        .arg("--background-opacity")
        .arg(terminal.background_opacity.to_string());
    if terminal.background_blur {
        command.arg("--background-blur");
    }
    command.arg(socket);
    if mode == LaunchMode::Workspace {
        command.arg(socket.with_file_name("eon.sock"));
    }
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
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                stop(&mut child);
                return;
            }
        }
    }
}

fn start_venus(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: managed_environment::TerminalConfig,
) -> Result<PresentationProcess, String> {
    PresentationProcess::start(venus_command(programs, config, socket, mode, terminal))
}

fn programs(venus_decorations: bool) -> Programs {
    Programs {
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        session_bin: nonempty_environment_path("EON_SESSION_BIN"),
        venus_decorations,
    }
}

fn configured_program(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.into())
}

fn configuration_directory() -> Result<PathBuf, String> {
    let path = nonempty_environment_path("EON_CONFIG_HOME")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_CONFIG_HOME")).map(|path| path.join("eon"))
        })
        .or_else(|| nonempty_environment_path("HOME").map(|path| path.join(".config/eon")))
        .ok_or_else(|| "HOME, XDG_CONFIG_HOME, and EON_CONFIG_HOME are unset".to_string())?;
    std::path::absolute(path)
        .map_err(|error| format!("cannot resolve Eon configuration root: {error}"))
}

fn prepare_configuration(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

fn runtime_directory(product: &str) -> PathBuf {
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

fn prepare_generation_runtime(root: &Path, generation: &str) -> Result<PathBuf, String> {
    prepare_runtime(root)?;
    prepare_runtime(&root.join("generations"))?;
    let runtime = generation_directory(root, generation);
    prepare_runtime(&runtime)?;
    Ok(runtime)
}

fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect runtime directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "runtime directory {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and no failure state.
    unsafe { libc::geteuid() }
}

#[allow(clippy::too_many_arguments)]
fn supervise(
    programs: &Programs,
    config: &Path,
    terminal: managed_environment::TerminalConfig,
    startup_lock: fs::File,
    socket: &Path,
    child: &[OsString],
    generation: &str,
    mode: LaunchMode,
) -> Result<i32, String> {
    let runtime = socket
        .parent()
        .ok_or_else(|| format!("Sessions socket {} has no parent", socket.display()))?;
    let component_generation =
        eon_manifest::component_revision(MANIFEST, "orbit").map_err(|error| error.to_string())?;
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    let mut sessions = recover_sessions(runtime, mode, &component_generation, deadline)?;
    if sessions.is_empty() {
        sessions.push(start_orbit(
            programs,
            config,
            socket,
            "session-1",
            &component_generation,
            child,
            deadline,
        )?);
    }
    let workspace = (mode == LaunchMode::Workspace)
        .then(|| {
            Workspace::with_recovered_sessions(
                runtime.to_path_buf(),
                sessions
                    .iter()
                    .map(|session| {
                        (
                            session.number,
                            workspace::Session {
                                id: session.id.clone(),
                                endpoint: session.endpoint.clone(),
                            },
                        )
                    })
                    .collect(),
            )
        })
        .transpose()?;
    let control_listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    let venus = match start_venus(programs, config, &sessions[0].endpoint, mode, terminal) {
        Ok(venus) => venus,
        Err(error) => {
            let stopped = stop_managed_sessions(&mut sessions, SESSION_START_TIMEOUT)
                .map_err(|stop_error| format!("{error}; cannot roll back Sessions: {stop_error}"));
            drop(control_listener);
            if stopped.is_ok() {
                let _ = fs::remove_dir(runtime);
            }
            return stopped.and(Err(error));
        }
    };
    let mut state = SupervisorState {
        component_generation,
        workspace,
        sessions,
        venus: Some(venus),
    };
    drop(startup_lock);
    let mut initial_status = None;

    loop {
        let mut index = 0;
        while index < state.sessions.len() {
            if let Some(code) = session_finished(&mut state.sessions[index])? {
                let session = state.sessions.remove(index);
                if let Some(workspace) = &mut state.workspace {
                    workspace
                        .session_exited(&session.id)
                        .map_err(|error| error.detail)?;
                }
                if session.id == "session-1" {
                    initial_status = Some(code);
                }
            } else {
                index += 1;
            }
        }

        if state.sessions.is_empty() {
            if let Some(process) = state.venus.take() {
                close_presentation(process);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Ok(initial_status.unwrap_or(0));
        }

        if reap_desktop(&mut state.venus)? {
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
            dispatch_control_request(
                request,
                &mut state,
                programs,
                config,
                SESSION_START_TIMEOUT,
                generation,
            )
        })? {
            if let Some(process) = state.venus.take() {
                close_presentation(process);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Ok(0);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

struct SupervisorState {
    component_generation: String,
    workspace: Option<Workspace>,
    sessions: Vec<RunningSession>,
    venus: Option<PresentationProcess>,
}

fn reap_desktop(desktop: &mut Option<PresentationProcess>) -> Result<bool, String> {
    let exited = match desktop.as_mut() {
        Some(process) => process
            .child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
            .is_some(),
        None => false,
    };
    if exited {
        *desktop = None;
    }
    Ok(exited)
}

fn open_supervisor_startup_lock(path: &Path) -> Result<fs::File, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("cannot open Eon startup lock {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "cannot inspect Eon startup lock {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != effective_uid() {
        return Err(format!(
            "Eon startup lock {} must be an owned regular file",
            path.display()
        ));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "cannot protect Eon startup lock {}: {error}",
                path.display()
            )
        })?;
    Ok(file)
}

fn lock_supervisor_startup(path: &Path) -> Result<fs::File, String> {
    let file = open_supervisor_startup_lock(path)?;
    loop {
        match file.lock() {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(format!(
                    "cannot lock Eon supervisor startup {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(file)
}

fn try_lock_supervisor_startup(path: &Path) -> Option<fs::File> {
    let file = open_supervisor_startup_lock(path).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn dispatch_control_request(
    request: Request,
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
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
        } if expected != (mode == LaunchMode::Workspace) => (
            ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                "launch-mode-mismatch",
                format!("supervisor owns {} mode", mode.name()),
            ))),
            false,
        ),
        Request {
            action: Action::Present { .. },
            ..
        } => {
            let result = reap_desktop(&mut state.venus).and_then(|_| {
                if let Some(venus) = &state.venus {
                    return venus.present();
                }
                let session = state
                    .sessions
                    .first()
                    .ok_or_else(|| "supervisor has no live Session to present".to_string())?;
                let terminal = managed_environment::terminal_presentation(config)?;
                state.venus = Some(start_venus(
                    programs,
                    config,
                    &session.endpoint,
                    mode,
                    terminal,
                )?);
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
        } if target == generation => {
            match stop_managed_sessions(&mut state.sessions, socket_timeout) {
                Ok(sessions) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Stopped(Stopped {
                        generation: generation.into(),
                        sessions,
                    })),
                    true,
                ),
                Err(detail) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "stop-failed",
                        detail,
                    ))),
                    true,
                ),
            }
        }
        Request {
            action: Action::Stop { generation: target },
            ..
        } => (
            ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                "generation-mismatch",
                format!("supervisor owns generation {generation}, not {target}"),
            ))),
            false,
        ),
        request => match state.workspace.as_mut() {
            Some(workspace) => match workspace.dispatch(&request.id, request.action, |session| {
                let running = start_orbit(
                    programs,
                    config,
                    &session.endpoint,
                    &session.id,
                    &state.component_generation,
                    &[],
                    Instant::now() + socket_timeout,
                )?;
                state.sessions.push(running);
                Ok(())
            }) {
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
            },
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
                LaunchMode::Workspace => "supervisor accepts EONW v1 presentation requests",
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

fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{
        ControlListener, LaunchMode, Programs, effective_uid, lock_supervisor_startup,
        prepare_configuration, prepare_runtime, venus_command, xdg_path,
    };
    use std::{
        ffi::OsString,
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    pub(super) fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eon-test-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn terminal_presentation(
        background_opacity: f32,
        background_blur: bool,
    ) -> super::managed_environment::TerminalConfig {
        super::managed_environment::TerminalConfig {
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
        let socket = Path::new("/runtime/orbit.sock");

        let workspace = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Workspace,
            terminal_presentation(0.88, true),
        );
        assert_eq!(
            workspace.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--background-opacity",
                "0.88",
                "--background-blur",
                "/runtime/orbit.sock",
                "/runtime/eon.sock",
            ]
            .map(OsString::from)
        );

        let terminal = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Terminal,
            terminal_presentation(1.0, false),
        );
        assert_eq!(
            terminal.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--background-opacity", "1", "/runtime/orbit.sock"].map(OsString::from)
        );

        programs.venus_decorations = false;
        let undecorated = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Terminal,
            terminal_presentation(0.0, true),
        );
        assert_eq!(
            undecorated
                .get_args()
                .map(OsString::from)
                .collect::<Vec<_>>(),
            [
                "--no-decorations",
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

    #[test]
    fn startup_lock_serializes_stale_cleanup_and_bind() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let startup_lock = root.join("startup.lock");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        let held = lock_supervisor_startup(&startup_lock).unwrap();
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&startup_lock)
            .unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));

        fs::remove_file(&socket).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        drop(held);
        contender.lock().unwrap();

        let error = ControlListener::bind(&socket).err().unwrap();
        assert!(error.contains("already active"));
        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        drop(replacement);
        fs::remove_dir_all(root).unwrap();
    }
}
