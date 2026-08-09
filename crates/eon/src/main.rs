mod workspace;

use eon_workspace_protocol::{
    Action, Direction, Error as ProtocolError, Failure, HEADER_BYTES, MAX_DETAIL_BYTES, Request,
    Response, declared_message_len, decode_request, decode_response, encode_request,
    encode_response,
};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workspace::{
    Workspace, failure_human, failure_json, human as human_output, json as json_output,
};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v1.json");
const USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach | workspace [--json] | tab create [--json] | pane create [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
    shell: PathBuf,
    session_bin: Option<PathBuf>,
}

struct ManagedPrograms {
    nu: PathBuf,
    helix: PathBuf,
    yazi: PathBuf,
    ya: PathBuf,
    lazygit: PathBuf,
    nu_config: PathBuf,
    nu_env: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedTool {
    Nu,
    Helix,
    Yazi,
    Ya,
    LazyGit,
}

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments = arguments.collect();
    let result = match managed_tool(&invocation) {
        Some(tool) => launch_managed(tool, arguments),
        None => execute(arguments),
    };
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("eon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn managed_tool(invocation: &OsStr) -> Option<ManagedTool> {
    let name = Path::new(invocation).file_name()?.to_str()?;
    match name {
        "eon-nu" | "nu" => Some(ManagedTool::Nu),
        "eon-hx" | "hx" => Some(ManagedTool::Helix),
        "eon-yazi" | "yazi" => Some(ManagedTool::Yazi),
        "eon-ya" | "ya" => Some(ManagedTool::Ya),
        "eon-lazygit" | "eon-lg" | "lazygit" => Some(ManagedTool::LazyGit),
        _ => None,
    }
}

fn launch_managed(tool: ManagedTool, arguments: Vec<OsString>) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut command = managed_command(tool, &managed_programs(), &config, &arguments);
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn managed_command(
    tool: ManagedTool,
    programs: &ManagedPrograms,
    config: &Path,
    arguments: &[OsString],
) -> Command {
    let program = match tool {
        ManagedTool::Nu => &programs.nu,
        ManagedTool::Helix => &programs.helix,
        ManagedTool::Yazi => &programs.yazi,
        ManagedTool::Ya => &programs.ya,
        ManagedTool::LazyGit => &programs.lazygit,
    };
    let mut command = Command::new(program);
    match tool {
        ManagedTool::Nu => {
            command
                .arg("--config")
                .arg(&programs.nu_config)
                .arg("--env-config")
                .arg(&programs.nu_env);
        }
        ManagedTool::Yazi | ManagedTool::Ya => {
            command.env_remove("YAZI_CONFIG_HOME");
        }
        ManagedTool::LazyGit => {
            command
                .env_remove("CONFIG_DIR")
                .env_remove("LG_CONFIG_FILE")
                .env("XDG_CONFIG_DIRS", config);
        }
        ManagedTool::Helix => {
            command
                .env_remove("CARGO_MANIFEST_DIR")
                .env_remove("HELIX_RUNTIME")
                .env_remove("HELIX_STEEL_CONFIG");
        }
    }
    command
        .args(arguments)
        .env("EON_CONFIG_HOME", config)
        .env("XDG_CONFIG_HOME", config);
    command
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    match arguments.as_slice() {
        [] if runtime_directory().join("eon.sock").exists() => attach(),
        [] => run(&[]),
        [command] if command == "run" => run(&[]),
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            run(child)
        }
        [command] if command == "attach" => attach(),
        [command, ..]
            if command == "workspace"
                || command == "tab"
                || command == "pane"
                || command == "focus" =>
        {
            control(&arguments)
        }
        [command] if command == "versions" => {
            println!(
                "{}",
                eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?
            );
            Ok(0)
        }
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            println!("{}", path.display());
            Ok(0)
        }
        _ => Err(USAGE.into()),
    }
}

fn run(child: &[OsString]) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    supervise(
        &programs(),
        &config,
        &runtime.join("orbit.sock"),
        child,
        Duration::from_secs(5),
    )
}

