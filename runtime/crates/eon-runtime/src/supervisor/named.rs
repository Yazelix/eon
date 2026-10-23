//! Named workspaces outlive their graphical clients. Each supervisor owns one layout.
use super::*;
use crate::{
    control::{PendingRequest, send_action},
    sessions::{OrbitLaunch, acquire_named_sessions, start_orbit_with_run},
};
use eon_workspace_protocol::v13::{Compatibility, RegistryAction, WorkspaceInfo};
use serde::{Deserialize, Serialize};
use std::io::{Seek, SeekFrom};
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt, process::CommandExt};
pub(crate) mod service;

const MAX_PRESENTATIONS: usize = 16;

pub(crate) fn local_command(
    inputs: &Inputs,
    arguments: &[OsString],
) -> Option<Result<i32, String>> {
    let values = arguments
        .iter()
        .map(|value| value.to_str())
        .collect::<Option<Vec<_>>>()?;
    Some(match values.as_slice() {
        ["workspaces"] | ["workspaces", "--json"] => list(inputs).and_then(|values| {
            print_workspaces(
                &values,
                arguments.last().is_some_and(|value| value == "--json"),
            )
        }),
        ["workspace", "create", "--name", name] => create(inputs, name).and_then(|value| {
            let value = ensure(inputs, &value.id, false)?;
            print_workspaces(&[value], false)
        }),
        ["workspace", "stop", id] => stop_named(inputs, id, false).map(|_| 0),
        ["workspace", "forget", id] => forget(inputs, id).map(|()| 0),
        _ => return None,
    })
}

pub(crate) fn print_workspaces(values: &[WorkspaceInfo], json: bool) -> Result<i32, String> {
    let output = if json {
        let values = values.iter().map(|value| serde_json::json!({"id": value.id, "name": value.name, "build": value.build})).collect::<Vec<_>>();
        format!(
            "{}\n",
            serde_json::to_string(&values).map_err(|error| error.to_string())?
        )
    } else {
        values
            .iter()
            .map(|value| format!("{}\t{}\n", value.id, value.name))
            .collect()
    };
    crate::control::write_stdout(output)?;
    Ok(0)
}

pub(crate) fn stop_named(inputs: &Inputs, id: &str, scoped: bool) -> Result<(), String> {
    let value = ensure(inputs, id, scoped)?;
    match send_action(
        Path::new(std::ffi::OsStr::from_bytes(&value.control_endpoint)),
        Action::Registry(RegistryAction::Stop {
            workspace: id.into(),
        }),
    ) {
        Ok(ControlResponse::Workspace(Response::Workspace(_))) => Ok(()),
        Ok(ControlResponse::Workspace(Response::Failure(error))) => Err(error.detail),
        Ok(_) => Err("workspace returned the wrong Stop response".into()),
        Err(error) => Err(error.detail),
    }
}

