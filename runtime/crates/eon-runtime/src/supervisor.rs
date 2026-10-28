use super::{
    attention::{self, Attention},
    cli::Inputs,
    control::{
        ControlListener, ControlResponse, EndpointFailure, EndpointFailureKind, SocketIdentity,
        connect_control, failure, probe_launch_mode, probe_presentable_runtime, send_action_on,
        socket_identity,
    },
    diagnostics::DiagnosticLog,
    generation::{current_generation, generation_directory, record_launcher, remove_launcher},
    managed_environment::{self, nonempty_environment_path},
    notification, platform, quota,
    sessions::{
        RunningSession, TAB_VARIABLE, TERMINAL_VARIABLE, acquire_generation_sessions,
        recover_sessions, session_finished, start_orbit, stop_managed_sessions,
    },
    workspace::{self, LaunchCommand, SessionEnd, SessionOperation, WorkspaceSet},
};
use eon_workspace_protocol::v23::{
    Action, Availability, Ended, Failure, LifecycleResponse, Request, Response, Runtime, Snapshot,
    Stopped, VERSION, WorkspaceAction,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::fd::OwnedFd,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const SESSION_START_TIMEOUT: Duration = Duration::from_secs(5);
/// Bounds Venus startup admission, which a busy machine can slow well past session start.
pub(super) const PRESENTATION_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
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
    inputs: &Inputs,
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
        validate_runtime(inputs, &info, generation)
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LaunchMode {
    Workspace,
    Terminal,
}

impl LaunchMode {
    fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Terminal => "eonbase",
        }
    }
}

pub(super) fn launch_current(
    inputs: &Inputs,
    mode: LaunchMode,
    child: &[OsString],
    attach_existing: bool,
    decorations: bool,
    application_id: &str,
) -> Result<i32, String> {
    let generation = current_generation(inputs)?;
    let root = runtime_directory(if mode == LaunchMode::Terminal {
        "eonbase"
    } else {
        "eon"
    });
    prepare_runtime(&root)?;
    let lifecycle_lock_path = supervisor_lock_path(&root, &generation);
    let runtime = generation_directory(&root, &generation);
    let socket = runtime.join("eon.sock");
    let mut missing_deadline = None;
    let lifecycle_lock = loop {
        let endpoint_failure = match probe_supervisor(inputs, &socket, &generation) {
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
                let presentation_error =
                    match present_at(inputs, &runtime, &generation, mode, supervisor) {
                        Ok(code) => return Ok(code),
                        Err(error) => error,
                    };
                match probe_supervisor(inputs, &socket, &generation) {
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
        match probe_supervisor(inputs, &socket, &generation) {
            Err(error) if error.kind == EndpointFailureKind::Dead => break lifecycle_lock,
            Ok(_) => {
                drop(lifecycle_lock);
                continue;
            }
            Err(error) => return Err(error.detail),
        }
    };
    let log = DiagnosticLog::open(&generation, mode.name());
    let result = (|| {
        let config = configuration_directory()?;
        let terminal =
            managed_environment::terminal_presentation(&config, &inputs.defaults, Some(&log))?;
        let bar_widgets = window_bar_widgets(mode, &config, inputs, Some(&log))?;
        let popups = (mode == LaunchMode::Workspace)
            .then(|| managed_environment::popup_catalog(&config, &inputs.defaults, Some(&log)))
            .transpose()?;
        let startup_animation = (mode == LaunchMode::Workspace)
            .then(|| managed_environment::startup_animation(&config, &inputs.defaults, Some(&log)))
            .transpose()?
            .flatten();
        prepare_generation_runtime(&root, &generation)?;
        if mode == LaunchMode::Workspace
            && let Err(error) = record_launcher(inputs, &runtime)
        {
            log.record(&error);
        }
        prepare_configuration(&config)?;
        supervise(
            inputs,
            &config,
            terminal,
            &bar_widgets,
            popups,
            startup_animation,
            lifecycle_lock,
            &runtime,
            child,
            &generation,
            mode,
            application_id,
            decorations,
            &log,
        )
    })();
    log.record(&match &result {
        Ok(code) => format!("supervisor exited with status {code}"),
        Err(error) => format!("supervisor failed: {error}"),
    });
    result.or_else(|error| {
        if attach_existing && fs::symlink_metadata(&socket).is_ok() {
            attach_competing_supervisor(inputs, &socket, &runtime, &generation, mode, &error)
        } else {
            Err(error)
        }
    })
}

fn attach_competing_supervisor(
    inputs: &Inputs,
    socket: &Path,
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    launch_error: &str,
) -> Result<i32, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match probe_supervisor(inputs, socket, generation) {
            Ok((active_mode, supervisor)) => {
                if active_mode != mode {
                    return Err(format!(
                        "{launch_error}; competing supervisor started in {} mode, requested {} mode",
                        active_mode.name(),
                        mode.name()
                    ));
                }
                return present_at(inputs, runtime, generation, mode, supervisor);
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

fn validate_runtime(inputs: &Inputs, info: &Runtime, generation: &str) -> Result<(), String> {
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
    let components = inputs.component_report()?;
    if info.component_report != components {
        return Err("supervisor reports a different component graph".into());
    }
    Ok(())
}

pub(super) fn present_at(
    inputs: &Inputs,
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
        Action::Workspace(WorkspaceAction::Present {
            workspace: mode == LaunchMode::Workspace,
        }),
    )
    .map_err(|error| error.detail)?
    {
        ControlResponse::Lifecycle(LifecycleResponse::Runtime(info)) => {
            validate_runtime(inputs, &info, generation)?;
            Ok(0)
        }
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure)) => {
            Err(format!("cannot present Eon Desktop: {}", failure.detail))
        }
        _ => Err("supervisor returned the wrong EONW result for presentation".into()),
    }
}

/// The `[bar] widgets` list for one window open, which both its bar and the quota readers follow;
/// a terminal window has no bar.
fn window_bar_widgets(
    mode: LaunchMode,
    config: &Path,
    inputs: &Inputs,
    log: Option<&DiagnosticLog>,
) -> Result<Vec<&'static str>, String> {
    match mode {
        LaunchMode::Workspace => managed_environment::bar_widgets(config, &inputs.defaults, log),
        LaunchMode::Terminal => Ok(Vec::new()),
    }
}

#[allow(clippy::too_many_arguments)]
fn venus_command(
    inputs: &Inputs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: &managed_environment::TerminalConfig,
    bar_widgets: &[&str],
    application_id: &str,
    decorations: bool,
    warning_close_last_tab: bool,
    animate_working_badge: bool,
) -> Command {
    let mut command = Command::new(&inputs.venus);
    command
        .env_remove(TERMINAL_VARIABLE)
        .env_remove(TAB_VARIABLE);
    if !decorations {
        command.arg("--no-decorations");
    }
    command
        .arg("--application-id")
        .arg(application_id)
        .arg("--background-opacity")
        .arg(terminal.background_opacity.to_string())
        .arg("--padding-color")
        .arg(terminal.padding_color.as_str());
    if terminal.background_blur {
        command.arg("--background-blur");
    }
    if let Some(color) = &terminal.cursor_trail_color {
        command.arg("--cursor-trail-color").arg(color);
    }
    if let Some(family) = &terminal.font_family {
        command.arg("--font-family").arg(family);
    }
    for family in &terminal.font_fallbacks {
        command.arg("--font-fallback").arg(family);
    }
    for (flag, value) in [
        (
            "--cursor-trail-duration-v1",
            terminal
                .cursor_trail_duration
                .map(|value| value.to_string()),
        ),
        (
            "--font-size",
            terminal.font_size.map(|value| value.to_string()),
        ),
        (
            "--line-height",
            terminal.line_height.map(|value| value.to_string()),
        ),
        ("--columns", terminal.columns.map(|value| value.to_string())),
        ("--rows", terminal.rows.map(|value| value.to_string())),
    ] {
        if let Some(value) = value {
            command.arg(flag).arg(value);
        }
    }
    if mode == LaunchMode::Workspace {
        if let Some(color) = &terminal.focus_color {
            command.arg("--focus-color").arg(color);
        }
        for (flag, width) in [
            ("--border-width", terminal.border_width),
            ("--focus-border-width", terminal.focus_border_width),
        ] {
            if let Some(width) = width {
                command.arg(flag).arg(width.to_string());
            }
        }
        if bar_widgets.contains(&"version") {
            command.arg("--eon-version").arg(inputs.version);
        }
        command.arg("--bar-items").arg(bar_widgets.join(","));
        if let Ok(executable) = env::current_exe() {
            command.env("EON_NEW_WINDOW_EXECUTABLE", executable);
        }
        command
            .arg("--pane-frames")
            .arg(terminal.pane_frames.to_string())
            .arg("--warning-close-last-tab")
            .arg(warning_close_last_tab.to_string())
            .arg("--animate-working-badge")
            .arg(animate_working_badge.to_string())
            .arg("--workspace");
    }
    command.arg(socket);
    command.env("EON_CONFIG_HOME", config);
    command
}

struct PresentationProcess {
    child: Child,
    control: UnixStream,
    log: Arc<DiagnosticLog>,
    relay: Option<JoinHandle<()>>,
    reported: bool,
}