fn control(arguments: &[OsString]) -> Result<i32, String> {
    let (action, json) = parse_control_arguments(arguments)?;
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    let socket = runtime.join("eon.sock");
    let mut stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(error) => {
            return Ok(report_failure(
                &failure(
                    "missing-supervisor",
                    format!(
                        "no active Eon supervisor at {}: {error}; run `eon` first",
                        socket.display()
                    ),
                ),
                json,
            ));
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("cannot configure Eon control client: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("cannot configure Eon control client: {error}"))?;
    let request = match encode_request(&Request {
        id: request_id(),
        action,
    }) {
        Ok(request) => request,
        Err(error) => {
            return Ok(report_failure(&protocol_failure(error), json));
        }
    };
    stream
        .write_all(&request)
        .map_err(|error| format!("cannot send Eon action: {error}"))?;

    let mut response = vec![0; HEADER_BYTES];
    stream
        .read_exact(&mut response)
        .map_err(|error| format!("cannot read Eon action result: {error}"))?;
    let length = declared_message_len(&response)
        .map_err(|error| format!("Eon supervisor returned a malformed result: {error}"))?;
    response.resize(length, 0);
    stream
        .read_exact(&mut response[HEADER_BYTES..])
        .map_err(|error| format!("cannot read Eon action result: {error}"))?;
    match decode_response(&response)
        .map_err(|error| format!("Eon supervisor returned a malformed result: {error}"))?
    {
        Response::Snapshot(snapshot) => {
            print!(
                "{}",
                if json {
                    json_output(&snapshot)
                } else {
                    human_output(&snapshot)
                }
            );
            Ok(0)
        }
        Response::Failure(failure) => Ok(report_failure(&failure, json)),
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
                return Err(USAGE.into());
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
        _ => return Err(USAGE.into()),
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

fn attach() -> Result<i32, String> {
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    let socket = runtime.join("orbit.sock");
    let workspace = runtime.join("eon.sock");
    if !workspace.exists() {
        return Err(format!(
            "no active Eon supervisor at {}; run `eon` first",
            workspace.display()
        ));
    }
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    Command::new(programs().venus)
        .arg(socket)
        .arg(workspace)
        .env("XDG_CONFIG_HOME", config)
        .status()
        .map(status_code)
        .map_err(|error| format!("cannot launch Eon Desktop: {error}"))
}

fn programs() -> Programs {
    Programs {
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        shell: configured_program("EON_SHELL", "eon-nu"),
        session_bin: nonempty_environment_path("EON_SESSION_BIN"),
    }
}

fn managed_programs() -> ManagedPrograms {
    ManagedPrograms {
        nu: configured_program("EON_NU", "nu"),
        helix: configured_program("EON_HX", "hx"),
        yazi: configured_program("EON_YAZI", "yazi"),
        ya: configured_program("EON_YA", "ya"),
        lazygit: configured_program("EON_LAZYGIT", "lazygit"),
        nu_config: configured_program("EON_NU_CONFIG", "config.nu"),
        nu_env: configured_program("EON_NU_ENV", "env.nu"),
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

fn runtime_directory() -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join("eon"))
        })
        .unwrap_or_else(|| env::temp_dir().join(format!("eon-{}", effective_uid())))
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

fn supervise(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
    socket_timeout: Duration,
) -> Result<i32, String> {
    let runtime = socket
        .parent()
        .ok_or_else(|| format!("Sessions socket {} has no parent", socket.display()))?;
    let mut initial = start_orbit(programs, config, socket, child, socket_timeout)?;
    let control_listener =
        create_control_listener(&runtime.join("eon.sock")).inspect_err(|_| stop(&mut initial))?;

    let mut venus = match Command::new(&programs.venus)
        .arg(socket)
        .arg(runtime.join("eon.sock"))
        .env("XDG_CONFIG_HOME", config)
        .spawn()
    {
        Ok(venus) => Some(venus),
        Err(error) => {
            stop(&mut initial);
            return Err(format!("cannot launch Eon Desktop: {error}"));
        }
    };
    let mut workspace =
        Workspace::with_initial_session(runtime.to_path_buf(), socket.to_path_buf());
    let mut sessions = vec![RunningSession {
        id: "session-1".into(),
        child: Some(initial),
    }];
    let mut initial_status = None;

    loop {
        for session in &mut sessions {
            let Some(process) = session.child.as_mut() else {
                continue;
            };
            if let Some(status) = process
                .try_wait()
                .map_err(|error| format!("cannot observe Sessions: {error}"))?
            {
                let code = status_code(status);
                session.child = None;
                workspace
                    .session_exited(&session.id)
                    .map_err(|error| error.detail)?;
                if session.id == "session-1" {
                    initial_status = Some(code);
                    if let Some(mut process) = venus.take() {
                        stop(&mut process);
                    }
                }
            }
        }

        if sessions.iter().all(|session| session.child.is_none()) {
            if let Some(mut process) = venus.take() {
                stop(&mut process);
            }
            return initial_status.ok_or("initial Sessions exit status is unavailable".into());
        }

        let venus_exited = if let Some(process) = venus.as_mut() {
            process
                .try_wait()
                .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
                .is_some()
        } else {
            false
        };
        if venus_exited {
            venus = None;
            eprintln!(
                "Eon Desktop exited; Sessions remains active. Run `eon attach` to reconnect."
            );
        }

        accept_control_client(
            &control_listener.listener,
            &mut workspace,
            &mut sessions,
            programs,
            config,
            socket_timeout,
        )?;
        thread::sleep(Duration::from_millis(25));
    }
}

struct RunningSession {
    id: String,
    child: Option<Child>,
}

fn start_orbit(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
    socket_timeout: Duration,
) -> Result<Child, String> {
    let previous_socket = socket_identity(socket)?;
    let mut orbit_command = orbit_command(programs, config, socket, child)?;
    let mut orbit = orbit_command
        .spawn()
        .map_err(|error| format!("cannot launch Sessions: {error}"))?;
    if let Err(error) = wait_for_socket(&mut orbit, socket, previous_socket, socket_timeout) {
        stop(&mut orbit);
        return Err(error);
    }
    Ok(orbit)
}

fn orbit_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
) -> Result<Command, String> {
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .arg("--")
        .env("EON_CONFIG_HOME", config)
        .env("XDG_CONFIG_HOME", config);
    if let Some(session_bin) = &programs.session_bin {
        let mut paths = vec![session_bin.clone()];
        if let Some(path) = env::var_os("PATH") {
            paths.extend(env::split_paths(&path));
        }
        orbit_command.env(
            "PATH",
            env::join_paths(paths)
                .map_err(|error| format!("cannot construct Eon Session PATH: {error}"))?,
        );
    }
    if child.is_empty() {
        orbit_command.arg(&programs.shell);
    } else {
        orbit_command.args(child);
    }
    Ok(orbit_command)
}

struct ControlListener {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl Drop for ControlListener {
    fn drop(&mut self) {
        // The open listener prevents inode reuse; chmod may legitimately change ctime.
        if socket_identity(&self.path)
            .ok()
            .flatten()
            .is_some_and(|identity| (identity.0, identity.1) == self.identity)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_control_listener(path: &Path) -> Result<ControlListener, String> {
    match UnixStream::connect(path) {
        Ok(_) => {
            return Err(format!(
                "an Eon supervisor is already active at {}",
                path.display()
            ));
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::ConnectionRefused => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect Eon control socket {}: {error}",
                path.display()
            ));
        }
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == effective_uid() => {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "cannot remove stale Eon control socket {}: {error}",
                    path.display()
                )
            })?;
        }
        Ok(_) => {
            return Err(format!(
                "Eon control path {} must be an owned Unix socket",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect Eon control path {}: {error}",
                path.display()
            ));
        }
    }
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("cannot bind Eon control socket {}: {error}", path.display()))?;
    let identity = socket_identity(path)?
        .ok_or_else(|| format!("Eon control socket {} disappeared", path.display()))?;
    let control = ControlListener {
        listener,
        path: path.into(),
        identity: (identity.0, identity.1),
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "cannot protect Eon control socket {}: {error}",
            path.display()
        )
    })?;
    control
        .listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure Eon control socket: {error}"))?;
    Ok(control)
}