pub(crate) fn forget(inputs: &Inputs, id: &str) -> Result<(), String> {
    inputs.component_report()?;
    let root = state_root()?;
    let _registry = lock_supervisor_lifecycle(&root.join("registry.lock"))?;
    registration(id)?;
    let state = state_directory(id)?;
    let runtime = runtime_path(id)?;
    let config = configuration_directory()?;
    let catalog = managed_environment::popup_catalog(&config, &inputs.defaults)?;
    let layout = Workspace::load_layout(
        &state.join("layout.json"),
        id,
        runtime.clone(),
        env::home_dir(),
        catalog,
    )?;
    if !layout.stopped {
        return Err("workspace must complete an explicit Stop before Forget".into());
    }
    prepare_named_runtime(id)?;
    let _supervisor = try_lock_supervisor_lifecycle(&runtime.join("supervisor.lock"))?
        .ok_or("workspace supervisor is still running; retry Forget after Stop completes")?;
    // Reuse exact UID, record, PID/start and endpoint checks. This never sends Stop.
    let sessions = acquire_named_sessions(
        &runtime,
        LaunchMode::Workspace,
        id,
        &[],
        &[],
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    if !sessions.is_empty() {
        return Err("workspace still has live or unowned terminal runs; Forget refused".into());
    }
    match connect_control(&runtime.join("eon.sock")) {
        Err(error) if error.kind == EndpointFailureKind::Dead => {}
        Err(error) => return Err(error.detail),
        Ok(_) => return Err("workspace still has a live control endpoint; Forget refused".into()),
    }
    let mut files = Vec::new();
    for (directory, persistent) in [(&state, true), (&runtime, false)] {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or("unexpected workspace artifact; Forget refused")?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            let expected = if persistent {
                (matches!(name, "workspace.json" | "layout.json") || name.starts_with(".layout-"))
                    && metadata.is_file()
            } else if name == "eon.sock" {
                metadata.file_type().is_socket()
            } else {
                matches!(name, "supervisor.lock" | "supervisor.log") && metadata.is_file()
            };
            if !expected || metadata.uid() != effective_uid() || metadata.mode() & 0o7777 != 0o600 {
                return Err(format!(
                    "unowned or unresolved workspace artifact {}; Forget refused",
                    entry.path().display()
                ));
            }
            files.push((persistent, name.to_owned()));
        }
    }
    // Remove the registration from discovery atomically before deleting only its validated files.
    let retired = root.join(format!(".forget-{id}-{}", request_id()));
    fs::rename(&state, &retired).map_err(|error| format!("cannot retire workspace: {error}"))?;
    let cleanup = (|| -> std::io::Result<()> {
        fs::File::open(&root)?.sync_all()?;
        for (persistent, name) in files {
            fs::remove_file(if persistent {
                retired.join(name)
            } else {
                runtime.join(name)
            })?;
        }
        fs::remove_dir(&runtime)?;
        fs::remove_dir(&retired)?;
        fs::File::open(&root)?.sync_all()
    })();
    cleanup.map_err(|error| {
        format!(
            "workspace {id} was retired but cleanup failed: {error}; inspect {} and {}",
            retired.display(),
            runtime.display()
        )
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    format: u32,
    id: String,
    name: String,
    directory: Vec<u8>,
}

pub(crate) fn state_root() -> Result<PathBuf, String> {
    let root = nonempty_environment_path("XDG_STATE_HOME")
        .filter(|path| path.is_absolute())
        .or_else(|| env::home_dir().map(|home| home.join(".local/state")))
        .ok_or("cannot resolve workspace state directory")?
        .join("eon/workspaces");
    prepare_runtime(&root)?;
    Ok(root)
}

fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn state_directory(id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("invalid named workspace identity".into());
    }
    let path = state_root()?.join(id);
    validate_private_directory(&path)?;
    Ok(path)
}

pub(crate) fn runtime_path(id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("invalid named workspace identity".into());
    }
    let path = base_runtime_directory("eon").join("n").join(id);
    if path
        .join("session-18446744073709551614.sock.management")
        .as_os_str()
        .as_bytes()
        .len()
        >= 108
    {
        return Err("named workspace runtime path exceeds the Unix socket limit".into());
    }
    Ok(path)
}

fn prepare_named_runtime(id: &str) -> Result<PathBuf, String> {
    let path = runtime_path(id)?;
    prepare_runtime(&base_runtime_directory("eon"))?;
    prepare_runtime(path.parent().unwrap())?;
    prepare_runtime(&path)?;
    Ok(path)
}

fn registration(id: &str) -> Result<Registration, String> {
    let path = state_directory(id)?.join("workspace.json");
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|error| format!("cannot read workspace registration: {error}"))?;
    let meta = file.metadata().map_err(|error| error.to_string())?;
    if !meta.is_file()
        || meta.uid() != effective_uid()
        || meta.mode() & 0o7777 != 0o600
        || meta.len() > 8192
    {
        return Err("workspace registration must be an owned bounded mode-0600 file".into());
    }
    let mut bytes = Vec::new();
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let value: Registration = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid workspace registration: {error}"))?;
    if value.format != 1
        || value.id != id
        || value.name.is_empty()
        || value.name.len() > 128
        || value.name.chars().any(char::is_control)
        || value.directory.len() > 4096
        || value.directory.contains(&0)
        || !Path::new(std::ffi::OsStr::from_bytes(&value.directory)).is_absolute()
    {
        return Err("invalid workspace registration identity, name or directory".into());
    }
    Ok(value)
}

pub(crate) fn info(inputs: &Inputs, id: &str) -> Result<WorkspaceInfo, String> {
    inputs.component_report()?;
    let registration = registration(id)?;
    Ok(WorkspaceInfo {
        id: registration.id,
        name: registration.name,
        build: current_generation(inputs)?,
        control_endpoint: runtime_path(id)?
            .join("eon.sock")
            .as_os_str()
            .as_bytes()
            .to_vec(),
        compatibility: Compatibility::default(),
        stopped: Workspace::persisted_stopped(&state_directory(id)?.join("layout.json"), id)?,
    })
}

