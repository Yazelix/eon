//! Window-owned OpenSSH channels. The remote supervisor remains the layout authority.
use crate::{
    cli::Inputs,
    control::{ControlResponse, connect_control},
    supervisor::{self, named},
};
use eon_workspace_protocol::v13::{
    Action, RegistryAction, Request, Response, WorkspaceGroup, WorkspaceInfo, WorkspaceSnapshot,
    decode_response, encode_request, encode_response,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const SSH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONFIG: u64 = 8 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cancellation {
    stopped: AtomicBool,
    stream: Mutex<Option<UnixStream>>,
}
impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(stream) = self.stream.lock().unwrap().as_ref() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("connection cancelled".into())
        } else {
            Ok(())
        }
    }
    fn exchange(&self, socket: &Path, action: Action) -> Result<Response, String> {
        self.request(
            socket,
            Request {
                id: supervisor::request_id(),
                action,
            },
        )
    }
    fn request(&self, socket: &Path, request: Request) -> Result<Response, String> {
        self.request_retained(socket, request)
            .map(|(_, response)| response)
    }
    fn request_retained(
        &self,
        socket: &Path,
        request: Request,
    ) -> Result<(UnixStream, Response), String> {
        self.check()?;
        let stream = connect_control(socket).map_err(|error| error.detail)?;
        *self.stream.lock().unwrap() = Some(stream.try_clone().map_err(|error| error.to_string())?);
        // Cancellation can race between the first check and registering the connected channel.
        self.check()?;
        let result = crate::control::send_request_on(
            stream.try_clone().map_err(|error| error.to_string())?,
            request,
        )
        .map_err(|error| error.detail);
        self.stream.lock().unwrap().take();
        self.check()?;
        match result? {
            ControlResponse::Workspace(Response::Failure(error)) => Err(error.detail),
            ControlResponse::Workspace(response) => Ok((stream, response)),
            ControlResponse::Lifecycle(_) => {
                Err("workspace returned an unexpected lifecycle response".into())
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Remote {
    pub(crate) host: String,
    program: String,
    #[serde(default)]
    cache: Vec<u8>,
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && !host.chars().any(|ch| ch.is_control() || ch.is_whitespace())
}
fn config_path() -> Result<PathBuf, String> {
    let directory = supervisor::configuration_directory()?;
    supervisor::prepare_configuration(&directory)?;
    Ok(directory.join("remotes.json"))
}
pub(crate) fn remotes() -> Result<Vec<Remote>, String> {
    let path = config_path()?;
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read SSH targets: {error}")),
    };
    let meta = file.metadata().map_err(|error| error.to_string())?;
    if !meta.is_file()
        || meta.uid() != supervisor::effective_uid()
        || meta.mode() & 0o7777 != 0o600
        || meta.len() > MAX_CONFIG
    {
        return Err("SSH targets must be an owned bounded mode-0600 file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let values: Vec<Remote> =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid SSH targets: {error}"))?;
    if values.len() > eon_workspace_protocol::v13::MAX_HOSTS - 1 {
        return Err("too many SSH targets".into());
    }
    let mut hosts = std::collections::HashSet::new();
    for value in &values {
        if !valid_host(&value.host) || !hosts.insert(&value.host) {
            return Err("invalid or duplicate SSH host".into());
        }
        validate_program(&value.program)?;
        if !value.cache.is_empty() {
            cached(value)?;
        }
    }
    Ok(values)
}
fn validate_program(program: &str) -> Result<(), String> {
    if program != "eon-server" && !Path::new(program).is_absolute()
        || program.len() > 4096
        || program.chars().any(char::is_control)
    {
        return Err("server program must be an absolute path".into());
    }
    Ok(())
}
fn save_remotes(values: &[Remote]) -> Result<(), String> {
    let path = config_path()?;
    let bytes = serde_json::to_vec(values).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_CONFIG {
        return Err("SSH navigation cache exceeds its limit".into());
    }
    let temporary = path.with_file_name(format!(".remotes-{}", supervisor::request_id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    fs::File::open(path.parent().unwrap())
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}
pub(crate) fn add(host: &str, program: Option<&str>) -> Result<(), String> {
    if !valid_host(host) {
        return Err("invalid OpenSSH host alias".into());
    }
    let program = program.unwrap_or("eon-server");
    validate_program(program)?;
    let path = config_path()?;
    let _lock = supervisor::lock_supervisor_lifecycle(&path.with_file_name("remotes.lock"))?;
    let mut values = remotes()?;
    if let Some(value) = values.iter_mut().find(|value| value.host == host) {
        value.program = program.into();
    } else {
        if values.len() >= eon_workspace_protocol::v13::MAX_HOSTS - 1 {
            return Err("too many SSH targets".into());
        }
        values.push(Remote {
            host: host.into(),
            program: program.into(),
            cache: Vec::new(),
        });
    }
    save_remotes(&values)
}
fn cached(remote: &Remote) -> Result<Vec<WorkspaceInfo>, String> {
    if remote.cache.is_empty() {
        return Ok(Vec::new());
    }
    match decode_response(&remote.cache)
        .map_err(|error| format!("invalid navigation cache: {error}"))?
    {
        Response::Registry(values) => Ok(values),
        _ => Err("navigation cache has the wrong response type".into()),
    }
}
fn update_cache(host: &str, workspaces: &[WorkspaceInfo]) -> Result<(), String> {
    let path = config_path()?;
    let _lock = supervisor::lock_supervisor_lifecycle(&path.with_file_name("remotes.lock"))?;
    let mut values = remotes()?;
    if let Some(remote) = values.iter_mut().find(|value| value.host == host) {
        remote.cache = encode_response(&Response::Registry(workspaces.to_vec()))
            .map_err(|error| error.to_string())?;
        save_remotes(&values)?;
    }
    Ok(())
}
pub(crate) fn navigation(inputs: &Inputs) -> Result<Vec<WorkspaceGroup>, String> {
    let mut values = vec![WorkspaceGroup {
        host: String::new(),
        offline: false,
        workspaces: named::list(inputs)?,
    }];
    for remote in remotes()? {
        values.push(WorkspaceGroup {
            host: remote.host.clone(),
            offline: true,
            workspaces: cached(&remote)?,
        });
    }
    Ok(values)
}
fn configured(host: &str) -> Result<Remote, String> {
    remotes()?
        .into_iter()
        .find(|remote| remote.host == host)
        .ok_or_else(|| format!("SSH target {host} is not configured; run eon remote add first"))
}
fn bootstrap_command(program: &str) -> String {
    // The login shell sees fixed syntax only (including when it is Nushell).
    // POSIX printf decodes the opaque executable path; stdin stays with EONW.
    let encoded = program
        .bytes()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect::<String>();
    format!("sh -c 'exec \"$(printf \"%b\" \"{encoded}\")\" bootstrap'")
}

struct Master {
    remote: Remote,
    root: PathBuf,
    control: PathBuf,
    child: Child,
}
impl Master {
    fn start(remote: Remote, root: &Path, cancel: &Cancellation) -> Result<Self, String> {
        cancel.check()?;
        supervisor::prepare_runtime(root)?;
        let control = root.join("ssh");
        if control.as_os_str().as_bytes().len() >= 108 {
            return Err("SSH control path is too long".into());
        }
        let stderr = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("ssh.log"))
            .map_err(|error| error.to_string())?;
        let child = Command::new("ssh")
            .args(["-M", "-N", "-T", "-S"])
            .arg(&control)
            .args([
                "-o",
                "ControlPersist=no",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=no",
                "-o",
                "StreamLocalBindMask=0177",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
            ])
            .arg(&remote.host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|error| format!("cannot launch OpenSSH: {error}"))?;
        let mut master = Self {
            remote,
            root: root.into(),
            control,
            child,
        };
        let deadline = Instant::now() + SSH_TIMEOUT;
        loop {
            cancel.check()?;
            if let Some(status) = master.child.try_wait().map_err(|error| error.to_string())? {
                let mut detail = String::new();
                fs::File::open(root.join("ssh.log"))
                    .map_err(|error| error.to_string())?
                    .take(8192)
                    .read_to_string(&mut detail)
                    .map_err(|error| error.to_string())?;
                return Err(format!("OpenSSH exited {status}: {}", detail.trim()));
            }
            if master.control.exists() {
                return Ok(master);
            }
            if Instant::now() >= deadline {
                return Err("OpenSSH connection timed out".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new("ssh");
        command.arg("-S").arg(&self.control).args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ControlMaster=no",
            "-o",
            "ProxyCommand=false",
            "-o",
            "ConnectTimeout=10",
        ]);
        command
    }
    fn bootstrap(&self, action: RegistryAction, cancel: &Cancellation) -> Result<Response, String> {
        let request = encode_request(&Request {
            id: supervisor::request_id(),
            action: Action::Registry(action),
        })
        .map_err(|error| error.to_string())?;
        let mut command = self.command();
        command
            .arg(&self.remote.host)
            .arg(bootstrap_command(&self.remote.program));
        let bytes = run_command(command, &request, 2 * 1024 * 1024, cancel)?;
        let response = decode_response(&bytes)
            .map_err(|error| format!("invalid remote bootstrap response: {error}"))?;
        if let Response::Failure(error) = response {
            return Err(error.detail);
        }
        Ok(response)
    }
    fn forward(
        &self,
        local: &Path,
        remote: &[u8],
        cancel: &Cancellation,
        remove: bool,
    ) -> Result<(), String> {
        let remote =
            std::str::from_utf8(remote).map_err(|_| "SSH socket endpoints must be UTF-8")?;
        if remote.contains(':')
            || remote.chars().any(char::is_control)
            || !Path::new(remote).is_absolute()
            || local.as_os_str().as_bytes().contains(&b':')
        {
            return Err("unsupported SSH Unix socket path".into());
        }
        let mut command = self.command();
        command
            .args([
                "-O",
                if remove { "cancel" } else { "forward" },
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=no",
                "-o",
                "StreamLocalBindMask=0177",
                "-L",
            ])
            .arg(format!("{}:{remote}", local.display()))
            .arg(&self.remote.host);
        run_command(command, &[], 8192, cancel).map(|_| ())
    }
}
impl Drop for Master {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // This directory was reserved for this connection and contains only its sockets and log.
        let _ = fs::remove_file(&self.control);
    }
}

fn run_command(
    mut command: Command,
    input: &[u8],
    limit: usize,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    cancel.check()?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(8193).read_to_end(&mut bytes).map(|_| bytes)
    });
    let write = child.stdin.take().unwrap().write_all(input);
    let deadline = Instant::now() + SSH_TIMEOUT;
    let status = loop {
        if write.is_err() || cancel.cancelled() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err(if cancel.cancelled() {
                "connection cancelled".into()
            } else {
                "OpenSSH command failed or timed out".into()
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error.to_string());
            }
        }
    };
    let bytes = out
        .join()
        .map_err(|_| "SSH output reader failed")?
        .map_err(|error| error.to_string())?;
    let detail = err
        .join()
        .map_err(|_| "SSH error reader failed")?
        .map_err(|error| error.to_string())?;
    let status = status?;
    if !status.success() {
        return Err(format!(
            "OpenSSH exited {status}: {}",
            String::from_utf8_lossy(&detail).trim()
        ));
    }
    if bytes.len() > limit {
        return Err("OpenSSH response exceeds its size limit".into());
    }
    Ok(bytes)
}

