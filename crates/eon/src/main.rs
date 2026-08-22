mod control;
mod generation;
mod managed_environment;
mod workspace;

use control::{
    ControlListener, ControlResponse, EndpointFailure, EndpointFailureKind, SocketIdentity,
    connect_control, failure, probe_launch_mode, probe_presentable_runtime, report_failure,
    send_action, send_action_on, socket_identity,
};
use eon_workspace_protocol::{
    Action, Availability, Direction, LifecycleResponse, MAX_PANES, Request, Response, Runtime,
    Stopped, VERSION,
};
use generation::{
    attach_generation, current_generation, generation_directory, generations_command,
    stop_generation, valid_generation,
};
use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage, EndpointIdentity, LiveIdentity,
    ObjectIdentity, ProcessOutcome, Record as ManagementRecord,
    ServerMessage as ManagementServerMessage, TerminationReason, Tombstone,
};
use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
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

fn session_number(value: &str) -> Option<usize> {
    let number = value.strip_prefix("session-")?;
    if number.is_empty() || number == "0" || number.starts_with('0') {
        return None;
    }
    number
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| number.parse().ok())
        .flatten()
}

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

struct RecordSnapshot {
    object: ObjectIdentity,
    record: ManagementRecord,
}

struct ReadyClaim {
    file: fs::File,
    path: PathBuf,
    object: ObjectIdentity,
}

impl ReadyClaim {
    fn create(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("Sessions record {} has no parent", path.display()))?;
        validate_private_directory(parent)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                format!(
                    "cannot create Sessions Ready claim {}: {error}",
                    path.display()
                )
            })?;
        let metadata = file.metadata().map_err(|error| {
            format!(
                "cannot inspect Sessions Ready claim {}: {error}",
                path.display()
            )
        })?;
        let claim = Self {
            object: object_identity(&metadata),
            file,
            path: path.to_path_buf(),
        };
        claim
            .file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                format!(
                    "cannot protect Sessions Ready claim {}: {error}",
                    path.display()
                )
            })?;
        if !claim.exact_path_is_empty()? {
            return Err(format!(
                "Sessions Ready claim {} changed while it was created",
                path.display()
            ));
        }
        Ok(claim)
    }

    fn retains_path(&self) -> Result<bool, String> {
        let current = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "cannot revalidate Sessions Ready claim {}: {error}",
                    self.path.display()
                ));
            }
        };
        Ok(current.file_type().is_file()
            && current.uid() == effective_uid()
            && current.mode() & 0o7777 == 0o600
            && object_identity(&current) == self.object)
    }

    fn exact_path_is_empty(&self) -> Result<bool, String> {
        let metadata = self.file.metadata().map_err(|error| {
            format!(
                "cannot inspect Sessions Ready claim {}: {error}",
                self.path.display()
            )
        })?;
        if !metadata.file_type().is_file()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o7777 != 0o600
            || object_identity(&metadata) != self.object
        {
            return Err(format!(
                "Sessions Ready claim {} changed while retained",
                self.path.display()
            ));
        }
        Ok(metadata.len() == 0 && self.retains_path()?)
    }

    fn lock_for_rollback(&self, deadline: Instant) -> Result<bool, String> {
        loop {
            match self.file.try_lock() {
                Ok(()) => return self.exact_path_is_empty(),
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(format!(
                        "Sessions Ready claim {} remained busy",
                        self.path.display()
                    ));
                }
                Err(fs::TryLockError::Error(error)) => {
                    return Err(format!(
                        "cannot lock Sessions Ready claim {}: {error}",
                        self.path.display()
                    ));
                }
            }
        }
    }

    fn remove_empty(&self) -> Result<(), String> {
        if !self.exact_path_is_empty()? {
            return Err(format!(
                "Sessions Ready claim {} changed before cleanup",
                self.path.display()
            ));
        }
        fs::remove_file(&self.path).map_err(|error| {
            format!(
                "cannot remove Sessions Ready claim {}: {error}",
                self.path.display()
            )
        })
    }
}