pub(crate) fn list(inputs: &Inputs) -> Result<Vec<WorkspaceInfo>, String> {
    inputs.component_report()?;
    let root = state_root()?;
    let _registry = lock_supervisor_lifecycle(&root.join("registry.lock"))?;
    list_registered(inputs, &root)
}

fn list_registered(inputs: &Inputs, root: &Path) -> Result<Vec<WorkspaceInfo>, String> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().filter(|id| valid_id(id)) else {
            continue;
        };
        result.push(info(inputs, id)?);
    }
    if result.len() > eon_workspace_protocol::v13::MAX_WORKSPACES {
        return Err("workspace registry exceeds its limit".into());
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(result)
}

pub(crate) fn create(inputs: &Inputs, name: &str) -> Result<WorkspaceInfo, String> {
    inputs.component_report()?;
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err("workspace name must contain 1–128 bytes without control characters".into());
    }
    let directory = env::current_dir()
        .map_err(|error| error.to_string())?
        .as_os_str()
        .as_bytes()
        .to_vec();
    if directory.len() > 4096 {
        return Err("workspace directory exceeds its size limit".into());
    }
    let root = state_root()?;
    let _lock = lock_supervisor_lifecycle(&root.join("registry.lock"))?;
    if list_registered(inputs, &root)?.len() >= eon_workspace_protocol::v13::MAX_WORKSPACES {
        return Err("workspace registry is full".into());
    }
    let mut random = [0; 8];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| error.to_string())?;
    let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = root.join(&id);
    let value = Registration {
        format: 1,
        id: id.clone(),
        name: name.into(),
        directory,
    };
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("workspace registration exceeds its size limit".into());
    }
    let temporary = root.join(format!(".create-{id}"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temporary.join("workspace.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::File::open(&temporary)?.sync_all()?;
        fs::rename(&temporary, &path)?;
        fs::File::open(&root)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary.join("workspace.json"));
        let _ = fs::remove_dir(&temporary);
    }
    result.map_err(|error| format!("cannot register workspace: {error}"))?;
    info(inputs, &id)
}

pub(crate) fn ensure(inputs: &Inputs, id: &str, scoped: bool) -> Result<WorkspaceInfo, String> {
    ensure_with_welcome(inputs, id, scoped, false)
}

pub(crate) fn ensure_for_window(inputs: &Inputs, id: &str) -> Result<WorkspaceInfo, String> {
    ensure_with_welcome(inputs, id, false, true)
}

