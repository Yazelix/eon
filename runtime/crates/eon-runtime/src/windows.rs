use super::{
    cli::Inputs,
    control::{ControlResponse, EndpointFailureKind, connect_control, send_action, write_stdout},
    generation::private_directory_exists,
    supervisor::{
        SESSION_START_TIMEOUT, base_runtime_directory, prepare_runtime, request_id,
        runtime_directory, validate_private_directory,
    },
    workspace::json_escape,
};
use eon_workspace_protocol::v13::{Action, Response, WindowAction, WindowState, WorkspaceAction};
use std::{
    collections::hash_map::RandomState,
    env,
    ffi::OsStr,
    fs::{self, DirBuilder, OpenOptions},
    hash::BuildHasher,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const WINDOW_RUNTIME: &str = "EON_WINDOW_RUNTIME_DIR";

fn valid_window_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn windows_root() -> PathBuf {
    base_runtime_directory("eon").join("w")
}

fn existing_windows() -> Result<Vec<(String, PathBuf)>, String> {
    let base = base_runtime_directory("eon");
    if !private_directory_exists(&base, "runtime directory")? {
        return Ok(Vec::new());
    }
    let parent = windows_root();
    if !private_directory_exists(&parent, "window directory")? {
        return Ok(Vec::new());
    }
    let mut windows = Vec::new();
    for entry in fs::read_dir(&parent)
        .map_err(|error| format!("cannot list {}: {error}", parent.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot inspect Eon window: {error}"))?;
        let id = entry.file_name();
        let id = id.to_str().ok_or("Eon window ID is not UTF-8")?;
        if !valid_window_id(id) {
            return Err(format!(
                "invalid Eon window ID {id:?} in {}",
                parent.display()
            ));
        }
        validate_private_directory(&entry.path())?;
        match connect_control(&entry.path().join("window.sock")) {
            Ok(_) => {}
            Err(error) if error.kind == EndpointFailureKind::Dead => {
                // Legacy windows keep their generation inventory. A reserved
                // named-window directory alone is only diagnostic state.
                if !entry.path().join("generations").is_dir() {
                    continue;
                }
            }
            Err(error) => return Err(error.detail),
        }
        windows.push((id.to_owned(), entry.path()));
    }
    windows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(windows)
}

fn selected_window(id: &OsStr) -> Result<PathBuf, String> {
    let id = id.to_str().ok_or("Eon window ID must be UTF-8")?;
    if !valid_window_id(id) {
        return Err(format!("invalid Eon window ID {id:?}"));
    }
    let parent = windows_root();
    validate_private_directory(&base_runtime_directory("eon"))?;
    validate_private_directory(&parent)?;
    let root = parent.join(id);
    validate_private_directory(&root)?;
    Ok(root)
}

fn window_command(root: &Path) -> Result<Command, String> {
    let binary = env::current_exe()
        .map_err(|error| format!("cannot find the running Eon executable: {error}"))?;
    let mut command = Command::new(binary);
    command.env(WINDOW_RUNTIME, root);
    Ok(command)
}

pub(super) fn new_window(inputs: &Inputs) -> Result<i32, String> {
    let workspace = super::supervisor::named::create(inputs, "Workspace")?;
    new_named_window("", &workspace.id, true)
}

pub(crate) fn new_named_window(host: &str, workspace: &str, welcome: bool) -> Result<i32, String> {
    let base = base_runtime_directory("eon");
    prepare_runtime(&base)?;
    let parent = windows_root();
    prepare_runtime(&parent)?;
    let (id, root) = loop {
        let id = format!("{:08x}", RandomState::new().hash_one(request_id()) as u32);
        let root = parent.join(&id);
        if root.join("window.sock").as_os_str().as_bytes().len() >= 108 {
            return Err(format!(
                "Eon runtime root {} is too long for a window control socket",
                base.display()
            ));
        }
        match DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => break (id, root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot reserve Eon window {id}: {error}")),
        }
    };
    let log = root.join("launch.log");
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&log)
        .map_err(|error| format!("cannot create {}: {error}", log.display()))?;
    let mut command = window_command(&root)?;
    command
        .args(["__window", host, workspace])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    if welcome {
        command.arg("--welcome");
    }
    // SAFETY: setsid is async-signal-safe and has no parent-process side effects.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start Eon window {id}: {error}"))?;
    let socket = root.join("window.sock");
    let deadline = Instant::now() + SESSION_START_TIMEOUT.saturating_mul(4);
    loop {
        if inspect_window(&socket).is_ok() {
            write_stdout(format!("{id}\n"))?;
            return Ok(0);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon window {id}: {error}"))?
        {
            let detail = fs::read_to_string(&log).unwrap_or_default();
            return Err(format!(
                "Eon window {id} exited {status}: {}",
                detail.trim().chars().take(1000).collect::<String>()
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Eon window {id} did not become ready; its process and log remain at {} for inspection",
                root.display()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn window_action(id: &OsStr, stop: bool, json: bool) -> Result<i32, String> {
    let root = selected_window(id)?;
    if root.join("window.sock").exists() {
        let current = inspect_window(&root.join("window.sock"))?;
        if stop
            && !json
            && let Some(target) = &current.target
        {
            eprint!(
                "Stop workspace {} ({}) in window {}? [y/N] ",
                target.name,
                target.id,
                id.to_string_lossy()
            );
            let answer = super::generation::read_stop_confirmation()?;
            if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
                write_stdout(format!(
                    "cancelled; no Stop sent for window {}\n",
                    id.to_string_lossy()
                ))?;
                return Ok(0);
            }
        }
        let state = named_action_at(
            &root,
            current.generation,
            if stop {
                WindowAction::Stop
            } else {
                WindowAction::Present
            },
        )?;
        if stop {
            let deadline = Instant::now() + SESSION_START_TIMEOUT;
            while root.try_exists().map_err(|error| error.to_string())? {
                if Instant::now() >= deadline {
                    return Err("workspace stopped, but window cleanup has not completed".into());
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        if json {
            write_stdout(format!(
                "{}\n",
                serde_json::json!({"window": id.to_string_lossy(), "connection": format!("{:?}", state.connection)})
            ))?;
        }
        return Ok(0);
    }
    let mut command = window_command(&root)?;
    command.args(if stop {
        &["stop", "all"][..]
    } else {
        &["attach"][..]
    });
    if json {
        command.arg("--json");
    }
    let status = command
        .status()
        .map_err(|error| format!("cannot run Eon window action: {error}"))?;
    Ok(status.code().unwrap_or(1))
}

pub(super) fn stop_all_windows() -> Result<i32, String> {
    let current = runtime_directory("eon");
    let mut windows = existing_windows()?;
    let mut caller_windows = Vec::new();
    if let Some(socket) = env::var_os("EON_WORKSPACE_SOCKET").filter(|value| !value.is_empty()) {
        for (_, root) in &windows {
            if root.join("window.sock").exists() {
                let state = inspect_window(&root.join("window.sock"))?;
                if state.host.is_empty()
                    && state
                        .target
                        .as_ref()
                        .is_some_and(|target| target.control_endpoint == socket.as_bytes())
                {
                    caller_windows.push(root.clone());
                }
            }
        }
    }
    windows.sort_by_key(|(_, root)| root == &current || caller_windows.contains(root));
    let mut status = 0;
    for (id, _) in windows {
        eprintln!("Stop Eon window {id}:");
        match window_action(OsStr::new(&id), true, false) {
            Ok(code) => status = status.max(code),
            Err(error) => {
                eprintln!("{error}");
                status = status.max(1);
            }
        }
    }
    Ok(status)
}

pub(super) fn list_windows(json: bool) -> Result<i32, String> {
    let mut output = if json {
        String::from("{\"windows\":[")
    } else {
        String::new()
    };
    for (index, (id, root)) in existing_windows()?.iter().enumerate() {
        if root.join("window.sock").exists() {
            let state = inspect_window(&root.join("window.sock"))?;
            if json {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::json!({"id": id, "host": state.host, "workspace": state.target.map(|target| target.id), "connection": format!("{:?}", state.connection)}).to_string());
            } else {
                output.push_str(&format!(
                    "window {id}\t{}\t{}\t{:?}\n",
                    if state.host.is_empty() {
                        "Local"
                    } else {
                        &state.host
                    },
                    state
                        .target
                        .as_ref()
                        .map_or("none", |target| target.name.as_str()),
                    state.connection
                ));
            }
            continue;
        }
        let listing = window_command(root)?
            .args(["generations", "--json"])
            .output()
            .map_err(|error| format!("cannot inspect Eon window {id}: {error}"))?;
        if !listing.status.success() {
            return Err(format!(
                "cannot inspect Eon window {id}: {}",
                String::from_utf8_lossy(&listing.stderr)
            ));
        }
        let generations = String::from_utf8(listing.stdout)
            .map_err(|_| format!("Eon window {id} returned non-UTF-8 generations"))?;
        if json {
            if index > 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"inventory\":{}}}",
                json_escape(id),
                generations.trim()
            ));
        } else {
            output.push_str(&format!("window {id}\n{generations}"));
        }
    }
    if json {
        output.push_str("]}\n");
    } else if output.is_empty() {
        output.push_str("no Eon windows\n");
    }
    write_stdout(output)?;
    Ok(0)
}

fn inspect_window(socket: &Path) -> Result<WindowState, String> {
    match send_action(socket, Action::Workspace(WorkspaceAction::Inspect))
        .map_err(|error| error.detail)?
    {
        ControlResponse::Workspace(Response::Window(state)) => Ok(*state),
        ControlResponse::Workspace(Response::Failure(error)) => Err(error.detail),
        _ => Err("window returned an invalid connection state".into()),
    }
}

fn named_action(root: &Path, action: WindowAction) -> Result<WindowState, String> {
    let socket = root.join("window.sock");
    let current = inspect_window(&socket)?;
    named_action_at(root, current.generation, action)
}

fn named_action_at(
    root: &Path,
    generation: u64,
    action: WindowAction,
) -> Result<WindowState, String> {
    match send_action(
        &root.join("window.sock"),
        Action::Window { generation, action },
    )
    .map_err(|error| error.detail)?
    {
        ControlResponse::Workspace(Response::Window(state)) => Ok(*state),
        ControlResponse::Workspace(Response::Failure(error)) => Err(error.detail),
        _ => Err("window returned an invalid action response".into()),
    }
}

pub(crate) fn connect(host: &str, workspace: &str, window: Option<&OsStr>) -> Result<i32, String> {
    let root = choose_window(window)?;
    if let Some(root) = root {
        named_action(
            &root,
            WindowAction::Select {
                host: host.into(),
                workspace: workspace.into(),
            },
        )?;
        Ok(0)
    } else {
        new_named_window(host, workspace, false)
    }
}

pub(crate) fn disconnect(window: Option<&OsStr>) -> Result<i32, String> {
    let root = choose_window(window)?.ok_or("there is no local window to disconnect")?;
    named_action(&root, WindowAction::Disconnect)?;
    Ok(0)
}

fn choose_window(window: Option<&OsStr>) -> Result<Option<PathBuf>, String> {
    if let Some(window) = window {
        return selected_window(window).map(Some);
    }
    let mut windows = existing_windows()?;
    if windows.len() > 1 {
        return Err("more than one local window exists; specify --window ID".into());
    }
    Ok(windows.pop().map(|(_, root)| root))
}