impl PresentationProcess {
    fn start(
        mut command: Command,
        admission: bool,
        snapshot: Option<Snapshot>,
        timeout: Duration,
        log: &Arc<DiagnosticLog>,
    ) -> Result<Self, String> {
        let deadline = Instant::now() + timeout;
        let (control, input) = UnixStream::pair()
            .map_err(|error| format!("cannot create Eon Desktop presentation control: {error}"))?;
        if admission {
            command.stdout(Stdio::from(OwnedFd::from(input.try_clone().map_err(
                |error| format!("cannot create Eon Desktop readiness channel: {error}"),
            )?)));
        }
        command
            .env(
                "EON_VENUS_PRESENTATION_CONTROL",
                if admission { "stdin-ready-v1" } else { "stdin" },
            )
            .stdin(Stdio::from(OwnedFd::from(input)))
            .stderr(Stdio::piped());
        if env::var_os("RUST_BACKTRACE").is_none() {
            command.env("RUST_BACKTRACE", "1");
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?;
        // Command retains its Stdio handles; release them so child exit yields EOF.
        drop(command);
        log.record(&format!("Eon Desktop started: pid {}", child.id()));
        let stderr = child.stderr.take().expect("Eon Desktop stderr is piped");
        let mut process = Self {
            child,
            control,
            log: Arc::clone(log),
            relay: None,
            reported: false,
        };
        process.relay = Some(
            log.relay(stderr)
                .map_err(|error| format!("cannot relay Eon Desktop diagnostics: {error}"))?,
        );
        if admission {
            let not_ready = || format!("Venus was not ready within {timeout:?}");
            let result = (|| {
                let time_left = || {
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|duration| !duration.is_zero())
                        .ok_or_else(not_ready)
                };
                if let Some(snapshot) = snapshot {
                    let bytes =
                        eon_workspace_protocol::v23::encode_response(&Response::Snapshot(snapshot))
                            .map_err(|error| format!("cannot encode startup workspace: {error}"))?;
                    let mut remaining = bytes.as_slice();
                    while !remaining.is_empty() {
                        process
                            .control
                            .set_write_timeout(Some(time_left()?))
                            .map_err(|error| error.to_string())?;
                        match process.control.write(remaining) {
                            Ok(0) => {
                                return Err("Venus closed while receiving startup workspace".into());
                            }
                            Ok(count) => remaining = &remaining[count..],
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                return Err(not_ready());
                            }
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                }
                let mut ready = [0; 8];
                let mut remaining = &mut ready[..];
                while !remaining.is_empty() {
                    process
                        .control
                        .set_read_timeout(Some(time_left()?))
                        .map_err(|error| error.to_string())?;
                    match process.control.read(remaining) {
                        Ok(0) => return Err("Venus closed before native startup admission; see its diagnostic in the supervisor output".into()),
                        Ok(count) => remaining = &mut remaining[count..],
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            return Err(not_ready());
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                }
                if ready != *b"ready-v1" {
                    return Err("invalid Venus startup readiness response".into());
                }
                Ok(())
            })();
            if let Err(error) = result {
                stop(&mut process.child);
                return Err(format!("cannot admit Eon Desktop presentation: {error}"));
            }
        }
        process
            .control
            .set_write_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| format!("cannot bound Eon Desktop presentation control: {error}"))?;
        Ok(process)
    }

    /// Reports how Eon Desktop exited, if it has, recording it once.
    fn exited(&mut self) -> Result<Option<ExitStatus>, String> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?;
        if let Some(status) = status
            && !self.reported
        {
            self.reported = true;
            // Let the relay copy Eon Desktop's last output ahead of its exit line.
            if let Some(relay) = self.relay.take() {
                let deadline = Instant::now() + Duration::from_secs(1);
                while !relay.is_finished() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
            }
            self.log.record(&format!("Eon Desktop exited: {status}"));
        }
        Ok(status)
    }

    fn present(&self) -> Result<(), String> {
        (&self.control)
            .write_all(b"present\n")
            .map_err(|error| format!("cannot signal Eon Desktop presentation: {error}"))
    }
}

impl Drop for PresentationProcess {
    fn drop(&mut self) {
        let _ = self.control.shutdown(Shutdown::Both);
        let deadline = Instant::now() + SESSION_START_TIMEOUT;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        stop(&mut self.child);
        let _ = self.exited();
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
    if product == "eon"
        && let Some(root) = nonempty_environment_path("EON_WINDOW_RUNTIME_DIR")
    {
        return root;
    }
    base_runtime_directory(product)
}

pub(super) fn base_runtime_directory(product: &str) -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join(product))
        })
        .unwrap_or_else(|| platform::fallback_runtime_directory(product, effective_uid()))
}

fn prepare_generation_runtime(root: &Path, generation: &str) -> Result<PathBuf, String> {
    prepare_runtime(root)?;
    prepare_runtime(&root.join("generations"))?;
    let runtime = generation_directory(root, generation);
    prepare_runtime(&runtime)?;
    Ok(runtime)
}

fn xdg_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| path.is_absolute())
}

