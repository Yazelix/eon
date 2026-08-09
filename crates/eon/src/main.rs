mod workspace;

use std::{
    env,
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workspace::{Action, ActionError, Direction, Workspace};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v1.json");
const USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach | workspace [--json] | tab create [--json] | pane create [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
const MAX_CONTROL_REQUEST_BYTES: u64 = 4_096;
const MAX_CONTROL_RESPONSE_BYTES: u64 = 128 * 1_024;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
}

fn main() -> ExitCode {
    match execute(env::args_os().skip(1).collect()) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("eon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    match arguments.as_slice() {
        [] if runtime_directory().join("orbit.sock").exists() => attach(),
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
    let mut stream = UnixStream::connect(&socket).map_err(|error| {
        format!(
            "no active Eon supervisor at {}: {error}; run `eon` first",
            socket.display()
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("cannot configure Eon control client: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("cannot configure Eon control client: {error}"))?;
    let request = encode_control_request(&request_id(), json, &action);
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("cannot send Eon action: {error}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| format!("cannot finish Eon action: {error}"))?;

    let mut response = Vec::new();
    stream
        .take(MAX_CONTROL_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|error| format!("cannot read Eon action result: {error}"))?;
    if response.len() as u64 > MAX_CONTROL_RESPONSE_BYTES {
        return Err("Eon action result exceeded its size limit".into());
    }
    let response = String::from_utf8(response)
        .map_err(|_| "Eon supervisor returned non-UTF-8 output".to_string())?;
    let (status, output) = response
        .split_once('\n')
        .ok_or_else(|| "Eon supervisor returned a malformed result".to_string())?;
    match status {
        "ok" => {
            print!("{output}");
            Ok(0)
        }
        "error" => {
            if json {
                print!("{output}");
            } else {
                eprint!("{output}");
            }
            Ok(2)
        }
        _ => Err("Eon supervisor returned a malformed result".into()),
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

fn encode_control_request(request_id: &str, json: bool, action: &Action) -> String {
    let action = match action {
        Action::Inspect => "inspect".into(),
        Action::CreateTab => "tab-create".into(),
        Action::CreatePane => "pane-create".into(),
        Action::FocusId(id) => format!("focus-id\t{id}"),
        Action::Focus(Direction::Left) => "focus-left".into(),
        Action::Focus(Direction::Right) => "focus-right".into(),
        Action::Focus(Direction::Up) => "focus-up".into(),
        Action::Focus(Direction::Down) => "focus-down".into(),
    };
    format!(
        "{request_id}\t{}\t{action}\n",
        if json { "json" } else { "human" }
    )
}

fn attach() -> Result<i32, String> {
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    let socket = runtime.join("orbit.sock");
    if !socket.exists() {
        return Err(format!(
            "no active Sessions socket at {}; run `eon` first",
            socket.display()
        ));
    }
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    Command::new(programs().venus)
        .arg(socket)
        .env("XDG_CONFIG_HOME", config)
        .status()
        .map(status_code)
        .map_err(|error| format!("cannot launch Eon Desktop: {error}"))
}

fn programs() -> Programs {
    Programs {
        orbit: env::var_os("EON_ORBIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "yazelix-orbit".into()),
        venus: env::var_os("EON_VENUS")
            .map(PathBuf::from)
            .unwrap_or_else(|| "yazelix-venus".into()),
    }
}

fn configuration_directory() -> Result<PathBuf, String> {
    if let Some(path) = nonempty_environment_path("EON_CONFIG_HOME") {
        return Ok(path);
    }
    if let Some(path) = xdg_path(nonempty_environment_path("XDG_CONFIG_HOME")) {
        return Ok(path.join("eon"));
    }
    nonempty_environment_path("HOME")
        .map(|path| path.join(".config/eon"))
        .ok_or_else(|| "HOME, XDG_CONFIG_HOME, and EON_CONFIG_HOME are unset".into())
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
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .env("XDG_CONFIG_HOME", config);
    if !child.is_empty() {
        orbit_command.arg("--").args(child);
    }
    let mut orbit = orbit_command
        .spawn()
        .map_err(|error| format!("cannot launch Sessions: {error}"))?;
    if let Err(error) = wait_for_socket(&mut orbit, socket, previous_socket, socket_timeout) {
        stop(&mut orbit);
        return Err(error);
    }
    Ok(orbit)
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
    let mut bytes = Vec::new();
    let read = Read::by_ref(&mut stream)
        .take(MAX_CONTROL_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes);
    let json_hint = bytes
        .split(|byte| *byte == b'\t')
        .nth(1)
        .is_some_and(|value| value == b"json");
    let request = match read {
        Ok(_) if bytes.len() as u64 <= MAX_CONTROL_REQUEST_BYTES => parse_control_request(&bytes),
        Ok(_) => Err(ActionError::new(
            "malformed-action",
            "control request exceeded 4096 bytes",
        )),
        Err(error) => Err(ActionError::new(
            "malformed-action",
            format!("cannot read complete control request: {error}"),
        )),
    };
    let response = match request {
        Ok(request) => {
            let result = workspace.dispatch(&request.id, request.action, |session| {
                let child = start_orbit(programs, config, &session.endpoint, &[], socket_timeout)?;
                sessions.push(RunningSession {
                    id: session.id.clone(),
                    child: Some(child),
                });
                Ok(())
            });
            match result {
                Ok(()) => format!(
                    "ok\n{}",
                    if request.json {
                        workspace.json()
                    } else {
                        workspace.human()
                    }
                ),
                Err(error) => control_error(&error, request.json),
            }
        }
        Err(error) => control_error(&error, json_hint),
    };
    debug_assert!(response.len() as u64 <= MAX_CONTROL_RESPONSE_BYTES);
    let _ = stream.write_all(response.as_bytes());
}

#[derive(Debug)]
struct ControlRequest {
    id: String,
    json: bool,
    action: Action,
}

fn parse_control_request(bytes: &[u8]) -> Result<ControlRequest, ActionError> {
    let request = std::str::from_utf8(bytes)
        .map_err(|_| ActionError::new("malformed-action", "control request is not UTF-8"))?;
    let request = request.strip_suffix('\n').ok_or_else(|| {
        ActionError::new(
            "malformed-action",
            "control request is not newline terminated",
        )
    })?;
    if request.contains('\n') || request.contains('\r') {
        return Err(ActionError::new(
            "malformed-action",
            "control request contains multiple lines",
        ));
    }
    let fields: Vec<&str> = request.split('\t').collect();
    let id = fields[0];
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(ActionError::new(
            "malformed-action",
            "control request identity is invalid",
        ));
    }
    let json = match fields.get(1).copied() {
        Some("human") => false,
        Some("json") => true,
        _ => {
            return Err(ActionError::new(
                "malformed-action",
                "control output must be human or json",
            ));
        }
    };
    let action = match fields.get(2).copied() {
        Some("inspect") if fields.len() == 3 => Action::Inspect,
        Some("tab-create") if fields.len() == 3 => Action::CreateTab,
        Some("pane-create") if fields.len() == 3 => Action::CreatePane,
        Some("focus-left") if fields.len() == 3 => Action::Focus(Direction::Left),
        Some("focus-right") if fields.len() == 3 => Action::Focus(Direction::Right),
        Some("focus-up") if fields.len() == 3 => Action::Focus(Direction::Up),
        Some("focus-down") if fields.len() == 3 => Action::Focus(Direction::Down),
        Some("focus-id") if fields.len() == 4 && !fields[3].is_empty() => {
            Action::FocusId(fields[3].into())
        }
        _ => {
            return Err(ActionError::new(
                "malformed-action",
                "unknown or invalid control action",
            ));
        }
    };
    Ok(ControlRequest {
        id: id.into(),
        json,
        action,
    })
}

fn control_error(error: &ActionError, json: bool) -> String {
    format!("error\n{}", if json { error.json() } else { error.human() })
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
        Action, Programs, create_control_listener, effective_uid, parse_control_request,
        prepare_configuration, prepare_runtime, supervise, xdg_path,
    };
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
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
            &Programs { orbit, venus },
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
            format!("{}\n{}\nnew\n", config.display(), socket.display())
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
    fn private_control_parser_rejects_malformed_actions() {
        let request = parse_control_request(b"request-1\tjson\tfocus-id\tpane-1\n").unwrap();
        assert!(matches!(request.action, Action::FocusId(id) if id == "pane-1"));

        for malformed in [
            b"request-1\tjson\tunknown\n".as_slice(),
            b"request-1\tjson\tinspect\nsecond\n".as_slice(),
            b"request 1\tjson\tinspect\n".as_slice(),
            b"request-1\tjson\tinspect".as_slice(),
        ] {
            assert_eq!(
                parse_control_request(malformed).unwrap_err().code,
                "malformed-action"
            );
        }
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