pub(crate) fn registry(
    host: &str,
    action: RegistryAction,
    cancel: &Cancellation,
) -> Result<Response, String> {
    let remote = configured(host)?;
    let parent = supervisor::base_runtime_directory("eon").join("r");
    supervisor::prepare_runtime(&parent)?;
    let root = parent.join(format!("{:x}", std::process::id()));
    // A registry operation is serialized per CLI process, with a fresh private directory.
    let root = root.with_file_name(format!("r{}", supervisor::request_id()));
    fs::DirBuilder::new()
        .recursive(false)
        .create(&root)
        .map_err(|error| error.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let master = Master::start(remote, &root, cancel)?;
        let response = master.bootstrap(action, cancel)?;
        if let Response::Registry(values) = &response
            && let Err(error) = update_cache(host, values)
        {
            eprintln!("Eon workspace navigation cache: {error}");
        }
        Ok(response)
    })();
    let _ = fs::remove_dir_all(root);
    result
}

pub(crate) struct Connection {
    pub(crate) info: WorkspaceInfo,
    socket: PathBuf,
    master: Option<Master>,
    _presentation: UnixStream,
    forwarded: HashMap<(String, String), (PathBuf, Vec<u8>)>,
    next_endpoint: usize,
}
impl Connection {
    pub(crate) fn open(
        inputs: &Inputs,
        host: &str,
        workspace: &str,
        root: &Path,
        cancel: &Cancellation,
    ) -> Result<Self, String> {
        let (info, master) = if host.is_empty() {
            (named::ensure(inputs, workspace, false)?, None)
        } else {
            let master = Master::start(configured(host)?, root, cancel)?;
            let response = master.bootstrap(
                RegistryAction::Ensure {
                    workspace: workspace.into(),
                },
                cancel,
            )?;
            let Response::Hello(info) = response else {
                return Err("remote did not return a workspace handshake".into());
            };
            (info, Some(master))
        };
        if info.id != workspace {
            return Err("remote handshake selected a different workspace".into());
        }
        let socket = if let Some(master) = &master {
            let local = root.join("eon.sock");
            master.forward(&local, &info.control_endpoint, cancel, false)?;
            local
        } else {
            PathBuf::from(OsStr::from_bytes(&info.control_endpoint))
        };
        let Response::Hello(accepted) = cancel.exchange(
            &socket,
            Action::Hello {
                workspace: workspace.into(),
            },
        )?
        else {
            return Err("workspace handshake is missing".into());
        };
        let mut identity = info.clone();
        identity.stopped = accepted.stopped;
        if accepted != identity {
            return Err("workspace identity changed during connection".into());
        }
        let (presentation, response) = cancel.request_retained(
            &socket,
            Request {
                id: supervisor::request_id(),
                action: Action::Presentation {
                    workspace: workspace.into(),
                },
            },
        )?;
        if response != Response::Hello(accepted.clone()) {
            return Err("presentation handshake changed workspace identity".into());
        }
        Ok(Self {
            info: accepted,
            socket,
            master,
            _presentation: presentation,
            forwarded: HashMap::new(),
            next_endpoint: 0,
        })
    }
    pub(crate) fn stopped(&self, inputs: &Inputs, cancel: &Cancellation) -> Result<bool, String> {
        if let Some(master) = &self.master {
            let Response::Registry(workspaces) = master.bootstrap(RegistryAction::List, cancel)?
            else {
                return Err("remote registry returned an invalid response".into());
            };
            Ok(workspaces
                .iter()
                .find(|info| info.id == self.info.id)
                .is_some_and(|info| info.stopped))
        } else {
            Ok(named::info(inputs, &self.info.id)?.stopped)
        }
    }

