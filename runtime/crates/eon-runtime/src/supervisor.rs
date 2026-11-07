use super::{
    attention::{self, Attention},
    cli::Inputs,
    control::{
        Accepted, ControlListener, ControlResponse, Dispatched, EndpointFailure,
        EndpointFailureKind, Incoming, SocketIdentity, connect_control, failure, probe_launch_mode,
        probe_presentable_runtime, send_action_on, socket_identity, stop_failure,
    },
    diagnostics::DiagnosticLog,
    events::Projection,
    generation::{current_generation, generation_directory, record_launcher, remove_launcher},
    managed_environment::{self, PopupCommand, TerminalAccess, nonempty_environment_path},
    notification, pair, platform, quota,
    running::Running,
    scrollback::{self, Snapshots},
    sessions::{
        self, AttachName, RunningSession, TAB_VARIABLE, TERMINAL_VARIABLE,
        acquire_generation_sessions, bound_io, name_presentation, recover_sessions,
        session_finished, session_finished_within, start_orbit, stop_managed_sessions,
    },
    subscriptions::{Subscriber, Subscribers},
    workspace::{self, LaunchCommand, SessionEnd, SessionOperation, WorkspaceSet},
};
use eon_workspace_protocol::v37::{
    Action, Availability, BusyTerminals, Editing, Ended, FILES_ENTRY, Failure, InvokeIntent,
    LifecycleResponse, Request, Response, Runtime, Screen, Sent, Snapshot, TargetId,
    TerminalHistory, TerminalInput, TerminalText, VERSION, WindowEntry, WindowState,
    WorkspaceAction,
};
use orbit_protocol::management::{LeaseInput, Screen as OrbitScreen};
use std::{
    borrow::Cow,
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::fd::OwnedFd,
    os::unix::{
        ffi::OsStrExt,
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
/// How long an ending supervisor waits to close its detach notice (EON-C8).
const NOTICE_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
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
            Self::Workspace => "eon",
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
        let bar = window_bar(mode, &config, inputs, Some(&log))?;
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
            &bar,
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

/// The `[bar]` settings for one window open, whose widgets both its bar and the quota readers
/// follow; a terminal window has no bar.
fn window_bar(
    mode: LaunchMode,
    config: &Path,
    inputs: &Inputs,
    log: Option<&DiagnosticLog>,
) -> Result<managed_environment::Bar, String> {
    match mode {
        LaunchMode::Workspace => managed_environment::bar(config, &inputs.defaults, log),
        LaunchMode::Terminal => Ok(managed_environment::Bar {
            widgets: Vec::new(),
            load: inputs.defaults.bar_load,
            disk_path: None,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn venus_command(
    inputs: &Inputs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: &managed_environment::TerminalConfig,
    bar: &managed_environment::Bar,
    application_id: &str,
    decorations: bool,
    warning_close_last_tab: bool,
    animate_working_badge: bool,
    number_keys: bool,
) -> Command {
    let mut command = Command::new(&inputs.venus);
    managed_environment::remove_agent_session(&mut command)
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
        let shown = |widget| bar.widgets.contains(&widget);
        if shown("version") {
            command.arg("--eon-version").arg(inputs.version);
        }
        if let Some(icon) = &inputs.icon {
            command.arg("--eon-icon").arg(icon);
        }
        let load = &bar.load;
        if shown("ram") {
            command
                .arg("--ram-warning-percent")
                .arg(load.ram_warning_percent.to_string())
                .arg("--ram-critical-percent")
                .arg(load.ram_critical_percent.to_string());
        }
        if shown("cpu") {
            command
                .arg("--cpu-warning-percent")
                .arg(load.cpu_warning_percent.to_string())
                .arg("--cpu-average-seconds")
                .arg(load.cpu_average_seconds.to_string());
        }
        if shown("disk") {
            command
                .arg("--disk-warning-percent")
                .arg(load.disk_warning_percent.to_string())
                .arg("--disk-critical-percent")
                .arg(load.disk_critical_percent.to_string());
            if let Some(path) = &bar.disk_path {
                command.arg("--disk-path").arg(path);
            }
        }
        command.arg("--bar-items").arg(bar.widgets.join(","));
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
            .arg("--number-keys")
            .arg(number_keys.to_string())
            .arg("--workspace");
    }
    command.arg(socket);
    command.env("EON_CONFIG_HOME", config);
    command
}

/// The bytes of one native focus report from Venus (VEN-C14).
const FOCUS_REPORT: usize = 8;
/// The most report bytes one loop turn reads; the rest waits for the next
/// turn, so a burst of reports cannot hold up the loop.
const FOCUS_READ_BUDGET: usize = 512;

/// The native window focus its Venus last reported on the presentation
/// control stream (EON-C31). It is unknown before the first report and once
/// reports fail.
#[derive(Default)]
struct NativeFocus {
    focused: Option<bool>,
    report: [u8; FOCUS_REPORT],
    used: usize,
    ended: bool,
}

enum FocusFailure {
    Invalid,
    Ended,
    Io(io::Error),
}

impl NativeFocus {
    /// Reads the reports `stream` holds now. A failure leaves focus unknown
    /// and ends reports from this Venus.
    fn poll(&mut self, stream: &UnixStream) -> Result<(), FocusFailure> {
        if self.ended {
            return Ok(());
        }
        let result = self.read(stream);
        if result.is_err() {
            self.ended = true;
            self.focused = None;
        }
        result
    }

    fn read(&mut self, stream: &UnixStream) -> Result<(), FocusFailure> {
        let mut buffer = [0; 64];
        let mut budget = FOCUS_READ_BUDGET;
        while budget > 0 {
            let limit = buffer.len().min(budget);
            let count = match platform::read_available(stream, &mut buffer[..limit]) {
                Ok(0) => return Err(FocusFailure::Ended),
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(FocusFailure::Io(error)),
            };
            budget -= count;
            for &byte in &buffer[..count] {
                self.report[self.used] = byte;
                self.used += 1;
                if self.used == FOCUS_REPORT {
                    self.used = 0;
                    self.focused = Some(match &self.report {
                        b"focus:0\n" => false,
                        b"focus:1\n" => true,
                        _ => return Err(FocusFailure::Invalid),
                    });
                }
            }
        }
        Ok(())
    }
}

struct PresentationProcess {
    child: Child,
    /// The name each terminal's Orbit admits to attach, read at spawn; none
    /// if it could not be read, which leaves every terminal unattached
    /// (EON-C11).
    name: Option<AttachName>,
    control: UnixStream,
    log: Arc<DiagnosticLog>,
    relay: Option<JoinHandle<()>>,
    reported: bool,
    /// This process's own reports; a replacement starts unknown.
    focus: NativeFocus,
}

impl PresentationProcess {
    /// Starts Venus and names it to `sessions` before it can learn of them
    /// (EON-C11).
    fn start(
        mut command: Command,
        admission: bool,
        snapshot: Option<Snapshot>,
        timeout: Duration,
        sessions: &mut [RunningSession],
        log: &Arc<DiagnosticLog>,
    ) -> Result<Self, String> {
        let deadline = Instant::now() + timeout;
        let (control, input) = UnixStream::pair()
            .map_err(|error| format!("cannot create Eon Desktop presentation control: {error}"))?;
        command
            .stdout(Stdio::from(OwnedFd::from(input.try_clone().map_err(
                |error| format!("cannot create Eon Desktop presentation return channel: {error}"),
            )?)))
            .env(
                "EON_VENUS_PRESENTATION_CONTROL",
                if admission {
                    "stdin-ready-focus-v1"
                } else {
                    "stdin-focus-v1"
                },
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
        let name = AttachName::of(child.id())
            .inspect_err(|error| {
                log.note(&format!(
                    "Cannot let Eon Desktop attach to its terminals: {error}"
                ));
            })
            .ok();
        let stderr = child.stderr.take().expect("Eon Desktop stderr is piped");
        let mut process = Self {
            child,
            name,
            control,
            log: Arc::clone(log),
            relay: None,
            reported: false,
            focus: NativeFocus::default(),
        };
        process.relay = Some(
            log.relay(stderr)
                .map_err(|error| format!("cannot relay Eon Desktop diagnostics: {error}"))?,
        );
        name_presentation(sessions, name, true, log);
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
                        eon_workspace_protocol::v37::encode_response(&Response::Snapshot(snapshot))
                            .map_err(|error| format!("cannot encode startup snapshot: {error}"))?;
                    let mut remaining = bytes.as_slice();
                    while !remaining.is_empty() {
                        bound_io(
                            &process.control,
                            process.control.set_write_timeout(Some(time_left()?)),
                        )
                        .map_err(|error| error.to_string())?;
                        match process.control.write(remaining) {
                            Ok(0) => {
                                return Err("Venus closed while receiving startup snapshot".into());
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
                    bound_io(
                        &process.control,
                        process.control.set_read_timeout(Some(time_left()?)),
                    )
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
        bound_io(
            &process.control,
            process
                .control
                .set_write_timeout(Some(Duration::from_millis(250))),
        )
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

    /// Reads this Venus's native focus reports, recording once when they fail.
    fn read_native_focus(&mut self) {
        let failure = match self.focus.poll(&self.control) {
            Ok(()) => return,
            Err(FocusFailure::Invalid) => "Eon Desktop focus reports are invalid".to_owned(),
            // Reports end with an exited Venus, which is no failure.
            Err(FocusFailure::Ended) if matches!(self.child.try_wait(), Ok(Some(_))) => return,
            Err(FocusFailure::Ended) => "Eon Desktop focus reports ended".to_owned(),
            Err(FocusFailure::Io(error)) => format!("Eon Desktop focus reports failed: {error}"),
        };
        self.log
            .record(&format!("{failure}; treating the window as unfocused"));
    }

    /// Whether this Venus last reported that its native window has focus.
    fn viewing(&self) -> bool {
        self.focus.focused == Some(true)
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
                session.number,
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
    bar: &managed_environment::Bar,
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
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("cannot record supervisor start: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| "supervisor start time exceeds Unix milliseconds")?;
    let root = runtime
        .parent()
        .and_then(Path::parent)
        .expect("generation runtime has a root");
    let window = super::windows::runtime_window(root);
    let running = (mode == LaunchMode::Workspace)
        .then(|| {
            Running::start(
                base_runtime_directory("eon"),
                root,
                generation,
                Arc::clone(log),
            )
        })
        .transpose()?;
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
    let number_keys = managed_environment::number_keys(config, &inputs.defaults, Some(log))?;
    let agents = managed_environment::agents(config, &inputs.defaults, Some(log))?;
    // Each edit rereads `[editor]`; a malformed one fails the start as other settings do.
    if mode == LaunchMode::Workspace {
        managed_environment::editor_command(config, &inputs.defaults, Some(log))?;
    }
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
    let (mut sessions, recovered_pair) = recover_sessions(
        runtime,
        mode,
        generation,
        log.orbit_directory().ok(),
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    let mut scrollback = Snapshots::new(runtime);
    scrollback.sweep(sessions.iter().map(|session| session.id.as_str()), log);
    let control_listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    let initial_endpoint = runtime.join("orbit.sock");
    let initial_session_id = "terminal-1";
    let presentation_socket = if mode == LaunchMode::Workspace {
        runtime.join("eon.sock")
    } else {
        sessions.first().map_or_else(
            || initial_endpoint.clone(),
            |session| session.endpoint.clone(),
        )
    };
    let fresh_animation = startup_animation.is_some() && sessions.is_empty() && !recovered_pair;
    let admitted = if terminal.requires_startup_admission() || fresh_animation {
        let snapshot = if mode == LaunchMode::Workspace {
            let workspaces = if sessions.is_empty() && !recovered_pair {
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
            };
            let mut snapshot = workspaces.snapshot();
            if let Some(running) = &running {
                running.apply(
                    &mut snapshot,
                    WindowEntry {
                        window: window.clone(),
                        generation: generation.into(),
                        name: Some(workspaces.window_name()),
                        started_at: Some(started_at),
                        live_terminals: Some(
                            live_sessions(&sessions, Some(&workspaces)).count() as u16
                        ),
                        state: WindowState::Live,
                    },
                );
            }
            Some(snapshot)
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
            bar,
            application_id,
            decorations,
            warning_close_last_tab,
            animate_working_badge,
            number_keys,
        );
        Some(PresentationProcess::start(
            command,
            true,
            snapshot,
            PRESENTATION_ADMISSION_TIMEOUT,
            &mut sessions,
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
    if sessions.is_empty() && (mode == LaunchMode::Terminal || recovered_pair) {
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
                        config,
                        &inputs.defaults,
                        Some(log),
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
                        PairStart::Fresh(startup_animation.as_ref()),
                        admitted.as_ref().and_then(|venus| venus.name),
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
        started_at,
        window,
        running,
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
            .then(|| quota::Quotas::start(&inputs.codexbar, &bar.widgets)),
        attention,
        pane_labels: super::pane_labels::Labels::default(),
        agents,
        notices: notification::Notices::default(),
        subscribers: Subscribers::new(generation),
        scrollback,
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
                bar,
                application_id,
                decorations,
                warning_close_last_tab,
                animate_working_badge,
                number_keys,
            ),
            false,
            None,
            PRESENTATION_ADMISSION_TIMEOUT,
            &mut state.sessions,
            log,
        ),
    };
    state.venus = match presentation {
        Ok(venus) => Some(venus),
        Err(error) => {
            let session_cleanup = stop_managed_sessions(&mut state.sessions, SESSION_START_TIMEOUT);
            // A recovered Scrollback editor that stopped leaves no snapshot.
            state.retain_snapshots();
            state.scrollback.remove_empty_root();
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
    // Terminals started after an admitted Venus spawned.
    state.name_presentation(true);

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
                        "Eon Desktop exited; terminals remain active. Run `eon window attach {}` to reconnect.",
                        super::windows::durable_window_id(state.window.as_deref(), generation)
                    )),
                    LaunchMode::Terminal => log.note(&format!(
                        "Eon Desktop exited; terminal remains active. Run `eonbase attach {generation}` to reconnect."
                    )),
                }
            }

            state.tend(Instant::now());
            // Also while an incomplete Stop waits, so the notice's confirmed
            // Quit retries it.
            if answer_detach_notice(&mut state, runtime, mode) {
                return Ok(state.initial_status.unwrap_or(0));
            }

            let accepted = control_listener.accept(|request| {
                dispatch(request, &mut state, inputs, config, runtime, generation)
            })?;
            state.end_turn(Instant::now());
            match accepted {
                Accepted::None => thread::sleep(Duration::from_millis(25)),
                Accepted::Replied { shutdown: true } => {
                    return Ok(state.initial_status.unwrap_or(0));
                }
                Accepted::Replied { shutdown: false } => {}
                Accepted::Subscribed(stream, subscriber) => {
                    state
                        .subscribers
                        .attach(stream, *subscriber, Instant::now());
                }
            }
        }
    })();
    // Stop, quit and the window's final close end every subscription.
    state.subscribers.end(generation);
    drop(control_listener);
    if let Some(process) = state.venus.take() {
        drop(process);
    }
    state.notices.close_and_wait(NOTICE_CLOSE_TIMEOUT);
    state.retain_snapshots();
    state.scrollback.remove_empty_root();
    remove_launcher(runtime);
    let _ = fs::remove_dir(runtime);
    status
}

struct SupervisorState {
    started_at: u64,
    window: Option<String>,
    running: Option<Running>,
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
    /// The window's `[agents]` settings, read when it started.
    agents: managed_environment::Agents,
    /// The detach notice; full Eon only shows one (EON-C8).
    notices: notification::Notices,
    /// Event subscribers and the stream they read (EON-C39).
    subscribers: Subscribers,
    /// Scrollback popups' private snapshots (EON-C10).
    scrollback: Snapshots,
    log: Arc<DiagnosticLog>,
}

impl SupervisorState {
    /// The name the running Venus attaches by (EON-C11).
    fn presentation_name(&self) -> Option<AttachName> {
        self.venus.as_ref().and_then(|venus| venus.name)
    }

    /// Names the running Venus to each terminal not yet named to it.
    fn name_presentation(&mut self, wait: bool) {
        let venus = self.presentation_name();
        name_presentation(&mut self.sessions, venus, wait, &self.log);
    }

    /// This full-Eon window as EON-C23's `this` entry describes it.
    fn this_window(&self) -> Option<WindowEntry> {
        Some(WindowEntry {
            window: self.window.clone(),
            generation: self.component_generation.clone(),
            name: Some(self.workspaces.as_ref()?.window_name()),
            started_at: Some(self.started_at),
            live_terminals: Some(
                live_sessions(&self.sessions, self.workspaces.as_ref()).count() as u16,
            ),
            state: WindowState::Live,
        })
    }

    /// What this window's detach notice says (EON-C8).
    fn notice_window(&self) -> Option<notification::NoticeWindow> {
        self.this_window()
            .map(|this| notification::NoticeWindow::new(&this))
    }

    /// The shown workspace with its quotas and attention.
    fn workspace_snapshot(&mut self) -> Option<Snapshot> {
        self.workspaces.as_ref()?;
        self.workspace_snapshot_in(None).ok()
    }

    fn workspace_snapshot_in(&mut self, workspace: Option<&str>) -> Result<Snapshot, Failure> {
        let workspaces = self
            .workspaces
            .as_ref()
            .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
        // An accepted action may remove its caller's workspace.
        let mut snapshot = workspaces.snapshot_in(workspace).or_else(|error| {
            if error.code == "unknown-session" {
                workspaces.snapshot_in(None)
            } else {
                Err(error)
            }
        })?;
        if let (Some(running), Some(this)) = (&self.running, self.this_window()) {
            running.apply(&mut snapshot, this);
        }
        self.pane_labels.present(&mut snapshot, workspaces.home());
        if let Some(quotas) = &self.quotas {
            quotas.apply(&mut snapshot);
        }
        if let Some(attention) = &mut self.attention {
            attention.present(
                &mut snapshot,
                &workspaces.snapshots(),
                self.venus
                    .as_ref()
                    .is_some_and(PresentationProcess::viewing),
                Instant::now(),
            );
        }
        snapshot.busy = self.busy(None);
        Ok(snapshot)
    }

    /// The window's busy terminals without `exclude`, derived from the
    /// poller's cached replies; eonbase has none (EON-C11).
    fn busy(&self, exclude: Option<&str>) -> Option<BusyTerminals> {
        let (Some(attention), Some(workspaces)) = (&self.attention, &self.workspaces) else {
            return None;
        };
        let mut snapshots = workspaces.snapshots();
        for snapshot in &mut snapshots {
            self.pane_labels.present(snapshot, workspaces.home());
        }
        Some(attention.busy(&snapshots, &self.sessions, exclude, Instant::now()))
    }

    /// The window now as EON-C39 projects it: every workspace prepared as
    /// an inspection prepares it, with the same `viewing`, so a subscriber
    /// never makes a raise seen that Inspect would not, and every busy
    /// terminal without EONW's bound. Eonbase has none.
    fn projection(&mut self) -> Option<Projection> {
        let workspaces = self.workspaces.as_ref()?;
        let viewing = self
            .venus
            .as_ref()
            .is_some_and(PresentationProcess::viewing);
        let now = Instant::now();
        let every = workspaces.snapshots();
        let mut prepared = Vec::with_capacity(every.len());
        for own in &every {
            let mut snapshot = workspaces.snapshot_in(Some(&own.workspace)).ok()?;
            if let (Some(running), Some(this)) = (&self.running, self.this_window()) {
                running.apply(&mut snapshot, this);
            }
            self.pane_labels.present(&mut snapshot, workspaces.home());
            if let Some(attention) = &mut self.attention {
                attention.present(&mut snapshot, &every, viewing, now);
            }
            prepared.push(snapshot);
        }
        let busy = match &self.attention {
            Some(attention) => {
                let mut labelled = every;
                for snapshot in &mut labelled {
                    self.pane_labels.present(snapshot, workspaces.home());
                }
                attention.busy_terminals(&labelled, &self.sessions, now)
            }
            None => Vec::new(),
        };
        Some(Projection::new(&prepared, busy))
    }

    /// Removes the snapshots of Scrollback popups whose terminals ended
    /// (EON-C10).
    fn retain_snapshots(&mut self) {
        let sessions = &self.sessions;
        self.scrollback.retain(
            |terminal, uncertain| {
                sessions
                    .iter()
                    .any(|session| session.id == terminal && !session.is_finished())
                    || uncertain.is_some_and(sessions::launch_record_remains)
            },
            &self.log,
        );
    }

    /// The loop turn's work before its control request: follows the window's
    /// terminals, focus and attention. The turn publishes it to subscribers
    /// once, with its action's change (EON-C39).
    fn tend(&mut self, now: Instant) {
        // Read first, so attention this turn sees the latest native focus.
        if self.attention.is_some()
            && let Some(venus) = &mut self.venus
        {
            venus.read_native_focus();
        }
        self.retain_snapshots();
        if !self.stopping {
            sessions::drain_given_up_requests(&mut self.sessions);
            // The backstop for a naming that failed or came late.
            self.name_presentation(false);
            self.follow_attention();
            if let Some(running) = &mut self.running {
                running.tick(now, || {
                    (
                        self.workspaces
                            .as_ref()
                            .expect("full Eon has a workspace")
                            .window_name(),
                        live_sessions(&self.sessions, self.workspaces.as_ref()).count() as u16,
                    )
                });
            }
            if let Some(workspaces) = &self.workspaces {
                self.pane_labels.reconcile(&workspaces.pane_sessions());
                // A starting quick search stops counting as shown once its fzf
                // listens or its pair is hidden (EON-C18).
                if pair::any_starting() {
                    for (session, shown) in workspaces.pairs() {
                        if let Some(running) = self.sessions.iter().find(|item| item.id == session)
                        {
                            pair::settle_starting(&pair::socket(&running.endpoint), shown);
                        }
                    }
                }
            }
        } else {
            self.pane_labels.reconcile(&[]);
        }
    }

    /// Ends a loop turn: publishes its changes to subscribers, unless the
    /// turn's action already did, after it was applied and before its reply
    /// (EON-C39). So a turn publishes once, attention the poller applied
    /// included.
    fn end_turn(&mut self, now: Instant) {
        if !self.subscribers.take_served() {
            self.serve_subscribers(now);
            self.subscribers.take_served();
        }
    }

    /// Publishes the changes since it last ran to the subscribers and writes
    /// to them without waiting; with none, it does nothing (EON-C39). While a Stop is incomplete it
    /// publishes nothing but still writes, and ends subscribers that hung up
    /// or went too slow.
    fn serve_subscribers(&mut self, now: Instant) {
        if self.stopping {
            self.subscribers.maintain(now);
        } else if !self.subscribers.is_empty()
            && let Some(projection) = self.projection()
        {
            self.subscribers.turn(projection, now);
        }
        self.subscribers.idle();
    }

    /// Follows focus, then polls program status, between control requests
    /// and never inside one (EON-C31).
    fn follow_attention(&mut self) {
        let (Some(attention), Some(workspace)) = (&mut self.attention, &self.workspaces) else {
            return;
        };
        let (targets, focus) = attention::targets(&workspace.snapshots());
        attention.observe(
            focus,
            self.venus
                .as_ref()
                .is_some_and(PresentationProcess::viewing),
        );
        attention.poll(&targets, &mut self.sessions, Instant::now());
        attention.retain(&targets, &self.sessions);
    }
}

fn report_session_end(result: Result<(), Failure>, log: &DiagnosticLog) -> Result<(), String> {
    match result {
        Err(error) if error.code == "terminal-start" => {
            // A pending tab whose first pane fails must not tear down other tabs' terminals.
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
            let venus = state.presentation_name();
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
                            PairStart::Later,
                            venus,
                        )
                    });
                report_session_end(result, &state.log)?;
            }
            if session.id == "terminal-1" {
                state.initial_status = Some(code);
            }
        } else {
            index += 1;
        }
    }
    Ok(())
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
            .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
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
        .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
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
        .map_err(|detail| failure("session-close-failed", detail))?;
    let caller = caller_session(state, inputs, config, request)?;
    let workspace = state
        .workspaces
        .as_mut()
        .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
    let reply_workspace = workspace.caller_workspace_id(caller.as_deref())?;
    let sessions = workspace.prepare_close_workspace(&request.id, caller.as_deref(), target)?;
    stop_closed_sessions(state, sessions, "session-close-failed")?;
    closed_response(
        state,
        runtime,
        generation,
        "session-close-failed",
        reply_workspace.as_deref(),
    )
}

/// Stops a close's terminals together and prunes each confirmed end in close
/// order, so a failed stop leaves exactly the unended terminals represented
/// (EON-C8).
fn stop_closed_sessions(
    state: &mut SupervisorState,
    sessions: Vec<String>,
    code: &str,
) -> Result<(), Failure> {
    let (ended, errors) = stop_owned_sessions(&mut state.sessions, &sessions, &state.log);
    for (id, natural_status) in ended {
        state
            .workspaces
            .as_mut()
            .expect("workspace close retains its owner")
            .session_ended(&id, SessionEnd::Closed, |_| {
                Err("a close cannot start a replacement terminal".into())
            })?;
        if id == "terminal-1" && natural_status.is_some() {
            state.initial_status = natural_status;
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(failure(code, errors.join("; ")))
    }
}

/// A close that stopped the window's last terminal ends its generation, and
/// leaves no workspace to snapshot. As Stop does, it first stops any run of
/// the generation that no tab tracks, such as a terminal that became ready
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

/// Answers the detach notice's actions (EON-C8). Quit first asks through the
/// notice's own "Quit NAME?" step; its confirmed Quit is the person's consent,
/// so this runs Stop directly and never relies on `eon quit`'s prompt. True
/// when the window stopped; a failed Stop says so in a notice that names
/// the window's stop command for the retry, and keeps the supervisor for it,
/// as an incomplete Stop does.
fn answer_detach_notice(state: &mut SupervisorState, runtime: &Path, mode: LaunchMode) -> bool {
    while let Some(event) = state.notices.next_event() {
        let Some(window) = state.notice_window() else {
            continue;
        };
        match event {
            // A Quit chosen just before the window was attached again: the
            // notice is closing, so no confirmation follows it on screen.
            notification::Event::Confirm if state.venus.is_some() => {}
            notification::Event::Confirm => state.notices.confirm(window),
            notification::Event::Quit => {
                state
                    .log
                    .note("Quitting this window from its detach notice.");
                match stop_generation_sessions(state, runtime, mode) {
                    Ok(_) => return true,
                    Err(detail) => {
                        state.log.note(&format!(
                            "cannot quit this window from its detach notice: {detail}"
                        ));
                        state.notices.failure(window);
                        return false;
                    }
                }
            }
        }
    }
    false
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
    // Stop stops a pending tab's pair but does not report it (EON-C18).
    let pending = state
        .workspaces
        .as_ref()
        .map_or_else(Vec::new, WorkspaceSet::pending_pair_sessions);
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
    Ok(stopped
        .into_iter()
        .filter(|session| !pending.contains(session))
        .collect())
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
                "the calling terminal is not live in this Eon session",
            )
        })
}

/// Whether a pair starts the window: then its terminal plays the startup
/// Anima first and its quick search names the other running windows.
enum PairStart<'a> {
    Fresh(Option<&'a managed_environment::StartupAnimation>),
    Later,
}

/// The packaged Files pair's command for terminal `session`, from its
/// configured `argv`. The product's Files is always the packaged `eon-files`
/// (EON-C10); only test fixtures' stand-in commands run as given.
fn pair_command(
    argv: Vec<OsString>,
    quick: bool,
    session: &workspace::Session,
    start: PairStart<'_>,
) -> Result<Vec<OsString>, String> {
    if argv != [OsStr::new("eon-files")] {
        return Ok(argv);
    }
    let runtime = session
        .endpoint
        .parent()
        .ok_or("pair endpoint has no runtime directory")?;
    let mut command = argv;
    let mut delay = Duration::ZERO;
    if let PairStart::Fresh(animation) = start {
        // Its Yazi, and so quick search, starts once these end.
        delay = pair::fresh_start(animation.map(|animation| animation.duration_seconds));
        if let Some(animation) = animation {
            command.extend([
                OsString::from("--eon-anima"),
                animation.style.clone().into(),
                animation.duration_seconds.to_string().into(),
            ]);
        }
        command.extend([
            OsString::from("--eon-notice"),
            runtime.join("eon.sock").into_os_string(),
        ]);
    }
    let socket = pair::socket(&session.endpoint);
    if quick {
        command.push("--eon-quick".into());
        pair::quick_search_requested(&socket, delay);
    }
    let id = pair::client_id(&session.endpoint);
    command.extend([
        OsString::from("--eon-pair-socket"),
        socket.into_os_string(),
        OsString::from("--client-id"),
        id.to_string().into(),
    ]);
    Ok(command)
}

/// A committed tab directory is a pick (EON-C17), so it adds one Zoxide visit
/// unless `[shell] zoxide = false`. Zoxide runs and is reaped on its own worker
/// beside the supervisor; any failure, the worker's included, is logged and
/// changes nothing else.
fn record_visit(inputs: &Inputs, config: &Path, log: &Arc<DiagnosticLog>, directory: &[u8]) {
    let Some(session_bin) = &inputs.managed.shell_bin else {
        return log.record("Zoxide visit skipped: this Eon has no packaged Zoxide");
    };
    match managed_environment::zoxide_enabled(config, &inputs.defaults, None) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => return log.record(&format!("Zoxide visit skipped: {error}")),
    }
    let mut zoxide = Command::new(session_bin.join("zoxide"));
    zoxide
        .args(["add", "--"])
        .arg(OsStr::from_bytes(directory))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let worker = Arc::clone(log);
    let started = thread::Builder::new().spawn(move || {
        let failure = match zoxide.status() {
            Ok(status) if status.success() => return,
            Ok(status) => format!("zoxide {status}"),
            Err(error) => error.to_string(),
        };
        worker.record(&format!("Zoxide visit failed: {failure}"));
    });
    if let Err(error) = started {
        log.record(&format!("Zoxide visit failed: {error}"));
    }
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
    pair_start: PairStart<'_>,
    venus: Option<AttachName>,
) -> Result<(), String> {
    let (session, directory, command, tab) = match operation {
        SessionOperation::Start {
            session,
            directory,
            command,
            tab,
        } => (session, directory, command, tab),
        SessionOperation::Stop(id) => return stop_owned_session(sessions, &id, log).map(|_| ()),
        SessionOperation::SettlePair(id) => {
            if let Some(session) = sessions.iter().find(|session| session.id == id)
                && let Err(error) = sessions::settle_pending_pair(&session.endpoint)
            {
                // Recovery would stop this Files instead of adopting it.
                log.record(&error);
            }
            return Ok(());
        }
    };
    let mut pending = false;
    let child = match command {
        LaunchCommand::Files {
            argv,
            quick,
            pending: marked,
        } => {
            pending = marked;
            pair_command(argv, quick, &session, pair_start)?
        }
        LaunchCommand::Tool(argv) => argv,
        LaunchCommand::Pane if session.id == "terminal-1" => initial_child.clone(),
        LaunchCommand::Pane => Vec::new(),
    };
    if pending {
        sessions::mark_pending_pair(&session.endpoint)?;
    }
    let started = start_orbit(
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
    );
    let mut running = match started {
        Ok(running) => running,
        Err(error) => {
            // A rollback that could not confirm its cleanup may leave the pair
            // running: its marker stays, so a recovery stops it rather than
            // adopting it, and the record's cleanup removes the marker later.
            if pending
                && !sessions::launch_record_remains(&session.endpoint)
                && let Err(marker) = sessions::settle_pending_pair(&session.endpoint)
            {
                log.record(&marker);
            }
            return Err(error);
        }
    };
    // Before the reply that shows Venus the terminal (EON-C11).
    name_presentation(std::slice::from_mut(&mut running), venus, true, log);
    sessions.push(running);
    if session.id == "terminal-1" {
        initial_child.clear();
    }
    Ok(())
}

fn stop_owned_session(
    sessions: &mut Vec<RunningSession>,
    id: &str,
    log: &DiagnosticLog,
) -> Result<Option<i32>, String> {
    let (ended, errors) = stop_owned_sessions(sessions, &[id.to_owned()], log);
    match ended.into_iter().next() {
        Some((_, status)) => Ok(status),
        None => Err(errors.join("; ")),
    }
}

/// Stops the live terminals `ids` names together and reconciles the ends any
/// reached first, all under one shared bound, and removes each ended
/// terminal. Returns the ended terminals in `ids` order with a natural end's
/// status (`None` for a confirmed Stop), and why each other terminal stays
/// tracked. The caller prunes an ended terminal before a reap would see it,
/// so an Orbit lost on the way is recorded here.
fn stop_owned_sessions(
    sessions: &mut Vec<RunningSession>,
    ids: &[String],
    log: &DiagnosticLog,
) -> (Vec<(String, Option<i32>)>, Vec<String>) {
    // Reconciling, stopping and reconciling after a failed stop share one
    // bound (EON-C8).
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    let remaining = || deadline.saturating_duration_since(Instant::now());
    let mut errors = ids
        .iter()
        .filter(|id| !sessions.iter().any(|session| &session.id == *id))
        .map(|id| format!("terminal {id} is missing from the supervisor"))
        .collect::<Vec<_>>();
    let (closing, kept): (Vec<_>, Vec<_>) = std::mem::take(sessions)
        .into_iter()
        .enumerate()
        .partition(|(_, session)| ids.contains(&session.id));
    // A live lease is stopped. One that already closed belongs to a run that
    // ended first: it is reconciled, without a Stop, once the stops are sent.
    let mut settled = Vec::new();
    let mut stopping = Vec::new();
    let mut lagging = Vec::new();
    for (index, mut session) in closing {
        match session_finished_within(&mut session, Duration::ZERO) {
            Ok(None) => stopping.push((index, session)),
            Ok(status) => settled.push((index, session, status)),
            Err(_) => lagging.push((index, session)),
        }
    }
    let (positions, mut runs): (Vec<_>, Vec<_>) = stopping.into_iter().unzip();
    if !runs.is_empty()
        && let Err(mut detail) = stop_managed_sessions(&mut runs, remaining())
    {
        // A terminal that ended by itself while its Stop failed is an exit.
        let mut unconfirmed = false;
        for session in runs.iter_mut().filter(|session| !session.is_finished()) {
            if let Err(error) = session_finished_within(session, remaining()) {
                detail.push_str(&format!(
                    "; {}: cannot reconcile terminal after failed stop: {error}",
                    session.id
                ));
            }
            if !session.is_finished() {
                unconfirmed = true;
                detail.push_str(&session.orbit_log_suffix());
            }
        }
        if unconfirmed {
            errors.push(detail);
        }
    }
    for (index, mut session) in lagging {
        let status = match session_finished_within(&mut session, remaining()) {
            Ok(Some(status)) => Some(status),
            Ok(None) => {
                errors.push(format!(
                    "{}: terminal end was not confirmed{}",
                    session.id,
                    session.orbit_log_suffix()
                ));
                None
            }
            Err(error) => {
                errors.push(format!(
                    "{}: {error}{}",
                    session.id,
                    session.orbit_log_suffix()
                ));
                None
            }
        };
        settled.push((index, session, status));
    }
    // An ended run's status is already known, so reading it never waits.
    let stopped = positions.into_iter().zip(runs).map(|(index, mut session)| {
        let status = if session.is_finished() && !session.stopped() {
            session_finished_within(&mut session, Duration::ZERO)
                .ok()
                .flatten()
        } else {
            None
        };
        (index, session, status)
    });
    let mut ended = Vec::new();
    let mut unended = kept;
    for (index, session, status) in settled.into_iter().chain(stopped) {
        if !session.is_finished() {
            unended.push((index, session));
            continue;
        }
        if let Some(code) = status
            && session.owner_lost()
        {
            log.record(&session_end_line(&session, code));
        }
        ended.push((session.id.clone(), status));
    }
    unended.sort_by_key(|(index, _)| *index);
    *sessions = unended.into_iter().map(|(_, session)| session).collect();
    ended.sort_by_key(|(id, _)| ids.iter().position(|named| named == id));
    (ended, errors)
}

/// Dismisses every pending tab's pair after presentation detachment
/// (EON-C18): each stays on as its tab's Files, and its tab's first pane
/// starts.
fn dismiss_pending_pairs(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<(), String> {
    let venus = state.presentation_name();
    let SupervisorState {
        component_generation,
        workspaces,
        sessions,
        initial_child,
        log,
        ..
    } = state;
    let Some(workspaces) = workspaces.as_mut() else {
        return Ok(());
    };
    let result = workspaces.dismiss_pending_pairs(|operation| {
        apply_session_operation(
            inputs,
            config,
            log,
            component_generation,
            sessions,
            initial_child,
            operation,
            PairStart::Later,
            venus,
        )
    });
    report_session_end(result, log)
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
        // Dismissing a pending pair starts its tab's first terminal.
        dismiss_pending_pairs(state, inputs, config)?;
        // Full Eon's clean exit is a detach; a crash is not (EON-C8).
        if status.success()
            && let Some(window) = state.notice_window()
        {
            state
                .notices
                .show(window, &state.application_id, &state.log);
        }
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

/// Serves one control request: a `Subscribe` keeps its connection for the
/// event stream (EON-C39), and every other request gets one reply. The turn's
/// events, the request's change included, are published first and written
/// before the reply as far as each subscriber takes them without waiting.
fn dispatch(
    incoming: Incoming,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Dispatched {
    let request = match incoming {
        Incoming::Stop(target) => {
            let reply = stop_window(&target, state, runtime, generation);
            state.serve_subscribers(Instant::now());
            return reply;
        }
        Incoming::Request(request) => request,
    };
    if matches!(request.action, Action::Subscribe { .. }) && !state.stopping {
        return match subscribe(state, inputs, config, &request, generation) {
            Ok(subscriber) => Dispatched::Subscribe(Box::new(subscriber)),
            Err(error) => {
                Dispatched::Reply(ControlResponse::Workspace(Response::Failure(error)), false)
            }
        };
    }
    let reply = dispatch_control_request(request, state, inputs, config, runtime, generation);
    state.serve_subscribers(Instant::now());
    reply.into()
}

/// Accepts a subscription, which changes no selection, shown workspace or
/// popup and never counts as an attached Venus (EON-C39).
fn subscribe(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    request: &Request,
    generation: &str,
) -> Result<Subscriber, Failure> {
    let Action::Subscribe { filter, after } = &request.action else {
        unreachable!("only a Subscribe subscribes");
    };
    // Before the work below; the subscribers' own check follows their turn.
    state.subscribers.room()?;
    let caller = caller_session(state, inputs, config, request)?;
    let projection = state
        .projection()
        .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
    let window = state.window.clone();
    state.subscribers.subscribe(
        projection,
        filter,
        after.as_ref(),
        caller.as_deref(),
        generation,
        window,
        Instant::now(),
    )
}

/// Why generation `generation`, in independent window `window` or the
/// default namespace, refuses requests after a failed Stop. Eon names the
/// window by its durable ID, which every build's `eon window stop` selects
/// (EON-C8); eonbase keeps its generation form.
fn stop_incomplete(mode: LaunchMode, window: Option<&str>, generation: &str) -> String {
    match mode {
        LaunchMode::Workspace => format!(
            "window cleanup is incomplete; retry `eon window stop {}`",
            super::windows::durable_window_id(window, generation)
        ),
        LaunchMode::Terminal => {
            format!("generation cleanup is incomplete; retry `eonbase stop {generation}`")
        }
    }
}

/// Answers the Reserved Stop of `target`, also while an earlier Stop is
/// incomplete, so it retries that Stop (EON-C11).
fn stop_window(
    target: &str,
    state: &mut SupervisorState,
    runtime: &Path,
    generation: &str,
) -> Dispatched {
    if target != generation {
        let error = failure(
            "generation-mismatch",
            format!("supervisor owns generation {generation}, not {target}"),
        );
        return Dispatched::Stop(stop_failure(error), false);
    }
    let result = stop_generation_sessions(state, runtime, launch_mode(state));
    let shutdown = result.is_ok();
    let reply = match result {
        Ok(terminals) => eon_workspace_protocol::stop::Reply::Stopped {
            generation: generation.into(),
            terminals,
        },
        Err(detail) => stop_failure(failure("stop-failed", detail)),
    };
    Dispatched::Stop(reply, shutdown)
}

fn launch_mode(state: &SupervisorState) -> LaunchMode {
    if state.workspaces.is_some() {
        LaunchMode::Workspace
    } else {
        LaunchMode::Terminal
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
    let mode = launch_mode(state);
    if state.stopping
        && !matches!(
            request.action,
            Action::Workspace(
                WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation
            )
        )
    {
        let error = failure(
            "stop-incomplete",
            stop_incomplete(mode, state.window.as_deref(), generation),
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
            caller,
            ..
        } => {
            // The terminal running the inspection is not one it asks about.
            let exclude = caller.and_then(|caller| {
                state
                    .sessions
                    .iter()
                    .find(|session| session.is_terminal(&caller))
                    .map(|session| session.id.clone())
            });
            match runtime_status(inputs, generation, state, mode, exclude.as_deref()) {
                Ok(mut runtime) => {
                    if state.stopping {
                        runtime.attach = Availability {
                            available: false,
                            reason: stop_incomplete(mode, state.window.as_deref(), generation),
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
            }
        }
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
                let bar = window_bar(mode, config, inputs, Some(&state.log))?;
                let command = venus_command(
                    inputs,
                    config,
                    &socket,
                    mode,
                    &terminal,
                    &bar,
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
                    managed_environment::number_keys(config, &inputs.defaults, Some(&state.log))?,
                );
                if let Some(quotas) = &mut state.quotas {
                    quotas.show(&bar.widgets);
                }
                state.venus = Some(PresentationProcess::start(
                    command,
                    terminal.requires_startup_admission(),
                    state.workspace_snapshot(),
                    PRESENTATION_ADMISSION_TIMEOUT,
                    &mut state.sessions,
                    &state.log,
                )?);
                Ok(())
            });
            match result.and_then(|()| {
                state.notices.close();
                runtime_status(inputs, generation, state, mode, None)
            }) {
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
        Request {
            action:
                Action::ReadTerminal { .. }
                | Action::ReadTerminalHistory { .. }
                | Action::SendTerminal { .. },
            ..
        } => (
            ControlResponse::Workspace(
                terminal_access(request, state, inputs, config).unwrap_or_else(Response::Failure),
            ),
            false,
        ),
        Request {
            action: Action::EditTerminalHistory { .. },
            ..
        } => (
            ControlResponse::Workspace(
                edit_terminal_history(request, state, inputs, config)
                    .unwrap_or_else(Response::Failure),
            ),
            false,
        ),
        request => {
            if state.workspaces.is_none() {
                return (
                    ControlResponse::Workspace(Response::Failure(failure(
                        "session-unavailable",
                        "eonbase has no Eon session",
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
            let (mut action, jump) = match (request.action, &state.attention) {
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
            let visit = match &action {
                Action::Workspace(WorkspaceAction::SetTabDirectory { tab, directory }) => {
                    // A pick ends a starting quick search's grace: it came from
                    // quick search, which has then ended, or from Files or the CLI.
                    let pair = workspace
                        .pair(caller.as_deref(), tab, FILES_ENTRY)
                        .and_then(|(session, _)| {
                            state.sessions.iter().find(|running| running.id == session)
                        })
                        .map(|running| pair::socket(&running.endpoint));
                    Some((directory.clone(), pair))
                }
                _ => None,
            };
            // Alt+Z and Alt+Shift+Y on a live pair switch its screen or hide it
            // (EON-C18).
            let pair_switch = match &mut action {
                Action::InvokePopup {
                    tab, entry, intent, ..
                } => workspace
                    .pair(caller.as_deref(), tab, entry)
                    .and_then(|(session, shown)| {
                        // The identity the pair was launched with (pair_command).
                        let endpoint = &state
                            .sessions
                            .iter()
                            .find(|running| running.id == session)?
                            .endpoint;
                        let id = pair::client_id(endpoint);
                        let socket = pair::socket(endpoint);
                        let quick = pair::quick_search_shown(&socket);
                        // Only a quick search whose fzf has not answered yet.
                        let starting = quick && pair::starting(&socket).is_some();
                        let (dispatched, switch) =
                            pair::plan(entry, *intent, shown, quick, starting);
                        *intent = dispatched;
                        Some((switch, id, socket))
                    }),
                _ => None,
            };
            // The shown Workspaces picker's own key keeps it shown and presses Enter in it, so
            // it confirms the highlighted row instead of hiding the picker (EON-C33).
            let picker_confirm = match &mut action {
                Action::InvokePopup {
                    tab, entry, intent, ..
                } if *intent == InvokeIntent::Toggle => workspace
                    .shown_picker(caller.as_deref(), tab, entry)
                    .inspect(|_| *intent = InvokeIntent::Focus),
                _ => None,
            };
            // Scrollback reads the tab's shown terminal and writes its
            // snapshot before its popup starts the editor on it (EON-C10).
            // The window key is the person's; a terminal's request is an
            // agent's read, which `[agents] terminal_access` governs.
            let snapshot = match &action {
                Action::InvokePopup { tab, entry, .. } => {
                    if caller.is_some()
                        && state.agents.terminal_access == TerminalAccess::None
                        && workspace.is_scrollback(caller.as_deref(), entry)
                    {
                        return (
                            ControlResponse::Workspace(Response::Failure(failure(
                                "terminal-access-off",
                                "[agents] terminal_access is \"none\" for this window",
                            ))),
                            false,
                        );
                    }
                    match workspace.scrollback_source(caller.as_deref(), tab, entry) {
                        Some((target, terminal)) => match scrollback_snapshot(
                            &mut state.sessions,
                            &mut state.scrollback,
                            &target,
                            &terminal,
                            &state.log,
                        ) {
                            Ok(snapshot) => Some(snapshot),
                            Err(detail) => {
                                return (
                                    ControlResponse::Workspace(Response::Failure(failure(
                                        "popup-start",
                                        detail,
                                    ))),
                                    false,
                                );
                            }
                        },
                        None => None,
                    }
                }
                _ => None,
            };
            // The terminal Scrollback's popup starts as, once its snapshot
            // records it.
            let mut marked = None;
            let direct_cross_workspace = jump.is_none()
                && accepted.as_ref().is_some_and(|action| {
                    matches!(action, Action::FocusAttention { tab, .. }
                    if workspace.attention_destination(caller.as_deref(), tab).is_some())
                });
            let venus = state.venus.as_ref().and_then(|venus| venus.name);
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
                            &snapshot_command(command, snapshot.as_ref()),
                            inputs.managed.shell_bin.as_deref(),
                            directory,
                            config,
                            &inputs.defaults,
                            Some(log),
                        )
                    },
                    |operation| {
                        mark_before_start(snapshot.as_ref(), &mut marked, operation, |operation| {
                            apply_session_operation(
                                inputs,
                                config,
                                log,
                                component_generation,
                                sessions,
                                initial_child,
                                operation,
                                PairStart::Later,
                                venus,
                            )
                        })
                    },
                ),
            };
            if let Some(snapshot) = snapshot {
                settle_snapshot(
                    &mut state.scrollback,
                    snapshot,
                    marked,
                    result.is_ok(),
                    sessions,
                    log,
                );
            }
            if result.is_ok()
                && let Some((directory, pair)) = visit
            {
                if let Some(socket) = pair {
                    pair::quick_search_settled(&socket);
                }
                record_visit(inputs, config, log, &directory);
            }
            if result.is_ok()
                && let Some((switch, id, socket)) = pair_switch
            {
                let ya = inputs.managed.shell_bin.as_ref().map(|bin| bin.join("ya"));
                pair::run(switch, ya.as_deref(), id, socket, log);
            }
            if result.is_ok()
                && let Some(picker) = picker_confirm
                && let Some(session) = sessions.iter_mut().find(|session| session.id == picker)
                && let Err(error) = sessions::send_input(session, LeaseInput::Enter, 0, log)
            {
                log.record(&format!(
                    "session picker {picker} did not take its key: {}",
                    error.detail
                ));
            }
            // Only shown focus sees. A direct cross-workspace focus resets the cursor even
            // when a hidden caller named the terminal already focused in the shown workspace.
            if result.is_ok()
                && let (Some(action), Some(attention)) = (&accepted, &mut state.attention)
            {
                attention.handled(
                    action,
                    jump.map(|(_, key)| key),
                    &workspace.snapshots(),
                    state
                        .venus
                        .as_ref()
                        .is_some_and(PresentationProcess::viewing),
                    direct_cross_workspace,
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

/// Opens the retained history and live area of the pane or popup the
/// request names in the configured editor, in a Scrollback popup over its
/// tab, as the window's Scrollback key does for the terminal a tab shows
/// (EON-C35, EON-C10). Only in the shown workspace's active tab; it changes
/// no active tab, shown workspace or selection. Blocks the supervisor for the
/// read, as `eon pane history` does.
fn edit_terminal_history(
    request: Request,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<Response, Failure> {
    if state.workspaces.is_none() {
        return Err(failure("session-unavailable", "eonbase has no Eon session"));
    }
    if state.agents.terminal_access == TerminalAccess::None {
        return Err(failure(
            "terminal-access-off",
            "[agents] terminal_access is \"none\" for this window",
        ));
    }
    let caller = caller_session(state, inputs, config, &request)?;
    let Some(workspace) = &mut state.workspaces else {
        unreachable!("reconciliation keeps the workspace owner");
    };
    let caller = caller.and_then(|caller| workspace.session_target(&caller));
    let Action::EditTerminalHistory { target: id } = &request.action else {
        unreachable!("dispatch routes only history edits here");
    };
    let (target, entry) = workspace.scrollback_target(&request.id, id)?;
    let Some(session) = state
        .sessions
        .iter_mut()
        .find(|session| session.id == target.terminal)
    else {
        return Err(failure(
            "terminal-ended",
            format!("terminal {} has ended", target.terminal),
        ));
    };
    let history = sessions::read_history(session, &state.log)?;
    let (TargetId::Pane(source) | TargetId::Popup(source)) = &target.id;
    let snapshot = state
        .scrollback
        .write(source, &history)
        .map_err(|detail| failure("popup-start", detail))?;
    let mut marked = None;
    let venus = state.venus.as_ref().and_then(|venus| venus.name);
    let component_generation = &state.component_generation;
    let sessions = &mut state.sessions;
    let initial_child = &mut state.initial_child;
    let log = &state.log;
    // The target's tab is the shown workspace's active tab, which a request
    // without a caller acts on.
    let result = workspace.dispatch(
        &request.id,
        None,
        Action::InvokePopup {
            tab: target.tab.clone(),
            entry,
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
        |command, directory| {
            managed_environment::prepare_popup_command(
                &snapshot_command(command, Some(&snapshot)),
                inputs.managed.shell_bin.as_deref(),
                directory,
                config,
                &inputs.defaults,
                Some(log),
            )
        },
        |operation| {
            mark_before_start(Some(&snapshot), &mut marked, operation, |operation| {
                apply_session_operation(
                    inputs,
                    config,
                    log,
                    component_generation,
                    sessions,
                    initial_child,
                    operation,
                    PairStart::Later,
                    venus,
                )
            })
        },
    );
    let popup = marked.as_ref().map(|session| session.id.clone());
    settle_snapshot(
        &mut state.scrollback,
        snapshot,
        marked,
        result.is_ok(),
        sessions,
        log,
    );
    result.map_err(|error| failure(error.code, error.detail))?;
    let popup = popup
        .and_then(|session| workspace.session_target(&session))
        .ok_or_else(|| failure("popup-start", "the Scrollback popup did not start"))?;
    Ok(Response::Editing(Editing {
        target,
        caller,
        popup,
    }))
}

/// Scrollback's command for one press: `[editor] command` on its
/// `snapshot`, as `eon edit` builds it (EON-C10). Any other command is
/// itself.
fn snapshot_command<'a>(
    command: &'a PopupCommand,
    snapshot: Option<&scrollback::Snapshot>,
) -> Cow<'a, PopupCommand> {
    match (command, snapshot) {
        (PopupCommand::Scrollback, Some(snapshot)) => {
            Cow::Owned(PopupCommand::Edit(snapshot.file.clone().into_os_string()))
        }
        _ => Cow::Borrowed(command),
    }
}

/// Records in Scrollback's `snapshot` the terminal its editor starts as
/// before `start` runs that Start, so a recovery that adopts the editor
/// adopts its snapshot too; a failed record starts nothing (EON-C10).
fn mark_before_start(
    snapshot: Option<&scrollback::Snapshot>,
    marked: &mut Option<workspace::Session>,
    operation: SessionOperation,
    start: impl FnOnce(SessionOperation) -> Result<(), String>,
) -> Result<(), String> {
    if let (
        Some(snapshot),
        SessionOperation::Start {
            session,
            command: LaunchCommand::Tool(argv),
            ..
        },
    ) = (snapshot, &operation)
        && argv
            .last()
            .is_some_and(|last| last.as_os_str() == snapshot.file.as_os_str())
    {
        snapshot.mark(&session.id)?;
        *marked = Some(session.clone());
    }
    start(operation)
}

/// Gives a press's snapshot to the terminal its record names, or removes it
/// when no terminal can be using it. A failed start whose rollback could not
/// confirm cleanup keeps its launch record and may leave the editor running,
/// so its snapshot stays until that record goes, as a stop that ends the
/// terminal removes it, or for the recovery that adopts it (EON-C10).
fn settle_snapshot(
    store: &mut Snapshots,
    snapshot: scrollback::Snapshot,
    marked: Option<workspace::Session>,
    started: bool,
    running: &[RunningSession],
    log: &DiagnosticLog,
) {
    match marked {
        Some(session) if started || running.iter().any(|running| running.id == session.id) => {
            store.own(snapshot, session.id, None);
        }
        Some(session) if sessions::launch_record_remains(&session.endpoint) => {
            log.record(&format!(
                "keeping scrollback snapshot {} of terminal {}, which may still run",
                snapshot.file.display(),
                session.id
            ));
            store.own(snapshot, session.id, Some(session.endpoint));
        }
        _ => {
            if let Err(error) = store.discard(snapshot) {
                log.record(&error);
            }
        }
    }
}

/// Reads `terminal`'s history for Scrollback and writes it to a new
/// snapshot of the pane or popup `target` (EON-C10).
fn scrollback_snapshot(
    running: &mut [RunningSession],
    store: &mut Snapshots,
    target: &str,
    terminal: &str,
    log: &DiagnosticLog,
) -> Result<scrollback::Snapshot, String> {
    let session = running
        .iter_mut()
        .find(|session| session.id == terminal)
        .ok_or_else(|| format!("terminal {terminal} has ended"))?;
    let history = sessions::read_history(session, log)
        .map_err(|error| format!("cannot read the terminal's history: {}", error.detail))?;
    store.write(target, &history)
}

/// What a terminal request does to its target.
enum Operation {
    Read,
    History,
    Send(TerminalInput, bool),
}

/// Reads the screen or history of, or sends to, a pane or popup terminal in
/// any workspace of the window, shown or hidden, without changing any
/// workspace, selection or focus (EON-C35, EON-C36). Blocks the supervisor
/// until Orbit answers or the lease's deadline passes.
fn terminal_access(
    request: Request,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
) -> Result<Response, Failure> {
    if state.workspaces.is_none() {
        return Err(failure("session-unavailable", "eonbase has no Eon session"));
    }
    if state.agents.terminal_access == TerminalAccess::None {
        return Err(failure(
            "terminal-access-off",
            "[agents] terminal_access is \"none\" for this window",
        ));
    }
    let caller = caller_session(state, inputs, config, &request)?;
    let Some(workspaces) = &mut state.workspaces else {
        unreachable!("reconciliation keeps the workspace owner");
    };
    let caller = caller.and_then(|caller| workspaces.session_target(&caller));
    let (id, operation) = match request.action {
        Action::ReadTerminal { target } => (target, Operation::Read),
        Action::ReadTerminalHistory { target } => (target, Operation::History),
        Action::SendTerminal {
            target,
            input,
            force,
        } => (target, Operation::Send(input, force)),
        _ => unreachable!("dispatch routes only terminal reads and sends here"),
    };
    let target =
        workspaces.terminal_target(&request.id, matches!(operation, Operation::Send(..)), &id)?;
    let Some(session) = state
        .sessions
        .iter_mut()
        .find(|session| session.id == target.terminal)
    else {
        return Err(failure(
            "terminal-ended",
            format!("terminal {} has ended", target.terminal),
        ));
    };
    let screen = |screen| match screen {
        OrbitScreen::Primary => Screen::Primary,
        OrbitScreen::Alternate => Screen::Alternate,
    };
    Ok(match operation {
        Operation::Read => {
            let visible = sessions::read_visible(session, &state.log)?;
            Response::Text(TerminalText {
                target,
                caller,
                revision: visible.revision,
                screen: screen(visible.screen),
                rows: visible.rows,
                columns: visible.columns,
                title: visible.title,
                directory: visible.directory,
                omitted_rows: visible.omitted_rows,
                text: visible.text,
            })
        }
        Operation::History => {
            let history = sessions::read_history(session, &state.log)?;
            Response::History(TerminalHistory {
                target,
                caller,
                revision: history.revision,
                screen: screen(history.screen),
                rows: history.rows,
                columns: history.columns,
                title: history.title,
                directory: history.directory,
                omitted_lines: history.omitted_lines,
                cut_bytes: history.cut_bytes,
                text: history.text,
            })
        }
        Operation::Send(input, force) => {
            let input = match input {
                TerminalInput::Paste(text) => LeaseInput::Paste(text),
                TerminalInput::Enter => LeaseInput::Enter,
            };
            Response::Sent(Sent {
                target,
                caller,
                bracketed: sessions::send_input(
                    session,
                    input,
                    if force {
                        0
                    } else {
                        state.agents.send_typing_quiet_ms
                    },
                    &state.log,
                )?,
                enter_delay_ms: state.agents.send_enter_delay_ms,
            })
        }
    })
}

/// A close reply; one that ended the generation shuts the supervisor down.
fn closed(result: Result<Response, Failure>) -> (ControlResponse, bool) {
    let response = result.unwrap_or_else(Response::Failure);
    let ended = matches!(response, Response::Ended(_));
    (ControlResponse::Workspace(response), ended)
}

/// The live terminals lifecycle inspection counts: all but pending tabs'
/// pairs, which are popups and not durable terminals (EON-C18).
fn live_sessions<'a>(
    sessions: &'a [RunningSession],
    workspaces: Option<&WorkspaceSet>,
) -> impl Iterator<Item = &'a RunningSession> + 'a {
    let pending = workspaces.map_or_else(Vec::new, WorkspaceSet::pending_pair_sessions);
    sessions
        .iter()
        .filter(move |session| !session.is_finished() && !pending.contains(&session.id))
}

fn runtime_status(
    inputs: &Inputs,
    generation: &str,
    state: &SupervisorState,
    mode: LaunchMode,
    exclude: Option<&str>,
) -> Result<Runtime, String> {
    if mode == LaunchMode::Terminal && state.sessions.is_empty() {
        return Err("supervisor has no live terminals".into());
    }
    Ok(Runtime {
        generation: generation.into(),
        name: state.workspaces.as_ref().map(WorkspaceSet::window_name),
        started_at: state.started_at,
        eon_version: inputs.version.into(),
        workspace_protocol: VERSION,
        component_report: inputs.component_report()?.to_owned(),
        sessions: live_sessions(&state.sessions, state.workspaces.as_ref())
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
        busy: state.busy(exclude),
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
    use super::WorkspaceAction;
    #[cfg(target_os = "linux")]
    use super::apply_session_operation;
    use super::{
        Action, DiagnosticLog, Dispatched, LaunchMode, PairStart, SESSION_START_TIMEOUT,
        SupervisorState, WorkspaceSet, effective_uid, mark_before_start, pair_command,
        prepare_configuration, prepare_runtime, reap_finished_sessions, settle_snapshot,
        stop_generation_sessions, stop_owned_session, stop_owned_sessions, temporary_directory,
        venus_command, window_bar, xdg_path,
    };
    use crate::scrollback::{Snapshots, tests::history};
    use crate::sessions::{
        self,
        tests::{leased_orbit, orbit_pid, script},
    };
    use crate::workspace::{LaunchCommand, Session, SessionOperation};
    use orbit_protocol::management::Screen;
    use std::{
        ffi::OsString,
        fs,
        io::Read,
        os::unix::{
            fs::{MetadataExt, PermissionsExt},
            net::UnixStream,
        },
        path::{Path, PathBuf},
        process::Command,
        time::{Duration, Instant},
    };

    /// Scrollback's record names its editor's terminal before that Start
    /// runs, and the started popup owns the snapshot. A record that cannot be
    /// written starts nothing and leaves no snapshot. A failed start keeps
    /// its snapshot only while its launch record says the editor may still
    /// run (EON-C10).
    #[test]
    fn a_scrollback_record_precedes_its_editor_and_only_a_confirmed_failure_discards() {
        let runtime = temporary_directory();
        let log = DiagnosticLog::open_in(Ok(runtime.join("logs")), "g1-test", "eon");
        let mut store = Snapshots::new(&runtime);
        let session = |id: &str| Session {
            id: id.into(),
            endpoint: runtime.join(format!("{id}.sock")),
        };
        let start = |file: &Path, id: &str| SessionOperation::Start {
            session: session(id),
            directory: runtime.clone(),
            command: LaunchCommand::Tool(vec!["editor".into(), file.as_os_str().to_owned()]),
            tab: Some("t1".into()),
        };
        let record = |file: &Path| file.with_file_name(".terminal");

        let started = store.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
        let mut marked = None;
        mark_before_start(
            Some(&started),
            &mut marked,
            start(&started.file, "terminal-3"),
            |_| {
                assert_eq!(
                    fs::read_to_string(record(&started.file)).unwrap(),
                    "terminal-3"
                );
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(marked, Some(session("terminal-3")));
        let directory = started.file.parent().unwrap().to_owned();
        settle_snapshot(&mut store, started, marked, true, &[], &log);
        assert!(directory.exists());
        store.retain(|_, _| false, &log);
        assert!(!directory.exists());

        let refused = store.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
        fs::write(record(&refused.file), "planted").unwrap();
        let mut marked = None;
        assert!(
            mark_before_start(
                Some(&refused),
                &mut marked,
                start(&refused.file, "terminal-4"),
                |_| panic!("the editor started without its record"),
            )
            .is_err()
        );
        assert_eq!(marked, None);
        let directory = refused.file.parent().unwrap().to_owned();
        settle_snapshot(&mut store, refused, marked, false, &[], &log);
        assert!(!directory.exists());

        // A rollback that could not confirm its cleanup keeps the launch
        // record; the editor may run, so its snapshot stays until the record
        // goes, as a stop that ends the terminal removes it.
        let uncertain = |_: &str, endpoint: Option<&Path>| {
            endpoint.is_some_and(sessions::launch_record_remains)
        };
        for (id, unconfirmed) in [("terminal-5", false), ("terminal-6", true)] {
            let failed = store.write("p1", &history(Screen::Primary, 0, 0)).unwrap();
            if unconfirmed {
                fs::write(runtime.join(format!("{id}.sock.record")), "").unwrap();
            }
            let mut marked = None;
            assert!(
                mark_before_start(Some(&failed), &mut marked, start(&failed.file, id), |_| {
                    Err("start failed".into())
                })
                .is_err()
            );
            let file = failed.file.clone();
            settle_snapshot(&mut store, failed, marked, false, &[], &log);
            assert_eq!(
                fs::read_to_string(record(&file)).is_ok(),
                unconfirmed,
                "{id}"
            );
            store.retain(uncertain, &log);
            assert_eq!(file.exists(), unconfirmed, "{id}");
            if unconfirmed {
                fs::remove_file(runtime.join(format!("{id}.sock.record"))).unwrap();
                store.retain(uncertain, &log);
                assert!(!file.parent().unwrap().exists(), "{id}");
            }
        }
        assert!(!runtime.join("snapshots").exists());
        fs::remove_dir_all(runtime).unwrap();
    }

    fn window_agents() -> crate::managed_environment::Agents {
        crate::managed_environment::Agents {
            terminal_access: crate::managed_environment::TerminalAccess::Window,
            send_enter_delay_ms: 300,
            send_typing_quiet_ms: 2000,
        }
    }

    /// A supervisor keeps a `Subscribe`'s connection for the stream, which
    /// changes nothing in the window; it refuses one it cannot serve with
    /// an ordinary failure reply (EON-C39).
    #[test]
    fn a_subscription_keeps_its_connection_and_changes_nothing() {
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            inputs.defaults.popups.clone(),
            |_, _| Ok(Vec::new()),
            |_| Ok(()),
        )
        .unwrap();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: Vec::new(),
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        let subscribe = |state: &mut SupervisorState, caller: Option<&str>, filter| {
            super::dispatch(
                super::Incoming::Request(super::Request {
                    id: "subscribe-1".into(),
                    caller: caller.map(Into::into),
                    action: Action::Subscribe {
                        filter,
                        after: None,
                    },
                }),
                state,
                &inputs,
                &root,
                &root,
                "component-1",
            )
        };
        let refusal = |dispatched| match dispatched {
            Dispatched::Reply(
                super::ControlResponse::Workspace(super::Response::Failure(failure)),
                false,
            ) => failure.code,
            _ => panic!("a subscription is refused"),
        };
        let before = state.workspace_snapshot_in(None).unwrap();
        let Dispatched::Subscribe(subscriber) = subscribe(&mut state, None, Default::default())
        else {
            panic!("a subscription is served");
        };
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        state
            .subscribers
            .attach(stream, *subscriber, Instant::now());
        let mut message = vec![0; eon_workspace_protocol::v37::HEADER_BYTES];
        peer.read_exact(&mut message).unwrap();
        message.resize(
            eon_workspace_protocol::v37::declared_message_len(&message).unwrap(),
            0,
        );
        peer.read_exact(&mut message[eon_workspace_protocol::v37::HEADER_BYTES..])
            .unwrap();
        let eon_workspace_protocol::v37::SubscriptionMessage::Subscribed(subscribed) =
            eon_workspace_protocol::v37::decode_subscription_message(&message).unwrap()
        else {
            panic!("no SUBSCRIBED");
        };
        assert_eq!(subscribed.generation, "component-1");
        assert_eq!(
            subscribed.snapshot.unwrap().workspaces[0].workspace.id,
            before.workspace
        );
        // Serving turns changes no selection, shown workspace or popup.
        for _ in 0..3 {
            state.serve_subscribers(Instant::now());
        }
        assert_eq!(state.workspace_snapshot_in(None).unwrap(), before);
        assert!(state.venus.is_none());

        let missing = eon_workspace_protocol::v37::Filter {
            workspaces: vec!["s9".into()],
            ..Default::default()
        };
        assert_eq!(
            refusal(subscribe(&mut state, None, missing)),
            "unknown-session"
        );
        assert_eq!(
            refusal(subscribe(&mut state, Some("orbit-9"), Default::default())),
            "unknown-caller"
        );
        // While a Stop is incomplete, a subscriber that hung up still
        // leaves.
        let Dispatched::Subscribe(hung_up) = subscribe(&mut state, None, Default::default()) else {
            panic!("a subscription is served");
        };
        let (stream, hung_up_peer) = UnixStream::pair().unwrap();
        state.subscribers.attach(stream, *hung_up, Instant::now());
        assert_eq!(state.subscribers.len(), 2);
        state.stopping = true;
        assert_eq!(
            refusal(subscribe(&mut state, None, Default::default())),
            "stop-incomplete"
        );
        let Dispatched::Reply(
            super::ControlResponse::Workspace(super::Response::Failure(stopping)),
            false,
        ) = subscribe(&mut state, None, Default::default())
        else {
            panic!("a subscription is refused");
        };
        assert_eq!(stopping.code, "stop-incomplete");
        assert!(
            stopping
                .detail
                .starts_with("window cleanup is incomplete; retry `eon window stop "),
            "{}",
            stopping.detail
        );
        crate::subscriptions::tests::hang_up(hung_up_peer);
        state.serve_subscribers(Instant::now());
        assert_eq!(state.subscribers.len(), 1);
        state.stopping = false;
        // eonbase has no workspace to subscribe to.
        state.workspaces = None;
        assert_eq!(
            refusal(subscribe(&mut state, None, Default::default())),
            "session-unavailable"
        );
        // Stop, quit and the final close end the stream.
        state.subscribers.end("component-1");
        let mut rest = Vec::new();
        peer.read_to_end(&mut rest).unwrap();
        assert!(!rest.is_empty());
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    /// EON-C39: the events of a change an action makes are written to a
    /// subscriber before the acting client can read its reply, so a client
    /// that learns of a change by subscription never learns of it later than
    /// the client that made it.
    #[test]
    fn an_actions_events_reach_subscribers_before_its_reply() {
        let (root, inputs, mut state, peer) = subscribed_window();
        let control = super::ControlListener::bind(&root.join("eon.sock")).unwrap();
        let client = rename_client(&root, peer);
        serve_turn(&control, &mut state, &inputs, &root);
        let (reply, held) = client.join().unwrap();
        assert!(matches!(reply, super::Response::Snapshot(_)), "{reply:?}");
        assert_eq!(renames(&held), [Some("renamed".to_owned())], "{held:?}");
        drop(control);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    /// EON-C39: a turn publishes once, so a change its tending made and the
    /// change of its action coalesce into one event.
    #[test]
    fn a_turn_with_an_action_publishes_once() {
        let (root, inputs, mut state, peer) = subscribed_window();
        let control = super::ControlListener::bind(&root.join("eon.sock")).unwrap();
        // Stands in for a change the turn's tending makes.
        state
            .workspaces
            .as_mut()
            .unwrap()
            .dispatch(
                "tend-1",
                None,
                Action::Workspace(WorkspaceAction::RenameWorkspace {
                    workspace: None,
                    name: "tended".into(),
                }),
                |_, _| Ok(Vec::new()),
                |_| Ok(()),
            )
            .unwrap();
        let client = rename_client(&root, peer);
        serve_turn(&control, &mut state, &inputs, &root);
        let (_, held) = client.join().unwrap();
        assert_eq!(renames(&held), [Some("renamed".to_owned())], "{held:?}");
        drop(control);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    /// A pending window with one unfiltered subscriber that has read its
    /// snapshot, and that subscriber's end of the connection.
    fn subscribed_window() -> (PathBuf, crate::cli::Inputs, SupervisorState, UnixStream) {
        use eon_workspace_protocol::v37::SubscriptionMessage;
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            inputs.defaults.popups.clone(),
            |_, _| Ok(Vec::new()),
            |_| Ok(()),
        )
        .unwrap();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: Vec::new(),
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        let Dispatched::Subscribe(subscriber) = super::dispatch(
            super::Incoming::Request(super::Request {
                id: "subscribe-1".into(),
                caller: None,
                action: Action::Subscribe {
                    filter: Default::default(),
                    after: None,
                },
            }),
            &mut state,
            &inputs,
            &root,
            &root,
            "component-1",
        ) else {
            panic!("a subscription is served");
        };
        state.end_turn(Instant::now());
        let (stream, mut peer) = UnixStream::pair().unwrap();
        state
            .subscribers
            .attach(stream, *subscriber, Instant::now());
        assert!(matches!(
            subscription_messages(&mut peer)[..],
            [SubscriptionMessage::Subscribed(_), ..]
        ));
        (root, inputs, state, peer)
    }

    /// A client that renames the shown workspace, sent before this returns so
    /// the next loop turn serves it, then reads its reply and at once what
    /// `peer` holds; no loop turn runs between.
    fn rename_client(
        root: &Path,
        mut peer: UnixStream,
    ) -> std::thread::JoinHandle<(
        super::Response,
        Vec<eon_workspace_protocol::v37::SubscriptionMessage>,
    )> {
        use eon_workspace_protocol::v37::{
            HEADER_BYTES, declared_message_len, decode_response, encode_request,
        };
        use std::io::Write;
        let mut stream = UnixStream::connect(root.join("eon.sock")).unwrap();
        stream
            .write_all(
                &encode_request(&super::Request {
                    id: "rename-1".into(),
                    caller: None,
                    action: Action::Workspace(WorkspaceAction::RenameWorkspace {
                        workspace: None,
                        name: "renamed".into(),
                    }),
                })
                .unwrap(),
            )
            .unwrap();
        std::thread::spawn(move || {
            let mut reply = vec![0; HEADER_BYTES];
            stream.read_exact(&mut reply).unwrap();
            reply.resize(declared_message_len(&reply).unwrap(), 0);
            stream.read_exact(&mut reply[HEADER_BYTES..]).unwrap();
            let held = subscription_messages(&mut peer);
            (decode_response(&reply).unwrap(), held)
        })
    }

    /// Runs one supervisor loop turn as `run` does; it serves the waiting
    /// client.
    fn serve_turn(
        control: &super::ControlListener,
        state: &mut SupervisorState,
        inputs: &crate::cli::Inputs,
        root: &Path,
    ) {
        state.tend(Instant::now());
        let accepted = control
            .accept(|request| super::dispatch(request, state, inputs, root, root, "component-1"))
            .unwrap();
        state.end_turn(Instant::now());
        assert!(!matches!(accepted, super::Accepted::None));
    }

    /// The names of the `workspace-renamed` events in `messages`.
    fn renames(
        messages: &[eon_workspace_protocol::v37::SubscriptionMessage],
    ) -> Vec<Option<String>> {
        use eon_workspace_protocol::v37::{Change, SubscriptionMessage, WorkspaceChange};
        messages
            .iter()
            .filter_map(|message| match message {
                SubscriptionMessage::Event(event) => match &event.change {
                    Change::Workspace {
                        change: WorkspaceChange::Renamed,
                        workspace,
                    } => Some(workspace.workspace.name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Every whole subscription message already written to `peer`.
    fn subscription_messages(
        peer: &mut UnixStream,
    ) -> Vec<eon_workspace_protocol::v37::SubscriptionMessage> {
        use eon_workspace_protocol::v37::{declared_message_len, decode_subscription_message};
        peer.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 65536];
        loop {
            match peer.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        peer.set_nonblocking(false).unwrap();
        let mut messages = Vec::new();
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            let length = declared_message_len(rest).unwrap();
            messages.push(decode_subscription_message(&rest[..length]).unwrap());
            rest = &rest[length..];
        }
        messages
    }

    /// EON-C39: attention the poller applies in a loop turn reaches
    /// subscribers in that turn, as a real Orbit terminal reports it.
    #[cfg(target_os = "linux")]
    #[test]
    fn attention_reaches_subscribers_in_the_loop_turn_that_polls_it() {
        use eon_workspace_protocol::v37::{AttentionState, Change, SubscriptionMessage};
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let session = leased_orbit(
            &root,
            "sleep 0.5; printf '\\033]7501;state=working:app=claude-code\\007'; sleep 1; \
             printf '\\033]7501;state=done:app=claude-code\\007'; exec sleep 30",
        );
        let workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            None,
            vec![(
                1,
                super::workspace::Session {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                },
            )],
            inputs.defaults.popups.clone(),
        )
        .unwrap();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![session],
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: Some(crate::attention::Attention::new(true)),
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        let Dispatched::Subscribe(subscriber) = super::dispatch(
            super::Incoming::Request(super::Request {
                id: "subscribe-1".into(),
                caller: None,
                action: Action::Subscribe {
                    filter: eon_workspace_protocol::v37::Filter {
                        kinds: vec![eon_workspace_protocol::v37::EventKind::Attention],
                        ..Default::default()
                    },
                    after: None,
                },
            }),
            &mut state,
            &inputs,
            &root,
            &root,
            "component-1",
        ) else {
            panic!("a subscription is served");
        };
        let (stream, mut peer) = UnixStream::pair().unwrap();
        state
            .subscribers
            .attach(stream, *subscriber, Instant::now());
        subscription_messages(&mut peer);

        // After every turn, the subscriber has read what Inspect shows.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut read = None;
        loop {
            state.tend(Instant::now());
            state.end_turn(Instant::now());
            for message in subscription_messages(&mut peer) {
                if let SubscriptionMessage::Event(event) = message
                    && let Change::Attention { attention, .. } = event.change
                {
                    read = attention.map(|attention| attention.state);
                }
            }
            let shown = state
                .workspace_snapshot_in(None)
                .unwrap()
                .tabs
                .iter()
                .flat_map(|tab| tab.panes())
                .find_map(|pane| pane.attention.as_ref().map(|attention| attention.state));
            assert_eq!(read, shown, "Inspect shows what the turn did not send");
            if shown == Some(AttentionState::Finished) {
                break;
            }
            assert!(Instant::now() < deadline, "no finish within 20 s");
            std::thread::sleep(Duration::from_millis(25));
        }
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    /// A full-Eon window whose one pane runs a real Orbit agent that reports
    /// a fresh finish each time its gate, the returned FIFO, is written.
    #[cfg(target_os = "linux")]
    fn finishing_window(root: &Path) -> (SupervisorState, PathBuf) {
        let gate = root.join("gate");
        assert!(
            Command::new("mkfifo")
                .arg(&gate)
                .status()
                .unwrap()
                .success()
        );
        let session = leased_orbit(
            root,
            &format!(
                "while read -r _ < '{}'; do \
                 printf '\\033]7501;state=working:app=claude-code\\007'; \
                 printf '\\033]7501;state=done:app=claude-code\\007'; done",
                gate.display()
            ),
        );
        let inputs = crate::fixtures::inputs();
        let workspace = WorkspaceSet::with_recovered_sessions(
            root.to_path_buf(),
            root.to_path_buf(),
            None,
            vec![(
                1,
                Session {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                },
            )],
            inputs.defaults.popups.clone(),
        )
        .unwrap();
        let state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![session],
            venus: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: Some(crate::attention::Attention::new(true)),
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        (state, gate)
    }

    /// Attaches a Venus whose native focus reports are what the returned
    /// peer writes. Its process runs until the returned control end closes.
    #[cfg(target_os = "linux")]
    fn attach_reporting_venus(state: &mut SupervisorState) -> (UnixStream, UnixStream) {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec cat >/dev/null"]);
        let mut venus = super::PresentationProcess::start(
            command,
            false,
            None,
            super::PRESENTATION_ADMISSION_TIMEOUT,
            &mut [],
            &state.log,
        )
        .unwrap();
        let (control, peer) = UnixStream::pair().unwrap();
        let original = std::mem::replace(&mut venus.control, control);
        state.venus = Some(venus);
        (peer, original)
    }

    /// The pane's attention and the window's queue, as Inspect shows them.
    #[cfg(target_os = "linux")]
    fn inspected(
        state: &mut SupervisorState,
    ) -> (
        Option<eon_workspace_protocol::v37::Attention>,
        Vec<eon_workspace_protocol::v37::QueueEntry>,
    ) {
        // Each request ID is accepted once.
        static INSPECTIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let inspection = INSPECTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (response, ended) = super::dispatch_control_request(
            super::Request {
                id: format!("inspect-{inspection}"),
                caller: None,
                action: Action::Workspace(WorkspaceAction::Inspect),
            },
            state,
            &crate::fixtures::inputs(),
            Path::new("/unused"),
            Path::new("/unused"),
            "component-1",
        );
        assert!(!ended);
        let snapshot = match response {
            super::ControlResponse::Workspace(eon_workspace_protocol::v37::Response::Snapshot(
                snapshot,
            )) => snapshot,
            super::ControlResponse::Workspace(eon_workspace_protocol::v37::Response::Failure(
                failure,
            )) => panic!("Inspect failed: {} {}", failure.code, failure.detail),
            _ => panic!("Inspect answers with a snapshot"),
        };
        let attention = snapshot
            .tabs
            .iter()
            .flat_map(|tab| tab.panes())
            .find_map(|pane| pane.attention.clone());
        (attention, snapshot.attention_queue)
    }

    /// Writes the gate, then runs loop turns until the pane shows the fresh
    /// finish, and returns its state.
    #[cfg(target_os = "linux")]
    fn raise_finish(
        state: &mut SupervisorState,
        gate: &Path,
    ) -> eon_workspace_protocol::v37::AttentionState {
        use eon_workspace_protocol::v37::AttentionState;
        let changed = |attention: &Option<eon_workspace_protocol::v37::Attention>| {
            attention
                .as_ref()
                .and_then(|attention| attention.record.as_ref())
                .map(|record| record.changed_at)
        };
        let shown = |state: &mut SupervisorState| {
            let snapshot = state.workspace_snapshot_in(None).unwrap();
            snapshot.tabs[0].panes().next().unwrap().attention.clone()
        };
        let before = changed(&shown(state));
        fs::write(gate, "\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            state.tend(Instant::now());
            state.end_turn(Instant::now());
            let attention = shown(state);
            if let Some(shown) = attention.as_ref().map(|attention| attention.state)
                && shown != AttentionState::Working
                && changed(&attention) != before
            {
                return shown;
            }
            assert!(Instant::now() < deadline, "no finish within 20 s");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// EON-C31: attachment alone does not see a raise; the selected
    /// terminal's finish waits for its window's native focus.
    #[cfg(target_os = "linux")]
    #[test]
    fn selected_raise_waits_for_native_window_focus() {
        use eon_workspace_protocol::v37::AttentionState::{Finished, Idle};
        use std::io::Write;
        let root = temporary_directory();
        let (mut state, gate) = finishing_window(&root);
        let (mut peer, original) = attach_reporting_venus(&mut state);
        let queued = |state: &mut SupervisorState| {
            let (attention, queue) = inspected(state);
            (attention.map(|attention| attention.state), queue.len())
        };

        // No report yet: unknown focus is unfocused, in every path that
        // presents attention.
        assert_eq!(raise_finish(&mut state, &gate), Finished);
        assert_eq!(queued(&mut state), (Some(Finished), 1));
        state.projection().unwrap();
        let (pane, tab) = {
            let snapshot = state.workspace_snapshot_in(None).unwrap();
            (
                snapshot.tabs[0].panes().next().unwrap().id.clone(),
                snapshot.tabs[0].id.clone(),
            )
        };
        let (_, ended) = super::dispatch_control_request(
            super::Request {
                id: "focus".into(),
                caller: None,
                action: Action::FocusAttention { tab, target: pane },
            },
            &mut state,
            &crate::fixtures::inputs(),
            &root,
            &root,
            "component-1",
        );
        assert!(!ended);
        assert_eq!(queued(&mut state), (Some(Finished), 1), "focused by CLI");

        // Native focus sees it; losing focus leaves the next finish queued.
        peer.write_all(b"focus:1\n").unwrap();
        state.tend(Instant::now());
        assert_eq!(queued(&mut state), (Some(Idle), 0));
        peer.write_all(b"focus:0\n").unwrap();
        state.tend(Instant::now());
        assert_eq!(raise_finish(&mut state, &gate), Finished);
        assert_eq!(queued(&mut state), (Some(Finished), 1));

        drop(original);
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_focus_frames_are_partial_bounded_and_fail_closed() {
        use super::{FocusFailure, NativeFocus};
        use std::{io::Write, net::Shutdown, os::fd::OwnedFd};
        let focused = || {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let mut focus = NativeFocus::default();
            peer.write_all(b"focus:1").unwrap();
            assert!(focus.poll(&stream).is_ok());
            assert_eq!(focus.focused, None, "a partial report counted");
            peer.write_all(b"\n").unwrap();
            assert!(focus.poll(&stream).is_ok());
            assert_eq!(focus.focused, Some(true));
            (focus, stream, peer)
        };

        // The last complete report wins; repeats and silence keep it.
        let (mut focus, stream, mut peer) = focused();
        for (bytes, expected) in [
            (&b"focus:0\nfocus:1\nfocus:0\n"[..], false),
            (b"focus:0\n", false),
            (b"", false),
            (&b"focus:1\n".repeat(64)[..], true),
        ] {
            peer.write_all(bytes).unwrap();
            assert!(focus.poll(&stream).is_ok());
            assert_eq!(focus.focused, Some(expected));
        }
        // A turn reads at most 512 bytes; the rest waits for the next turn.
        peer.write_all(&b"focus:0\n".repeat(64)).unwrap();
        peer.write_all(b"focus:1\n").unwrap();
        assert!(focus.poll(&stream).is_ok());
        assert_eq!(focus.focused, Some(false), "read past one turn's bound");
        assert!(focus.poll(&stream).is_ok());
        assert_eq!(focus.focused, Some(true), "a burst ended reports");

        // Each failure leaves focus unknown, for good.
        let malformed = |bytes: &[u8]| {
            let (mut focus, stream, mut peer) = focused();
            peer.write_all(bytes).unwrap();
            let failure = focus.poll(&stream);
            peer.write_all(b"focus:1\n").unwrap();
            assert!(focus.poll(&stream).is_ok());
            assert_eq!(focus.focused, None);
            failure
        };
        assert!(matches!(
            malformed(b"focus:2\nfocus:1\n"),
            Err(FocusFailure::Invalid)
        ));
        let (mut focus, stream, peer) = focused();
        (&peer).write_all(b"focus").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        assert!(matches!(focus.poll(&stream), Err(FocusFailure::Ended)));
        assert_eq!(focus.focused, None);
        let mut focus = NativeFocus {
            focused: Some(true),
            ..NativeFocus::default()
        };
        let unreadable = UnixStream::from(OwnedFd::from(fs::File::open("/dev/null").unwrap()));
        assert!(matches!(focus.poll(&unreadable), Err(FocusFailure::Io(_))));
        assert_eq!(focus.focused, None);
    }

    /// EON-C31: a replacement Venus inherits no focus report, and one whose
    /// reports end counts as unfocused, without stopping anything.
    #[cfg(target_os = "linux")]
    #[test]
    fn replacement_venus_does_not_inherit_focus() {
        use eon_workspace_protocol::v37::AttentionState::{Finished, Idle};
        use std::{io::Write, net::Shutdown};
        let root = temporary_directory();
        let (mut state, gate) = finishing_window(&root);
        let run = state.sessions[0].run_id().to_owned();
        let (mut first, original) = attach_reporting_venus(&mut state);
        first.write_all(b"focus:1\n").unwrap();
        state.tend(Instant::now());
        assert_eq!(raise_finish(&mut state, &gate), Idle, "seen while focused");

        drop(original);
        state.venus = None;
        let (mut second, original) = attach_reporting_venus(&mut state);
        assert_eq!(raise_finish(&mut state, &gate), Finished, "inherited focus");
        second.write_all(b"focus:1\n").unwrap();
        state.tend(Instant::now());
        assert_eq!(
            inspected(&mut state).0.map(|attention| attention.state),
            Some(Idle)
        );
        second.shutdown(Shutdown::Write).unwrap();
        state.tend(Instant::now());
        assert_eq!(
            raise_finish(&mut state, &gate),
            Finished,
            "kept focus after EOF"
        );

        // Reports that end with a live Venus are recorded; with an exited
        // one they are not.
        drop(original);
        state.venus = None;
        let (third, original) = attach_reporting_venus(&mut state);
        drop(original);
        state.venus.as_mut().unwrap().child.wait().unwrap();
        third.shutdown(Shutdown::Write).unwrap();
        state.tend(Instant::now());
        let ended = fs::read_dir(root.join("logs"))
            .unwrap()
            .filter_map(|entry| fs::read_to_string(entry.unwrap().path()).ok())
            .collect::<String>()
            .matches("Eon Desktop focus reports ended")
            .count();
        assert_eq!(ended, 1);

        assert!(state.venus.is_some() && !state.stopping);
        assert_eq!(
            (state.sessions.len(), state.sessions[0].run_id()),
            (1, run.as_str())
        );
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn attaching_an_already_presented_window_retries_notice_close() {
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let session = leased_orbit(&root, "exec sleep 30");
        let workspace = WorkspaceSet::with_recovered_sessions(
            root.clone(),
            root.clone(),
            None,
            vec![(
                1,
                super::workspace::Session {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                },
            )],
            inputs.defaults.popups.clone(),
        )
        .unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec cat >/dev/null"]);
        let venus = super::PresentationProcess::start(
            command,
            false,
            None,
            super::PRESENTATION_ADMISSION_TIMEOUT,
            &mut [],
            &log,
        )
        .unwrap();
        let (notices, take_close) = crate::notification::Notices::record_closes();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![session],
            venus: Some(venus),
            initial_child: Vec::new(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices,
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log,
        };
        let closes = (0..2)
            .map(|attempt| {
                let (response, ended) = super::dispatch_control_request(
                    super::Request {
                        id: format!("attach-{attempt}"),
                        caller: None,
                        action: Action::Workspace(WorkspaceAction::Present { workspace: true }),
                    },
                    &mut state,
                    &inputs,
                    &root,
                    &root,
                    "component-1",
                );
                assert!(!ended);
                assert!(matches!(
                    response,
                    super::ControlResponse::Lifecycle(super::LifecycleResponse::Runtime(_))
                ));
                take_close()
            })
            .collect::<Vec<_>>();
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
        assert_eq!(closes, [true, true]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detach_notice_counts_the_terminal_started_by_pair_dismissal() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let pair_root = temporary_directory();
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = std::env::var_os("ORBIT_BIN").unwrap().into();
        let workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            inputs.defaults.popups.clone(),
            |_, _| Ok(Vec::new()),
            |_| Ok(()),
        )
        .unwrap();
        // The pending tab's pair, which detachment dismisses and keeps.
        let mut pair = leased_orbit(&pair_root, "exec sleep 30");
        pair.id = "terminal-2".into();
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let mut venus = super::PresentationProcess::start(
            command,
            false,
            None,
            super::PRESENTATION_ADMISSION_TIMEOUT,
            &mut [],
            &log,
        )
        .unwrap();
        venus.child.wait().unwrap();
        let (notices, take_show) = crate::notification::Notices::record_shows();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![pair],
            venus: Some(venus),
            initial_child: ["/bin/sh", "-c", "exec sleep 30"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            initial_status: None,
            stopping: false,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices,
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log,
        };
        // The pending pair is a popup and not counted (EON-C18).
        assert_eq!(state.this_window().unwrap().live_terminals, Some(0));
        assert!(super::reap_presentation(&mut state, &inputs, &root).unwrap());
        let shown = take_show();
        let after = state.this_window().unwrap();
        let expected = crate::notification::NoticeWindow::new(&after);
        let tab = state.workspaces.as_ref().unwrap().snapshot().tabs[0].clone();
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(pair_root).unwrap();
        assert!(!tab.pending);
        assert_eq!(tab.popups.len(), 1);
        assert_eq!(tab.selected_popup, None);
        assert_eq!(after.live_terminals, Some(2));
        assert_eq!(shown, Some(expected));
    }

    /// Kills a test-owned Orbit's worker, which sends Orbit down its failure
    /// path: it removes its Live record and exits without a result.
    fn kill_worker(session: &sessions::RunningSession) {
        kill_orbit_worker(orbit_pid(session));
    }

    fn kill_orbit_worker(orbit: i32) {
        let worker = children(orbit)
            .into_iter()
            .find(|pid| program(*pid).ends_with(b"yazelix-orbit-worker"))
            .expect("Orbit runs its worker");
        // SAFETY: signals the test-owned Orbit's worker process.
        assert_eq!(unsafe { libc::kill(worker, libc::SIGKILL) }, 0);
    }

    #[cfg(target_os = "linux")]
    fn children(pid: i32) -> Vec<i32> {
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            .unwrap()
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn children(pid: i32) -> Vec<i32> {
        let mut children = [0; 64];
        // SAFETY: proc_listchildpids writes at most the buffer's byte size of pids.
        let count = unsafe {
            libc::proc_listchildpids(
                pid,
                children.as_mut_ptr().cast(),
                std::mem::size_of_val(&children) as libc::c_int,
            )
        };
        children[..usize::try_from(count).unwrap()].to_vec()
    }

    /// The program `pid` runs: its first argument on Linux, its executable
    /// path on macOS. Empty once the process is gone.
    #[cfg(target_os = "linux")]
    fn program(pid: i32) -> Vec<u8> {
        let arguments = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        arguments.split(|byte| *byte == 0).next().unwrap().to_vec()
    }

    #[cfg(target_os = "macos")]
    fn program(pid: i32) -> Vec<u8> {
        let mut path = vec![0; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: proc_pidpath writes at most the buffer's length.
        let length =
            unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        path.truncate(usize::try_from(length).unwrap_or(0));
        path
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
        sessions[1].id = "terminal-2".into();
        sessions[2].id = "terminal-3".into();
        let log = DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "eon");
        // A dead worker sends Orbit down its failure path, which removes its
        // Live record and publishes no result: the run reconciles as ended.
        let first_orbit_log = sessions[0].orbit_log.clone().unwrap();
        kill_worker(&sessions[0]);
        assert_eq!(
            stop_owned_session(&mut sessions, "terminal-1", &log),
            Ok(Some(1))
        );
        assert_eq!(sessions.len(), 2);
        let owner_loss = fs::read_dir(roots[0].join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(
            owner_loss.lines().any(|line| {
                line.contains("terminal terminal-1 ended with status 1; Orbit exited without publishing a terminal result")
                    && line.ends_with(&format!("; Orbit log: {}", first_orbit_log.display()))
            }),
            "{owner_loss}"
        );

        // A record that no longer validates leaves the stopped run
        // unreconcilable: the close fails and the terminal stays tracked.
        // Only the tracked id is renamed, so its Orbit log still says terminal-1.
        let second_orbit_log = sessions[0].orbit_log.clone().unwrap();
        let record = sessions::tests::record(&sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let error = stop_owned_session(&mut sessions, "terminal-2", &log).unwrap_err();
        assert!(
            error.ends_with(&format!("; Orbit log: {}", second_orbit_log.display())),
            "{error}"
        );
        assert_eq!(sessions.len(), 2);
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
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
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
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
            log.matches("terminal terminal-2 stays in its tab after a failed stop")
                .count(),
            1
        );
        assert_eq!(
            stop_owned_session(&mut state.sessions, "terminal-3", &state.log),
            Ok(None)
        );
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_close_stops_its_terminals_together_and_keeps_only_the_unended() {
        // Each child ignores SIGHUP, so its Orbit waits out the 500 ms grace
        // before SIGKILL (ORB-C12): stopped one at a time, the seven healthy
        // terminals the close names would take 3.5 seconds.
        let roots = (0..9).map(|_| temporary_directory()).collect::<Vec<_>>();
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "trap '' HUP; exec sleep 30"))
            .collect::<Vec<_>>();
        for (number, session) in sessions.iter_mut().enumerate() {
            session.id = format!("terminal-{}", number + 1);
        }
        let log = DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "eon");
        // A record that no longer validates leaves terminal-4 unreconcilable.
        let record = sessions::tests::record(&sessions[3]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        // The close leaves terminal-2 alone; terminal-10 does not exist.
        let ids = [
            "terminal-9",
            "terminal-8",
            "terminal-7",
            "terminal-6",
            "terminal-5",
        ]
        .into_iter()
        .chain(["terminal-4", "terminal-10", "terminal-3", "terminal-1"])
        .map(String::from)
        .collect::<Vec<_>>();

        let started = Instant::now();
        let (ended, errors) = stop_owned_sessions(&mut sessions, &ids, &log);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(2500),
            "the close's stops ran one at a time: {elapsed:?}"
        );
        assert_eq!(
            ended,
            [
                "terminal-9",
                "terminal-8",
                "terminal-7",
                "terminal-6",
                "terminal-5"
            ]
            .into_iter()
            .chain(["terminal-3", "terminal-1"])
            .map(|id| (id.to_owned(), None))
            .collect::<Vec<_>>()
        );
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(
            errors[0],
            "terminal terminal-10 is missing from the supervisor"
        );
        assert!(errors[1].starts_with("terminal-4: "), "{errors:?}");
        // The terminals that stay keep their order.
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["terminal-2", "terminal-4"]
        );
        assert_eq!(
            stop_owned_session(&mut sessions, "terminal-2", &log),
            Ok(None)
        );
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_close_waits_for_unreconciled_ends_within_one_shared_bound() {
        let roots = (0..3).map(|_| temporary_directory()).collect::<Vec<_>>();
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "exec sleep 30"))
            .collect::<Vec<_>>();
        for (number, session) in sessions.iter_mut().enumerate() {
            session.id = format!("terminal-{}", number + 1);
        }
        let log = DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "eon");
        // Two runs whose cleanup never completes: each could hold the close
        // for the whole bound by itself.
        sessions::tests::stall(&sessions[0]);
        sessions::tests::stall(&sessions[2]);
        let ids = ["terminal-1", "terminal-2", "terminal-3"].map(String::from);

        let started = Instant::now();
        let (ended, errors) = stop_owned_sessions(&mut sessions, &ids, &log);
        let elapsed = started.elapsed();
        assert!(
            elapsed < SESSION_START_TIMEOUT + Duration::from_millis(1500),
            "the close waited past its one shared bound: {elapsed:?}"
        );
        assert_eq!(ended, [("terminal-2".to_owned(), None)]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].starts_with("terminal-1: "), "{errors:?}");
        assert!(errors[1].starts_with("terminal-3: "), "{errors:?}");
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["terminal-1", "terminal-3"]
        );
        for session in &mut sessions {
            sessions::tests::end_stalled(session);
        }
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
        sessions[1].id = "terminal-2".into();
        sessions[2].id = "terminal-3".into();
        // A closed lease whose run still holds its Live record leaves the end
        // pending: the first reap waits for it, later reaps must not.
        sessions::tests::close_lease(&sessions[2]);
        // The terminal exits on its own, never sent Stop, and its final record
        // then no longer validates.
        let orbit = orbit_pid(&sessions[0]);
        fs::write(&go, "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !crate::platform::process(orbit.cast_unsigned())
            .unwrap()
            .expect("Orbit was reaped")
            .ended
        {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let record = sessions::tests::record(&sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let mut state = SupervisorState {
            started_at: 1,
            window: None,
            running: None,
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
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(roots[0].join("logs")), "g1-test", "eon"),
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
                .matches("terminal terminal-1 stays in its tab until its end reconciles")
                .count(),
            1
        );
        assert_eq!(
            log()
                .matches("terminal terminal-3 stays in its tab until its end reconciles")
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
        assert_eq!(state.sessions[0].id, "terminal-2");
        assert_eq!(
            stop_owned_session(&mut state.sessions, "terminal-2", &state.log),
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
        let orbit_log = orbit_logs.join(format!("component-1-terminal-1-{}.log", session.run_id()));
        assert_eq!(session.orbit_log.as_ref(), Some(&orbit_log));
        // A supervisor that recovers the run rebuilds its log path. Orbit may
        // refuse the new lease as Busy until it sees the old one close. On
        // macOS, Busy is also reported when Orbit closes before the peer check.
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
            started_at: 1,
            window: None,
            running: None,
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
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
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
            started_at: 1,
            window: None,
            running: None,
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
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };

        let error = stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap_err();
        assert!(
            error.contains("terminal-1: terminal owner exited without completing Stop")
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
                    line.contains("terminal terminal-1 ended with status 1; Orbit exited without publishing a terminal result")
                        && line.ends_with(&format!("; Orbit log: {}", orbit_log.display()))
                })
                .count(),
            1,
            "{supervisor_log}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// After a failed Stop, Eon names the window by its durable ID, never
    /// `default`, which another build's `eon` takes as its own window, and
    /// eonbase names its own Stop.
    #[test]
    fn stop_incomplete_names_the_command_that_stops_this_window() {
        use super::{LaunchMode, stop_incomplete};
        let generation = "g1-0123456789abcdef0123456789abcdef";
        assert_eq!(
            stop_incomplete(LaunchMode::Workspace, None, generation),
            "window cleanup is incomplete; retry `eon window stop 01234567`"
        );
        assert_eq!(
            stop_incomplete(LaunchMode::Workspace, Some("abcd1234"), generation),
            "window cleanup is incomplete; retry `eon window stop abcd1234`"
        );
        assert_eq!(
            stop_incomplete(LaunchMode::Terminal, None, generation),
            format!("generation cleanup is incomplete; retry `eonbase stop {generation}`")
        );
    }

    /// A pending pair's command carries the runtime's launch flags only for
    /// the packaged Files, the startup flags only for the window's first pair,
    /// and the DDS client ID its screen switches use (EON-C18).
    #[test]
    fn pair_commands_carry_quick_start_socket_and_client_id_only_for_packaged_files() {
        let session = crate::workspace::Session {
            id: "terminal-2".into(),
            endpoint: "/runtime/g1/terminal-2.sock".into(),
        };
        let id = crate::pair::client_id(&session.endpoint);
        let socket = crate::pair::socket(&session.endpoint);
        assert_eq!(socket, Path::new("/runtime/g1/terminal-2.quick.sock"));
        let animation = crate::managed_environment::StartupAnimation {
            enabled: true,
            style: "wave".into(),
            duration_seconds: 3,
        };
        let text = |argv: Vec<OsString>| {
            argv.into_iter()
                .map(|argument| argument.into_string().unwrap())
                .collect::<Vec<_>>()
        };
        let tail = [
            "--eon-pair-socket".to_owned(),
            socket.to_str().unwrap().to_owned(),
            "--client-id".to_owned(),
            id.to_string(),
        ];
        let files = || vec![OsString::from("eon-files")];
        assert_eq!(
            text(
                pair_command(files(), true, &session, PairStart::Fresh(Some(&animation))).unwrap()
            ),
            [
                &[
                    "eon-files",
                    "--eon-anima",
                    "wave",
                    "3",
                    "--eon-notice",
                    "/runtime/g1/eon.sock",
                    "--eon-quick"
                ][..],
                &tail.iter().map(String::as_str).collect::<Vec<_>>(),
            ]
            .concat()
        );
        assert_eq!(
            text(pair_command(files(), true, &session, PairStart::Fresh(None)).unwrap()),
            [
                &[
                    "eon-files",
                    "--eon-notice",
                    "/runtime/g1/eon.sock",
                    "--eon-quick"
                ][..],
                &tail.iter().map(String::as_str).collect::<Vec<_>>(),
            ]
            .concat()
        );
        assert_eq!(
            text(pair_command(files(), false, &session, PairStart::Later).unwrap()),
            [
                &["eon-files"][..],
                &tail.iter().map(String::as_str).collect::<Vec<_>>(),
            ]
            .concat()
        );
        // A test fixture's stand-in Files command runs exactly as given.
        let replaced = vec![OsString::from("my-files"), OsString::from("--flag")];
        assert_eq!(
            pair_command(replaced.clone(), true, &session, PairStart::Fresh(None)).unwrap(),
            replaced
        );
    }

    /// The window's first pair plays its startup Anima, then reads the
    /// notice, before its Yazi can start quick search: a key pressed in that
    /// time, and up to the startup grace after it, acts on the quick search
    /// still to come (EON-C18).
    #[test]
    fn a_startup_pairs_quick_search_counts_as_starting_through_its_anima() {
        let session = |name: &str| crate::workspace::Session {
            id: "terminal-2".into(),
            endpoint: format!("/runtime/{name}/terminal-2.sock").into(),
        };
        let animation = crate::managed_environment::StartupAnimation {
            enabled: true,
            style: "wave".into(),
            duration_seconds: 3,
        };
        let files = || vec![OsString::from("eon-files")];
        let grace = |session: &crate::workspace::Session| {
            crate::pair::starting(&crate::pair::socket(&session.endpoint))
                .unwrap()
                .saturating_duration_since(Instant::now())
        };
        let animated = session("g-anima-on");
        pair_command(files(), true, &animated, PairStart::Fresh(Some(&animation))).unwrap();
        // Anima 3 s, its 2.25 s overrun bound, the 1 s notice, then 2 s.
        assert!(
            grace(&animated) > Duration::from_secs(8),
            "{:?}",
            grace(&animated)
        );
        let still = session("g-anima-off");
        pair_command(files(), true, &still, PairStart::Fresh(None)).unwrap();
        assert!(grace(&still) > Duration::from_millis(2500));
        assert!(grace(&still) <= Duration::from_secs(3));
        let later = session("g-later");
        pair_command(files(), true, &later, PairStart::Later).unwrap();
        assert!(grace(&later) <= Duration::from_secs(2));
    }

    /// A pending tab's pair carries a marker until its tab commits: recovery
    /// stops a marked pair and reports it, so the launch-directory fallback
    /// applies, adopts an unmarked (committed) one as before, and removes
    /// stale markers (EON-C18).
    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_stops_a_pending_pair_but_adopts_a_committed_one() {
        for pending in [true, false] {
            let root = temporary_directory();
            let mut session = leased_orbit(&root, "exec sleep 30");
            let endpoint = session.endpoint.clone();
            let marker = sessions::pending_pair_marker(&endpoint);
            sessions::mark_pending_pair(&endpoint).unwrap();
            // A commit settles the marker through the supervisor.
            let inputs = crate::fixtures::inputs();
            let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
            let mut owned = vec![session];
            apply_session_operation(
                &inputs,
                &root,
                &log,
                "component-1",
                &mut owned,
                &mut Vec::new(),
                crate::workspace::SessionOperation::SettlePair("terminal-1".into()),
                PairStart::Later,
                None,
            )
            .unwrap();
            assert!(!marker.exists());
            session = owned.pop().unwrap();
            if pending {
                sessions::mark_pending_pair(&endpoint).unwrap();
            }
            let stale = sessions::pending_pair_marker(&root.join("terminal-9.sock"));
            sessions::mark_pending_pair(&root.join("terminal-9.sock")).unwrap();
            drop(session);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let (mut recovered, pair) = loop {
                match sessions::recover_sessions(
                    &root,
                    LaunchMode::Workspace,
                    "component-1",
                    None,
                    deadline,
                ) {
                    Ok(result) => break result,
                    Err(error)
                        if error.ends_with("is already managed")
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            assert_eq!(pair, pending);
            assert!(!marker.exists());
            assert!(!stale.exists());
            if pending {
                assert!(recovered.is_empty());
                assert!(!endpoint.exists());
            } else {
                assert_eq!(recovered.len(), 1);
                assert_eq!(recovered[0].id, "terminal-1");
                assert_eq!(
                    stop_owned_session(&mut recovered, "terminal-1", &log),
                    Ok(None)
                );
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn startup_snapshot_transfer_uses_the_admission_deadline() {
        use eon_workspace_protocol::v37::{
            Column, Pane, PopupGeometry, Response, Snapshot, Tab, encode_response,
        };
        use std::time::{Duration, Instant};

        let root = temporary_directory();
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v37::WorkspaceInfo {
                id: "s1".into(),
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
                            session: format!("terminal-{number}"),
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
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v37::Sidebar::Open,
            attention_queue: Vec::new(),
        };
        let expected = encode_response(&Response::Snapshot(snapshot.clone())).unwrap();
        let received = root.join("snapshot");
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
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
            &mut [],
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
            &mut [],
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
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "echo \"panicked: backtrace=$RUST_BACKTRACE\" >&2; kill -KILL $$",
        ]);
        let mut process = super::PresentationProcess::start(
            command,
            false,
            None,
            Duration::from_secs(2),
            &mut [],
            &log,
        )
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
                &window_bar(mode, &root, inputs, None).unwrap(),
                "eon",
                true,
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
            (
                "",
                "codex_quota_5h,codex_quota_7d,claude_quota_5h,claude_quota_7d,claude_quota_scoped,cpu,ram,disk,clock,version",
                true,
            ),
            (
                "[bar]\nwidgets = ['clock', 'version', 'ram']\n",
                "clock,version,ram",
                true,
            ),
            (
                "[bar]\nwidgets = ['cpu', 'codex_quota_5h']\n",
                "cpu,codex_quota_5h",
                false,
            ),
            ("[bar]\nwidgets = []\n", "", false),
            (
                "[bar]\nshow_version = false\n",
                "codex_quota_5h,codex_quota_7d,claude_quota_5h,claude_quota_7d,claude_quota_scoped,cpu,ram,disk,clock",
                false,
            ),
            (
                "[bar]\nshow_version = true\n",
                "codex_quota_5h,codex_quota_7d,claude_quota_5h,claude_quota_7d,claude_quota_scoped,cpu,ram,disk,clock,version",
                true,
            ),
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
            // The Eon icon reaches every workspace window, whatever the widgets, as the opaque
            // path Eon supplied.
            assert!(launch_input(&inputs, LaunchMode::Workspace, "--eon-icon").is_empty());
            let mut iconic = crate::fixtures::inputs();
            iconic.icon = Some("/fixture/eon-icon.rgba".into());
            assert_eq!(
                launch_input(&iconic, LaunchMode::Workspace, "--eon-icon"),
                ["/fixture/eon-icon.rgba"]
            );
            assert!(launch_input(&iconic, LaunchMode::Terminal, "--eon-icon").is_empty());
        }
        // Load levels and the disk path reach Venus with the chip they belong to, and never a
        // terminal window. `~` names the supervisor's HOME.
        let levels = |inputs: &crate::Inputs, mode| {
            [
                "--ram-warning-percent",
                "--ram-critical-percent",
                "--cpu-warning-percent",
                "--cpu-average-seconds",
                "--disk-warning-percent",
                "--disk-critical-percent",
                "--disk-path",
            ]
            .map(|flag| {
                launch_input(inputs, mode, flag)
                    .first()
                    .map_or_else(String::new, |value| value.to_str().unwrap().to_owned())
            })
        };
        let home = crate::managed_environment::nonempty_environment_path("HOME").unwrap();
        assert!(home.is_absolute());
        let home = home.to_str().unwrap();
        fs::write(root.join("config.toml"), "").unwrap();
        assert_eq!(
            levels(&inputs, LaunchMode::Workspace),
            ["80", "90", "90", "30", "85", "95", home]
        );
        fs::write(
            root.join("config.toml"),
            "[bar]\nwidgets = ['ram']\nram_warning_percent = 1\nram_critical_percent = 1\n",
        )
        .unwrap();
        assert_eq!(
            levels(&inputs, LaunchMode::Workspace),
            ["1", "1", "", "", "", "", ""]
        );
        fs::write(
            root.join("config.toml"),
            "[bar]\nwidgets = ['cpu']\ncpu_warning_percent = 100\ncpu_average_seconds = 600\n",
        )
        .unwrap();
        assert_eq!(
            levels(&inputs, LaunchMode::Workspace),
            ["", "", "100", "600", "", "", ""]
        );
        for (path, measured) in [
            ("~/work/eon", format!("{home}/work/eon")),
            ("/srv/data", "/srv/data".to_owned()),
        ] {
            fs::write(
                root.join("config.toml"),
                format!(
                    "[bar]\nwidgets = ['disk']\ndisk_warning_percent = 1\n\
                     disk_critical_percent = 100\ndisk_path = '{path}'\n"
                ),
            )
            .unwrap();
            assert_eq!(
                levels(&inputs, LaunchMode::Workspace),
                ["", "", "", "", "1", "100", &measured],
                "{path}"
            );
        }
        assert_eq!(
            levels(&inputs, LaunchMode::Terminal),
            ["", "", "", "", "", "", ""]
        );
        inputs.defaults.bar_widgets = &["ram", "clock"];
        fs::write(root.join("config.toml"), "[bar]\nunknown = true\n").unwrap();
        assert_eq!(
            launch_input(&inputs, LaunchMode::Workspace, "--bar-items"),
            ["ram,clock"]
        );
        for source in ["[bar]\nwidgets = ['gpu']\n", "[bar]\nwidgets = ['RAM']\n"] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert_eq!(
                launch_input(&inputs, LaunchMode::Workspace, "--bar-items"),
                [""]
            );
        }
        // Known names are checked where the bar launches; malformed input fails every read.
        for (source, error, every_read) in [
            (
                "[bar]\nwidgets = ['ram', 'ram']\n",
                "repeats \"ram\"",
                false,
            ),
            ("[bar]\nwidgets = [1]\n", "invalid type", true),
            ("[bar]\nwidgets = 'ram'\n", "invalid type", true),
            (
                "[bar]\nshow_version = 'no'\n",
                "bar.show_version must be true or false",
                true,
            ),
            (
                "[bar]\nram_warning_percent = 0\n",
                "bar.ram_warning_percent must be from 1 through 100",
                false,
            ),
            (
                "[bar]\nram_critical_percent = 101\n",
                "bar.ram_critical_percent must be from 1 through 100",
                false,
            ),
            (
                "[bar]\nram_warning_percent = 95\n",
                "must not be below bar.ram_warning_percent",
                false,
            ),
            (
                "[bar]\ncpu_warning_percent = 256\n",
                "bar.cpu_warning_percent must be from 1 through 100",
                false,
            ),
            (
                "[bar]\ncpu_average_seconds = 1\n",
                "bar.cpu_average_seconds must be from 2 through 600",
                false,
            ),
            (
                "[bar]\ncpu_average_seconds = 65538\n",
                "bar.cpu_average_seconds must be from 2 through 600",
                false,
            ),
            ("[bar]\ncpu_warning_percent = -1\n", "invalid value", true),
            ("[bar]\ncpu_average_seconds = 2.5\n", "invalid type", true),
            (
                "[bar]\ndisk_warning_percent = 0\n",
                "bar.disk_warning_percent must be from 1 through 100",
                false,
            ),
            (
                "[bar]\ndisk_critical_percent = 101\n",
                "bar.disk_critical_percent must be from 1 through 100",
                false,
            ),
            (
                "[bar]\ndisk_warning_percent = 96\n",
                "bar.disk_critical_percent must not be below bar.disk_warning_percent",
                false,
            ),
            (
                "[bar]\ndisk_path = 'work'\n",
                "bar.disk_path must be ~, absolute, or start with ~/",
                false,
            ),
            ("[bar]\ndisk_path = 1\n", "invalid type", true),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert_eq!(
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .is_err(),
                every_read,
                "{source:?}"
            );
            let failure = window_bar(LaunchMode::Workspace, &root, &inputs, None).unwrap_err();
            assert!(failure.contains(error), "{source:?}: {failure}");
            assert!(window_bar(LaunchMode::Terminal, &root, &inputs, None).is_ok());
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
                &inputs.defaults.bar(),
                "eon",
                true,
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
                    &inputs.defaults.bar(),
                    "eon",
                    true,
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

        for (source, expected) in [
            ("", "2"),
            ("[terminal]\ncursor_trail_duration = 3.0\n", "3"),
            ("[terminal]\ncursor_trail_duration = 0\n", "0"),
        ] {
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
                    &inputs.defaults.bar(),
                    "eon",
                    true,
                    true,
                    true,
                    true,
                );
                let args = command.get_args().collect::<Vec<_>>();
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
                &inputs.defaults.bar(),
                "eon",
                true,
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
                    &inputs.defaults.bar(),
                    "eon",
                    true,
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
                        &inputs.defaults.bar(),
                        "eon",
                        true,
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
                    &inputs.defaults.bar(),
                    "eon",
                    true,
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
                &inputs.defaults.bar(),
                "eon",
                true,
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
                    &inputs.defaults.bar(),
                    "eon",
                    true,
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
            &inputs.defaults.bar(),
            "eon",
            true,
            false,
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
                "--cpu-warning-percent",
                "90",
                "--cpu-average-seconds",
                "30",
                "--bar-items",
                "cpu",
                "--pane-frames",
                "true",
                "--warning-close-last-tab",
                "false",
                "--animate-working-badge",
                "false",
                "--number-keys",
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
            &inputs.defaults.bar(),
            "eonbase",
            true,
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
            // Venus's link openers and `eon window new` must not carry the session of
            // an agent whose shell started Eon (eon-geigu).
            assert!(
                environment
                    .iter()
                    .any(|(key, value)| *key == "CLAUDE_CODE_MESSAGING_TOKEN" && value.is_none())
            );
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
            &inputs.defaults.bar(),
            "eonova",
            false,
            true,
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