fn accept_control_client(
    listener: &UnixListener,
    workspace: &mut Workspace,
    sessions: &mut Vec<RunningSession>,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
) -> Result<(), String> {
    match listener.accept() {
        Ok((stream, _)) => handle_control_client(
            stream,
            workspace,
            sessions,
            programs,
            config,
            socket_timeout,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(error) => return Err(format!("cannot accept Eon control client: {error}")),
    }
    Ok(())
}

fn handle_control_client(
    mut stream: UnixStream,
    workspace: &mut Workspace,
    sessions: &mut Vec<RunningSession>,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
) {
    let timeout = Some(Duration::from_millis(250));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return;
    }
    let response = match read_control_request(&mut stream) {
        Ok(request) => match workspace.dispatch(&request.id, request.action, |session| {
            let child = start_orbit(programs, config, &session.endpoint, &[], socket_timeout)?;
            sessions.push(RunningSession {
                id: session.id.clone(),
                child: Some(child),
            });
            Ok(())
        }) {
            Ok(()) => Response::Snapshot(workspace.snapshot()),
            Err(error) => Response::Failure(failure(error.code, error.detail)),
        },
        Err(error) => Response::Failure(error),
    };
    if let Ok(encoded) = encode_response(&response).or_else(|error| {
        encode_response(&Response::Failure(failure(
            "unrepresentable-state",
            format!("cannot encode Eon workspace result: {error}"),
        )))
    }) {
        let _ = stream.write_all(&encoded);
    }
}