fn ensure_with_welcome(
    inputs: &Inputs,
    id: &str,
    scoped: bool,
    welcome: bool,
) -> Result<WorkspaceInfo, String> {
    let value = {
        let _registry = lock_supervisor_lifecycle(&state_root()?.join("registry.lock"))?;
        let value = info(inputs, id)?;
        prepare_named_runtime(id)?;
        value
    };
    let socket = Path::new(std::ffi::OsStr::from_bytes(&value.control_endpoint));
    match send_action(
        socket,
        Action::Hello {
            workspace: id.into(),
        },
    ) {
        Ok(ControlResponse::Workspace(Response::Hello(accepted))) if accepted.id == id => {
            return Ok(accepted);
        }
        Ok(ControlResponse::Workspace(Response::Failure(error))) => return Err(error.detail),
        Ok(_) => return Err("workspace returned an invalid handshake".into()),
        Err(error) if error.kind == EndpointFailureKind::Dead => {}
        Err(error) => return Err(error.detail),
    }
    let launched = if scoped {
        let status = Command::new("systemctl")
            .args(["--user", "start", &format!("eon-workspace@{id}.service")])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(
                "cannot start workspace service; run eon-server service install and doctor".into(),
            );
        }
        None
    } else {
        let log_path = socket.with_file_name("supervisor.log");
        let log = fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&log_path)
            .map_err(|error| format!("cannot open workspace diagnostics: {error}"))?;
        let metadata = log.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err("workspace diagnostics must be an owned mode-0600 file".into());
        }
        let diagnostic_start = metadata.len();
        let mut command = Command::new(env::current_exe().map_err(|error| error.to_string())?);
        command
            .env_remove("EON_WINDOW_RUNTIME_DIR")
            .env_remove("EON_WORKSPACE_SOCKET")
            .args(["__serve-workspace", id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log.try_clone().map_err(|error| error.to_string())?);
        if welcome {
            command.arg("--welcome");
        }
        // A local detached supervisor has no relationship to any one window's process group.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let (status, exited) = std::sync::mpsc::sync_channel(1);
        // Spawn inside the successfully created waiter, so every Child has a
        // wait owner even when a window disconnects but remains alive.
        thread::Builder::new()
            .name("eon-local-supervisor".into())
            .spawn(move || {
                let _ = status.send(command.status());
            })
            .map_err(|error| format!("cannot start workspace supervisor waiter: {error}"))?;
        Some((exited, log, diagnostic_start, log_path))
    };
    let mut launched = launched;
    let deadline = Instant::now() + SESSION_START_TIMEOUT + Duration::from_secs(1);
    loop {
        match send_action(
            socket,
            Action::Hello {
                workspace: id.into(),
            },
        ) {
            Ok(ControlResponse::Workspace(Response::Hello(value))) if value.id == id => {
                if let Some((_, log, start, path)) = &mut launched {
                    let detail = launch_diagnostic(log, *start)?;
                    if !detail.is_empty() {
                        eprintln!(
                            "Eon workspace {id}: {detail}\nDiagnostics: {}",
                            path.display()
                        );
                    }
                }
                return Ok(value);
            }
            Ok(ControlResponse::Workspace(Response::Failure(error))) => return Err(error.detail),
            Ok(_) => return Err("workspace returned an invalid handshake".into()),
            // Bind publishes the socket before chmod. During our bounded startup
            // wait, retry incomplete publication but still require a valid Hello.
            Err(error)
                if matches!(
                    error.kind,
                    EndpointFailureKind::Dead | EndpointFailureKind::Corrupt
                ) && Instant::now() < deadline =>
            {
                if let Some((exited, log, start, path)) = &mut launched {
                    match exited.try_recv() {
                        Ok(status) => {
                            let status = status.map_err(|error| {
                                format!("cannot start workspace supervisor: {error}")
                            })?;
                            return Err(format!(
                                "workspace supervisor exited {status}: {} (diagnostics: {})",
                                launch_diagnostic(log, *start)?,
                                path.display()
                            ));
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return Err("workspace supervisor waiter disconnected".into());
                        }
                    }
                }
                thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(format!("workspace did not become ready: {}", error.detail)),
        }
    }
}

fn launch_diagnostic(log: &mut fs::File, start: u64) -> Result<String, String> {
    let end = log.metadata().map_err(|error| error.to_string())?.len();
    log.seek(SeekFrom::Start(start.max(end.saturating_sub(4096))))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    log.take(4096)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

struct Named<'a> {
    inputs: &'a Inputs,
    config: PathBuf,
    info: WorkspaceInfo,
    layout: PathBuf,
    workspace: Workspace,
    sessions: Vec<RunningSession>,
    scoped: bool,
    stopping: bool,
    presentations: Vec<UnixStream>,
    retiring: Vec<String>,
    next_retirement: Instant,
    quotas: quota::Quotas,
    next_recovery: Instant,
    startup_animation: Option<managed_environment::StartupAnimation>,
}

fn collect_operation(
    inputs: &Inputs,
    operations: &mut Vec<SessionOperation>,
    operation: SessionOperation,
) -> Result<(), String> {
    if let SessionOperation::Start {
        session,
        command: LaunchCommand::Project { tab, instance },
        ..
    } = &operation
    {
        directory_picker_command(
            inputs,
            session.endpoint.parent().ok_or("picker has no workspace")?,
            tab,
            instance,
            None,
        )?;
    }
    operations.push(operation);
    Ok(())
}

// Each workspace action launches at most one terminal. Feed its known failure
// through the existing reducer before effects, so it owns rollback policy too.
fn reject_start(operation: SessionOperation) -> Result<(), String> {
    match operation {
        SessionOperation::Start { .. } => Err("terminal launch failed".into()),
        SessionOperation::Stop(_) => Ok(()),
    }
}

fn apply_workspace_request(
    inputs: &Inputs,
    workspace: &mut Workspace,
    request: &Request,
    mut operate: impl FnMut(SessionOperation) -> Result<(), String>,
) -> Result<(), Failure> {
    match request.action.clone() {
        Action::ClosePane { pane } => {
            let id = workspace.prepare_close_pane(&request.id, &pane)?;
            operate(SessionOperation::Stop(id.clone()))
                .map_err(|error| failure("workspace-operation", error))?;
            workspace.session_ended(&id, SessionEnd::Closed, |_| {
                Err("close cannot launch a terminal".into())
            })?;
        }
        Action::Workspace(WorkspaceAction::CloseTab { tab }) => {
            for id in workspace.prepare_close_tab(&request.id, &tab)? {
                operate(SessionOperation::Stop(id.clone()))
                    .map_err(|error| failure("workspace-operation", error))?;
                workspace.session_ended(&id, SessionEnd::Closed, |_| {
                    Err("close cannot launch a terminal".into())
                })?;
            }
        }
        action => workspace.dispatch(
            &request.id,
            action,
            |command, directory| {
                managed_environment::prepare_popup_command(
                    command,
                    inputs.managed.shell_bin.as_deref(),
                    directory,
                    &inputs.defaults,
                )
            },
            operate,
        )?,
    }
    Ok(())
}