impl Drop for ReadyClaim {
    fn drop(&mut self) {
        if self
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() == 0)
            && fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| object_identity(&metadata) == self.object)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct ManagedCandidate {
    number: usize,
    endpoint: PathBuf,
    record_path: PathBuf,
    record: ObjectIdentity,
    identity: LiveIdentity,
}

fn artifact_path(endpoint: &Path, suffix: &str) -> PathBuf {
    let mut path = endpoint.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

fn object_identity(metadata: &fs::Metadata) -> ObjectIdentity {
    ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn read_management_record(path: &Path) -> Result<Option<RecordSnapshot>, String> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot open Sessions record {}: {error}",
                path.display()
            ));
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect Sessions record {}: {error}", path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > management::MAX_RECORD_BYTES as u64
    {
        return Err(format!(
            "Sessions record {} must be an owned mode-0600 regular file no larger than {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let object = object_identity(&metadata);
    let mut bytes = Vec::with_capacity(management::MAX_RECORD_BYTES.saturating_add(1));
    Read::by_ref(&mut file)
        .take(management::MAX_RECORD_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read Sessions record {}: {error}", path.display()))?;
    if bytes.len() > management::MAX_RECORD_BYTES {
        return Err(format!(
            "Sessions record {} exceeds {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let current = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot revalidate Sessions record {}: {error}",
            path.display()
        )
    })?;
    if !current.file_type().is_file()
        || current.uid() != effective_uid()
        || current.mode() & 0o7777 != 0o600
        || object_identity(&current) != object
    {
        return Err(format!(
            "Sessions record {} changed while it was being read",
            path.display()
        ));
    }
    let record = management::decode_record(&bytes)
        .map_err(|error| format!("invalid Sessions record {}: {error}", path.display()))?;
    Ok(Some(RecordSnapshot { object, record }))
}

fn endpoint_matches(path: &Path, expected: &EndpointIdentity) -> Result<(), String> {
    if expected.path != path.as_os_str().as_bytes() {
        return Err(format!(
            "Sessions identity names endpoint {}, expected {}",
            PathBuf::from(OsString::from_vec(expected.path.clone())).display(),
            path.display()
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect Sessions endpoint {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected.object
    {
        return Err(format!(
            "Sessions endpoint {} does not match its owned mode-0600 socket identity",
            path.display()
        ));
    }
    Ok(())
}

fn expected_session_endpoint(runtime: &Path, number: usize) -> PathBuf {
    if number == 1 {
        runtime.join("orbit.sock")
    } else {
        runtime.join(format!("session-{number}.sock"))
    }
}

fn validate_management_identity(
    identity: &LiveIdentity,
    component_generation: &str,
    expected_run: Option<&str>,
    require_live_endpoints: bool,
) -> Result<(usize, PathBuf), String> {
    let number = session_number(&identity.session_id)
        .ok_or_else(|| format!("invalid Sessions identity {:?}", identity.session_id))?;
    if expected_run.is_some_and(|expected| identity.run_id != expected) {
        return Err("Sessions record reports a different run identity".into());
    }
    if identity.component_generation != component_generation {
        return Err(format!(
            "Sessions record reports component generation {}, expected {component_generation}",
            identity.component_generation
        ));
    }
    if identity.record_generation != management::RECORD_GENERATION
        || identity.management_generation != management::VERSION
    {
        return Err("Sessions record reports an unsupported management generation".into());
    }
    if identity.uid != effective_uid() {
        return Err(format!(
            "Sessions record reports UID {}, expected {}",
            identity.uid,
            effective_uid()
        ));
    }
    let presentation = PathBuf::from(OsString::from_vec(identity.presentation.path.clone()));
    let runtime = presentation
        .parent()
        .ok_or_else(|| "Sessions presentation endpoint has no parent".to_string())?;
    let expected_presentation = expected_session_endpoint(runtime, number);
    if presentation != expected_presentation {
        return Err(format!(
            "Sessions identity {} uses unexpected presentation endpoint {}",
            identity.session_id,
            presentation.display()
        ));
    }
    let management_path = artifact_path(&presentation, ".management");
    if identity.management.path != management_path.as_os_str().as_bytes() {
        return Err(format!(
            "Sessions identity {} uses an unexpected management endpoint",
            identity.session_id
        ));
    }
    if require_live_endpoints {
        endpoint_matches(&presentation, &identity.presentation)?;
        endpoint_matches(&management_path, &identity.management)?;
    }
    Ok((number, presentation))
}

fn operation_timeout(deadline: Instant, operation: &str) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{operation} exceeded five seconds"))
}

fn unix_connect_with_timeout(path: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
    let address = socket2::SockAddr::unix(path)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.connect_timeout(&address, timeout)?;
    Ok(UnixStream::from(OwnedFd::from(socket)))
}

fn read_management_response(
    stream: &mut UnixStream,
    deadline: Instant,
    operation: &str,
) -> Result<ManagementServerMessage, String> {
    let mut read_exact = |mut bytes: &mut [u8], context: &str| -> Result<(), String> {
        while !bytes.is_empty() {
            let timeout = operation_timeout(deadline, operation)?;
            stream
                .set_read_timeout(Some(timeout))
                .map_err(|error| format!("cannot bound {operation}: {error}"))?;
            match stream.read(bytes) {
                Ok(0) => return Err(format!("{context}: unexpected end of file")),
                Ok(read) => bytes = &mut bytes[read..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("{context}: {error}")),
            }
        }
        Ok(())
    };
    let mut bytes = vec![0; management::HEADER_BYTES];
    read_exact(&mut bytes, "cannot read Sessions management result")?;
    let length = management::server_message_len(&bytes)
        .map_err(|error| format!("invalid Sessions management result: {error}"))?
        .ok_or("incomplete Sessions management header")?;
    bytes.resize(length, 0);
    read_exact(
        &mut bytes[management::HEADER_BYTES..],
        "cannot read complete Sessions management result",
    )?;
    management::decode_server_message(&bytes)
        .map_err(|error| format!("invalid Sessions management result: {error}"))
}

fn validate_management_peer(stream: &UnixStream, identity: &LiveIdentity) -> Result<(), String> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials points to writable ucred storage of the declared length.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    } == -1
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return Err("cannot validate Sessions management peer credentials".into());
    }
    if u32::try_from(credentials.pid).ok() != Some(identity.process_id)
        || credentials.uid != identity.uid
    {
        return Err("Sessions management peer differs from its Ready process identity".into());
    }
    let stat = fs::read_to_string(format!("/proc/{}/stat", identity.process_id))
        .map_err(|error| format!("cannot validate Sessions process identity: {error}"))?;
    let start = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("invalid Sessions process identity")?;
    if start != identity.process_start {
        return Err("Sessions process start differs from its Ready identity".into());
    }
    Ok(())
}

fn acquire_management(
    candidate: ManagedCandidate,
    deadline: Instant,
) -> Result<RunningSession, String> {
    let current = read_management_record(&candidate.record_path)?.ok_or_else(|| {
        format!(
            "Sessions record {} disappeared",
            candidate.record_path.display()
        )
    })?;
    if current.object != candidate.record
        || current.record != ManagementRecord::Live(candidate.identity.clone())
    {
        return Err(format!(
            "Sessions record {} changed before lease acquisition",
            candidate.record_path.display()
        ));
    }
    let management_path = PathBuf::from(OsString::from_vec(
        candidate.identity.management.path.clone(),
    ));
    endpoint_matches(&management_path, &candidate.identity.management)?;
    let timeout = operation_timeout(deadline, "Sessions lease acquisition")?;
    let mut stream = unix_connect_with_timeout(&management_path, timeout).map_err(|error| {
        format!(
            "cannot connect to Sessions management endpoint {}: {error}",
            management_path.display()
        )
    })?;
    validate_management_peer(&stream, &candidate.identity)?;
    let timeout = operation_timeout(deadline, "Sessions lease acquisition")?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("cannot bound Sessions lease acquisition: {error}"))?;
    let request = management::encode_client_message(&ManagementClientMessage::Acquire {
        expected: candidate.identity.clone(),
        record: candidate.record,
    })
    .map_err(|error| format!("cannot encode Sessions lease request: {error}"))?;
    stream
        .write_all(&request)
        .map_err(|error| format!("cannot send Sessions lease request: {error}"))?;
    match read_management_response(&mut stream, deadline, "Sessions lease acquisition")? {
        ManagementServerMessage::Lease(identity) if identity == candidate.identity => {}
        ManagementServerMessage::Busy => {
            return Err(format!(
                "Sessions run {} is already managed",
                candidate.identity.run_id
            ));
        }
        ManagementServerMessage::Failure(failure) => {
            return Err(format!(
                "Sessions rejected lease acquisition: {}",
                failure.detail
            ));
        }
        _ => return Err("Sessions returned the wrong lease result".into()),
    }
    stream
        .set_read_timeout(None)
        .and_then(|()| stream.set_write_timeout(None))
        .and_then(|()| stream.set_nonblocking(true))
        .map_err(|error| format!("cannot configure Sessions management lease: {error}"))?;
    Ok(RunningSession {
        number: candidate.number,
        id: candidate.identity.session_id.clone(),
        endpoint: candidate.endpoint,
        record: candidate.record_path,
        identity: candidate.identity,
        lease: stream,
        child: None,
    })
}

fn cleanup_record(path: &Path, expected: ObjectIdentity) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect ended Sessions record {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected
    {
        return Err(format!(
            "ended Sessions record {} changed before cleanup",
            path.display()
        ));
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "cannot remove ended Sessions record {}: {error}",
            path.display()
        )
    })
}