fn read_control_request(stream: &mut impl Read) -> Result<Request, Failure> {
    let incomplete = |error| {
        failure(
            "malformed-action",
            format!("cannot read complete EONW request: {error}"),
        )
    };
    let mut message = vec![0; HEADER_BYTES];
    stream.read_exact(&mut message).map_err(&incomplete)?;
    let length = declared_message_len(&message).map_err(protocol_failure)?;
    message.resize(length, 0);
    stream
        .read_exact(&mut message[HEADER_BYTES..])
        .map_err(incomplete)?;
    decode_request(&message).map_err(protocol_failure)
}

fn protocol_failure(error: ProtocolError) -> Failure {
    let code = if matches!(error, ProtocolError::UnsupportedVersion { .. }) {
        "unsupported-version"
    } else {
        "malformed-action"
    };
    failure(code, error.to_string())
}

fn report_failure(failure: &Failure, json: bool) -> i32 {
    if json {
        print!("{}", failure_json(failure));
    } else {
        eprint!("{}", failure_human(failure));
    }
    2
}

fn failure(code: impl Into<String>, detail: impl Into<String>) -> Failure {
    let mut detail = detail.into();
    detail.truncate(detail.floor_char_boundary(MAX_DETAIL_BYTES));
    if detail.is_empty() {
        detail = "unspecified Eon workspace failure".into();
    }
    Failure {
        code: code.into(),
        detail,
    }
}

type SocketIdentity = (u64, u64, i64, i64);

fn socket_identity(socket: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) => Ok(Some((
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect socket {}: {error}",
            socket.display()
        )),
    }
}