    pub(crate) fn exchange(
        &mut self,
        action: Action,
        cancel: &Cancellation,
    ) -> Result<WorkspaceSnapshot, String> {
        self.request(
            Request {
                id: supervisor::request_id(),
                action,
            },
            cancel,
        )
    }
    pub(crate) fn request(
        &mut self,
        request: Request,
        cancel: &Cancellation,
    ) -> Result<WorkspaceSnapshot, String> {
        let response = cancel.request(&self.socket, request)?;
        let Response::Workspace(mut value) = response else {
            return Err("workspace returned an invalid structure response".into());
        };
        if value.workspace != self.info.id {
            return Err("workspace changed identity".into());
        }
        let remote_root = Path::new(OsStr::from_bytes(&self.info.control_endpoint))
            .parent()
            .ok_or("workspace has no endpoint root")?;
        let mut live = std::collections::HashSet::new();
        for tab in &mut value.snapshot.tabs {
            for (session, endpoint) in tab
                .columns
                .iter_mut()
                .flat_map(|column| &mut column.panes)
                .map(|pane| (&pane.session, &mut pane.endpoint))
                .chain(
                    tab.popups
                        .iter_mut()
                        .map(|popup| (&popup.session, &mut popup.endpoint)),
                )
            {
                let binding = value
                    .runs
                    .iter()
                    .find(|run| &run.session == session)
                    .ok_or("missing terminal run binding")?;
                let path = Path::new(OsStr::from_bytes(endpoint));
                if path.parent() != Some(remote_root) || path.as_os_str().as_bytes().len() >= 108 {
                    return Err("terminal endpoint is outside its workspace".into());
                }
                let key = (session.clone(), binding.run.clone());
                if let Some(master) = &self.master {
                    if binding.live {
                        live.insert(key.clone());
                        if !self.forwarded.contains_key(&key) {
                            let local = master.root.join(format!("t{}.sock", self.next_endpoint));
                            self.next_endpoint += 1;
                            master.forward(&local, endpoint, cancel, false)?;
                            self.forwarded
                                .insert(key.clone(), (local, endpoint.clone()));
                        }
                        let (local, previous) = &self.forwarded[&key];
                        if previous != endpoint {
                            return Err("terminal endpoint changed within a run".into());
                        }
                        *endpoint = local.as_os_str().as_bytes().to_vec();
                    } else {
                        *endpoint = master
                            .root
                            .join(format!("stopped-{session}.sock"))
                            .as_os_str()
                            .as_bytes()
                            .to_vec();
                    }
                }
            }
        }
        if let Some(master) = &self.master {
            let expired: Vec<_> = self
                .forwarded
                .keys()
                .filter(|key| !live.contains(*key))
                .cloned()
                .collect();
            for key in expired {
                let (local, remote) = self.forwarded.remove(&key).unwrap();
                master.forward(&local, &remote, cancel, true)?;
                let _ = fs::remove_file(local);
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bootstrap_path_is_opaque_and_stdin_reaches_the_server() {
        let root = std::env::temp_dir().join(format!("eon-bootstrap-{}", supervisor::request_id()));
        fs::create_dir(&root).unwrap();
        let program = root.join("a b'c\"d$e`f\\g");
        fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$1\"\ncat\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new("sh")
            .args(["-c", &bootstrap_command(program.to_str().unwrap())])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"bounded EONW request")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"bootstrap\nbounded EONW request");
        fs::remove_dir_all(root).unwrap();
    }
}