impl Named<'_> {
    fn reconcile(&mut self) {
        let live = self
            .sessions
            .iter()
            .filter(|session| !session.is_finished())
            .map(|session| (session.id.clone(), session.run_id().into()))
            .collect::<Vec<_>>();
        self.workspace.reconcile_runs(&live);
    }
    fn save(&self) -> Result<(), String> {
        self.workspace.save_layout(&self.layout, &self.info.id)
    }
    fn snapshot(&self) -> Response {
        let mut value = self.workspace.named_snapshot(&self.info.id);
        self.quotas.apply(&mut value.snapshot);
        Response::Workspace(value)
    }
    fn require_resolved_launches(&self, only: Option<&str>) -> Result<(), String> {
        for slot in self
            .workspace
            .sessions()
            .filter(|slot| only.is_none_or(|id| slot.id == id))
        {
            if self.sessions.iter().any(|session| session.id == slot.id) {
                continue;
            }
            let mut record = slot.endpoint.as_os_str().to_os_string();
            record.push(".record");
            match fs::symlink_metadata(PathBuf::from(record)) {
                Ok(_) => {
                    return Err(format!(
                        "terminal {} has an unresolved launch outcome; its slot and tasks were preserved; retry after Ready is published",
                        slot.id
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot inspect terminal launch: {error}")),
            }
        }
        Ok(())
    }
    fn operate(&mut self, operation: SessionOperation) -> Result<(), String> {
        let (session, directory, command) = match operation {
            SessionOperation::Stop(id) => {
                self.require_resolved_launches(Some(&id))?;
                let expected = self
                    .workspace
                    .sessions()
                    .find(|slot| slot.id == id)
                    .map(|slot| slot.run.as_str());
                if self
                    .sessions
                    .iter()
                    .any(|session| session.id == id && Some(session.run_id()) == expected)
                {
                    stop_owned_session(&mut self.sessions, &id)?;
                }
                return Ok(());
            }
            SessionOperation::Start {
                session,
                directory,
                command,
            } => (session, directory, command),
        };
        let child = match command {
            LaunchCommand::Pane => Vec::new(),
            LaunchCommand::Tool(argv) => argv,
            LaunchCommand::Project { tab, instance } => {
                let animation = self.startup_animation.take();
                directory_picker_command(
                    self.inputs,
                    session
                        .endpoint
                        .parent()
                        .ok_or("terminal endpoint has no workspace")?,
                    &tab,
                    &instance,
                    animation.as_ref(),
                )?
            }
        };
        self.sessions.push(start_orbit_with_run(
            self.inputs,
            &self.config,
            &session.endpoint,
            &session.id,
            &self.info.build,
            &directory,
            &child,
            Instant::now() + SESSION_START_TIMEOUT,
            &OrbitLaunch {
                workspace: &self.info.id,
                run: &session.run,
                scoped: self.scoped,
            },
        )?);
        Ok(())
    }
    fn commit(
        &mut self,
        mut candidate: Workspace,
        operations: Vec<SessionOperation>,
        failed: Option<Workspace>,
    ) -> Result<(), Failure> {
        let (stops, starts): (Vec<_>, Vec<_>) = operations
            .into_iter()
            .partition(|operation| matches!(operation, SessionOperation::Stop(_)));
        // Destructive requests retain their slots until the exact run has acknowledged Stop.
        if let Err(error) = stops
            .into_iter()
            .try_for_each(|operation| self.operate(operation))
        {
            self.workspace.retain_requests_from(&candidate);
            self.reconcile();
            self.save().map_err(|error| failure("layout-save", error))?;
            return Err(failure("workspace-operation", error));
        }
        let live = self
            .sessions
            .iter()
            .map(|session| (session.id.clone(), session.run_id().into()))
            .collect::<Vec<_>>();
        candidate.reconcile_runs(&live);
        // Slots and run identities reach disk before any process may publish Ready.
        if let Err(error) = candidate.save_layout(&self.layout, &self.info.id) {
            self.workspace.retain_requests_from(&candidate);
            self.reconcile();
            return Err(failure("layout-save", error));
        }
        let result = starts.into_iter().try_for_each(|operation| {
            let SessionOperation::Start { ref session, .. } = operation else {
                unreachable!()
            };
            let mut record = session.endpoint.as_os_str().to_os_string();
            record.push(".record");
            self.operate(operation)
                .map_err(|error| (error, PathBuf::from(record)))
        });
        self.workspace = match (&result, failed) {
            (Err((_, record)), Some(mut failed))
                if fs::symlink_metadata(record)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                failed.retain_requests_from(&candidate);
                failed
            }
            _ => candidate,
        };
        self.reconcile();
        self.save().map_err(|error| failure("layout-save", error))?;
        result.map_err(|(error, _)| failure("workspace-operation", error))
    }
    fn apply_session_end(&mut self, id: &str) -> Result<(), Failure> {
        let mut candidate = self.workspace.clone();
        let mut operations = Vec::new();
        candidate.session_ended(id, SessionEnd::Exited, |operation| {
            operations.push(operation);
            Ok(())
        })?;
        let mut failed = self.workspace.clone();
        let _ = failed.session_ended(id, SessionEnd::Exited, reject_start);
        self.commit(candidate, operations, Some(failed))
    }
    fn reap(&mut self) -> Result<(), String> {
        let mut changed = false;
        if Instant::now() >= self.next_recovery {
            let pending = pending_claims(&self.workspace);
            if !pending.is_empty() {
                let runtime = self.layout_runtime();
                let mut recovered = acquire_named_sessions(
                    &runtime,
                    LaunchMode::Workspace,
                    &self.info.id,
                    &self.sessions,
                    &pending,
                    Instant::now() + SESSION_START_TIMEOUT,
                )?;
                changed = !recovered.is_empty();
                for session in &mut recovered {
                    if self.workspace.transient_sessions().contains(&session.id)
                        && self.workspace.sessions().any(|slot| {
                            !slot.live && slot.id == session.id && slot.run == session.run_id()
                        })
                    {
                        stop_managed_sessions(
                            std::slice::from_mut(session),
                            SESSION_START_TIMEOUT,
                        )?;
                    }
                    if !self
                        .workspace
                        .sessions()
                        .any(|slot| slot.id == session.id && slot.run == session.run_id())
                    {
                        eprintln!(
                            "Eon workspace {}: preserving unclaimed terminal {} run {}",
                            self.info.id,
                            session.id,
                            session.run_id()
                        );
                    }
                }
                recovered.retain(|session| !session.is_finished());
                self.sessions.extend(recovered);
            }
            self.next_recovery = Instant::now() + Duration::from_secs(1);
        }
        if self.stopping {
            return Ok(());
        }
        let mut index = 0;
        while index < self.sessions.len() {
            if session_finished(&mut self.sessions[index])?.is_some() {
                let session = self.sessions.remove(index);
                changed = true;
                if self
                    .workspace
                    .sessions()
                    .any(|slot| slot.id == session.id && slot.run == session.run_id())
                    && let Err(error) = self.apply_session_end(&session.id)
                {
                    if error.code != "workspace-operation" {
                        return Err(error.detail);
                    }
                    eprintln!("Eon workspace {}: {}", self.info.id, error.detail);
                }
            } else {
                index += 1;
            }
        }
        if changed {
            self.reconcile();
            self.save()?;
        }
        Ok(())
    }
    fn layout_runtime(&self) -> PathBuf {
        Path::new(std::ffi::OsStr::from_bytes(&self.info.control_endpoint))
            .parent()
            .expect("workspace control endpoint has a parent")
            .to_path_buf()
    }
    fn accept_presentation(&mut self, client: PendingRequest) {
        let Action::Presentation { workspace } = &client.request.action else {
            unreachable!()
        };
        let error = if workspace != &self.info.id {
            Some(failure("workspace-mismatch", "workspace identity differs"))
        } else if self.stopping {
            Some(failure(
                "stop-incomplete",
                "workspace cleanup is incomplete; retry Stop",
            ))
        } else if self.presentations.len() >= MAX_PRESENTATIONS {
            Some(failure(
                "presentation-capacity",
                "too many workspace presentation channels",
            ))
        } else {
            None
        };
        if let Some(error) = error {
            client.reply(Response::Failure(error));
            return;
        }
        let mut info = self.info.clone();
        info.stopped = self.workspace.stopped;
        match client.reply_and_retain(Response::Hello(info)) {
            Ok(stream) => self.presentations.push(stream),
            Err(error) => eprintln!(
                "Eon workspace {}: presentation admission failed: {error}",
                self.info.id
            ),
        }
    }
    fn poll_presentations(&mut self) -> Result<(), String> {
        let had_presentation = !self.presentations.is_empty();
        self.presentations.retain_mut(|stream| {
            matches!(stream.read(&mut [0]), Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted))
        });
        if had_presentation && self.presentations.is_empty() {
            for id in self.workspace.transient_sessions() {
                if !self.retiring.contains(&id) {
                    self.retiring.push(id);
                }
            }
            self.next_retirement = Instant::now();
        }
        if !self.stopping && !self.retiring.is_empty() && Instant::now() >= self.next_retirement {
            self.next_retirement = Instant::now() + Duration::from_secs(1);
            for id in std::mem::take(&mut self.retiring) {
                if !self.workspace.sessions().any(|session| session.id == id) {
                    continue;
                }
                if let Err(error) = self.operate(SessionOperation::Stop(id.clone())) {
                    eprintln!(
                        "Eon workspace {}: cannot retire detached picker: {error}",
                        self.info.id
                    );
                    self.retiring.push(id);
                    continue;
                }
                if let Err(error) = self.apply_session_end(&id) {
                    if error.code != "workspace-operation" {
                        return Err(error.detail);
                    }
                    eprintln!("Eon workspace {}: {}", self.info.id, error.detail);
                }
            }
        }
        Ok(())
    }
    fn dispatch(&mut self, request: Request) -> (Response, bool) {
        match self.action(request) {
            Ok(value) => value,
            Err(error) => (Response::Failure(error), false),
        }
    }
    fn action(&mut self, request: Request) -> Result<(Response, bool), Failure> {
        match request.action {
            Action::Hello { workspace } => {
                if workspace != self.info.id {
                    return Err(failure("workspace-mismatch", "workspace identity differs"));
                }
                let mut info = self.info.clone();
                info.stopped = self.workspace.stopped;
                return Ok((Response::Hello(info), false));
            }
            Action::Registry(RegistryAction::Stop { workspace }) if workspace == self.info.id => {
                self.stopping = true;
                self.next_recovery = Instant::now();
                self.reap()
                    .map_err(|error| failure("recovery-failed", error))?;
                self.require_resolved_launches(None)
                    .map_err(|error| failure("stop-failed", error))?;
                if let Err(error) = stop_managed_sessions(&mut self.sessions, SESSION_START_TIMEOUT)
                {
                    self.reconcile();
                    self.save().map_err(|error| failure("layout-save", error))?;
                    return Err(failure("stop-failed", error));
                }
                self.sessions.clear();
                self.reconcile();
                let mut stopped = self.workspace.clone();
                stopped.stopped = true;
                stopped
                    .save_layout(&self.layout, &self.info.id)
                    .map_err(|error| failure("layout-save", error))?;
                self.workspace = stopped;
                return Ok((self.snapshot(), true));
            }
            Action::Workspace(WorkspaceAction::Inspect) if !self.stopping => {
                return Ok((self.snapshot(), false));
            }
            _ => {}
        }
        if self.stopping {
            return Err(failure(
                "stop-incomplete",
                "workspace cleanup is incomplete; retry Stop",
            ));
        }
        let mut candidate = self.workspace.clone();
        let mut operations = Vec::new();
        apply_workspace_request(self.inputs, &mut candidate, &request, |operation| {
            collect_operation(self.inputs, &mut operations, operation)
        })?;
        let failed = operations
            .iter()
            .any(|operation| matches!(operation, SessionOperation::Start { .. }))
            .then(|| {
                let mut failed = self.workspace.clone();
                let _ = apply_workspace_request(self.inputs, &mut failed, &request, reject_start);
                failed
            });
        self.commit(candidate, operations, failed)?;
        Ok((self.snapshot(), false))
    }
}