fn wait_for_socket(
    child: &mut Child,
    socket: &Path,
    previous_socket: Option<SocketIdentity>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
        {
            return Err(format!(
                "Sessions exited before creating {} (status {})",
                socket.display(),
                status_code(status)
            ));
        }
        if socket_identity(socket)?.is_some_and(|identity| Some(identity) != previous_socket) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Sessions did not create {} within {} seconds",
                socket.display(),
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
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
        ManagedPrograms, ManagedTool, Programs, create_control_listener, effective_uid,
        managed_command, managed_tool, orbit_command, prepare_configuration, prepare_runtime,
        supervise, xdg_path,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eon-test-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn executable(path: &Path, source: &str) {
        fs::write(path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn managed_invocation_names_are_bounded() {
        use ManagedTool::{Helix, LazyGit, Nu, Ya, Yazi};

        for (name, expected) in [
            ("eon-nu", Some(Nu)),
            ("nu", Some(Nu)),
            ("eon-hx", Some(Helix)),
            ("hx", Some(Helix)),
            ("eon-yazi", Some(Yazi)),
            ("yazi", Some(Yazi)),
            ("eon-ya", Some(Ya)),
            ("ya", Some(Ya)),
            ("eon-lazygit", Some(LazyGit)),
            ("eon-lg", Some(LazyGit)),
            ("lazygit", Some(LazyGit)),
            ("lg", None),
            ("eon-starship", None),
            ("eon-zoxide", None),
            ("eon", None),
        ] {
            assert_eq!(managed_tool(Path::new(name).as_os_str()), expected);
        }
        assert_eq!(
            managed_tool(Path::new("/nix/store/example/bin/eon-nu").as_os_str()),
            Some(Nu)
        );
    }

    #[test]
    fn managed_commands_use_exact_programs_and_private_configuration() {
        use ManagedTool::{Helix, LazyGit, Nu, Ya, Yazi};

        let programs = ManagedPrograms {
            nu: "/managed/nu".into(),
            helix: "/managed/hx".into(),
            yazi: "/managed/yazi".into(),
            ya: "/managed/ya".into(),
            lazygit: "/managed/lazygit".into(),
            nu_config: "/managed/config.nu".into(),
            nu_env: "/managed/env.nu".into(),
        };
        let config = Path::new("/private/eon");
        for (tool, program, removed_variables) in [
            (
                Helix,
                "/managed/hx",
                &["CARGO_MANIFEST_DIR", "HELIX_RUNTIME", "HELIX_STEEL_CONFIG"][..],
            ),
            (Yazi, "/managed/yazi", &["YAZI_CONFIG_HOME"][..]),
            (Ya, "/managed/ya", &["YAZI_CONFIG_HOME"][..]),
            (
                LazyGit,
                "/managed/lazygit",
                &["CONFIG_DIR", "LG_CONFIG_FILE"][..],
            ),
        ] {
            let command = managed_command(tool, &programs, config, &["--version".into()]);
            assert_eq!(command.get_program(), program);
            assert_eq!(
                command.get_args().map(OsString::from).collect::<Vec<_>>(),
                vec![OsString::from("--version")]
            );
            for variable in ["EON_CONFIG_HOME", "XDG_CONFIG_HOME"] {
                assert_eq!(
                    command_environment(&command, variable),
                    Some(Some(config.as_os_str()))
                );
            }
            for variable in removed_variables {
                assert_eq!(command_environment(&command, variable), Some(None));
            }
            if tool == LazyGit {
                assert_eq!(
                    command_environment(&command, "XDG_CONFIG_DIRS"),
                    Some(Some(config.as_os_str()))
                );
            }
        }

        let command = managed_command(Nu, &programs, config, &["--version".into()]);
        assert_eq!(command.get_program(), "/managed/nu");
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--config",
                "/managed/config.nu",
                "--env-config",
                "/managed/env.nu",
                "--version",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn orbit_uses_managed_nu_only_for_default_sessions() {
        let programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            shell: "/managed/eon-nu".into(),
            session_bin: Some("/managed/bin".into()),
        };
        let config = Path::new("/private/eon");
        let socket = Path::new("/runtime/orbit.sock");

        let default = orbit_command(&programs, config, socket, &[]).unwrap();
        assert_eq!(
            default.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["serve", "/runtime/orbit.sock", "--", "/managed/eon-nu"].map(OsString::from)
        );
        for variable in ["EON_CONFIG_HOME", "XDG_CONFIG_HOME"] {
            assert_eq!(
                command_environment(&default, variable),
                Some(Some(config.as_os_str()))
            );
        }
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
            &["codex".into(), "--model".into(), "test".into()],
        )
        .unwrap();
        assert_eq!(
            explicit.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--",
                "codex",
                "--model",
                "test",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn venus_exit_leaves_orbit_and_its_command_running() {
        let root = temporary_directory();
        let orbit = root.join("orbit");
        let venus = root.join("venus");
        let socket = root.join("orbit.sock");
        let config = root.join("config");
        let orbit_log = root.join("orbit.log");
        let venus_log = root.join("venus.log");
        let venus_exited = root.join("venus-exited");
        let stop = root.join("stop");
        fs::write(&socket, "old").unwrap();

        executable(
            &orbit,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$XDG_CONFIG_HOME\" \"$@\" > '{}'\nsleep 0.05\nmv \"$2\" \"$2.old\"\nprintf new > \"$2\"\nwhile test ! -e '{}'; do sleep 0.01; done\nexit 23\n",
                orbit_log.display(),
                stop.display()
            ),
        );
        executable(
            &venus,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$XDG_CONFIG_HOME\" \"$@\" \"$(cat \"$1\")\" > '{}'\n: > '{}'
",
                venus_log.display(),
                venus_exited.display()
            ),
        );

        let stop_after_venus = thread::spawn({
            let venus_exited = venus_exited.clone();
            let stop = stop.clone();
            move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !venus_exited.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(venus_exited.exists());
                thread::sleep(Duration::from_millis(100));
                fs::write(stop, "").unwrap();
            }
        });

        let status = supervise(
            &Programs {
                orbit,
                venus,
                shell: root.join("eon-nu"),
                session_bin: None,
            },
            &config,
            &socket,
            &["codex".into(), "--model".into(), "test".into()],
            Duration::from_secs(2),
        )
        .unwrap();
        stop_after_venus.join().unwrap();

        assert_eq!(status, 23);
        assert_eq!(
            fs::read_to_string(orbit_log).unwrap(),
            format!(
                "{}\nserve\n{}\n--\ncodex\n--model\ntest\n",
                config.display(),
                socket.display()
            )
        );
        assert_eq!(
            fs::read_to_string(venus_log).unwrap(),
            format!(
                "{}\n{}\n{}\nnew\n",
                config.display(),
                socket.display(),
                root.join("eon.sock").display()
            )
        );
        fs::remove_dir_all(root).unwrap();
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
    fn control_listener_preserves_a_replacement_socket() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let control = create_control_listener(&socket).unwrap();
        fs::remove_file(&socket).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();

        drop(control);

        assert!(socket.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