fn recover_sessions(
    runtime: &Path,
    mode: LaunchMode,
    component_generation: &str,
    deadline: Instant,
) -> Result<Vec<RunningSession>, String> {
    let mut paths = fs::read_dir(runtime)
        .map_err(|error| {
            format!(
                "cannot list Sessions runtime {}: {error}",
                runtime.display()
            )
        })?
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_name().as_bytes().ends_with(b".record") => {
                Some(Ok(entry.path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(format!(
                "cannot inspect Sessions entry in {}: {error}",
                runtime.display()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > MAX_PANES {
        return Err(format!(
            "Sessions runtime {} exceeds the {MAX_PANES}-record recovery limit",
            runtime.display()
        ));
    }
    paths.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });

    let mut candidates = Vec::new();
    let mut numbers = HashSet::new();
    for record_path in paths {
        operation_timeout(deadline, "Sessions recovery")?;
        let snapshot = read_management_record(&record_path)?
            .ok_or_else(|| format!("Sessions record {} disappeared", record_path.display()))?;
        let identity = match &snapshot.record {
            ManagementRecord::Live(identity) => identity,
            ManagementRecord::Tombstone(tombstone) => {
                let (_, endpoint) = validate_management_identity(
                    &tombstone.identity,
                    component_generation,
                    None,
                    false,
                )?;
                if artifact_path(&endpoint, ".record") != record_path {
                    return Err(format!(
                        "ended Sessions record {} has the wrong identity path",
                        record_path.display()
                    ));
                }
                let management_path = PathBuf::from(OsString::from_vec(
                    tombstone.identity.management.path.clone(),
                ));
                loop {
                    if endpoint_removed(&endpoint, tombstone.identity.presentation.object)?
                        && endpoint_removed(&management_path, tombstone.identity.management.object)?
                    {
                        break;
                    }
                    operation_timeout(deadline, "ended Sessions cleanup")?;
                    thread::sleep(Duration::from_millis(25));
                }
                cleanup_record(&record_path, snapshot.object)?;
                continue;
            }
        };
        let (number, endpoint) =
            validate_management_identity(identity, component_generation, None, true)?;
        if artifact_path(&endpoint, ".record") != record_path {
            return Err(format!(
                "Sessions record {} has the wrong presentation identity",
                record_path.display()
            ));
        }
        if !numbers.insert(number) {
            return Err(format!("duplicate live Sessions identity session-{number}"));
        }
        candidates.push(ManagedCandidate {
            number,
            endpoint,
            record_path,
            record: snapshot.object,
            identity: identity.clone(),
        });
    }
    candidates.sort_by_key(|candidate| candidate.number);
    if mode == LaunchMode::Terminal && candidates.len() > 1 {
        return Err("EonTerm cannot recover more than one live Session".into());
    }

    let mut sessions = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        sessions.push(acquire_management(candidate, deadline)?);
    }
    Ok(sessions)
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
    let first_endpoint = sessions[0].endpoint.clone();
    let venus = match start_venus(programs, config, &first_endpoint, mode, terminal) {
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

struct RunningSession {
    number: usize,
    id: String,
    endpoint: PathBuf,
    record: PathBuf,
    identity: LiveIdentity,
    lease: UnixStream,
    child: Option<Child>,
}

fn start_orbit(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    component_generation: &str,
    child: &[OsString],
    deadline: Instant,
) -> Result<RunningSession, String> {
    let run_id = request_id();
    let record_path = artifact_path(socket, ".record");
    let mut orbit_command = orbit_command(
        programs,
        config,
        socket,
        session_id,
        &run_id,
        component_generation,
        child,
    )?;
    let claim = ReadyClaim::create(&record_path)?;
    let mut orbit = match orbit_command.spawn() {
        Ok(orbit) => orbit,
        Err(error) => {
            return match claim.remove_empty() {
                Ok(()) => Err(format!("cannot launch Sessions: {error}")),
                Err(cleanup_error) => Err(format!(
                    "cannot launch Sessions: {error}; cannot roll back Sessions: {cleanup_error}"
                )),
            };
        }
    };
    let result = (|| {
        loop {
            let record = if claim.retains_path()? {
                None
            } else {
                read_management_record(&record_path)?
            };
            match record {
                Some(RecordSnapshot {
                    object,
                    record: ManagementRecord::Live(identity),
                }) => {
                    if let Some(status) = orbit
                        .try_wait()
                        .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
                    {
                        return Err(format!(
                            "Sessions exited before its management lease was acquired (status {})",
                            status_code(status)
                        ));
                    }
                    let (number, endpoint) = validate_management_identity(
                        &identity,
                        component_generation,
                        Some(&run_id),
                        true,
                    )?;
                    if identity.session_id != session_id || endpoint != socket {
                        return Err("Sessions Ready identity does not match its launch".into());
                    }
                    return acquire_management(
                        ManagedCandidate {
                            number,
                            endpoint,
                            record_path: record_path.clone(),
                            record: object,
                            identity,
                        },
                        deadline,
                    );
                }
                Some(RecordSnapshot {
                    record: ManagementRecord::Tombstone(_),
                    ..
                }) => {
                    return Err("Sessions ended before its management lease was acquired".into());
                }
                None => {
                    if let Some(status) = orbit
                        .try_wait()
                        .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
                    {
                        return if claim.exact_path_is_empty()? {
                            Err(format!(
                                "Sessions exited before publishing Ready (status {})",
                                status_code(status)
                            ))
                        } else {
                            Err(format!(
                                "Sessions exited before its management lease was acquired (status {})",
                                status_code(status)
                            ))
                        };
                    }
                    if Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(25));
                    } else {
                        return Err("Sessions did not publish Ready within five seconds".into());
                    }
                }
            }
        }
    })();
    match result {
        Ok(mut session) => {
            session.child = Some(orbit);
            Ok(session)
        }
        Err(error) => match rollback_unleased_orbit(orbit, claim, deadline) {
            Ok(()) => Err(error),
            Err(stop_error) => Err(format!("{error}; cannot roll back Sessions: {stop_error}")),
        },
    }
}

fn rollback_unleased_orbit(
    mut orbit: Child,
    claim: ReadyClaim,
    deadline: Instant,
) -> Result<(), String> {
    let claim_error = match claim.lock_for_rollback(deadline) {
        Ok(true) => {
            while Instant::now() < deadline {
                match orbit.try_wait() {
                    Ok(Some(_)) => return claim.remove_empty(),
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(_) => break,
                }
            }
            stop(&mut orbit);
            return claim.remove_empty();
        }
        Ok(false) => None,
        Err(error) => Some(error),
    };
    let Ok(Some(RecordSnapshot {
        object,
        record: ManagementRecord::Live(identity),
    })) = read_management_record(&claim.path)
    else {
        return claim_error.map_or(Ok(()), Err);
    };
    if identity.process_id != orbit.id() {
        return claim_error.map_or(Ok(()), Err);
    }
    let mut session = acquire_management(
        ManagedCandidate {
            number: session_number(&identity.session_id).unwrap_or(1),
            endpoint: PathBuf::from(OsString::from_vec(identity.presentation.path.clone())),
            record_path: claim.path.clone(),
            record: object,
            identity,
        },
        deadline,
    )?;
    session.child = Some(orbit);
    let timeout = operation_timeout(deadline, "Sessions rollback")?;
    stop_managed_sessions(std::slice::from_mut(&mut session), timeout).map(|_| ())
}

fn orbit_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    run_id: &str,
    component_generation: &str,
    child: &[OsString],
) -> Result<Command, String> {
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .arg("--management-v1")
        .arg(session_id)
        .arg(run_id)
        .arg(component_generation)
        .arg("--ansi-palette-v1")
        .arg(EON_ANSI_PALETTE)
        .arg("--")
        .env("EON_CONFIG_HOME", config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(session_bin) = &programs.session_bin {
        orbit_command.env("PATH", managed_environment::session_path(session_bin)?);
    }
    if child.is_empty() {
        orbit_command.args(managed_environment::shell_command(config)?);
    } else {
        orbit_command.args(child);
    }
    Ok(orbit_command)
}

fn endpoint_removed(path: &Path, expected: ObjectIdentity) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Ok(metadata) if object_identity(&metadata) == expected => Ok(false),
        Ok(_) => Err(format!(
            "Sessions endpoint {} was replaced during cleanup",
            path.display()
        )),
        Err(error) => Err(format!(
            "cannot observe Sessions endpoint cleanup {}: {error}",
            path.display()
        )),
    }
}