pub(crate) fn serve(inputs: &Inputs, id: &str, scoped: bool, welcome: bool) -> Result<i32, String> {
    inputs.component_report()?;
    let registry_lock = lock_supervisor_lifecycle(&state_root()?.join("registry.lock"))?;
    let registration = registration(id)?;
    let runtime = prepare_named_runtime(id)?;
    let _lock = try_lock_supervisor_lifecycle(&runtime.join("supervisor.lock"))?
        .ok_or("workspace supervisor is already running")?;
    drop(registry_lock);
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let catalog = managed_environment::popup_catalog(&config, &inputs.defaults)?;
    let layout = state_directory(id)?.join("layout.json");
    let startup_animation = if welcome && !layout.try_exists().map_err(|error| error.to_string())? {
        managed_environment::startup_animation(&config, &inputs.defaults)?
    } else {
        None
    };
    let mut operations = Vec::new();
    let mut workspace = if layout.try_exists().map_err(|error| error.to_string())? {
        Workspace::load_layout(&layout, id, runtime.clone(), env::home_dir(), catalog)?
    } else {
        // Registration can precede the first launch; an absent layout beside records is corruption.
        if fs::read_dir(&runtime)
            .map_err(|error| error.to_string())?
            .any(|entry| {
                entry.is_ok_and(|entry| entry.file_name().as_bytes().ends_with(b".record"))
            })
        {
            return Err(
                "workspace layout is missing beside terminal records; tasks were left running"
                    .into(),
            );
        }
        Workspace::pending(
            runtime.clone(),
            PathBuf::from(std::ffi::OsStr::from_bytes(&registration.directory)),
            env::home_dir(),
            catalog,
            |command, directory| {
                managed_environment::prepare_popup_command(
                    command,
                    inputs.managed.shell_bin.as_deref(),
                    directory,
                    &inputs.defaults,
                )
            },
            |operation| collect_operation(inputs, &mut operations, operation),
        )?
    };
    // Validate the entire persisted structure before acquiring any management lease or stopping a picker.
    let pending = pending_claims(&workspace);
    let mut sessions = acquire_named_sessions(
        &runtime,
        LaunchMode::Workspace,
        id,
        &[],
        &pending,
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    if operations.is_empty() {
        let transient = workspace.transient_sessions();
        for session in &mut sessions {
            if transient.contains(&session.id)
                && workspace
                    .sessions()
                    .any(|slot| slot.id == session.id && slot.run == session.run_id())
            {
                stop_managed_sessions(std::slice::from_mut(session), SESSION_START_TIMEOUT)?;
            }
        }
        sessions.retain(|session| !session.is_finished());
        workspace.retire_transient_layout();
        for session in &sessions {
            if !workspace
                .sessions()
                .any(|slot| slot.id == session.id && slot.run == session.run_id())
            {
                eprintln!(
                    "Eon workspace {id}: preserving unclaimed terminal {} run {}",
                    session.id,
                    session.run_id()
                );
            }
        }
    }
    let initial_picker = operations.iter().find_map(|operation| match operation {
        SessionOperation::Start {
            session,
            command: LaunchCommand::Project { .. },
            ..
        } => Some(session.clone()),
        _ => None,
    });
    workspace.stopped = false;
    let mut state = Named {
        inputs,
        config,
        info: info(inputs, id)?,
        layout,
        workspace,
        sessions,
        scoped,
        stopping: false,
        presentations: Vec::new(),
        retiring: Vec::new(),
        next_retirement: Instant::now(),
        startup_animation,
        quotas: quota::Quotas::start(&inputs.codexbar),
        next_recovery: Instant::now() + Duration::from_secs(1),
    };
    if let Err(error) = state.commit(state.workspace.clone(), operations, None) {
        let Some(picker) = initial_picker.filter(|picker| {
            let mut record = picker.endpoint.as_os_str().to_os_string();
            record.push(".record");
            error.code == "workspace-operation"
                && fs::symlink_metadata(PathBuf::from(record))
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        }) else {
            return Err(error.detail);
        };
        eprintln!(
            "Eon workspace {id}: {}; starting its first pane",
            error.detail
        );
        state
            .apply_session_end(&picker.id)
            .map_err(|error| error.detail)?;
    }
    let listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    loop {
        state.reap()?;
        state.poll_presentations()?;
        if let Some(client) = listener.accept_request()? {
            if matches!(client.request.action, Action::Presentation { .. }) {
                state.accept_presentation(client);
            } else {
                let (response, stop) = state.dispatch(client.request.clone());
                client.reply(response);
                if stop {
                    return Ok(0);
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn pending_claims(workspace: &Workspace) -> Vec<PathBuf> {
    workspace
        .sessions()
        .filter(|slot| !slot.live)
        .map(|slot| {
            let mut path = slot.endpoint.as_os_str().to_os_string();
            path.push(".record");
            PathBuf::from(path)
        })
        .collect()
}