pub(super) fn prepare_runtime(path: &Path) -> Result<(), String> {
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

fn recovered_workspace_sessions(
    sessions: &[RunningSession],
) -> Result<Vec<(usize, workspace::Session)>, String> {
    sessions
        .iter()
        .map(|session| {
            Ok((
                session
                    .number
                    .ok_or("recovered a transient terminal as a durable pane")?,
                workspace::Session {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                },
            ))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn supervise(
    inputs: &Inputs,
    config: &Path,
    terminal: managed_environment::TerminalConfig,
    bar_widgets: &[&str],
    popups: Option<managed_environment::PopupCatalog>,
    startup_animation: Option<managed_environment::StartupAnimation>,
    lifecycle_lock: fs::File,
    runtime: &Path,
    child: &[OsString],
    generation: &str,
    mode: LaunchMode,
    application_id: &str,
    decorations: bool,
    log: &Arc<DiagnosticLog>,
) -> Result<i32, String> {
    let _lifecycle_lock = lifecycle_lock;
    let launch_directory = env::current_dir()
        .map_err(|error| format!("cannot resolve Eon launch directory: {error}"))?;
    if mode == LaunchMode::Workspace {
        workspace::validate_initial_directory(&launch_directory)?;
    }
    let warning_close_last_tab =
        managed_environment::warning_close_last_tab(config, &inputs.defaults, Some(log))?;
    let animate_working_badge =
        managed_environment::animate_working_badge(config, &inputs.defaults, Some(log))?;
    // Full Eon only; eonbase has no poller and no sidebar (EON-C31).
    let mut attention = if mode == LaunchMode::Workspace {
        Some(Attention::new(managed_environment::sidebar_enabled(
            config,
            &inputs.defaults,
            Some(log),
        )?))
    } else {
        None
    };
    let popup_catalog = popups.unwrap_or_else(|| managed_environment::PopupCatalog {
        geometry: inputs.defaults.popups.geometry,
        entries: Vec::new(),
    });
    let (mut sessions, recovered_picker) = recover_sessions(
        runtime,
        mode,
        generation,
        log.orbit_directory().ok(),
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    let control_listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    let initial_endpoint = runtime.join("orbit.sock");
    let initial_session_id = "session-1";
    let presentation_socket = if mode == LaunchMode::Workspace {
        runtime.join("eon.sock")
    } else {
        sessions.first().map_or_else(
            || initial_endpoint.clone(),
            |session| session.endpoint.clone(),
        )
    };
    let fresh_animation = startup_animation.is_some() && sessions.is_empty() && !recovered_picker;
    let admitted = if terminal.requires_startup_admission() || fresh_animation {
        let snapshot = if mode == LaunchMode::Workspace {
            Some(
                if sessions.is_empty() && !recovered_picker {
                    WorkspaceSet::pending(
                        runtime.to_path_buf(),
                        launch_directory.clone(),
                        env::home_dir(),
                        popup_catalog.clone(),
                        |_, _| Ok(Vec::new()),
                        |_| Ok(()),
                    )?
                } else {
                    let recovered = if sessions.is_empty() {
                        vec![(
                            1,
                            workspace::Session {
                                id: initial_session_id.into(),
                                endpoint: initial_endpoint.clone(),
                            },
                        )]
                    } else {
                        recovered_workspace_sessions(&sessions)?
                    };
                    WorkspaceSet::with_recovered_sessions(
                        runtime.to_path_buf(),
                        launch_directory.clone(),
                        env::home_dir(),
                        recovered,
                        popup_catalog.clone(),
                    )?
                }
                .snapshot(),
            )
        } else {
            None
        };
        let snapshot = snapshot.map(|mut snapshot| {
            if let Some(attention) = &mut attention {
                // A starting window holds one workspace.
                let workspaces = [snapshot.clone()];
                attention.present(&mut snapshot, &workspaces, false, Instant::now());
            }
            snapshot
        });
        let command = venus_command(
            inputs,
            config,
            &presentation_socket,
            mode,
            &terminal,
            bar_widgets,
            application_id,
            decorations,
            warning_close_last_tab,
            animate_working_badge,
        );
        Some(PresentationProcess::start(
            command,
            true,
            snapshot,
            PRESENTATION_ADMISSION_TIMEOUT,
            log,
        )?)
    } else {
        None
    };
    let mut initial_child = if sessions.is_empty() {
        child.to_vec()
    } else {
        Vec::new()
    };
    if sessions.is_empty() && (mode == LaunchMode::Terminal || recovered_picker) {
        sessions.push(start_orbit(
            inputs,
            config,
            &initial_endpoint,
            initial_session_id,
            generation,
            &launch_directory,
            &initial_child,
            None,
            log,
            // Starts after admission, which may have used most of its own deadline.
            Instant::now() + SESSION_START_TIMEOUT,
        )?);
        initial_child.clear();
    }
    let workspaces = if mode == LaunchMode::Workspace {
        Some(if sessions.is_empty() {
            WorkspaceSet::pending(
                runtime.to_path_buf(),
                launch_directory.clone(),
                env::home_dir(),
                popup_catalog.clone(),
                |command, directory| {
                    managed_environment::prepare_popup_command(
                        command,
                        inputs.managed.shell_bin.as_deref(),
                        directory,
                        &inputs.defaults,
                    )
                },
                |operation| {
                    apply_session_operation(
                        inputs,
                        config,
                        log,
                        generation,
                        &mut sessions,
                        &mut initial_child,
                        operation,
                        startup_animation.as_ref(),
                    )
                },
            )?
        } else {
            WorkspaceSet::with_recovered_sessions(
                runtime.to_path_buf(),
                launch_directory,
                env::home_dir(),
                recovered_workspace_sessions(&sessions)?,
                popup_catalog,
            )?
        })
    } else {
        None
    };
    let mut state = SupervisorState {
        component_generation: generation.to_owned(),
        application_id: application_id.into(),
        decorations,
        workspaces,
        sessions,
        venus: None,
        initial_child,
        initial_status: None,
        stopping: false,
        quotas: (mode == LaunchMode::Workspace)
            .then(|| quota::Quotas::start(&inputs.codexbar, bar_widgets)),
        attention,
        pane_labels: super::pane_labels::Labels::default(),
        log: Arc::clone(log),
    };
    let presentation = match admitted {
        Some(venus) => Ok(venus),
        None => PresentationProcess::start(
            venus_command(
                inputs,
                config,
                &presentation_socket,
                mode,
                &terminal,
                bar_widgets,
                application_id,
                decorations,
                warning_close_last_tab,
                animate_working_badge,
            ),
            false,
            None,
            PRESENTATION_ADMISSION_TIMEOUT,
            log,
        ),
    };
    state.venus = match presentation {
        Ok(venus) => Some(venus),
        Err(error) => {
            let session_cleanup = stop_managed_sessions(&mut state.sessions, SESSION_START_TIMEOUT);
            drop(control_listener);
            if session_cleanup.is_ok() {
                remove_launcher(runtime);
                let _ = fs::remove_dir(runtime);
            }
            let mut errors = vec![error];
            if let Err(cleanup) = session_cleanup {
                errors.push(format!("cannot roll back terminals: {cleanup}"));
            }
            return Err(errors.join("; "));
        }
    };

    let status = (|| {
        loop {
            if !state.stopping {
                reap_finished_sessions(&mut state, inputs, config)?;
            }

            // An incomplete Stop keeps its owner for a retry, even with no
            // tracked terminal left, such as a final close whose untracked
            // runs could not be acquired.
            if state.sessions.is_empty() && !state.stopping {
                return Ok(state.initial_status.unwrap_or(0));
            }

            if reap_presentation(&mut state, inputs, config)? && !state.stopping {
                match mode {
                    LaunchMode::Workspace => log.note(&format!(
                        "Eon Desktop exited; terminals remain active. Run `eon attach {generation}` to reconnect."
                    )),
                    LaunchMode::Terminal => log.note(&format!(
                        "Eon Desktop exited; terminal remains active. Run `eonbase attach {generation}` to reconnect."
                    )),
                }
            }

            if !state.stopping {
                state.follow_attention();
                if let Some(workspaces) = &state.workspaces {
                    state.pane_labels.reconcile(&workspaces.pane_sessions());
                }
            } else {
                state.pane_labels.reconcile(&[]);
            }

            if control_listener.accept(|request| {
                dispatch_control_request(request, &mut state, inputs, config, runtime, generation)
            })? {
                return Ok(state.initial_status.unwrap_or(0));
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    drop(control_listener);
    if let Some(process) = state.venus.take() {
        drop(process);
    }
    let popup_cleanup = stop_transient_popups(&mut state);
    remove_launcher(runtime);
    let _ = fs::remove_dir(runtime);
    match (status, popup_cleanup) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!(
            "{error}; cannot clean up transient popups: {cleanup}"
        )),
    }
}

struct SupervisorState {
    component_generation: String,
    application_id: String,
    decorations: bool,
    workspaces: Option<WorkspaceSet>,
    sessions: Vec<RunningSession>,
    venus: Option<PresentationProcess>,
    initial_child: Vec<OsString>,
    initial_status: Option<i32>,
    stopping: bool,
    quotas: Option<quota::Quotas>,
    /// Full Eon only; eonbase has no poller (EON-C31).
    attention: Option<Attention>,
    pane_labels: super::pane_labels::Labels,
    log: Arc<DiagnosticLog>,
}

impl SupervisorState {
    /// The shown workspace with its quotas and attention.
    fn workspace_snapshot(&mut self) -> Option<Snapshot> {
        self.workspaces.as_ref()?;
        self.workspace_snapshot_in(None).ok()
    }

    fn workspace_snapshot_in(&mut self, workspace: Option<&str>) -> Result<Snapshot, Failure> {
        let workspaces = self
            .workspaces
            .as_ref()
            .ok_or_else(|| failure("workspace-unavailable", "eonbase has no Eon workspace"))?;
        // An accepted action may remove its caller's workspace.
        let mut snapshot = workspaces.snapshot_in(workspace).or_else(|error| {
            if error.code == "unknown-workspace" {
                workspaces.snapshot_in(None)
            } else {
                Err(error)
            }
        })?;
        self.pane_labels.present(&mut snapshot, workspaces.home());
        if let Some(quotas) = &self.quotas {
            quotas.apply(&mut snapshot);
        }
        if let Some(attention) = &mut self.attention {
            attention.present(
                &mut snapshot,
                &workspaces.snapshots(),
                self.venus.is_some(),
                Instant::now(),
            );
        }
        Ok(snapshot)
    }

    /// Follows focus, then polls program status, between control requests
    /// and never inside one (EON-C31).
    fn follow_attention(&mut self) {
        let (Some(attention), Some(workspace)) = (&mut self.attention, &self.workspaces) else {
            return;
        };
        let (targets, focus) = attention::targets(&workspace.snapshots());
        attention.observe(focus, self.venus.is_some());
        attention.poll(&targets, &mut self.sessions, Instant::now());
        attention.retain(&targets, &self.sessions);
    }
}

fn report_session_end(result: Result<(), Failure>, log: &DiagnosticLog) -> Result<(), String> {
    match result {
        Err(error) if error.code == "session-start" => {
            // A failed picker fallback must not tear down other tabs' terminals.
            log.note(&format!(
                "Cannot start the tab's first pane: {}",
                error.detail
            ));
            Ok(())
        }
        result => result.map_err(|error| error.detail),
    }
}

/// The line that records a terminal's end, naming Orbit's log when Orbit
/// exited without publishing its result.
fn session_end_line(session: &RunningSession, code: i32) -> String {
    format!(
        "terminal {} ended with status {code}{}",
        session.id,
        if session.owner_lost() {
            format!(
                "; Orbit exited without publishing a terminal result{}",
                session.orbit_log_suffix()
            )
        } else {
            String::new()
        }
    )
}

/// Notes once that a terminal which cannot be stopped or reconciled stays in
/// its tab for a retry.
fn report_kept_terminal(session: &mut RunningSession, log: &DiagnosticLog, error: &str) {
    if !session.reap_failure_reported {
        session.reap_failure_reported = true;
        log.note(&format!(
            "terminal {} stays in its tab {}: {error}{}",
            session.id,
            if session.stop_failed() {
                "after a failed stop"
            } else {
                "until its end reconciles"
            },
            session.orbit_log_suffix()
        ));
    }
}

fn reap_finished_sessions(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<(), String> {
    let mut index = 0;
    while index < state.sessions.len() {
        let session = &mut state.sessions[index];
        let finished = match session_finished(session) {
            // A terminal that cannot be reconciled, after a failed Stop or an
            // exit of its own, stays in its tab for a retry and must not end
            // the supervisor or other tabs (EON-C8).
            Err(error) => {
                report_kept_terminal(session, &state.log, &error);
                None
            }
            Ok(finished) => finished,
        };
        if let Some(code) = finished {
            let session = state.sessions.remove(index);
            state.log.record(&session_end_line(&session, code));
            if let Some(workspace) = &mut state.workspaces {
                let component_generation = &state.component_generation;
                let sessions = &mut state.sessions;
                let initial_child = &mut state.initial_child;
                let log = &state.log;
                let result =
                    workspace.session_ended(&session.id, SessionEnd::Exited, |operation| {
                        apply_session_operation(
                            inputs,
                            config,
                            log,
                            component_generation,
                            sessions,
                            initial_child,
                            operation,
                            None,
                        )
                    });
                report_session_end(result, &state.log)?;
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

/// Stops a terminal, or reconciles the end it reached first. The caller
/// removes an ended terminal before a reap would see it, so an Orbit lost on
/// the way is recorded here.
fn stop_workspace_session(
    session: &mut RunningSession,
    log: &DiagnosticLog,
) -> Result<Option<i32>, String> {
    let status = match session_finished(session)? {
        Some(status) => Some(status),
        None => match stop_managed_sessions(std::slice::from_mut(session), SESSION_START_TIMEOUT) {
            Ok(_) => None,
            Err(stop_error) => match session_finished(session) {
                Ok(Some(status)) => Some(status),
                Ok(None) => return Err(stop_error),
                Err(reconcile_error) => {
                    return Err(format!(
                        "{stop_error}; cannot reconcile terminal after failed stop: {reconcile_error}"
                    ));
                }
            },
        },
    };
    if let Some(code) = status
        && session.owner_lost()
    {
        log.record(&session_end_line(session, code));
    }
    Ok(status)
}

fn close_workspace_tab(
    request: &Request,
    tab: &str,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Result<Response, Failure> {
    let started = Instant::now();
    let result: Result<Option<String>, Failure> = (|| {
        reap_finished_sessions(state, inputs, config)
            .map_err(|detail| failure("tab-close-failed", detail))?;
        let caller = caller_session(state, inputs, config, request)?;
        let workspace = state
            .workspaces
            .as_mut()
            .ok_or_else(|| failure("workspace-unavailable", "eonbase has no Eon workspace"))?;
        let reply_workspace = workspace.caller_workspace_id(caller.as_deref())?;
        let sessions = workspace.prepare_close_tab(&request.id, caller.as_deref(), tab)?;

        state.log.record(&format!(
            "closing tab {tab}: stopping {}",
            sessions.join(", ")
        ));
        stop_closed_sessions(state, sessions, "tab-close-failed")?;
        Ok(reply_workspace)
    })();
    let elapsed = started.elapsed().as_millis();
    state.log.record(&match &result {
        Ok(_) => format!("closed tab {tab} in {elapsed} ms"),
        Err(error) => format!(
            "tab {tab} close failed after {elapsed} ms: {}",
            error.detail
        ),
    });
    let reply_workspace = result?;
    closed_response(
        state,
        runtime,
        generation,
        "tab-close-failed",
        reply_workspace.as_deref(),
    )
}

fn close_workspace_pane(
    request: &Request,
    pane: &str,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Result<Response, Failure> {
    reap_finished_sessions(state, inputs, config)
        .map_err(|detail| failure("pane-close-failed", detail))?;
    let caller = caller_session(state, inputs, config, request)?;
    let workspace = state
        .workspaces
        .as_mut()
        .ok_or_else(|| failure("workspace-unavailable", "eonbase has no Eon workspace"))?;
    let reply_workspace = workspace.caller_workspace_id(caller.as_deref())?;
    let sessions = workspace.prepare_close_pane(&request.id, caller.as_deref(), pane)?;
    stop_closed_sessions(state, sessions, "pane-close-failed")?;
    closed_response(
        state,
        runtime,
        generation,
        "pane-close-failed",
        reply_workspace.as_deref(),
    )
}

fn close_workspace(
    request: &Request,
    target: &str,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Result<Response, Failure> {
    reap_finished_sessions(state, inputs, config)
        .map_err(|detail| failure("workspace-close-failed", detail))?;
    let caller = caller_session(state, inputs, config, request)?;
    let workspace = state
        .workspaces
        .as_mut()
        .ok_or_else(|| failure("workspace-unavailable", "eonbase has no Eon workspace"))?;
    let reply_workspace = workspace.caller_workspace_id(caller.as_deref())?;
    let sessions = workspace.prepare_close_workspace(&request.id, caller.as_deref(), target)?;
    stop_closed_sessions(state, sessions, "workspace-close-failed")?;
    closed_response(
        state,
        runtime,
        generation,
        "workspace-close-failed",
        reply_workspace.as_deref(),
    )
}

/// Stops a close's terminals one at a time and prunes each confirmed end, so
/// a failed stop leaves every remaining terminal represented.
fn stop_closed_sessions(
    state: &mut SupervisorState,
    sessions: Vec<String>,
    code: &str,
) -> Result<(), Failure> {
    for id in sessions {
        let natural_status = stop_owned_session(&mut state.sessions, &id, &state.log)
            .map_err(|detail| failure(code, format!("{id}: {detail}")))?;
        state
            .workspaces
            .as_mut()
            .expect("workspace close retains its owner")
            .session_ended(&id, SessionEnd::Closed, |_| {
                Err("a close cannot start a replacement terminal".into())
            })?;
        if id == "session-1" && natural_status.is_some() {
            state.initial_status = natural_status;
        }
    }
    Ok(())
}

/// A close that stopped the window's last terminal ends its generation, and
/// leaves no workspace to snapshot. As Stop does, it first stops any run of
/// the generation that no tab tracks, such as a picker that became ready
/// after its start timed out; failing that, it fails as Stop does.
fn closed_response(
    state: &mut SupervisorState,
    runtime: &Path,
    generation: &str,
    code: &str,
    reply_workspace: Option<&str>,
) -> Result<Response, Failure> {
    if !state.sessions.is_empty()
        && state
            .workspaces
            .as_ref()
            .is_some_and(WorkspaceSet::has_sessions)
    {
        let snapshot = state.workspace_snapshot_in(reply_workspace)?;
        return Ok(Response::Snapshot(snapshot));
    }
    stop_generation_sessions(state, runtime, LaunchMode::Workspace)
        .map_err(|detail| failure(code, detail))?;
    Ok(Response::Ended(Ended {
        generation: generation.into(),
    }))
}

/// Stops every run of this generation, including runs no tab tracks, and
/// names them. The supervisor refuses other requests from here on, and a
/// complete stop leaves it no terminal.
fn stop_generation_sessions(
    state: &mut SupervisorState,
    runtime: &Path,
    mode: LaunchMode,
) -> Result<Vec<String>, String> {
    state.stopping = true;
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    let untracked = acquire_generation_sessions(
        runtime,
        mode,
        &state.component_generation,
        &state.sessions,
        state.log.orbit_directory().ok(),
        deadline,
    )?;
    state.sessions.extend(untracked);
    let running = state
        .sessions
        .iter()
        .map(|session| !session.is_finished())
        .collect::<Vec<_>>();
    let result = stop_managed_sessions(
        &mut state.sessions,
        deadline.saturating_duration_since(Instant::now()),
    );
    // Reaping pauses once the generation stops, so a run whose Orbit was
    // lost on the way is recorded here, and a failure names the Orbit log of
    // each run this Stop left unstopped.
    let mut orbit_logs = String::new();
    for session in state
        .sessions
        .iter_mut()
        .zip(running)
        .filter_map(|(session, running)| running.then_some(session))
    {
        if session.owner_lost()
            && let Ok(Some(code)) = session_finished(session)
        {
            state.log.record(&session_end_line(session, code));
        }
        if !session.stopped() {
            orbit_logs.push_str(&session.orbit_log_suffix());
        }
    }
    let stopped = result.map_err(|error| format!("{error}{orbit_logs}"))?;
    state.sessions.clear();
    Ok(stopped)
}

/// The live terminal session a request's `EON_TERMINAL` caller names; a
/// terminal that has ended since the last reconciliation names none.
fn caller_session(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    request: &Request,
) -> Result<Option<String>, Failure> {
    let Some(caller) = &request.caller else {
        return Ok(None);
    };
    reap_finished_sessions(state, inputs, config)
        .map_err(|detail| failure("unavailable", detail))?;
    state
        .sessions
        .iter()
        .find(|session| {
            session.is_terminal(caller)
                && state.workspaces.as_ref().is_none_or(|workspaces| {
                    workspaces.owns_session(&workspace::Session {
                        id: session.id.clone(),
                        endpoint: session.endpoint.clone(),
                    })
                })
        })
        .map(|session| Some(session.id.clone()))
        .ok_or_else(|| {
            failure(
                "unknown-caller",
                "the calling terminal is not live in this Eon workspace",
            )
        })
}

fn directory_picker_command(
    inputs: &Inputs,
    runtime: &Path,
    tab: &str,
    instance: &str,
    startup_animation: Option<&managed_environment::StartupAnimation>,
) -> Result<Vec<OsString>, String> {
    let session_bin = inputs
        .managed
        .shell_bin
        .as_ref()
        .ok_or("the installed Eon package has no directory picker")?;
    let mut command = vec![session_bin.join("eon-directory-picker").into_os_string()];
    if let Some(animation) = startup_animation {
        command.extend([
            OsString::from("__startup-anima"),
            animation.style.clone().into(),
            animation.duration_seconds.to_string().into(),
        ]);
    } else {
        command.push("__directory-picker".into());
    }
    command.extend([
        runtime.join("eon.sock").into_os_string(),
        tab.into(),
        instance.into(),
    ]);
    Ok(command)
}

#[allow(clippy::too_many_arguments)]
fn apply_session_operation(
    inputs: &Inputs,
    config: &Path,
    log: &DiagnosticLog,
    component_generation: &str,
    sessions: &mut Vec<RunningSession>,
    initial_child: &mut Vec<OsString>,
    operation: SessionOperation,
    startup_animation: Option<&managed_environment::StartupAnimation>,
) -> Result<(), String> {
    let (session, directory, command, tab) = match operation {
        SessionOperation::Start {
            session,
            directory,
            command,
            tab,
        } => (session, directory, command, tab),
        SessionOperation::Stop(id) => return stop_owned_session(sessions, &id, log).map(|_| ()),
    };
    let child = match command {
        LaunchCommand::Project { tab, instance } => directory_picker_command(
            inputs,
            session
                .endpoint
                .parent()
                .ok_or("directory picker endpoint has no runtime directory")?,
            &tab,
            &instance,
            startup_animation,
        )?,
        LaunchCommand::Tool(argv) => argv,
        LaunchCommand::Pane if session.id == "session-1" => initial_child.clone(),
        LaunchCommand::Pane => Vec::new(),
    };
    let running = start_orbit(
        inputs,
        config,
        &session.endpoint,
        &session.id,
        component_generation,
        &directory,
        &child,
        tab.as_deref(),
        log,
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    sessions.push(running);
    if session.id == "session-1" {
        initial_child.clear();
    }
    Ok(())
}

fn stop_owned_session(
    sessions: &mut Vec<RunningSession>,
    id: &str,
    log: &DiagnosticLog,
) -> Result<Option<i32>, String> {
    let index = sessions
        .iter()
        .position(|session| session.id == id)
        .ok_or_else(|| format!("terminal {id} is missing from the supervisor"))?;
    let session = &mut sessions[index];
    let status = stop_workspace_session(session, log)
        .map_err(|error| format!("{error}{}", session.orbit_log_suffix()))?;
    sessions.remove(index);
    Ok(status)
}

fn cancel_transient_popups(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<(), String> {
    let ids = state
        .workspaces
        .as_ref()
        .map(WorkspaceSet::transient_sessions)
        .unwrap_or_default();
    for id in ids {
        if let Some(index) = state.sessions.iter().position(|session| session.id == id) {
            // A picker that cannot be stopped stays in its tab, where the reap
            // and the next detachment retry it, and must not end the
            // supervisor or keep the other pickers from being cleaned
            // (EON-C18).
            if let Err(error) = stop_workspace_session(&mut state.sessions[index], &state.log) {
                report_kept_terminal(&mut state.sessions[index], &state.log, &error);
                continue;
            }
            state.sessions.remove(index);
        }
        let SupervisorState {
            component_generation,
            workspaces,
            sessions,
            initial_child,
            log,
            ..
        } = state;
        let result = workspaces
            .as_mut()
            .ok_or("transient popup has no workspace owner")?
            .session_ended(&id, SessionEnd::Exited, |operation| {
                apply_session_operation(
                    inputs,
                    config,
                    log,
                    component_generation,
                    sessions,
                    initial_child,
                    operation,
                    None,
                )
            });
        report_session_end(result, log)?;
    }
    Ok(())
}

fn stop_transient_popups(state: &mut SupervisorState) -> Result<(), String> {
    let ids = state
        .workspaces
        .as_ref()
        .map(WorkspaceSet::transient_sessions)
        .unwrap_or_default();
    for id in ids {
        if state.sessions.iter().any(|session| session.id == id) {
            stop_owned_session(&mut state.sessions, &id, &state.log)?;
        }
    }
    Ok(())
}

fn reap_presentation(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<bool, String> {
    let exited = match state.venus.as_mut() {
        Some(process) => process.exited()?,
        None => None,
    };
    let Some(status) = exited else {
        return Ok(false);
    };
    state.venus = None;
    if !state.stopping {
        // Full Eon's clean exit is a detach; a crash is not (EON-C8).
        if state.workspaces.is_some() && status.success() {
            notification::notify_first_detach(&state.application_id);
        }
        cancel_transient_popups(state, inputs, config)?;
    }
    Ok(true)
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
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> (ControlResponse, bool) {
    let mode = if state.workspaces.is_some() {
        LaunchMode::Workspace
    } else {
        LaunchMode::Terminal
    };
    if state.stopping
        && !matches!(
            request.action,
            Action::Workspace(
                WorkspaceAction::InspectRuntime
                    | WorkspaceAction::InspectPresentation
                    | WorkspaceAction::Stop { .. }
            )
        )
    {
        let error = failure(
            "stop-incomplete",
            "generation cleanup is incomplete; retry Stop",
        );
        return (
            match request.action {
                Action::Workspace(WorkspaceAction::Present { .. }) => {
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(error))
                }
                _ => ControlResponse::Workspace(Response::Failure(error)),
            },
            false,
        );
    }
    match request {
        Request {
            action:
                Action::Workspace(
                    WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation,
                ),
            ..
        } => match runtime_status(inputs, generation, &state.sessions, mode) {
            Ok(mut runtime) => {
                if state.stopping {
                    runtime.attach = Availability {
                        available: false,
                        reason: "generation cleanup is incomplete; retry Stop".into(),
                    };
                }
                (
                    ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)),
                    false,
                )
            }
            Err(detail) => (
                ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                    "unrepresentable-state",
                    detail,
                ))),
                false,
            ),
        },
        Request {
            action:
                Action::Workspace(WorkspaceAction::Present {
                    workspace: expected,
                }),
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
            if let Err(detail) = reap_finished_sessions(state, inputs, config) {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "presentation-unavailable",
                        detail,
                    ))),
                    false,
                );
            }
            if state.sessions.is_empty() {
                return (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "generation-ending",
                        "the last terminal exited while Eon Desktop was reopening",
                    ))),
                    true,
                );
            }
            let result = reap_presentation(state, inputs, config).and_then(|_| {
                if let Some(venus) = &state.venus {
                    return venus.present();
                }
                let socket = if mode == LaunchMode::Workspace {
                    runtime.join("eon.sock")
                } else {
                    state
                        .sessions
                        .first()
                        .ok_or("eonbase has no live terminal")?
                        .endpoint
                        .clone()
                };
                let terminal = managed_environment::terminal_presentation(
                    config,
                    &inputs.defaults,
                    Some(&state.log),
                )?;
                let bar_widgets = window_bar_widgets(mode, config, inputs, Some(&state.log))?;
                let command = venus_command(
                    inputs,
                    config,
                    &socket,
                    mode,
                    &terminal,
                    &bar_widgets,
                    &state.application_id,
                    state.decorations,
                    managed_environment::warning_close_last_tab(
                        config,
                        &inputs.defaults,
                        Some(&state.log),
                    )?,
                    managed_environment::animate_working_badge(
                        config,
                        &inputs.defaults,
                        Some(&state.log),
                    )?,
                );
                if let Some(quotas) = &mut state.quotas {
                    quotas.show(&bar_widgets);
                }
                state.venus = Some(PresentationProcess::start(
                    command,
                    terminal.requires_startup_admission(),
                    state.workspace_snapshot(),
                    PRESENTATION_ADMISSION_TIMEOUT,
                    &state.log,
                )?);
                Ok(())
            });
            match result.and_then(|()| runtime_status(inputs, generation, &state.sessions, mode)) {
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
            action: Action::Workspace(WorkspaceAction::Stop { generation: target }),
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
            let result = stop_generation_sessions(state, runtime, mode);
            let shutdown = result.is_ok();
            let response = match result {
                Ok(mut sessions) => {
                    sessions.retain(|session| !workspace::is_directory_picker_session(session));
                    LifecycleResponse::Stopped(Stopped {
                        generation: generation.into(),
                        sessions,
                    })
                }
                Err(detail) => LifecycleResponse::Failure(failure("stop-failed", detail)),
            };
            (ControlResponse::Lifecycle(response), shutdown)
        }
        Request {
            action: Action::Workspace(WorkspaceAction::CloseWorkspace { ref workspace }),
            ..
        } => closed(close_workspace(
            &request, workspace, state, inputs, config, runtime, generation,
        )),
        Request {
            action: Action::Workspace(WorkspaceAction::CloseTab { ref tab }),
            ..
        } => closed(close_workspace_tab(
            &request, tab, state, inputs, config, runtime, generation,
        )),
        Request {
            action: Action::ClosePane { ref pane },
            ..
        } => closed(close_workspace_pane(
            &request, pane, state, inputs, config, runtime, generation,
        )),
        request => {
            if state.workspaces.is_none() {
                return (
                    ControlResponse::Workspace(Response::Failure(failure(
                        "workspace-unavailable",
                        "eonbase has no Eon workspace",
                    ))),
                    false,
                );
            }
            let caller = match caller_session(state, inputs, config, &request) {
                Ok(caller) => caller,
                Err(error) => return (ControlResponse::Workspace(Response::Failure(error)), false),
            };
            let Some(workspace) = &mut state.workspaces else {
                unreachable!("reconciliation keeps the workspace owner");
            };
            let reply_workspace = match workspace.caller_workspace_id(caller.as_deref()) {
                Ok(workspace) => workspace,
                Err(error) => return (ControlResponse::Workspace(Response::Failure(error)), false),
            };
            // A queue jump is the FocusAttention of its target in the target's
            // workspace, which it shows; an empty queue is rejected and changes
            // nothing (EON-C31, EON-C33).
            let (action, jump) = match (request.action, &state.attention) {
                (action @ (Action::NextAttention | Action::PreviousAttention), Some(attention)) => {
                    let Some((target_workspace, action, key)) = attention.jump(
                        &workspace.snapshots(),
                        matches!(action, Action::NextAttention),
                    ) else {
                        return (
                            ControlResponse::Workspace(Response::Failure(failure(
                                "no-attention",
                                "No agent needs attention",
                            ))),
                            false,
                        );
                    };
                    (action, Some((target_workspace, key)))
                }
                (action, _) => (action, None),
            };
            let accepted = matches!(
                action,
                Action::ToggleSidebar | Action::FocusAttention { .. }
            )
            .then(|| action.clone());
            let component_generation = &state.component_generation;
            let sessions = &mut state.sessions;
            let initial_child = &mut state.initial_child;
            let log = &state.log;
            let result = match (&jump, &accepted) {
                (Some((target_workspace, _)), Some(Action::FocusAttention { tab, target })) => {
                    workspace.jump(
                        &request.id,
                        caller.as_deref(),
                        target_workspace,
                        tab,
                        target,
                    )
                }
                _ => workspace.dispatch(
                    &request.id,
                    caller.as_deref(),
                    action,
                    |command, directory| {
                        managed_environment::prepare_popup_command(
                            command,
                            inputs.managed.shell_bin.as_deref(),
                            directory,
                            &inputs.defaults,
                        )
                    },
                    |operation| {
                        apply_session_operation(
                            inputs,
                            config,
                            log,
                            component_generation,
                            sessions,
                            initial_child,
                            operation,
                            None,
                        )
                    },
                ),
            };
            // Only the shown workspace's focus sees, so a hidden caller's
            // FocusAttention sees nothing.
            if result.is_ok()
                && let (Some(action), Some(attention)) = (&accepted, &mut state.attention)
            {
                attention.handled(
                    action,
                    jump.map(|(_, key)| key),
                    &workspace.snapshots(),
                    state.venus.is_some(),
                );
            }
            match result {
                Ok(()) => (
                    ControlResponse::Workspace(
                        match state.workspace_snapshot_in(reply_workspace.as_deref()) {
                            Ok(snapshot) => Response::Snapshot(snapshot),
                            Err(error) => Response::Failure(error),
                        },
                    ),
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
    }
}

/// A close reply; one that ended the generation shuts the supervisor down.
fn closed(result: Result<Response, Failure>) -> (ControlResponse, bool) {
    let response = result.unwrap_or_else(Response::Failure);
    let ended = matches!(response, Response::Ended(_));
    (ControlResponse::Workspace(response), ended)
}

fn runtime_status(
    inputs: &Inputs,
    generation: &str,
    sessions: &[RunningSession],
    mode: LaunchMode,
) -> Result<Runtime, String> {
    if mode == LaunchMode::Terminal && sessions.is_empty() {
        return Err("supervisor has no live terminals".into());
    }
    Ok(Runtime {
        generation: generation.into(),
        eon_version: inputs.version.into(),
        workspace_protocol: VERSION,
        component_report: inputs.component_report()?.to_owned(),
        sessions: sessions
            .iter()
            .filter(|session| {
                !session.is_finished() && !workspace::is_directory_picker_session(&session.id)
            })
            .map(|session| session.id.clone())
            .collect(),
        attach: Availability {
            available: true,
            reason: match mode {
                LaunchMode::Workspace => "supervisor accepts EONW v8 presentation requests",
                LaunchMode::Terminal => "supervisor owns one eonbase terminal",
            }
            .into(),
        },
        stop: Availability {
            available: true,
            reason: "generation-aware supervisor owns these terminals".into(),
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
        Action, DiagnosticLog, LaunchMode, SupervisorState, WorkspaceAction, WorkspaceSet,
        cancel_transient_popups, effective_uid, prepare_configuration, prepare_runtime,
        reap_finished_sessions, stop_generation_sessions, stop_owned_session, temporary_directory,
        venus_command, window_bar_widgets, xdg_path,
    };
    use crate::sessions::{
        self,
        tests::{leased_orbit, orbit_pid, script},
    };
    use std::{
        ffi::OsString,
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        process::Command,
    };

    /// Kills a test-owned Orbit's worker, which sends Orbit down its failure
    /// path: it removes its Live record and exits without a result.
    fn kill_worker(session: &sessions::RunningSession) {
        kill_orbit_worker(orbit_pid(session));
    }

    fn kill_orbit_worker(orbit: i32) {
        let worker = fs::read_to_string(format!("/proc/{orbit}/task/{orbit}/children"))
            .unwrap()
            .split_whitespace()
            .map(|pid| pid.parse::<i32>().unwrap())
            .find(|pid| {
                fs::read(format!("/proc/{pid}/cmdline"))
                    .unwrap_or_default()
                    .split(|byte| *byte == 0)
                    .next()
                    .is_some_and(|program| program.ends_with(b"yazelix-orbit-worker"))
            })
            .expect("Orbit runs its worker");
        // SAFETY: signals the test-owned Orbit's worker process.
        assert_eq!(unsafe { libc::kill(worker, libc::SIGKILL) }, 0);
    }

    #[test]
    fn a_terminal_whose_stop_fails_during_a_close_ends_or_stays_only_in_its_tab() {
        let roots = [
            temporary_directory(),
            temporary_directory(),
            temporary_directory(),
        ];
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "exec sleep 30"))
            .collect::<Vec<_>>();
        sessions[1].id = "session-2".into();
        sessions[2].id = "session-3".into();
        let log = DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "workspace");
        // A dead worker sends Orbit down its failure path, which removes its
        // Live record and publishes no result: the run reconciles as ended.
        let first_orbit_log = sessions[0].orbit_log.clone().unwrap();
        kill_worker(&sessions[0]);
        assert_eq!(
            stop_owned_session(&mut sessions, "session-1", &log),
            Ok(Some(1))
        );
        assert_eq!(sessions.len(), 2);
        let owner_loss = fs::read_dir(roots[0].join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(
            owner_loss.lines().any(|line| {
                line.contains("terminal session-1 ended with status 1; Orbit exited without publishing a terminal result")
                    && line.ends_with(&format!("; Orbit log: {}", first_orbit_log.display()))
            }),
            "{owner_loss}"
        );

        // A record that no longer validates leaves the stopped run
        // unreconcilable: the close fails and the terminal stays tracked.
        // Only the tracked id is renamed, so its Orbit log still says session-1.
        let second_orbit_log = sessions[0].orbit_log.clone().unwrap();
        let record = sessions::tests::record(&sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let error = stop_owned_session(&mut sessions, "session-2", &log).unwrap_err();
        assert!(
            error.ends_with(&format!("; Orbit log: {}", second_orbit_log.display())),
            "{error}"
        );
        assert_eq!(sessions.len(), 2);
        let mut state = SupervisorState {
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            log,
        };
        let inputs = crate::fixtures::inputs();
        for _ in 0..2 {
            assert_eq!(
                reap_finished_sessions(&mut state, &inputs, &roots[0]),
                Ok(())
            );
        }
        assert_eq!(state.sessions.len(), 2);
        let log = fs::read_dir(roots[0].join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert_eq!(
            log.matches("terminal session-2 stays in its tab after a failed stop")
                .count(),
            1
        );
        assert_eq!(
            stop_owned_session(&mut state.sessions, "session-3", &state.log),
            Ok(None)
        );
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_terminal_whose_own_exit_cannot_be_reconciled_stays_only_in_its_tab() {
        let roots = [
            temporary_directory(),
            temporary_directory(),
            temporary_directory(),
        ];
        let go = roots[0].join("go");
        let mut sessions = vec![
            leased_orbit(
                &roots[0],
                &format!(
                    "while [ ! -e '{}' ]; do sleep 0.02; done; exit 7",
                    go.display()
                ),
            ),
            leased_orbit(&roots[1], "exec sleep 30"),
            leased_orbit(&roots[2], "exec sleep 30"),
        ];
        sessions[1].id = "session-2".into();
        sessions[2].id = "session-3".into();
        // A closed lease whose run still holds its Live record leaves the end
        // pending: the first reap waits for it, later reaps must not.
        sessions::tests::close_lease(&sessions[2]);
        // The terminal exits on its own, never sent Stop, and its final record
        // then no longer validates.
        let orbit = orbit_pid(&sessions[0]);
        fs::write(&go, "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !fs::read_to_string(format!("/proc/{orbit}/stat"))
            .unwrap()
            .contains(") Z ")
        {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let record = sessions::tests::record(&sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let mut state = SupervisorState {
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            log: DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "workspace"),
        };
        let inputs = crate::fixtures::inputs();
        for _ in 0..2 {
            assert_eq!(
                reap_finished_sessions(&mut state, &inputs, &roots[0]),
                Ok(())
            );
        }
        let retry = std::time::Instant::now();
        assert_eq!(
            reap_finished_sessions(&mut state, &inputs, &roots[0]),
            Ok(())
        );
        assert!(retry.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(state.sessions.len(), 3);
        let log = || {
            fs::read_dir(roots[0].join("logs"))
                .unwrap()
                .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
                .collect::<String>()
        };
        assert_eq!(
            log()
                .matches("terminal session-1 stays in its tab until its end reconciles")
                .count(),
            1
        );
        assert_eq!(
            log()
                .matches("terminal session-3 stays in its tab until its end reconciles")
                .count(),
            1
        );

        // Once the record validates again, a later reap ends the terminal.
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            reap_finished_sessions(&mut state, &inputs, &roots[0]),
            Ok(())
        );
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(state.initial_status, Some(7));
        kill_worker(&state.sessions[1]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.sessions.len() > 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the pending end never reconciled"
            );
            assert_eq!(
                reap_finished_sessions(&mut state, &inputs, &roots[0]),
                Ok(())
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(state.sessions[0].id, "session-2");
        assert_eq!(
            stop_owned_session(&mut state.sessions, "session-2", &state.log),
            Ok(None)
        );
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn orbit_stderr_is_logged_and_owner_loss_names_the_file() {
        let root = temporary_directory();
        let launcher = root.join("orbit-with-stderr");
        script(
            &launcher,
            "#!/bin/sh\nprintf '%s\\n' 'orbit test diagnostic' >&2\nexec \"$ORBIT_BIN\" \"$@\"\n",
        );
        let session = sessions::tests::leased_orbit_with_program(&root, "exec sleep 30", &launcher);
        let orbit = orbit_pid(&session);
        let orbit_logs = root.join("launch-logs/orbit");
        let orbit_log = orbit_logs.join(format!("component-1-session-1-{}.log", session.run_id()));
        assert_eq!(session.orbit_log.as_ref(), Some(&orbit_log));
        // A supervisor that recovers the run rebuilds its log path. Orbit may
        // refuse the new lease as Busy until it sees the old one close.
        drop(session);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let sessions = loop {
            match sessions::recover_sessions(
                &root,
                LaunchMode::Workspace,
                "component-1",
                Some(&orbit_logs),
                deadline,
            ) {
                Ok((sessions, _)) => break sessions,
                Err(error)
                    if error.ends_with("is already managed")
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert_eq!(sessions[0].orbit_log.as_ref(), Some(&orbit_log));
        kill_orbit_worker(orbit);
        let mut state = SupervisorState {
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "workspace"),
        };
        let inputs = crate::fixtures::inputs();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.sessions.is_empty() {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            assert_eq!(reap_finished_sessions(&mut state, &inputs, &root), Ok(()));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let stderr = fs::read_to_string(&orbit_log).unwrap();
        assert!(stderr.contains("orbit test diagnostic\n"), "{stderr}");
        assert_eq!(
            fs::metadata(&orbit_log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let supervisor_log = fs::read_dir(root.join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        let failure = supervisor_log
            .lines()
            .find(|line| line.contains("Orbit exited without publishing a terminal result"))
            .expect("the supervisor recorded Orbit's owner loss");
        assert!(
            failure.ends_with(&format!("; Orbit log: {}", orbit_log.display())),
            "{failure}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_generation_stop_that_loses_orbit_names_its_log() {
        let root = temporary_directory();
        let session = leased_orbit(&root, "exec sleep 30");
        let orbit_log = session.orbit_log.clone().unwrap();
        kill_worker(&session);
        let mut state = SupervisorState {
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions: vec![session],
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "workspace"),
        };

        let error = stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap_err();
        assert!(
            error.contains("session-1: terminal owner exited without completing Stop")
                && error.ends_with(&format!("; Orbit log: {}", orbit_log.display())),
            "{error}"
        );
        assert!(state.stopping);
        let supervisor_log = fs::read_dir(root.join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| fs::read_to_string(path).unwrap())
            .collect::<String>();
        assert_eq!(
            supervisor_log
                .lines()
                .filter(|line| {
                    line.contains("terminal session-1 ended with status 1; Orbit exited without publishing a terminal result")
                        && line.ends_with(&format!("; Orbit log: {}", orbit_log.display()))
                })
                .count(),
            1,
            "{supervisor_log}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_picker_whose_stop_fails_on_detach_stays_only_in_its_tab() {
        fn prepared(_: &crate::PopupCommand, _: &Path) -> Result<Vec<OsString>, String> {
            Ok(Vec::new())
        }
        let roots = [
            temporary_directory(),
            temporary_directory(),
            temporary_directory(),
        ];
        let runtime = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let mut workspace = WorkspaceSet::pending(
            runtime.clone(),
            runtime.clone(),
            Some(runtime.clone()),
            inputs.defaults.popups.clone(),
            prepared,
            |_| Ok(()),
        )
        .unwrap();
        workspace
            .dispatch(
                "t2",
                None,
                Action::Workspace(WorkspaceAction::CreateTab),
                prepared,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            workspace.transient_sessions(),
            ["directory-picker-1", "directory-picker-2"]
        );
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "exec sleep 30"))
            .collect::<Vec<_>>();
        sessions[0].id = "directory-picker-1".into();
        sessions[1].id = "directory-picker-2".into();
        sessions[2].id = "session-3".into();
        // A record that no longer validates makes the first picker's stop fail.
        let record = sessions::tests::record(&sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let mut state = SupervisorState {
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions,
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            log: DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "workspace"),
        };
        let ids = |state: &SupervisorState| {
            state
                .sessions
                .iter()
                .map(|session| session.id.clone())
                .collect::<Vec<_>>()
        };
        for _ in 0..2 {
            assert_eq!(
                cancel_transient_popups(&mut state, &inputs, &runtime),
                Ok(())
            );
        }
        // The other picker was cleaned and its tab, whose fixture first pane
        // cannot start, removed; the failed picker keeps its pending tab.
        assert_eq!(ids(&state), ["directory-picker-1", "session-3"]);
        let snapshot = state.workspaces.as_ref().unwrap().snapshot();
        assert_eq!(snapshot.tabs.len(), 1);
        assert!(snapshot.tabs[0].pending);
        assert_eq!(snapshot.tabs[0].popups.len(), 1);
        // The failed first-pane start left an Orbit log directory beside the
        // supervisor's log.
        let log = fs::read_dir(roots[0].join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| fs::read_to_string(path).unwrap())
            .collect::<String>();
        assert_eq!(
            log.matches("terminal directory-picker-1 stays in its tab after a failed stop")
                .count(),
            1
        );

        // Once the record validates again, a later detachment cleans it after
        // its stopped run ends; each retry takes one look.
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.sessions.len() > 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the failed picker was never cleaned"
            );
            assert_eq!(
                cancel_transient_popups(&mut state, &inputs, &runtime),
                Ok(())
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(ids(&state), ["session-3"]);
        assert_eq!(
            stop_owned_session(&mut state.sessions, "session-3", &state.log),
            Ok(None)
        );
        for root in roots.into_iter().chain([runtime]) {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn startup_snapshot_transfer_uses_the_admission_deadline() {
        use eon_workspace_protocol::v23::{
            Column, Pane, PopupGeometry, Response, Snapshot, Tab, encode_response,
        };
        use std::time::{Duration, Instant};

        let root = temporary_directory();
        let snapshot = Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v23::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 64,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: Vec::new(),
            tabs: (1..=64)
                .map(|number| Tab {
                    id: format!("t{number}"),
                    directory: vec![b'/'; 4096],
                    pending: false,
                    selected_pane: Some(format!("p{number}")),
                    selected_popup: None,
                    columns: vec![Column {
                        panes: vec![Pane {
                            label: format!("p{number}"),
                            pinned_name: None,
                            id: format!("p{number}"),
                            session: format!("session-{number}"),
                            endpoint: format!("/runtime/orbit-{number}.sock").into_bytes(),
                            live: true,
                            attention: None,
                        }],
                        expanded: format!("p{number}"),
                    }],
                    popups: Vec::new(),
                    attention_rollup: None,
                })
                .collect(),
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v23::Sidebar::Open,
        };
        let expected = encode_response(&Response::Snapshot(snapshot.clone())).unwrap();
        let received = root.join("snapshot");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "workspace");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "sleep 0.6; dd bs=\"$1\" count=1 iflag=fullblock of=\"$2\" status=none || exit; printf ready-v1; exec cat >/dev/null",
            "venus",
        ]).arg(expected.len().to_string()).arg(&received);
        let process = super::PresentationProcess::start(
            command,
            true,
            Some(snapshot.clone()),
            Duration::from_secs(2),
            &log,
        )
        .unwrap();
        assert_eq!(fs::read(received).unwrap(), expected);
        drop(process);

        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 10"]);
        let started = Instant::now();
        let error = super::PresentationProcess::start(
            command,
            true,
            Some(snapshot),
            Duration::from_millis(300),
            &log,
        )
        .err()
        .unwrap();
        assert_eq!(
            error,
            "cannot admit Eon Desktop presentation: Venus was not ready within 300ms"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        let log = fs::read_dir(root.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            fs::read_to_string(log)
                .unwrap()
                .contains("Eon Desktop exited: signal: 9 (SIGKILL)")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diagnostic_log_keeps_presentation_stderr_and_exit_signal() {
        use std::{
            thread,
            time::{Duration, Instant},
        };

        let root = temporary_directory();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "workspace");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "echo \"panicked: backtrace=$RUST_BACKTRACE\" >&2; kill -KILL $$",
        ]);
        let mut process =
            super::PresentationProcess::start(command, false, None, Duration::from_secs(2), &log)
                .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while process.exited().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        drop(process);
        let path = fs::read_dir(root.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(path).unwrap();
        let backtrace = std::env::var("RUST_BACKTRACE").unwrap_or_else(|_| "1".into());
        let output = text
            .find(&format!("panicked: backtrace={backtrace}\n"))
            .expect(&text);
        let exit = text
            .find("Eon Desktop exited: signal: 9 (SIGKILL)")
            .expect(&text);
        assert!(text.contains("Eon Desktop started: pid "), "{text}");
        assert!(output < exit, "{text}");
        assert_eq!(text.matches("Eon Desktop exited").count(), 1, "{text}");
        fs::remove_dir_all(root).unwrap();
    }

    fn terminal_presentation(
        background_opacity: f32,
        background_blur: bool,
    ) -> crate::managed_environment::TerminalConfig {
        crate::managed_environment::TerminalConfig {
            background_opacity,
            background_blur,
            ..crate::fixtures::inputs().defaults.terminal
        }
    }

    #[test]
    fn version_badge_uses_supervising_product_and_visibility_at_each_launch() {
        let mut inputs = crate::fixtures::inputs();
        inputs.version = "9.8.7-alpha.2+proof";
        let root = temporary_directory();
        let launch_input = |inputs: &crate::Inputs, mode: LaunchMode, flag: &str| {
            let command = venus_command(
                inputs,
                &root,
                Path::new("/runtime.sock"),
                mode,
                &inputs.defaults.terminal,
                &window_bar_widgets(mode, &root, inputs, None).unwrap(),
                "eon",
                true,
                true,
                true,
            );
            let args = command.get_args().collect::<Vec<_>>();
            args.windows(2)
                .filter(|args| args[0] == flag)
                .map(|args| args[1].to_owned())
                .collect::<Vec<_>>()
        };
        for (source, widgets, visible) in [
            ("", "codex_quota,claude_quota,cpu,ram,clock,version", true),
            (
                "[bar]\nwidgets = ['clock', 'version', 'ram']\n",
                "clock,version,ram",
                true,
            ),
            (
                "[bar]\nwidgets = ['cpu', 'codex_quota']\n",
                "cpu,codex_quota",
                false,
            ),
            ("[bar]\nwidgets = []\n", "", false),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert_eq!(
                launch_input(&inputs, LaunchMode::Workspace, "--bar-items"),
                [widgets]
            );
            let versions = launch_input(&inputs, LaunchMode::Workspace, "--eon-version");
            if visible {
                assert_eq!(versions, [inputs.version]);
            } else {
                assert!(versions.is_empty());
            }
            for flag in ["--bar-items", "--eon-version"] {
                assert!(launch_input(&inputs, LaunchMode::Terminal, flag).is_empty());
            }
        }
        inputs.defaults.bar_widgets = &["ram", "clock"];
        fs::write(root.join("config.toml"), "[bar]\nunknown = true\n").unwrap();
        assert_eq!(
            launch_input(&inputs, LaunchMode::Workspace, "--bar-items"),
            ["ram,clock"]
        );
        // Names are checked only where the bar launches; malformed input fails every read.
        for (source, error, every_read) in [
            (
                "[bar]\nwidgets = ['gpu']\n",
                "unknown widget \"gpu\"",
                false,
            ),
            (
                "[bar]\nwidgets = ['RAM']\n",
                "unknown widget \"RAM\"",
                false,
            ),
            (
                "[bar]\nwidgets = ['ram', 'ram']\n",
                "repeats \"ram\"",
                false,
            ),
            ("[bar]\nwidgets = [1]\n", "invalid type", true),
            ("[bar]\nwidgets = 'ram'\n", "invalid type", true),
            (
                "[bar]\nshow_version = false\n",
                "replaced by bar.widgets",
                true,
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert_eq!(
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .is_err(),
                every_read,
                "{source:?}"
            );
            let failure =
                window_bar_widgets(LaunchMode::Workspace, &root, &inputs, None).unwrap_err();
            assert!(failure.contains(error), "{source:?}: {failure}");
            assert!(window_bar_widgets(LaunchMode::Terminal, &root, &inputs, None).is_ok());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_typography_reaches_both_product_launches() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        fs::write(
            root.join("config.toml"),
            r#"
[terminal]
font_family = "DejaVu Sans Mono"
font_fallbacks = ["DejaVu Sans", "Symbols Nerd Font Mono"]
font_size = 20.5
line_height = 1.5
columns = 100
rows = 30
"#,
        )
        .unwrap();
        for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            let command = venus_command(
                &inputs,
                &root,
                Path::new("/orbit.sock"),
                mode,
                &terminal,
                inputs.defaults.bar_widgets,
                "eon",
                true,
                true,
                true,
            );
            let args = command
                .get_args()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>();
            assert!(args.windows(14).any(|args| args
                == [
                    "--font-family",
                    "DejaVu Sans Mono",
                    "--font-fallback",
                    "DejaVu Sans",
                    "--font-fallback",
                    "Symbols Nerd Font Mono",
                    "--font-size",
                    "20.5",
                    "--line-height",
                    "1.5",
                    "--columns",
                    "100",
                    "--rows",
                    "30"
                ]));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_cursor_tail_reaches_both_product_launches() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for value in ["random", "preset:ice", "custom:#12ABCF"] {
            fs::write(
                root.join("config.toml"),
                format!(
                    "[terminal]\ncursor_trail_color = '{value}'\ncursor_trail_duration = 2.5\n"
                ),
            )
            .unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            assert!(terminal.requires_startup_admission());
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &root,
                    Path::new("/runtime.sock"),
                    mode,
                    &terminal,
                    inputs.defaults.bar_widgets,
                    "eon",
                    true,
                    true,
                    true,
                );
                let args = command.get_args().collect::<Vec<_>>();
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--cursor-trail-color", value]),
                    "cursor color did not reach Venus"
                );
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--cursor-trail-duration-v1", "2.5"])
                );
            }
        }

        for source in ["", "[terminal]\ncursor_trail_duration = 3.0\n"] {
            fs::write(root.join("config.toml"), source).unwrap();
            let mut defaults = crate::fixtures::inputs().defaults;
            defaults.terminal.cursor_trail_duration = Some(2.0);
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &defaults, None).unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &root,
                    Path::new("/runtime.sock"),
                    mode,
                    &terminal,
                    inputs.defaults.bar_widgets,
                    "eon",
                    true,
                    true,
                    true,
                );
                let args = command.get_args().collect::<Vec<_>>();
                let expected = if source.is_empty() { "2" } else { "3" };
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--cursor-trail-duration-v1", expected])
                );
                assert!(!terminal.requires_startup_admission());
            }
        }

        fs::write(root.join("config.toml"), "").unwrap();
        let terminal =
            crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                .unwrap();
        assert!(!terminal.requires_startup_admission());
        assert!(
            venus_command(
                &inputs,
                &root,
                Path::new("/runtime.sock"),
                LaunchMode::Workspace,
                &terminal,
                inputs.defaults.bar_widgets,
                "eon",
                true,
                true,
                true,
            )
            .get_args()
            .all(|argument| argument != "--cursor-trail-color")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn padding_color_configuration_reaches_each_launch() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for value in [
            None,
            Some("extend"),
            Some("extend-always"),
            Some("background"),
        ] {
            fs::write(
                root.join("config.toml"),
                value.map_or_else(String::new, |v| {
                    format!("[terminal]\npadding_color = \"{v}\"\n")
                }),
            )
            .unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &root,
                    Path::new("/runtime.sock"),
                    mode,
                    &terminal,
                    inputs.defaults.bar_widgets,
                    "eon",
                    true,
                    true,
                    true,
                );
                assert!(
                    command
                        .get_args()
                        .collect::<Vec<_>>()
                        .windows(2)
                        .any(|pair| pair == ["--padding-color", value.unwrap_or("extend")])
                );
            }
        }
        for value in [
            "\"bad\"",
            "true",
            "3",
            "\"Extend\"",
            "\"extend\"\npadding_color = \"background\"",
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[terminal]\npadding_color = {value}\n"),
            )
            .unwrap();
            assert!(
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains("padding_color")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_outline_widths_reach_only_workspace_launches() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for setting in ["border_width", "focus_border_width"] {
            let flag = format!("--{}", setting.replace('_', "-"));
            for value in [None, Some(1), Some(2), Some(3)] {
                fs::write(
                    root.join("config.toml"),
                    value.map_or_else(String::new, |value| {
                        format!("[terminal]\n{setting} = {value}\n")
                    }),
                )
                .unwrap();
                let terminal = crate::managed_environment::terminal_presentation(
                    &root,
                    &inputs.defaults,
                    None,
                )
                .unwrap();
                for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                    let command = venus_command(
                        &inputs,
                        &root,
                        Path::new("/runtime.sock"),
                        mode,
                        &terminal,
                        inputs.defaults.bar_widgets,
                        "eon",
                        true,
                        true,
                        true,
                    );
                    let args = command.get_args().collect::<Vec<_>>();
                    assert_eq!(
                        args.windows(2)
                            .find(|pair| pair[0] == flag.as_str())
                            .map(|pair| pair[1].to_str().unwrap().parse::<u8>().unwrap()),
                        value.filter(|_| mode == LaunchMode::Workspace)
                    );
                }
            }
            for value in ["0", "4", "1.5", "\"2\""] {
                fs::write(
                    root.join("config.toml"),
                    format!("[terminal]\n{setting} = {value}\n"),
                )
                .unwrap();
                assert!(
                    crate::managed_environment::terminal_presentation(
                        &root,
                        &inputs.defaults,
                        None
                    )
                    .is_err(),
                    "accepted {setting} = {value}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_focus_color_reaches_only_workspace_launches() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for value in [None, Some("preset:ice"), Some("custom:#12ABCF")] {
            fs::write(
                root.join("config.toml"),
                value.map_or_else(String::new, |value| {
                    format!("[terminal]\nfocus_color = '{value}'\n")
                }),
            )
            .unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &root,
                    Path::new("/runtime.sock"),
                    mode,
                    &terminal,
                    inputs.defaults.bar_widgets,
                    "eon",
                    true,
                    true,
                    true,
                );
                let args = command.get_args().collect::<Vec<_>>();
                assert_eq!(
                    args.windows(2)
                        .find(|pair| pair[0] == "--focus-color")
                        .map(|pair| pair[1].to_str().unwrap()),
                    value.filter(|_| mode == LaunchMode::Workspace)
                );
            }
            assert_eq!(terminal.requires_startup_admission(), value.is_some());
        }
        // Like cursor-trail colors, invalid strings must reach Venus's parser through
        // startup admission, before the runtime creates any fresh terminal.
        for value in ["random", "preset:unknown", "custom:#12345g"] {
            fs::write(
                root.join("config.toml"),
                format!("[terminal]\nfocus_color = '{value}'\n"),
            )
            .unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            assert!(
                terminal.requires_startup_admission(),
                "{value} bypasses Venus validation"
            );
            let command = venus_command(
                &inputs,
                &root,
                Path::new("/runtime.sock"),
                LaunchMode::Workspace,
                &terminal,
                inputs.defaults.bar_widgets,
                "eon",
                true,
                true,
                true,
            );
            assert!(
                command
                    .get_args()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|pair| pair == ["--focus-color", value])
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_pane_frames_reach_only_workspace_launches() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for (source, expected) in [
            ("", "true"),
            ("[terminal]\npane_frames = true\n", "true"),
            ("[terminal]\npane_frames = false\n", "false"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &root,
                    Path::new("/runtime.sock"),
                    mode,
                    &terminal,
                    inputs.defaults.bar_widgets,
                    "eon",
                    true,
                    true,
                    true,
                );
                let args = command.get_args().collect::<Vec<_>>();
                let values = args
                    .windows(2)
                    .filter(|pair| pair[0] == "--pane-frames")
                    .map(|pair| pair[1])
                    .collect::<Vec<_>>();
                if mode == LaunchMode::Workspace {
                    assert_eq!(values, [expected], "{source}");
                } else {
                    assert!(values.is_empty());
                }
            }
        }
        for value in ["\"false\"", "0", "true\npane_frames = false"] {
            fs::write(
                root.join("config.toml"),
                format!("[terminal]\npane_frames = {value}\n"),
            )
            .unwrap();
            assert!(
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains("pane_frames")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn venus_receives_workspace_endpoint_only_in_workspace_mode() {
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = "/managed/orbit".into();
        inputs.venus = "/managed/venus".into();
        inputs.managed.shell_bin = None;
        inputs.defaults.bar_widgets = &["cpu"];
        let config = Path::new("/config/eon");
        let socket = Path::new("/runtime/eon.sock");

        let workspace = venus_command(
            &inputs,
            config,
            socket,
            LaunchMode::Workspace,
            &terminal_presentation(0.88, true),
            inputs.defaults.bar_widgets,
            "eon",
            true,
            false,
            false,
        );
        assert_eq!(
            workspace.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--application-id",
                "eon",
                "--background-opacity",
                "0.88",
                "--padding-color",
                "extend",
                "--background-blur",
                "--bar-items",
                "cpu",
                "--pane-frames",
                "true",
                "--warning-close-last-tab",
                "false",
                "--animate-working-badge",
                "false",
                "--workspace",
                "/runtime/eon.sock",
            ]
            .map(OsString::from)
        );

        let terminal = venus_command(
            &inputs,
            config,
            Path::new("/runtime/orbit.sock"),
            LaunchMode::Terminal,
            &terminal_presentation(1.0, false),
            inputs.defaults.bar_widgets,
            "eonbase",
            true,
            true,
            true,
        );
        for command in [&workspace, &terminal] {
            let environment = command.get_envs().collect::<Vec<_>>();
            assert!(environment.iter().all(|(key, _)| *key != "XDG_CONFIG_HOME"));
            assert!(environment.iter().any(|(key, value)| {
                *key == "EON_CONFIG_HOME" && *value == Some(config.as_os_str())
            }));
        }
        assert_eq!(
            terminal.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--application-id",
                "eonbase",
                "--background-opacity",
                "1",
                "--padding-color",
                "extend",
                "/runtime/orbit.sock",
            ]
            .map(OsString::from)
        );

        let undecorated = venus_command(
            &inputs,
            config,
            Path::new("/runtime/orbit.sock"),
            LaunchMode::Terminal,
            &terminal_presentation(0.0, true),
            inputs.defaults.bar_widgets,
            "eonova",
            false,
            true,
            true,
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
                "--padding-color",
                "extend",
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