fn wait_and_finalize_tombstone(
    session: &mut RunningSession,
    reason: TerminationReason,
    response: Option<&Tombstone>,
    deadline: Instant,
) -> Result<ProcessOutcome, String> {
    let (outcome, record_object) = loop {
        let snapshot = read_management_record(&session.record)?.ok_or_else(|| {
            format!(
                "ended Sessions record {} disappeared",
                session.record.display()
            )
        })?;
        match snapshot.record {
            ManagementRecord::Live(identity) if identity == session.identity => {
                operation_timeout(deadline, "Sessions tombstone reconciliation")?;
                thread::sleep(Duration::from_millis(25));
            }
            ManagementRecord::Tombstone(tombstone)
                if tombstone.identity == session.identity && tombstone.reason == reason =>
            {
                if response.is_some_and(|response| response != &tombstone) {
                    return Err("Sessions stop result differs from its terminal record".into());
                }
                break (tombstone.outcome, snapshot.object);
            }
            _ => return Err("Sessions terminal record does not match the acquired run".into()),
        }
    };
    let management_path =
        PathBuf::from(OsString::from_vec(session.identity.management.path.clone()));
    loop {
        if endpoint_removed(&session.endpoint, session.identity.presentation.object)?
            && endpoint_removed(&management_path, session.identity.management.object)?
        {
            break;
        }
        operation_timeout(deadline, "Sessions endpoint cleanup")?;
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(child) = &mut session.child {
        loop {
            if child
                .try_wait()
                .map_err(|error| format!("cannot reap ended Sessions: {error}"))?
                .is_some()
            {
                break;
            }
            operation_timeout(deadline, "Sessions process reaping")?;
            thread::sleep(Duration::from_millis(25));
        }
    }
    cleanup_record(&session.record, record_object)?;
    Ok(outcome)
}

fn session_finished(session: &mut RunningSession) -> Result<Option<i32>, String> {
    let ended = match session.lease.read(&mut [0]) {
        Ok(0) => true,
        Ok(_) => return Err("Sessions sent an unsolicited management result".into()),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Ok(None),
        Err(_) => true,
    };
    if !ended {
        return Ok(None);
    }
    let outcome = wait_and_finalize_tombstone(
        session,
        TerminationReason::NaturalExit,
        None,
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    Ok(Some(match outcome {
        ProcessOutcome::ExitCode(code) => code,
        ProcessOutcome::Signal(_) => 1,
    }))
}

fn stop_managed_sessions(
    sessions: &mut [RunningSession],
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + timeout;
    let request = management::encode_client_message(&ManagementClientMessage::Stop)
        .map_err(|error| format!("cannot encode Sessions stop: {error}"))?;
    let mut writes = Vec::with_capacity(sessions.len());
    for session in sessions.iter_mut() {
        let result = (|| {
            let remaining = operation_timeout(deadline, "Sessions stop")?;
            session
                .lease
                .set_nonblocking(false)
                .and_then(|()| session.lease.set_write_timeout(Some(remaining)))
                .map_err(|error| format!("cannot bound Sessions stop: {error}"))?;
            session
                .lease
                .write_all(&request)
                .map_err(|error| format!("cannot send Sessions stop: {error}"))
        })();
        writes.push(result);
    }

    let mut errors = Vec::new();
    for (session, write) in sessions.iter_mut().zip(writes) {
        let response = if write.is_ok() {
            match read_management_response(&mut session.lease, deadline, "Sessions stop response") {
                Ok(ManagementServerMessage::Stopped(tombstone))
                    if tombstone.identity == session.identity
                        && tombstone.reason == TerminationReason::ExplicitStop =>
                {
                    Some(Ok(tombstone))
                }
                Ok(ManagementServerMessage::Failure(failure)) => Some(Err(format!(
                    "Sessions rejected stop for {}: {}",
                    session.id, failure.detail
                ))),
                Ok(_) => Some(Err(format!(
                    "Sessions returned the wrong stop result for {}",
                    session.id
                ))),
                Err(_) => None,
            }
        } else {
            None
        };
        let result = match response {
            Some(Ok(tombstone)) => wait_and_finalize_tombstone(
                session,
                TerminationReason::ExplicitStop,
                Some(&tombstone),
                deadline,
            ),
            Some(Err(error)) => Err(error),
            None => wait_and_finalize_tombstone(
                session,
                TerminationReason::ExplicitStop,
                None,
                deadline,
            ),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {error}", session.id));
        }
    }
    if errors.is_empty() {
        Ok(sessions.iter().map(|session| session.id.clone()).collect())
    } else {
        Err(errors.join("; "))
    }
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
                let component_generation = state
                    .sessions
                    .first()
                    .ok_or("supervisor has no component generation")?
                    .identity
                    .component_generation
                    .clone();
                state.sessions.push(start_orbit(
                    programs,
                    config,
                    &session.endpoint,
                    &session.id,
                    &component_generation,
                    &[],
                    Instant::now() + socket_timeout,
                )?);
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
        ControlListener, EON_ANSI_PALETTE, LaunchMode, Programs, effective_uid,
        lock_supervisor_startup, orbit_command, prepare_configuration, prepare_runtime,
        read_management_response, session_number, unix_connect_with_timeout, venus_command,
        xdg_path,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        io::Write,
        os::unix::{
            fs::{MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
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
    fn session_identity_is_positive_canonical_decimal() {
        assert_eq!(session_number("session-1"), Some(1));
        assert_eq!(session_number("session-256"), Some(256));
        for invalid in [
            "session-0",
            "session-01",
            "session-",
            "session-a",
            "Session-1",
            "pane-1",
            "session-184467440737095516160",
        ] {
            assert_eq!(session_number(invalid), None, "accepted {invalid}");
        }
    }

    #[test]
    fn unix_connection_attempt_obeys_timeout() {
        let root = temporary_directory();
        let path = root.join("backlog.sock");
        let socket =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        socket.listen(0).unwrap();
        let listener = UnixListener::from(std::os::fd::OwnedFd::from(socket));
        let queued = UnixStream::connect(&path).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let target = path.clone();
        let attempt = thread::spawn(move || {
            sender
                .send(unix_connect_with_timeout(
                    &target,
                    Duration::from_millis(50),
                ))
                .unwrap();
        });

        let result = receiver.recv_timeout(Duration::from_secs(1));
        drop(queued);
        drop(listener);
        attempt.join().unwrap();

        assert!(
            result
                .expect("Unix connection exceeded its timeout")
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn management_response_obeys_one_deadline_across_partial_reads() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let response =
            super::management::encode_server_message(&super::ManagementServerMessage::Busy)
                .unwrap();
        let sender = thread::spawn(move || {
            for byte in response {
                if writer.write_all(&[byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        let result = read_management_response(
            &mut reader,
            started + Duration::from_millis(80),
            "Sessions management response",
        );
        let elapsed = started.elapsed();
        drop(reader);
        sender.join().unwrap();

        assert!(result.is_err(), "received {result:?}");
        assert!(elapsed >= Duration::from_millis(40));
        assert!(elapsed < Duration::from_secs(1));
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn orbit_uses_configured_argv_only_for_default_sessions() {
        let root = temporary_directory();
        let programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: Some("/managed/bin".into()),
            venus_decorations: true,
        };
        let config = root.as_path();
        let socket = Path::new("/runtime/orbit.sock");
        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-fish\", \"--no-config\"]\n",
        )
        .unwrap();

        let default = orbit_command(
            &programs,
            config,
            socket,
            "session-1",
            "run-1",
            "component-1",
            &[],
        )
        .unwrap();
        assert_eq!(
            default.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v1",
                "session-1",
                "run-1",
                "component-1",
                "--ansi-palette-v1",
                EON_ANSI_PALETTE,
                "--",
                "eon-fish",
                "--no-config",
            ]
            .map(OsString::from)
        );
        assert_eq!(
            command_environment(&default, "EON_CONFIG_HOME"),
            Some(Some(config.as_os_str()))
        );
        assert_eq!(command_environment(&default, "XDG_CONFIG_HOME"), None);
        assert_eq!(
            std::env::split_paths(
                default
                    .get_envs()
                    .find(|(name, _)| *name == "PATH")
                    .unwrap()
                    .1
                    .unwrap(),
            )
            .next(),
            Some(PathBuf::from("/managed/bin"))
        );

        let explicit = orbit_command(
            &programs,
            config,
            socket,
            "session-2",
            "run-2",
            "component-2",
            &["codex".into(), "--model".into(), "test".into()],
        )
        .unwrap();
        assert_eq!(
            explicit.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v1",
                "session-2",
                "run-2",
                "component-2",
                "--ansi-palette-v1",
                EON_ANSI_PALETTE,
                "--",
                "codex",
                "--model",
                "test",
            ]
            .map(OsString::from)
        );
        fs::remove_dir_all(root).unwrap();
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
