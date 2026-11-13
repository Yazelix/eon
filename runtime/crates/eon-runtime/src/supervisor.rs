use super::{
    attention::{self, Attention},
    cli::Inputs,
    control::{
        Accepted, ControlListener, ControlResponse, Dispatched, EndpointFailure,
        EndpointFailureKind, Incoming, SocketIdentity, connect_control, failure, probe_launch_mode,
        probe_presentable_runtime, send_action_on, socket_identity, stop_failure,
    },
    diagnostics::DiagnosticLog,
    editor::{self, Sockets},
    events::Projection,
    generation::{current_generation, generation_directory, record_launcher, remove_launcher},
    managed_environment::{self, PopupCommand, TerminalAccess, nonempty_environment_path},
    notification, pair, platform, quota,
    running::Running,
    scrollback::{self, Snapshots},
    session_registry::{self, Registration, Registry},
    sessions::{
        self, AttachName, ManagedCandidate, RunningSession, TAB_VARIABLE, TERMINAL_VARIABLE,
        acquire_generation_sessions, acquire_runs, bound_io, name_presentation, recover_sessions,
        session_finished, session_finished_within, start_orbit, stop_managed_sessions,
        untracked_runs,
    },
    subscriptions::{Subscriber, Subscribers},
    workspace::{self, LaunchCommand, SessionEnd, SessionOperation, WorkspaceSet},
};
use eon_workspace_protocol::v43::{
    Action, Availability, BusyTerminals, CloseConsent, CloseEnd, CloseScope, CloseTerminal,
    Editing, Ended, FILES_ENTRY, Failure, InvokeIntent, LifecycleResponse, Request, Response,
    Runtime, Screen, Sent, Snapshot, TargetId, TerminalHistory, TerminalInput, TerminalText,
    UntrackedRun, VERSION, WindowEntry, WindowState, WorkspaceAction, WorkspaceTarget,
};
use orbit_protocol::management::{LeaseInput, ProcessOutcome, Screen as OrbitScreen};
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

pub(super) use eon_workspace_protocol::{
    PRESENTATION_ADMISSION_TIMEOUT, SESSION_CLEANUP_TIMEOUT, TERMINAL_START_TIMEOUT,
};
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
        validate_runtime(inputs, &info, generation)
            .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?;
        if !info.attach.available {
            // Fresh bootstrap has no admitted member; a competing launch waits for it.
            let kind = if info.name.is_none() && info.sessions.is_empty() {
                EndpointFailureKind::Starting
            } else {
                EndpointFailureKind::Incompatible
            };
            return Err(EndpointFailure::new(kind, info.attach.reason));
        }
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

/// What one full-Eon launch asks of the fresh window it starts (EON-C18).
#[derive(Default)]
pub(super) struct LaunchOptions {
    /// `--dir`: t1's absolute launch directory, where it starts its
    /// first pane instead of its pair.
    pub(super) directory: Option<PathBuf>,
    /// `--no-anima`: skip the startup Anima.
    pub(super) no_anima: bool,
}

/// Why bare `eon --dir` presents nothing: it names a fresh window's
/// first tab, and this window already runs (EON-C18).
const RUNNING_WINDOW_DIRECTORY: &str = "this Eon window is already running, so --dir has no fresh first tab to start; use `eon tab create --dir DIR` for a tab there or `eon window new --dir DIR` for a new window";

pub(super) fn launch_current(
    inputs: &Inputs,
    mode: LaunchMode,
    child: &[OsString],
    attach_existing: bool,
    decorations: bool,
    application_id: &str,
    options: &LaunchOptions,
) -> Result<i32, String> {
    if let Some(directory) = &options.directory {
        workspace::validate_initial_directory(directory)?;
    }
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
                if options.directory.is_some() {
                    return Err(RUNNING_WINDOW_DIRECTORY.into());
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
                            EndpointFailureKind::Dead
                                | EndpointFailureKind::Starting
                                | EndpointFailureKind::Unreachable
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
                    EndpointFailureKind::Dead
                        | EndpointFailureKind::Starting
                        | EndpointFailureKind::Unreachable
                ) =>
            {
                error
            }
            Err(error) => return Err(error.detail),
        };
        let lifecycle_lock = if matches!(
            endpoint_failure.kind,
            EndpointFailureKind::Dead | EndpointFailureKind::Starting
        ) {
            let Some(lock) = try_lock_supervisor_lifecycle(&lifecycle_lock_path)? else {
                let deadline = missing_deadline.get_or_insert_with(|| {
                    Instant::now() + SESSION_CLEANUP_TIMEOUT.saturating_add(Duration::from_secs(1))
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
            .flatten()
            .filter(|_| !options.no_anima && options.directory.is_none());
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
            options.directory.clone(),
            &generation,
            mode,
            application_id,
            decorations,
            &log,
            TERMINAL_START_TIMEOUT,
        )
    })();
    log.record(&match &result {
        Ok(code) => format!("supervisor exited with status {code}"),
        Err(error) => format!("supervisor failed: {error}"),
    });
    result.or_else(|error| {
        if attach_existing && fs::symlink_metadata(&socket).is_ok() {
            if options.directory.is_some() {
                return Err(format!("{error}; {RUNNING_WINDOW_DIRECTORY}"));
            }
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
                    EndpointFailureKind::Dead
                        | EndpointFailureKind::Starting
                        | EndpointFailureKind::Unreachable
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
        None,
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
    venus: &Path,
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
    let mut command = venus_process_command(venus, config, mode);
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
    command
        .arg("--detect-local-hosts")
        .arg(terminal.detect_local_hosts.to_string());
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
            command
                .arg("--eon-version")
                .arg(inputs.version)
                .arg("--eon-release-stage")
                .arg(inputs.release_stage.as_str());
        }
        if let Some(icon) = &inputs.icon {
            command.arg("--eon-icon").arg(icon);
        }
        let load = &bar.load;
        if shown("ram") {
            command
                .arg("--ram-notice-percent")
                .arg(load.ram_notice_percent.to_string())
                .arg("--ram-warning-percent")
                .arg(load.ram_warning_percent.to_string())
                .arg("--ram-critical-percent")
                .arg(load.ram_critical_percent.to_string());
        }
        if shown("cpu") {
            command
                .arg("--cpu-notice-percent")
                .arg(load.cpu_notice_percent.to_string())
                .arg("--cpu-warning-percent")
                .arg(load.cpu_warning_percent.to_string())
                .arg("--cpu-critical-percent")
                .arg(load.cpu_critical_percent.to_string())
                .arg("--cpu-average-seconds")
                .arg(load.cpu_average_seconds.to_string())
                .arg("--cpu-temperature-warning-celsius")
                .arg(load.cpu_temperature_warning_celsius.to_string())
                .arg("--cpu-temperature-critical-celsius")
                .arg(load.cpu_temperature_critical_celsius.to_string());
        }
        if shown("disk") {
            command
                .arg("--disk-notice-percent")
                .arg(load.disk_notice_percent.to_string())
                .arg("--disk-warning-percent")
                .arg(load.disk_warning_percent.to_string())
                .arg("--disk-critical-percent")
                .arg(load.disk_critical_percent.to_string());
            if let Some(path) = &bar.disk_path {
                command.arg("--disk-path").arg(path);
            }
        }
        command.arg("--bar-items").arg(bar.widgets.join(","));
        command
            .arg("--panel-heading")
            .arg(terminal.panel_heading.as_str())
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
    command
}

fn venus_process_command(venus: &Path, config: &Path, mode: LaunchMode) -> Command {
    let mut command = Command::new(venus);
    managed_environment::remove_agent_session(&mut command)
        .env_remove(TERMINAL_VARIABLE)
        .env_remove(TAB_VARIABLE)
        .env("EON_CONFIG_HOME", config);
    if mode == LaunchMode::Workspace
        && let Ok(executable) = env::current_exe()
    {
        command.env("EON_NEW_WINDOW_EXECUTABLE", executable);
    }
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
    /// True only after draining complete reports. A failure leaves focus unknown
    /// and ends reports from this Venus; exhausting the budget leaves freshness uncertain.
    fn poll(&mut self, stream: &UnixStream) -> Result<bool, FocusFailure> {
        if self.ended {
            return Ok(false);
        }
        let result = self.read(stream);
        if result.is_err() {
            self.ended = true;
            self.focused = None;
        }
        result
    }

    fn read(&mut self, stream: &UnixStream) -> Result<bool, FocusFailure> {
        let mut buffer = [0; 64];
        let mut budget = FOCUS_READ_BUDGET;
        while budget > 0 {
            let limit = buffer.len().min(budget);
            let count = match platform::read_available(stream, &mut buffer[..limit]) {
                Ok(0) => return Err(FocusFailure::Ended),
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(self.used == 0);
                }
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
        Ok(false)
    }
}

struct PresentationProcess {
    child: Child,
    executable: PathBuf,
    arguments: Vec<OsString>,
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
        let executable = PathBuf::from(command.get_program());
        let arguments = command.get_args().map(OsStr::to_owned).collect();
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
            executable,
            arguments,
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
                        eon_workspace_protocol::v43::encode_response(&Response::Snapshot(snapshot))
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
    fn read_native_focus(&mut self) -> bool {
        let failure = match self.focus.poll(&self.control) {
            Ok(fresh) => return fresh,
            Err(FocusFailure::Invalid) => "Eon Desktop focus reports are invalid".to_owned(),
            // Reports end with an exited Venus, which is no failure.
            Err(FocusFailure::Ended) if matches!(self.child.try_wait(), Ok(Some(_))) => {
                return false;
            }
            Err(FocusFailure::Ended) => "Eon Desktop focus reports ended".to_owned(),
            Err(FocusFailure::Io(error)) => format!("Eon Desktop focus reports failed: {error}"),
        };
        self.log
            .record(&format!("{failure}; treating the window as unfocused"));
        false
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
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
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
    rustix::process::geteuid().as_raw()
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
    lifecycle_lock: LifecycleLock,
    runtime: &Path,
    child: &[OsString],
    directory: Option<PathBuf>,
    generation: &str,
    mode: LaunchMode,
    application_id: &str,
    decorations: bool,
    log: &Arc<DiagnosticLog>,
    start_timeout: Duration,
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
    let workspace_ids = if mode == LaunchMode::Workspace {
        let range = if lifecycle_lock.created {
            0
        } else {
            replacement_id_range(platform::boot_time)?
        };
        Some(workspace::IdAllocator::new(runtime.to_path_buf(), range)?)
    } else {
        None
    };
    let _lifecycle_lock = lifecycle_lock;
    // A launch `--dir` gives t1 no pair (EON-C18).
    let pair = directory.is_none();
    let launch_directory = match directory {
        Some(directory) => directory,
        None => env::current_dir()
            .map_err(|error| format!("cannot resolve Eon launch directory: {error}"))?,
    };
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
    // An editor that resolves to nothing fails only the edit.
    if mode == LaunchMode::Workspace {
        managed_environment::editor_setting(config, &inputs.defaults, Some(log))?;
        managed_environment::shell_command(config, &inputs.defaults, Some(log))?;
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
    let mut failed_starts = sessions::FailedStarts::default();
    let (mut sessions, recovered_pair) = recover_sessions(
        runtime,
        mode,
        generation,
        log.orbit_directory().ok(),
        Instant::now() + SESSION_CLEANUP_TIMEOUT,
    )?;
    if let Some(attention) = &mut attention {
        for session in &sessions {
            attention.recover(session.run_id());
        }
    }
    let mut scrollback = Snapshots::new(runtime);
    scrollback.sweep(sessions.iter().map(|session| session.id.as_str()), log);
    let mut editor_sockets = Sockets::recover(runtime, &sessions, log);
    let control_listener = ControlListener::bind(&runtime.join("eon.sock"))?;
    let recovering = mode == LaunchMode::Workspace && !sessions.is_empty();
    let (registry, mut registry_setup_error) = match (mode == LaunchMode::Workspace)
        .then(Registry::new)
        .transpose()
    {
        Ok(registry) => (registry, None),
        Err(error) => (None, Some(error)),
    };
    #[cfg(test)]
    let registry = registry.map(|mut registry| {
        if env::var_os("EON_C38_FAIL_WORKER").is_some() {
            registry.inject(Some(session_registry::Fault::WorkerSpawn));
        } else if env::var_os("EON_C38_OBSERVE_BUSY_RETRY").is_some() {
            registry.inject(Some(session_registry::Fault::ObserveBusyRetry));
        }
        registry
    });
    let mut registry_worker = match registry
        .as_ref()
        .map(|registry| session_registry::Worker::start(registry.clone()))
        .transpose()
    {
        Ok(worker) => worker,
        Err(error) => {
            registry_setup_error = Some(error);
            None
        }
    };
    if let Some(worker) = &mut registry_worker
        && let Err(error) = worker.submit(session_registry::Operation::Register)
    {
        registry_setup_error = Some(error);
        registry_worker = None;
    }
    if !recovering && let Some(error) = registry_setup_error.take() {
        return Err(error.to_string());
    }
    let registration = if mode == LaunchMode::Workspace && !recovering {
        let starting = Runtime {
            venus: None,
            generation: generation.into(),
            name: None,
            started_at,
            eon_version: inputs.version.into(),
            workspace_protocol: VERSION,
            component_report: inputs.component_report()?.to_owned(),
            sessions: Vec::new(),
            attach: Availability {
                available: false,
                reason: "session registration is pending".into(),
            },
            stop: Availability {
                available: true,
                reason: "generation-aware supervisor is starting".into(),
            },
            busy: None,
        };
        let Some(registration) = await_initial_registration(
            &control_listener,
            registry_worker
                .as_mut()
                .ok_or("session registry worker is unavailable")?,
            &starting,
        )?
        else {
            remove_launcher(runtime);
            return Ok(0);
        };
        Some(registration)
    } else {
        None
    };
    let startup_error = |detail: String| match &registration {
        Some(registration) => format!("{detail}; registered persistent_id={}", registration.id),
        None => detail,
    };
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
                WorkspaceSet::registered_pending(
                    registry.clone(),
                    registration.as_ref().unwrap().clone(),
                    workspace_ids.as_ref().unwrap().clone(),
                    launch_directory.clone(),
                    env::home_dir(),
                    popup_catalog.clone(),
                    pair,
                    |_, _| Ok(Default::default()),
                    |_| Ok(()),
                )
                .map_err(&startup_error)?
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
                    recovered_workspace_sessions(&sessions).map_err(&startup_error)?
                };
                WorkspaceSet::registered_recovered(
                    registry.clone(),
                    registration.clone(),
                    workspace_ids.as_ref().unwrap().clone(),
                    launch_directory.clone(),
                    env::home_dir(),
                    recovered,
                    popup_catalog.clone(),
                )
                .map_err(&startup_error)?
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
            &inputs.venus,
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
        Some(
            PresentationProcess::start(
                command,
                true,
                snapshot,
                PRESENTATION_ADMISSION_TIMEOUT,
                &mut sessions,
                log,
            )
            .map_err(&startup_error)?,
        )
    } else {
        None
    };
    let mut initial_child = if sessions.is_empty() {
        child.to_vec()
    } else {
        Vec::new()
    };
    let initialized = (|| {
        if sessions.is_empty() && (mode == LaunchMode::Terminal || recovered_pair) {
            sessions.push(
                start_orbit(
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
                    Instant::now() + start_timeout,
                    &mut failed_starts,
                )
                .map_err(&startup_error)?,
            );
            initial_child.clear();
        }
        let mut workspaces = if mode == LaunchMode::Workspace {
            Some(if sessions.is_empty() {
                WorkspaceSet::registered_pending(
                    registry.clone(),
                    registration.as_ref().unwrap().clone(),
                    workspace_ids.as_ref().unwrap().clone(),
                    launch_directory.clone(),
                    env::home_dir(),
                    popup_catalog.clone(),
                    pair,
                    |command, directory| {
                        managed_environment::prepare_popup_command(
                            command,
                            &inputs.managed,
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
                            &mut failed_starts,
                            &mut initial_child,
                            &mut editor_sockets,
                            operation,
                            PairStart::Fresh(startup_animation.as_ref()),
                            admitted.as_ref().and_then(|venus| venus.name),
                        )
                    },
                )
                .map_err(&startup_error)?
            } else {
                WorkspaceSet::registered_recovered(
                    registry.clone(),
                    registration.clone(),
                    workspace_ids.as_ref().unwrap().clone(),
                    launch_directory,
                    env::home_dir(),
                    recovered_workspace_sessions(&sessions).map_err(&startup_error)?,
                    popup_catalog,
                )
                .map_err(&startup_error)?
            })
        } else {
            None
        };
        if let Some(error) = registry_setup_error
            && let Some(workspaces) = &mut workspaces
        {
            let target = workspaces.snapshot().workspace;
            workspaces.registered_recovery(&target, Err(error));
        }
        Ok::<_, String>(workspaces)
    })();
    let (workspaces, startup_failed) = match initialized {
        Ok(workspaces) => (workspaces, false),
        Err(error) => {
            failed_starts.reap(log);
            if failed_starts.is_empty() {
                return Err(error);
            }
            log.note(&error);
            (None, true)
        }
    };
    let registration_pending = recovering && registry_worker.is_some();
    let mut state = SupervisorState {
        failed_starts,
        registry_worker,
        registry_pending: registration_pending
            .then(|| RegistryPending::Recovery(workspaces.as_ref().unwrap().snapshot().workspace)),
        started_at,
        window,
        running,
        component_generation: generation.to_owned(),
        application_id: application_id.into(),
        decorations,
        workspaces,
        sessions,
        venus: None,
        venus_path: None,
        initial_child,
        initial_status: startup_failed.then_some(1),
        stopping: startup_failed.then_some(StopReason::BootstrapFailure),
        quotas: (mode == LaunchMode::Workspace)
            .then(|| quota::Quotas::start(&inputs.codexbar, &bar.widgets)),
        attention,
        pane_labels: super::pane_labels::Labels::default(),
        agents,
        notices: notification::Notices::default(),
        subscribers: Subscribers::new(generation),
        scrollback,
        editor_sockets,
        log: Arc::clone(log),
    };
    let presentation = if startup_failed {
        drop(admitted);
        Ok(None)
    } else {
        match admitted {
            Some(venus) => Ok(Some(venus)),
            None => PresentationProcess::start(
                venus_command(
                    inputs,
                    &inputs.venus,
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
            )
            .map(Some),
        }
    };
    state.venus = match presentation {
        Ok(venus) => venus,
        Err(error) => {
            let session_cleanup =
                stop_managed_sessions(&mut state.sessions, SESSION_CLEANUP_TIMEOUT);
            // A recovered Scrollback editor that stopped leaves no snapshot.
            state.retain_popup_files(Instant::now());
            state.scrollback.remove_empty_root();
            state.failed_starts.reap(log);
            let cleanup_succeeded = session_cleanup.is_ok();
            let mut errors = vec![error];
            if let Err(cleanup) = session_cleanup {
                errors.push(format!("cannot roll back terminals: {cleanup}"));
            }
            let error = startup_error(errors.join("; "));
            if state.failed_starts.is_empty() {
                drop(control_listener);
                if cleanup_succeeded {
                    remove_launcher(runtime);
                    let _ = fs::remove_dir(runtime);
                }
                return Err(error);
            }
            log.note(&error);
            state.stopping = Some(StopReason::BootstrapFailure);
            state.initial_status = Some(1);
            None
        }
    };
    // Terminals started after an admitted Venus spawned.
    state.name_presentation(true);

    let status = loop {
        let result: Result<i32, String> = (|| {
            loop {
                state.poll_registry(inputs, config, runtime, generation);
                if state.stopping.is_none() {
                    reap_finished_sessions(&mut state, inputs, config)?;
                }

                // An incomplete Stop keeps its owner for a retry, even with no
                // tracked terminal left, such as a final close whose untracked
                // runs could not be acquired.
                if state.may_exit() {
                    return Ok(state.initial_status.unwrap_or(0));
                }

                if reap_presentation(&mut state, inputs, config)? && state.stopping.is_none() {
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
                    Accepted::Deferred(stream) => {
                        if let Some(RegistryPending::Action {
                            stream: pending, ..
                        }) = &mut state.registry_pending
                        {
                            *pending = Some(stream);
                        }
                    }
                }
            }
        })();
        state.failed_starts.reap(log);
        if state.failed_starts.is_empty() {
            break result;
        }
        if let Err(error) = result {
            log.note(&error);
        }
        state.stopping.get_or_insert(StopReason::StopRequired);
        thread::sleep(Duration::from_millis(25));
    };
    // Stop, quit and the window's final close end every subscription.
    state.subscribers.end(generation);
    drop(control_listener);
    if let Some(process) = state.venus.take() {
        drop(process);
    }
    state.notices.close_and_wait(NOTICE_CLOSE_TIMEOUT);
    state.retain_popup_files(Instant::now());
    state.scrollback.remove_empty_root();
    remove_launcher(runtime);
    let _ = fs::remove_dir(runtime);
    status
}

fn await_initial_registration(
    listener: &ControlListener,
    worker: &mut session_registry::Worker,
    starting: &Runtime,
) -> Result<Option<Registration>, String> {
    loop {
        let accepted = listener.accept(|incoming| match incoming {
            Incoming::Stop(target) if target == starting.generation => Dispatched::Stop(
                eon_workspace_protocol::stop::Reply::Stopped {
                    generation: target,
                    terminals: Vec::new(),
                },
                true,
            ),
            Incoming::Stop(_) => Dispatched::Stop(
                stop_failure(failure(
                    "generation-mismatch",
                    "supervisor owns another generation",
                )),
                false,
            ),
            Incoming::Request(Request {
                action:
                    Action::Workspace(
                        WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation,
                    ),
                ..
            }) => Dispatched::Reply(
                ControlResponse::Lifecycle(LifecycleResponse::Runtime(starting.clone())),
                false,
            ),
            Incoming::Request(_) => Dispatched::Reply(
                ControlResponse::Workspace(Response::Failure(failure(
                    "registry-busy",
                    "initial session registration is pending; no terminal has started",
                ))),
                false,
            ),
        })?;
        if matches!(accepted, Accepted::Replied { shutdown: true }) {
            return Ok(None);
        }
        if let Some(result) = worker.poll() {
            return result.map_err(|error| error.to_string());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    BootstrapFailure,
    StopRequired,
}

struct SupervisorState {
    registry_worker: Option<session_registry::Worker>,
    registry_pending: Option<RegistryPending>,
    started_at: u64,
    window: Option<String>,
    running: Option<Running>,
    component_generation: String,
    application_id: String,
    decorations: bool,
    workspaces: Option<WorkspaceSet>,
    sessions: Vec<RunningSession>,
    failed_starts: sessions::FailedStarts,
    venus: Option<PresentationProcess>,
    /// Last accepted presentation, retained across detach and failed reload.
    venus_path: Option<PathBuf>,
    initial_child: Vec<OsString>,
    initial_status: Option<i32>,
    stopping: Option<StopReason>,
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
    editor_sockets: Sockets,
    log: Arc<DiagnosticLog>,
}

enum RegistryPending {
    Recovery(String),
    Action {
        request: Request,
        stream: Option<UnixStream>,
    },
}

impl SupervisorState {
    fn may_exit(&mut self) -> bool {
        self.terminals_finished() && self.stopping != Some(StopReason::StopRequired)
    }

    fn terminals_finished(&mut self) -> bool {
        self.failed_starts.reap(&self.log);
        self.sessions.iter().all(RunningSession::is_finished) && self.failed_starts.is_empty()
    }

    fn poll_registry(&mut self, inputs: &Inputs, config: &Path, runtime: &Path, generation: &str) {
        let Some(result) = self
            .registry_worker
            .as_mut()
            .and_then(|worker| worker.poll())
        else {
            return;
        };
        match self.registry_pending.take() {
            Some(RegistryPending::Recovery(target)) if self.stopping.is_none() => {
                if let Some(workspaces) = &mut self.workspaces {
                    workspaces.registered_recovery(&target, result);
                }
            }
            Some(RegistryPending::Action { request, stream }) => {
                if let Some(workspaces) = &mut self.workspaces {
                    workspaces.release_registration(&request.id);
                }
                let reply = match result {
                    Ok(Some(registration)) if self.stopping.is_none() => {
                        let reply = dispatch_registered_request(
                            request.clone(),
                            self,
                            inputs,
                            config,
                            runtime,
                            generation,
                            Some(registration),
                        );
                        if let Some(workspaces) = &mut self.workspaces {
                            workspaces.remember_registration(&request.id);
                        }
                        reply.0
                    }
                    result => {
                        let error = match result {
                            Err(error) => {
                                if (error.reserved_id.is_some()
                                    || error.publication
                                        == session_registry::Publication::Uncertain)
                                    && let Some(workspaces) = &mut self.workspaces
                                {
                                    workspaces.remember_registration(&request.id);
                                }
                                error.into_control()
                            }
                            Ok(None) => failure("unknown-session", "registered session is missing"),
                            Ok(Some(_)) => {
                                if let Some(workspaces) = &mut self.workspaces {
                                    workspaces.remember_registration(&request.id);
                                }
                                failure("stop-incomplete", "window is stopping")
                            }
                        };
                        ControlResponse::Workspace(Response::Failure(error))
                    }
                };
                self.serve_subscribers(Instant::now());
                if let Some(stream) = stream {
                    super::control::complete(stream, reply);
                }
            }
            _ => {}
        }
    }

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

    /// Removes private popup files after their terminals end (EON-C10).
    fn retain_popup_files(&mut self, now: Instant) {
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
        self.editor_sockets.retain(sessions, now, &self.log);
    }

    /// The loop turn's work before its control request: follows the window's
    /// terminals, focus and attention. The turn publishes it to subscribers
    /// once, with its action's change (EON-C39).
    fn tend(&mut self, now: Instant) {
        self.failed_starts.reap(&self.log);
        // Read first, so attention this turn sees the latest native focus.
        if self.attention.is_some()
            && let Some(venus) = &mut self.venus
        {
            venus.read_native_focus();
        }
        self.retain_popup_files(now);
        if self.stopping.is_none() {
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
        if self.stopping.is_some() {
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

/// The line that records a terminal's end, including signal numbers for
/// signalled programs and Orbit's exit status or signal when Orbit exited
/// without publishing its result.
fn session_end_line(session: &RunningSession, status: i32) -> String {
    match session.end() {
        Some(sessions::SessionEnd::Managed(_, ProcessOutcome::Signal(signal))) => format!(
            "terminal {} ended with status {} (signal {signal})",
            session.id,
            sessions::outcome_status(ProcessOutcome::Signal(signal)),
        ),
        Some(sessions::SessionEnd::OwnerLost(Some(ProcessOutcome::ExitCode(code)))) => format!(
            "terminal {} ended without a terminal result (Orbit status {code}); Orbit exited without publishing a terminal result{}",
            session.id,
            session.orbit_log_suffix(),
        ),
        Some(sessions::SessionEnd::OwnerLost(Some(ProcessOutcome::Signal(signal)))) => format!(
            "terminal {} ended without a terminal result (Orbit signal {signal}, status {}); Orbit exited without publishing a terminal result{}",
            session.id,
            sessions::outcome_status(ProcessOutcome::Signal(signal)),
            session.orbit_log_suffix(),
        ),
        Some(sessions::SessionEnd::OwnerLost(None)) => format!(
            "terminal {} ended with unknown status; Orbit exited without publishing a terminal result{}",
            session.id,
            session.orbit_log_suffix(),
        ),
        Some(sessions::SessionEnd::Managed(_, ProcessOutcome::ExitCode(_))) | None => {
            format!("terminal {} ended with status {status}", session.id)
        }
    }
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
                let failed_starts = &mut state.failed_starts;
                let initial_child = &mut state.initial_child;
                let editor_sockets = &mut state.editor_sockets;
                let log = &state.log;
                let result =
                    workspace.session_ended(&session.id, SessionEnd::Exited, |operation| {
                        apply_session_operation(
                            inputs,
                            config,
                            log,
                            component_generation,
                            sessions,
                            failed_starts,
                            initial_child,
                            editor_sockets,
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
        stop_closed_sessions(state, sessions, "tab-close-failed", &[])?;
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
    stop_closed_sessions(state, sessions, "pane-close-failed", &[])?;
    closed_response(
        state,
        runtime,
        generation,
        "pane-close-failed",
        reply_workspace.as_deref(),
    )
}

#[allow(clippy::too_many_arguments)]
fn close_workspace_column(
    request: &Request,
    pane: &str,
    consent: Option<&CloseConsent>,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Result<Response, Failure> {
    let (scope, candidates) =
        column_close_scope(request, pane, state, inputs, config, runtime, generation)?;
    if !consent.is_some_and(|consent| consent_matches(consent, &scope, generation)) {
        return Ok(Response::Confirm(scope));
    }
    // Keep topology fixed between comparing the scope and sending its stops.
    let reply_workspace = scope.terminals[0].target.workspace.clone();
    stop_consented(state, &scope, candidates)?;
    consented_close_response(state, runtime, generation, Some(&reply_workspace))
}

/// Rebuild the exact scope in this supervisor turn; marks never decide consent.
fn column_close_scope(
    request: &Request,
    pane: &str,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
) -> Result<(CloseScope, Vec<ManagedCandidate>), Failure> {
    reap_finished_sessions(state, inputs, config)
        .map_err(|detail| failure("column-close-failed", detail))?;
    let caller = caller_session(state, inputs, config, request)?;
    let workspace = state
        .workspaces
        .as_mut()
        .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
    let (closing, end) = workspace.prepare_close_column(&request.id, caller.as_deref(), pane)?;
    let mut snapshots = workspace.snapshots();
    for snapshot in &mut snapshots {
        state.pane_labels.present(snapshot, workspace.home());
    }
    let busy = state
        .attention
        .as_ref()
        .map(|attention| attention.busy_terminals(&snapshots, &state.sessions, Instant::now()));
    let (targets, _) = attention::targets(&snapshots);
    let mut terminals = Vec::new();
    for terminal in targets
        .iter()
        .map(|target| attention::busy_terminal(&snapshots, target))
        .filter(|terminal| closing.contains(&terminal.target.terminal))
    {
        let run = state
            .sessions
            .iter()
            .find(|session| session.id == terminal.target.terminal)
            .ok_or_else(|| {
                failure(
                    "column-close-failed",
                    format!(
                        "terminal {} is missing from the supervisor",
                        terminal.target.terminal
                    ),
                )
            })?;
        terminals.push(CloseTerminal {
            busy: busy
                .as_ref()
                .is_none_or(|busy| busy.iter().any(|item| item.target == terminal.target)),
            target: terminal.target,
            run: run.run_id().into(),
            name: terminal.name,
        });
    }
    let addressed = snapshots
        .iter()
        .find(|snapshot| {
            snapshot
                .tabs
                .iter()
                .any(|tab| tab.panes().any(|candidate| candidate.id == pane))
        })
        .expect("validated live pane");
    let session = addressed
        .workspaces
        .iter()
        .find(|member| member.id == addressed.workspace)
        .and_then(|member| member.name.as_deref())
        .unwrap_or(&addressed.workspace)
        .to_owned();
    let candidates = if end == CloseEnd::Window {
        untracked_runs(
            runtime,
            LaunchMode::Workspace,
            &state.component_generation,
            &state.sessions,
            Instant::now() + SESSION_CLEANUP_TIMEOUT,
        )
        .map_err(|detail| failure("column-close-failed", detail))?
    } else {
        Vec::new()
    };
    let untracked = candidates
        .iter()
        .map(|candidate| UntrackedRun {
            terminal: candidate.identity.session_id.clone(),
            run: candidate.identity.run_id.clone(),
        })
        .collect();
    Ok((
        CloseScope {
            generation: generation.into(),
            pane: pane.into(),
            end,
            session,
            terminals,
            untracked,
        },
        candidates,
    ))
}

fn consent_matches(consent: &CloseConsent, scope: &CloseScope, generation: &str) -> bool {
    let expected: std::collections::HashSet<_> = scope
        .terminals
        .iter()
        .map(|terminal| &terminal.run)
        .chain(scope.untracked.iter().map(|run| &run.run))
        .collect();
    let actual: std::collections::HashSet<_> = consent.runs.iter().collect();
    consent.generation == generation
        && scope.generation == generation
        && consent.end == scope.end
        && actual.len() == consent.runs.len()
        && actual == expected
}

fn stop_consented(
    state: &mut SupervisorState,
    scope: &CloseScope,
    candidates: Vec<ManagedCandidate>,
) -> Result<(), Failure> {
    let untracked = acquire_runs(
        candidates,
        state.log.orbit_directory().ok(),
        Instant::now() + SESSION_CLEANUP_TIMEOUT,
    )
    .map_err(|detail| failure("column-close-failed", detail))?;
    stop_consented_runs(state, scope, untracked)
}

fn stop_consented_runs(
    state: &mut SupervisorState,
    scope: &CloseScope,
    untracked: Vec<RunningSession>,
) -> Result<(), Failure> {
    state.sessions.extend(untracked);
    let untracked: Vec<_> = scope
        .untracked
        .iter()
        .map(|run| run.terminal.clone())
        .collect();
    let closing = scope
        .terminals
        .iter()
        .map(|terminal| terminal.target.terminal.clone())
        .chain(untracked.iter().cloned())
        .collect();
    let mut result = stop_closed_sessions(state, closing, "column-close-failed", &untracked);
    if let Err(error) = &mut result
        && !scope.untracked.is_empty()
    {
        // Untracked survivors return to discovery, which owns their later exit.
        state.sessions.retain(|session| {
            !scope
                .untracked
                .iter()
                .any(|run| run.run == session.run_id())
        });
        if state.sessions.is_empty() {
            state.stopping = Some(StopReason::StopRequired);
            *error = failure(
                "column-close-failed",
                format!(
                    "{}; {}",
                    stop_incomplete(
                        LaunchMode::Workspace,
                        state.window.as_deref(),
                        &scope.generation
                    ),
                    error.detail
                ),
            );
        }
    }
    result
}

/// A column close never sweeps: a late arrival stays for the person's Stop.
fn consented_close_response(
    state: &mut SupervisorState,
    runtime: &Path,
    generation: &str,
    reply_workspace: Option<&str>,
) -> Result<Response, Failure> {
    if !state.sessions.is_empty()
        && state
            .workspaces
            .as_ref()
            .is_some_and(WorkspaceSet::has_sessions)
    {
        return state
            .workspace_snapshot_in(reply_workspace)
            .map(Response::Snapshot);
    }
    state.stopping = Some(StopReason::StopRequired);
    let remaining = untracked_runs(
        runtime,
        LaunchMode::Workspace,
        &state.component_generation,
        &state.sessions,
        Instant::now() + SESSION_CLEANUP_TIMEOUT,
    );
    match remaining {
        Ok(runs) if runs.is_empty() => Ok(Response::Ended(Ended {
            generation: generation.into(),
        })),
        result => {
            let detail = match result {
                Ok(runs) => runs
                    .iter()
                    .map(|run| {
                        format!(
                            "{} started while the close ran and was not stopped",
                            run.identity.session_id
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
                Err(detail) => detail,
            };
            Err(failure(
                "column-close-failed",
                format!(
                    "{detail}; {}",
                    stop_incomplete(LaunchMode::Workspace, state.window.as_deref(), generation)
                ),
            ))
        }
    }
}

fn close_workspace(
    request: &Request,
    target: &WorkspaceTarget,
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
    stop_closed_sessions(state, sessions, "session-close-failed", &[])?;
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
    untracked: &[String],
) -> Result<(), Failure> {
    let (ended, errors) = stop_owned_sessions(&mut state.sessions, &sessions, &state.log);
    for (id, natural_status) in ended {
        if untracked.contains(&id) {
            continue;
        }
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
    stop_generation_sessions_until(
        state,
        runtime,
        mode,
        Instant::now() + SESSION_CLEANUP_TIMEOUT,
    )
}

fn stop_generation_sessions_until(
    state: &mut SupervisorState,
    runtime: &Path,
    mode: LaunchMode,
    deadline: Instant,
) -> Result<Vec<String>, String> {
    state.stopping = Some(StopReason::StopRequired);
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
        if session.is_finished()
            && !session.stopped()
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
    state.failed_starts.finish(deadline, &state.log)?;
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
    failed_starts: &mut sessions::FailedStarts,
    initial_child: &mut Vec<OsString>,
    editor_sockets: &mut Sockets,
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
        LaunchCommand::Editor(mut command) => {
            if let Some((kind, index)) = command.listener {
                let socket = editor_sockets.prepare(&session.endpoint, sessions, kind)?;
                command
                    .argv
                    .splice(index..index, ["--listen".into(), socket.into_os_string()]);
            }
            command.argv
        }
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
        Instant::now() + TERMINAL_START_TIMEOUT,
        failed_starts,
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
    let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
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
            && !session.stopped()
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
        failed_starts,
        initial_child,
        editor_sockets,
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
            failed_starts,
            initial_child,
            editor_sockets,
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
    if !status.success() {
        state.venus_path = None;
    }
    if state.stopping.is_none() {
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

fn validate_reload(inputs: &Inputs, venus: &Path, versions: [u16; 3]) -> Result<(), String> {
    let expected = inputs
        .presentation_protocols
        .as_ref()
        .map_err(Clone::clone)?;
    for ((name, requested), expected) in ["EONW", "ORBS", "ORBF"]
        .into_iter()
        .zip(versions)
        .zip(expected)
    {
        if requested != *expected {
            return Err(format!(
                "Venus uses {name} {requested}, but this window uses {expected}; move to a fresh window"
            ));
        }
    }
    let metadata = fs::metadata(venus).map_err(|error| format!("cannot inspect Venus: {error}"))?;
    if !venus.starts_with("/nix/store")
        || fs::canonicalize(venus)
            .map_err(|error| error.to_string())?
            .as_os_str()
            != venus.as_os_str()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o222 != 0
    {
        return Err("Venus must be a canonical immutable executable in /nix/store; bundle reload is unavailable".into());
    }
    Ok(())
}

fn reload_presentation(
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    target: &Path,
) -> Result<(), Failure> {
    let focused = |state: &mut SupervisorState| {
        if let Some(venus) = &mut state.venus {
            return venus.read_native_focus() && venus.viewing();
        }
        false
    };
    if !focused(state) {
        return Err(failure(
            "reload-refused",
            "Reload requires a focused Eon window; native focus is absent or unknown",
        ));
    }
    let path = config.join(managed_environment::CONFIG_FILE);
    managed_environment::read_configuration(&path)
        .and_then(|source| {
            managed_environment::validate_configuration_text(&source, &path, &inputs.defaults)
        })
        .map_err(|detail| failure("reload-refused", detail))?;
    let terminal =
        managed_environment::terminal_presentation(config, &inputs.defaults, Some(&state.log))
            .map_err(|detail| failure("reload-refused", detail))?;
    let bar = window_bar(LaunchMode::Workspace, config, inputs, Some(&state.log))
        .map_err(|detail| failure("reload-refused", detail))?;
    let candidate = venus_command(
        inputs,
        target,
        config,
        &runtime.join("eon.sock"),
        LaunchMode::Workspace,
        &terminal,
        &bar,
        &state.application_id,
        state.decorations,
        managed_environment::warning_close_last_tab(config, &inputs.defaults, Some(&state.log))
            .map_err(|detail| failure("reload-refused", detail))?,
        managed_environment::animate_working_badge(config, &inputs.defaults, Some(&state.log))
            .map_err(|detail| failure("reload-refused", detail))?,
        managed_environment::number_keys(config, &inputs.defaults, Some(&state.log))
            .map_err(|detail| failure("reload-refused", detail))?,
    );
    let accepted = state.venus.as_ref().unwrap();
    let previous = accepted.executable.clone();
    let mut rollback = venus_process_command(&previous, config, LaunchMode::Workspace);
    rollback.args(&accepted.arguments);
    // Both launch paths use flags resolved by this supervisor.
    let fallback = (previous != inputs.venus).then(|| {
        let mut command = venus_process_command(&inputs.venus, config, LaunchMode::Workspace);
        command.args(&accepted.arguments);
        (inputs.venus.clone(), command)
    });
    if !focused(state) {
        return Err(failure(
            "reload-refused",
            "Eon lost keyboard focus before reload; nothing replaced",
        ));
    }
    // Intentional replacement must not dismiss pending pickers or show a detach notice.
    drop(state.venus.take());
    let mut errors = Vec::new();
    for (path, command) in [(target.to_owned(), candidate), (previous, rollback)]
        .into_iter()
        .chain(fallback)
    {
        match PresentationProcess::start(
            command,
            true,
            state.workspace_snapshot(),
            PRESENTATION_ADMISSION_TIMEOUT,
            &mut state.sessions,
            &state.log,
        ) {
            Ok(venus) => {
                state.venus = Some(venus);
                state.venus_path = Some(path);
                return if errors.is_empty() {
                    if let Some(quotas) = &mut state.quotas {
                        quotas.show(&bar.widgets);
                    }
                    Ok(())
                } else {
                    Err(failure(
                        "reload-presentation-failed",
                        format!(
                            "{}; presentation restored with {}",
                            errors.join("; "),
                            state.venus_path.as_ref().unwrap().display()
                        ),
                    ))
                };
            }
            Err(error) => {
                state
                    .log
                    .record(&format!("Venus reload {}: {error}", path.display()));
                errors.push(error);
            }
        }
    }
    Err(failure(
        "reload-rollback-failed",
        format!(
            "{}; terminals and layout remain live for window attach",
            errors.join("; ")
        ),
    ))
}

pub(super) fn supervisor_lock_path(root: &Path, generation: &str) -> PathBuf {
    root.join(format!("supervisor-{generation}.lock"))
}

pub(super) struct LifecycleLock {
    file: fs::File,
    created: bool,
}

impl std::ops::Deref for LifecycleLock {
    type Target = fs::File;
    fn deref(&self) -> &fs::File {
        &self.file
    }
}

fn open_supervisor_lifecycle_lock(path: &Path) -> Result<LifecycleLock, String> {
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW);
    let (file, created) = match options.create_new(true).open(path) {
        Ok(file) => (file, true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
            options.create_new(false).open(path).map_err(|error| {
                format!("cannot open Eon lifecycle lock {}: {error}", path.display())
            })?,
            false,
        ),
        Err(error) => {
            return Err(format!(
                "cannot open Eon lifecycle lock {}: {error}",
                path.display()
            ));
        }
    };
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
    Ok(LifecycleLock { file, created })
}

/// Called while holding the lifecycle lock; no IDs are published before this barrier.
fn replacement_id_range(mut now: impl FnMut() -> io::Result<Duration>) -> Result<usize, String> {
    let mut sample =
        || now().map_err(|error| format!("cannot read supervisor boot clock: {error}"));
    let range = usize::try_from(sample()?.as_millis())
        .map_err(|_| "capacity: supervisor boot clock exceeds identity range")?;
    if range == 0 {
        return Err("capacity: a replacement cannot use identity range zero".into());
    }
    thread::sleep(Duration::from_millis(1));
    while sample()?.as_millis() <= range as u128 {
        thread::sleep(Duration::from_millis(1));
    }
    Ok(range)
}

pub(super) fn lock_supervisor_lifecycle(path: &Path) -> Result<LifecycleLock, String> {
    let file = open_supervisor_lifecycle_lock(path)?;
    let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT.saturating_add(Duration::from_secs(1));
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

pub(super) fn try_lock_supervisor_lifecycle(path: &Path) -> Result<Option<LifecycleLock>, String> {
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
    super::diagnostics::bench_trace("dispatch_begin", "");
    let request = match incoming {
        Incoming::Stop(target) => {
            let reply = stop_window(&target, state, runtime, generation);
            super::diagnostics::bench_trace("dispatch_end", "");
            state.serve_subscribers(Instant::now());
            return reply;
        }
        Incoming::Request(request) => request,
    };
    if matches!(request.action, Action::Subscribe { .. }) && state.stopping.is_none() {
        let reply = match subscribe(state, inputs, config, &request, generation) {
            Ok(subscriber) => Dispatched::Subscribe(Box::new(subscriber)),
            Err(error) => {
                Dispatched::Reply(ControlResponse::Workspace(Response::Failure(error)), false)
            }
        };
        super::diagnostics::bench_trace("dispatch_end", "");
        return reply;
    }
    super::diagnostics::bench_trace("request", &request.id);
    if state.stopping.is_none()
        && matches!(
            request.action,
            Action::Workspace(
                WorkspaceAction::NewWorkspace { .. } | WorkspaceAction::RenameWorkspace { .. }
            )
        )
    {
        let accepted = (|| {
            let caller = caller_session(state, inputs, config, &request)?;
            if matches!(
                request.action,
                Action::Workspace(WorkspaceAction::NewWorkspace { .. })
            ) {
                managed_environment::shell_command(config, &inputs.defaults, Some(&state.log))
                    .map_err(|detail| failure("invalid-config", detail))?;
            }
            let workspaces = state
                .workspaces
                .as_mut()
                .ok_or_else(|| failure("session-unavailable", "eonbase has no Eon session"))?;
            let operation =
                workspaces.prepare_registration(&request.id, caller.as_deref(), &request.action)?;
            let mut request = request;
            // Pin rename to the confirmed member before releasing the control turn.
            if let session_registry::Operation::Rename { id, .. } = &operation
                && let Action::Workspace(WorkspaceAction::RenameWorkspace { workspace, .. }) =
                    &mut request.action
            {
                *workspace = Some(WorkspaceTarget::PersistentId(id.clone()));
            }
            workspaces.reserve_registration(&request.id)?;
            let submitted = match state.registry_worker.as_mut() {
                Some(worker) => worker
                    .submit(operation)
                    .map_err(|error| error.into_control()),
                None => Err(failure(
                    "session-registry",
                    "registry worker is unavailable",
                )),
            };
            if let Err(error) = submitted {
                workspaces.release_registration(&request.id);
                return Err(error);
            }
            state.registry_pending = Some(RegistryPending::Action {
                request,
                stream: None,
            });
            Ok(())
        })();
        return match accepted {
            Ok(()) => Dispatched::Deferred,
            Err(error) => {
                Dispatched::Reply(ControlResponse::Workspace(Response::Failure(error)), false)
            }
        };
    }
    let reply = dispatch_control_request(request, state, inputs, config, runtime, generation);
    super::diagnostics::bench_trace("dispatch_end", "");
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
    if state.workspaces.is_some() || state.running.is_some() {
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
    dispatch_registered_request(request, state, inputs, config, runtime, generation, None)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_registered_request(
    request: Request,
    state: &mut SupervisorState,
    inputs: &Inputs,
    config: &Path,
    runtime: &Path,
    generation: &str,
    registration: Option<Registration>,
) -> (ControlResponse, bool) {
    let mode = launch_mode(state);
    if state.stopping.is_some()
        && !matches!(
            request.action,
            Action::Workspace(
                WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation
            )
        )
    {
        let error = failure(
            if matches!(
                request.action,
                Action::Workspace(WorkspaceAction::ReloadPresentation { .. })
            ) {
                "reload-refused"
            } else {
                "stop-incomplete"
            },
            stop_incomplete(mode, state.window.as_deref(), generation),
        );
        return (
            match request.action {
                Action::Workspace(
                    WorkspaceAction::Present { .. } | WorkspaceAction::ReloadPresentation { .. },
                ) => ControlResponse::Lifecycle(LifecycleResponse::Failure(error)),
                _ => ControlResponse::Workspace(Response::Failure(error)),
            },
            false,
        );
    }
    match request {
        Request {
            action:
                Action::Workspace(WorkspaceAction::ReloadPresentation {
                    venus,
                    eonw,
                    orbs,
                    orbf,
                }),
            ..
        } => {
            let result = if mode != LaunchMode::Workspace {
                Err(failure(
                    "reload-refused",
                    "Reload is available only for full Eon windows",
                ))
            } else {
                validate_reload(inputs, Path::new(&venus), [eonw, orbs, orbf])
                    .map_err(|detail| failure("reload-refused", detail))
                    .and_then(|()| {
                        reload_presentation(state, inputs, config, runtime, Path::new(&venus))
                    })
            };
            let response = match result {
                Ok(()) => runtime_status(inputs, generation, state, mode, None)
                    .map(LifecycleResponse::Runtime)
                    .unwrap_or_else(|detail| {
                        LifecycleResponse::Failure(failure("reload-presentation-failed", detail))
                    }),
                Err(error) => LifecycleResponse::Failure(error),
            };
            (ControlResponse::Lifecycle(response), false)
        }
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
                    if state.stopping.is_some() {
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
                    state.failed_starts.is_empty(),
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
                    state.venus_path.as_deref().unwrap_or(&inputs.venus),
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
                state.venus = Some(
                    PresentationProcess::start(
                        command,
                        terminal.requires_startup_admission(),
                        state.workspace_snapshot(),
                        PRESENTATION_ADMISSION_TIMEOUT,
                        &mut state.sessions,
                        &state.log,
                    )
                    .inspect_err(|_| state.venus_path = None)?,
                );
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
            action:
                Action::CloseColumn {
                    ref pane,
                    ref consent,
                },
            ..
        } => closed(close_workspace_column(
            &request,
            pane,
            consent.as_ref(),
            state,
            inputs,
            config,
            runtime,
            generation,
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
            if matches!(
                request.action,
                Action::Workspace(WorkspaceAction::NewWorkspace { .. })
            ) && let Err(detail) =
                managed_environment::shell_command(config, &inputs.defaults, Some(&state.log))
            {
                return (
                    ControlResponse::Workspace(Response::Failure(failure(
                        "invalid-config",
                        detail,
                    ))),
                    false,
                );
            }
            let Some(workspace) = &mut state.workspaces else {
                unreachable!("reconciliation keeps the workspace owner");
            };
            let reply_workspace =
                match workspace
                    .caller_workspace_id(caller.as_deref())
                    .and_then(|caller| match &request.action {
                        Action::Workspace(WorkspaceAction::InspectWorkspace {
                            workspace: target,
                        }) => workspace.inspected_workspace_id(target).map(Some),
                        _ => Ok(caller),
                    }) {
                    Ok(workspace) => workspace,
                    Err(error) => {
                        return (ControlResponse::Workspace(Response::Failure(error)), false);
                    }
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
            if matches!(
                action,
                Action::ToggleSidebar | Action::SetSidebar { open: true }
            ) && state
                .attention
                .as_ref()
                .is_none_or(|owner| owner.sidebar() == eon_workspace_protocol::v43::Sidebar::Off)
            {
                return (
                    ControlResponse::Workspace(Response::Failure(failure(
                        "unavailable",
                        "the agent sidebar is off; set [sidebar] enabled = true",
                    ))),
                    false,
                );
            }
            let accepted = matches!(
                action,
                Action::ToggleSidebar | Action::SetSidebar { .. } | Action::FocusAttention { .. }
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
            let failed_starts = &mut state.failed_starts;
            let initial_child = &mut state.initial_child;
            let editor_sockets = &mut state.editor_sockets;
            let log = &state.log;
            let result = match (&jump, &action) {
                (Some((target_workspace, _)), Action::FocusAttention { tab, target }) => workspace
                    .jump(
                        &request.id,
                        caller.as_deref(),
                        target_workspace,
                        tab,
                        target,
                    ),
                (_, Action::Edit { file }) => workspace.edit(
                    &request.id,
                    caller.as_deref(),
                    file.clone(),
                    |directory, file, live, available| {
                        managed_environment::prepare_edit(
                            config,
                            &inputs.defaults,
                            &inputs.managed,
                            directory,
                            file,
                            live,
                            available,
                            Some(log),
                        )
                    },
                    |session, file, kind, key| {
                        editor::reuse(&session.endpoint, file, &request.id, kind, key)
                    },
                    |operation| {
                        apply_session_operation(
                            inputs,
                            config,
                            log,
                            component_generation,
                            sessions,
                            failed_starts,
                            initial_child,
                            editor_sockets,
                            operation,
                            PairStart::Later,
                            venus,
                        )
                    },
                ),
                _ => workspace.apply_action(
                    &request.id,
                    caller.as_deref(),
                    action,
                    registration,
                    |command, directory| {
                        let argv = managed_environment::prepare_popup_command(
                            &snapshot_command(command, snapshot.as_ref()),
                            &inputs.managed,
                            directory,
                            config,
                            &inputs.defaults,
                            Some(log),
                        )?;
                        Ok(argv)
                    },
                    |operation| {
                        mark_before_start(snapshot.as_ref(), &mut marked, operation, |operation| {
                            apply_session_operation(
                                inputs,
                                config,
                                log,
                                component_generation,
                                sessions,
                                failed_starts,
                                initial_child,
                                editor_sockets,
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
    let failed_starts = &mut state.failed_starts;
    let initial_child = &mut state.initial_child;
    let editor_sockets = &mut state.editor_sockets;
    let log = &state.log;
    // The target's tab is the shown workspace's active tab, which a request
    // without a caller acts on.
    let result = workspace.apply_action(
        &request.id,
        None,
        Action::InvokePopup {
            tab: target.tab.clone(),
            entry,
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
        None,
        |command, directory| {
            managed_environment::prepare_popup_command(
                &snapshot_command(command, Some(&snapshot)),
                &inputs.managed,
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
                    failed_starts,
                    initial_child,
                    editor_sockets,
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
    if mode == LaunchMode::Terminal && state.sessions.is_empty() && state.stopping.is_none() {
        return Err("supervisor has no live terminals".into());
    }
    Ok(Runtime {
        venus: state
            .venus
            .as_ref()
            .map(|venus| venus.executable.to_string_lossy().into_owned()),
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
        Action, DiagnosticLog, Dispatched, LaunchMode, PairStart, SESSION_CLEANUP_TIMEOUT,
        StopReason, SupervisorState, WorkspaceSet, effective_uid, mark_before_start, pair_command,
        prepare_configuration, prepare_runtime, reap_finished_sessions, settle_snapshot,
        stop_generation_sessions, stop_owned_session, stop_owned_sessions, temporary_directory,
        venus_command, window_bar, xdg_path,
    };
    use crate::editor::Sockets;
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

    #[test]
    fn lifecycle_lock_creation_claims_short_ids_only_once() {
        let root = temporary_directory();
        fs::create_dir_all(&root).unwrap();
        let path = root.join("lifecycle.lock");
        let first = super::try_lock_supervisor_lifecycle(&path)
            .unwrap()
            .unwrap();
        assert!(first.created);
        assert!(
            super::try_lock_supervisor_lifecycle(&path)
                .unwrap()
                .is_none()
        );
        // The first attempt publishes nothing; its claim still cannot be reused.
        drop(first);
        let replacement = super::lock_supervisor_lifecycle(&path).unwrap();
        assert!(!replacement.created);
        let range = super::replacement_id_range(crate::platform::boot_time).unwrap();
        assert!(range > 0);
        drop(replacement);
        let next = super::lock_supervisor_lifecycle(&path).unwrap();
        assert!(!next.created);
        assert!(super::replacement_id_range(crate::platform::boot_time).unwrap() > range);
        drop(next);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_range_waits_for_the_clock_before_publication() {
        let mut samples = [1000, 1000, 1001].into_iter();
        assert_eq!(
            super::replacement_id_range(|| Ok(Duration::from_millis(samples.next().unwrap())))
                .unwrap(),
            1000
        );
        assert!(samples.next().is_none());
        assert!(
            super::replacement_id_range(|| Ok(Duration::ZERO))
                .unwrap_err()
                .starts_with("capacity:")
        );
        assert!(
            super::replacement_id_range(|| Err(std::io::Error::other("clock unavailable")))
                .unwrap_err()
                .contains("clock unavailable")
        );
    }

    /// Scrollback's record names its editor's terminal before that Start
    /// runs, and the started popup owns the snapshot. A record that cannot be
    /// written starts nothing and leaves no snapshot. A failed start keeps
    /// its snapshot only while its launch record says the editor may still
    /// run (EON-C10).
    fn consent_of(scope: &super::CloseScope) -> super::CloseConsent {
        super::CloseConsent {
            generation: scope.generation.clone(),
            end: scope.end,
            runs: scope
                .terminals
                .iter()
                .map(|terminal| terminal.run.clone())
                .chain(scope.untracked.iter().map(|run| run.run.clone()))
                .collect(),
        }
    }

    #[test]
    fn retained_starts_keep_an_empty_owner_alive_and_make_stop_retryable() {
        let root = temporary_directory();
        let (failed_starts, input, pid) = sessions::tests::retained_start(&root);
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eonbase");
        let mut state = SupervisorState {
            failed_starts,
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions: Vec::new(),
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: Snapshots::new(&root),
            editor_sockets: Sockets::default(),
            log,
        };
        assert!(
            !state.terminals_finished(),
            "empty topology abandoned its retained child"
        );
        let stopped = super::stop_generation_sessions_until(
            &mut state,
            &root,
            LaunchMode::Terminal,
            Instant::now() + Duration::from_millis(20),
        );
        assert!(
            stopped
                .unwrap_err()
                .contains("retained children are still running")
        );
        assert!(state.stopping.is_some() && !state.terminals_finished());
        assert!(
            crate::platform::process(pid).unwrap().is_some(),
            "Stop signalled an unleased child"
        );
        drop(input);
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
        while !state.terminals_finished() {
            assert!(Instant::now() < deadline, "retained child did not exit");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            !state.may_exit(),
            "failed Stop exited without its explicit retry"
        );
        assert!(
            super::stop_generation_sessions_until(
                &mut state,
                &root,
                LaunchMode::Terminal,
                Instant::now() + SESSION_CLEANUP_TIMEOUT,
            )
            .unwrap()
            .is_empty()
        );
        assert!(state.terminals_finished());
        assert!(crate::platform::process(pid).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unconfirmed_initial_start_exits_when_its_child_is_reaped() {
        const GENERATION: &str = "g1-00000000000000000000000000000001";
        let root = temporary_directory();
        let runtime = root.join("generations").join(GENERATION);
        prepare_runtime(&runtime).unwrap();
        let pid_file = root.join("child.pid");
        let gate = root.join("child-exit");
        let launcher = root.join("marked-orbit");
        script(
            &launcher,
            &format!(
                "#!/bin/sh\nprintf 1 > \"$2.record\"\nprintf '%s' \"$$\" > '{}'\ncount=0\nwhile [ ! -e '{}' ] && [ \"$count\" -lt 1000 ]; do count=$((count+1)); sleep 0.01; done\n",
                pid_file.display(),
                gate.display(),
            ),
        );
        let worker_root = root.clone();
        let worker_runtime = runtime.clone();
        let supervisor = std::thread::spawn(move || {
            let mut inputs = crate::fixtures::inputs();
            inputs.orbit = launcher;
            let log = DiagnosticLog::open_in(Ok(worker_root.join("logs")), GENERATION, "eonbase");
            let lock =
                super::lock_supervisor_lifecycle(&worker_root.join("lifecycle.lock")).unwrap();
            super::supervise(
                &inputs,
                &worker_root,
                inputs.defaults.terminal.clone(),
                &super::window_bar(LaunchMode::Terminal, &worker_root, &inputs, None).unwrap(),
                None,
                None,
                lock,
                &worker_runtime,
                &[],
                Some(worker_root.clone()),
                GENERATION,
                LaunchMode::Terminal,
                "",
                false,
                &log,
                Duration::from_millis(250),
            )
        });
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
        let pid = loop {
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "fake Orbit was not started");
            std::thread::sleep(Duration::from_millis(1));
        };
        let socket = runtime.join("eon.sock");
        let inspected = crate::control::send_action_from_until(
            &socket,
            None,
            Action::Workspace(eon_workspace_protocol::v43::WorkspaceAction::InspectRuntime),
            deadline,
        )
        .unwrap();
        assert!(
            matches!(inspected, crate::control::ControlResponse::Lifecycle(
            eon_workspace_protocol::v43::LifecycleResponse::Runtime(runtime)
        ) if !runtime.attach.available && runtime.attach.reason.contains("cleanup is incomplete"))
        );
        assert!(
            !supervisor.is_finished(),
            "startup failure discarded its owner"
        );
        assert!(crate::platform::process(pid).unwrap().is_some());
        fs::remove_file(runtime.join("orbit.sock.record")).unwrap();
        fs::write(gate, "exit").unwrap();
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
        while !supervisor.is_finished() {
            assert!(
                Instant::now() < deadline,
                "bootstrap owner did not exit after reaping its child"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(supervisor.join().unwrap().unwrap(), 1);
        assert!(crate::platform::process(pid).unwrap().is_none());
        assert!(!socket.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    fn column_request(
        state: &mut SupervisorState,
        root: &Path,
        pane: &str,
        consent: Option<&super::CloseConsent>,
    ) -> Result<super::Response, super::Failure> {
        let request = super::Request {
            id: super::request_id(),
            caller: None,
            action: Action::CloseColumn {
                pane: pane.into(),
                consent: consent.cloned(),
            },
        };
        super::close_workspace_column(
            &request,
            pane,
            consent,
            state,
            &crate::fixtures::inputs(),
            root,
            root,
            "g1-test",
        )
    }

    #[cfg(target_os = "linux")]
    fn column_scope(state: &mut SupervisorState, root: &Path, pane: &str) -> super::CloseScope {
        let super::Response::Confirm(scope) = column_request(state, root, pane, None).unwrap()
        else {
            panic!("every close asks")
        };
        scope
    }

    #[cfg(target_os = "linux")]
    fn start_column_run(
        state: &SupervisorState,
        root: &Path,
        id: &str,
        endpoint: &Path,
    ) -> sessions::RunningSession {
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = std::env::var_os("ORBIT_BIN").unwrap().into();
        super::start_orbit(
            &inputs,
            root,
            endpoint,
            id,
            "component-1",
            root,
            &["sh".into(), "-c".into(), "exec sleep 300".into()],
            None,
            &state.log,
            Instant::now() + SESSION_CLEANUP_TIMEOUT,
            &mut sessions::FailedStarts::default(),
        )
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn column_mutate(state: &mut SupervisorState, root: &Path, action: Action) {
        let mut starts = Vec::new();
        state
            .workspaces
            .as_mut()
            .unwrap()
            .dispatch(
                &super::request_id(),
                None,
                action,
                |_, _| Ok(Default::default()),
                |operation| {
                    if let SessionOperation::Start { session, .. } = operation {
                        starts.push(session);
                    }
                    Ok(())
                },
            )
            .unwrap();
        for session in starts {
            let run = start_column_run(state, root, &session.id, &session.endpoint);
            state.sessions.push(run);
        }
    }

    #[test]
    fn a_consent_matches_on_generation_runs_and_end_only() {
        use eon_workspace_protocol::v43::{
            CloseEnd, CloseScope, CloseTerminal, TargetId, TerminalTarget, UntrackedRun,
        };
        let mut scope = CloseScope {
            generation: "g1".into(),
            pane: "p1".into(),
            end: CloseEnd::Window,
            session: "s1".into(),
            terminals: vec![CloseTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p1".into()),
                    terminal: "terminal-1".into(),
                },
                run: "run-a".into(),
                name: "shell".into(),
                busy: false,
            }],
            untracked: vec![UntrackedRun {
                terminal: "terminal-9".into(),
                run: "run-b".into(),
            }],
        };
        let consent = consent_of(&scope);
        assert!(super::consent_matches(&consent, &scope, "g1"));
        let mut reversed = consent.clone();
        reversed.runs.reverse();
        assert!(super::consent_matches(&reversed, &scope, "g1"));
        scope.terminals[0].name = "cargo".into();
        scope.terminals[0].busy = true;
        assert!(super::consent_matches(&consent, &scope, "g1"));
        for bad in [
            super::CloseConsent {
                generation: "g2".into(),
                ..consent.clone()
            },
            super::CloseConsent {
                end: CloseEnd::Session,
                ..consent.clone()
            },
            super::CloseConsent {
                runs: vec!["run-a".into()],
                ..consent.clone()
            },
            super::CloseConsent {
                runs: vec!["run-a".into(), "run-b".into(), "run-c".into()],
                ..consent.clone()
            },
            super::CloseConsent {
                runs: vec!["run-a".into(), "run-b".into(), "run-a".into()],
                ..consent.clone()
            },
        ] {
            assert!(!super::consent_matches(&bad, &scope, "g1"));
        }
        assert!(!super::consent_matches(&consent, &scope, "g2"));
        scope.terminals[0].run = "replacement".into();
        assert!(!super::consent_matches(&consent, &scope, "g1"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_column_close_never_runs_without_a_consent_that_names_its_runs() {
        use eon_workspace_protocol::v43::{CloseEnd, Side};
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        // A one-pane final column always asks, even once the default shell is idle.
        state.sessions[0].default_shell = true;
        let deadline = Instant::now() + Duration::from_secs(10);
        let idle = loop {
            state.tend(Instant::now());
            let scope = column_scope(&mut state, &root, "p1");
            if !scope.terminals[0].busy {
                break scope;
            }
            assert!(Instant::now() < deadline, "no idle foreground observation");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(idle.end, CloseEnd::Window);
        assert_eq!(state.sessions.len(), 1);
        // A changed busy mark alone does not change consent.
        state.sessions[0].default_shell = false;
        let busy = column_scope(&mut state, &root, "p1");
        assert!(busy.terminals[0].busy);
        assert!(super::consent_matches(&consent_of(&idle), &busy, "g1-test"));
        column_mutate(
            &mut state,
            &root,
            Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
        );
        let changed = column_request(&mut state, &root, "p1", Some(&consent_of(&idle))).unwrap();
        assert!(matches!(changed, super::Response::Confirm(_)));
        assert_eq!(state.sessions.len(), 2);
        let two = column_scope(&mut state, &root, "p1");
        column_mutate(&mut state, &root, Action::CreateColumn);
        let moved = column_request(&mut state, &root, "p1", Some(&consent_of(&two))).unwrap();
        assert!(
            matches!(moved, super::Response::Confirm(_)),
            "end changed to column"
        );
        column_mutate(
            &mut state,
            &root,
            Action::Workspace(WorkspaceAction::FocusId("p1".into())),
        );
        let before_move = column_scope(&mut state, &root, "p1");
        column_mutate(&mut state, &root, Action::MovePaneSideways(Side::Right));
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&before_move))).unwrap(),
            super::Response::Confirm(_)
        ));
        // A listed pane exits: the old set is refused.
        let before_exit = column_scope(&mut state, &root, "p1");
        let exited = before_exit
            .terminals
            .iter()
            .find(|terminal| terminal.target.id != super::TargetId::Pane("p1".into()))
            .unwrap()
            .target
            .terminal
            .clone();
        stop_owned_session(&mut state.state.sessions, &exited, &state.state.log).unwrap();
        state
            .workspaces
            .as_mut()
            .unwrap()
            .session_ended(&exited, crate::workspace::SessionEnd::Closed, |_| Ok(()))
            .unwrap();
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&before_exit))).unwrap(),
            super::Response::Confirm(_)
        ));
        // Match closes precisely this column and leaves the other live column alone.
        let scope = column_scope(&mut state, &root, "p1");
        let expected = state.sessions.len() - scope.terminals.len();
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&scope))).unwrap(),
            super::Response::Snapshot(_)
        ));
        assert_eq!(state.sessions.len(), expected);
        // A final close that gains another session asks again.
        let anchor = state.workspaces.as_ref().unwrap().snapshot().tabs[0]
            .panes()
            .next()
            .unwrap()
            .id
            .clone();
        column_mutate(
            &mut state,
            &root,
            Action::LaunchTab {
                directory: root.as_os_str().as_encoded_bytes().to_vec(),
                argv: Vec::new(),
                focus: false,
            },
        );
        let nonfinal = column_scope(&mut state, &root, &anchor);
        assert_eq!(nonfinal.end, CloseEnd::Column);
        let tab = state
            .workspaces
            .as_ref()
            .unwrap()
            .snapshot()
            .tabs
            .last()
            .unwrap()
            .id
            .clone();
        let request = super::Request {
            id: super::request_id(),
            caller: None,
            action: Action::Workspace(WorkspaceAction::CloseTab { tab: tab.clone() }),
        };
        super::close_workspace_tab(
            &request,
            &tab,
            &mut state,
            &crate::fixtures::inputs(),
            &root,
            &root,
            "g1-test",
        )
        .unwrap();
        assert!(
            matches!(column_request(&mut state, &root, &anchor, Some(&consent_of(&nonfinal))).unwrap(),
            super::Response::Confirm(scope) if scope.end == CloseEnd::Window)
        );
        let final_scope = column_scope(&mut state, &root, &anchor);
        assert_eq!(final_scope.end, CloseEnd::Window);
        column_mutate(
            &mut state,
            &root,
            Action::Workspace(WorkspaceAction::NewWorkspace {
                name: None,
                focus: false,
            }),
        );
        let changed =
            column_request(&mut state, &root, &anchor, Some(&consent_of(&final_scope))).unwrap();
        assert!(
            matches!(changed, super::Response::Confirm(scope) if scope.end == CloseEnd::Session)
        );
        let scope = column_scope(&mut state, &root, &anchor);
        let record = sessions::tests::record(&state.sessions[0]).to_path_buf();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        let error =
            column_request(&mut state, &root, &anchor, Some(&consent_of(&scope))).unwrap_err();
        assert_eq!(error.code, "column-close-failed");
        assert!(
            state
                .workspaces
                .as_ref()
                .unwrap()
                .snapshot()
                .tabs
                .iter()
                .any(|tab| tab.panes().any(|pane| pane.id == anchor)),
            "a failed stop keeps its pane"
        );
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        reap_finished_sessions(&mut state, &crate::fixtures::inputs(), &root).unwrap();
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_consent_does_not_cover_a_run_that_reused_its_terminal_number() {
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        let endpoint = root.join("terminal-9.sock");
        let a = start_column_run(&state, &root, "terminal-9", &endpoint);
        let run_a = a.run_id().to_owned();
        let first = column_scope(&mut state, &root, "p1");
        assert_eq!(first.untracked[0].run, run_a);
        let mut leased = vec![a];
        sessions::stop_managed_sessions(&mut leased, SESSION_CLEANUP_TIMEOUT).unwrap();
        drop(leased);
        column_scope(&mut state, &root, "p1"); // retires A before the number is reused
        let b = start_column_run(&state, &root, "terminal-9", &endpoint);
        let run_b = b.run_id().to_owned();
        assert_ne!(run_a, run_b);
        assert!(
            matches!(column_request(&mut state, &root, "p1", Some(&consent_of(&first))).unwrap(), super::Response::Confirm(scope) if scope.untracked[0].run == run_b)
        );
        assert!(
            sessions::session_finished(&mut state.sessions[0])
                .unwrap()
                .is_none()
        );
        state.sessions.push(b);
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        // A failed pane start leaves its number free while its run can still be live.
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        let mut workspace = state.workspaces.take().unwrap();
        let mut failed = Vec::new();
        assert!(
            workspace
                .dispatch(
                    &super::request_id(),
                    None,
                    Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
                    |_, _| Ok(Default::default()),
                    |operation| {
                        if let SessionOperation::Start { session, .. } = operation {
                            failed.push(start_column_run(
                                &state,
                                &root,
                                &session.id,
                                &session.endpoint,
                            ));
                            return Err("start timed out and rollback failed".into());
                        }
                        Ok(())
                    }
                )
                .is_err()
        );
        state.workspaces = Some(workspace);
        let first = column_scope(&mut state, &root, "p1");
        assert_eq!(first.untracked[0].terminal, "terminal-2");
        sessions::stop_managed_sessions(&mut failed, SESSION_CLEANUP_TIMEOUT).unwrap();
        drop(failed);
        column_scope(&mut state, &root, "p1");
        column_mutate(
            &mut state,
            &root,
            Action::Workspace(WorkspaceAction::CreatePane { focus: false }),
        );
        assert!(
            matches!(column_request(&mut state, &root, "p1", Some(&consent_of(&first))).unwrap(), super::Response::Confirm(scope)
                if scope.untracked.is_empty() && scope.terminals.iter().any(|terminal|
                    terminal.target.terminal == "terminal-2" && terminal.run != first.untracked[0].run))
        );
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_final_column_close_stops_only_the_runs_it_listed() {
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        let held = start_column_run(&state, &root, "terminal-9", &root.join("terminal-9.sock"));
        let scope = column_scope(&mut state, &root, "p1");
        assert_eq!(scope.untracked.len(), 1);
        let mut missing = consent_of(&scope);
        missing.runs.pop();
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&missing)).unwrap(),
            super::Response::Confirm(_)
        ));
        // A lease held elsewhere fails before any tracked terminal is stopped.
        assert_eq!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&scope)))
                .unwrap_err()
                .code,
            "column-close-failed"
        );
        assert!(
            sessions::session_finished(&mut state.sessions[0])
                .unwrap()
                .is_none()
        );
        sessions::tests::release_lease(held);
        fs::write(root.join("bad.record"), "malformed").unwrap();
        assert_eq!(
            column_request(&mut state, &root, "p1", None)
                .unwrap_err()
                .code,
            "column-close-failed"
        );
        assert!(
            sessions::session_finished(&mut state.sessions[0])
                .unwrap()
                .is_none()
        );
        fs::remove_file(root.join("bad.record")).unwrap();
        let request = super::Request {
            id: super::request_id(),
            caller: None,
            action: Action::CloseColumn {
                pane: "p1".into(),
                consent: Some(consent_of(&scope)),
            },
        };
        let (scope, candidates) = super::column_close_scope(
            &request,
            "p1",
            &mut state,
            &crate::fixtures::inputs(),
            &root,
            &root,
            "g1-test",
        )
        .unwrap();
        super::stop_consented(&mut state, &scope, candidates).unwrap();
        assert!(state.sessions.is_empty());
        let mut late =
            start_column_run(&state, &root, "terminal-10", &root.join("terminal-10.sock"));
        let error =
            super::consented_close_response(&mut state, &root, "g1-test", None).unwrap_err();
        assert_eq!(error.code, "column-close-failed");
        assert!(
            error
                .detail
                .contains("terminal-10 started while the close ran and was not stopped")
        );
        assert!(error.detail.contains("eon window stop "));
        assert!(state.stopping.is_some());
        assert!(sessions::session_finished(&mut late).unwrap().is_none());
        state.sessions.push(late);
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        let extra = start_column_run(&state, &root, "terminal-9", &root.join("terminal-9.sock"));
        let extra_run = extra.run_id().to_owned();
        sessions::tests::release_lease(extra);
        let scope = column_scope(&mut state, &root, "p1");
        assert_eq!(
            scope.untracked,
            [super::UntrackedRun {
                terminal: "terminal-9".into(),
                run: extra_run,
            }]
        );
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&scope))).unwrap(),
            super::Response::Ended(_)
        ));
        assert!(state.stopping.is_some() && state.sessions.is_empty());
        drop(state);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_close_after_a_failed_column_stop_retries_the_old_leases_busy() {
        for stop in [false, true] {
            let root = temporary_directory();
            let (mut state, _) = finishing_window(&root);
            sessions::stop_managed_sessions(&mut state.sessions, SESSION_CLEANUP_TIMEOUT).unwrap();
            state.sessions.clear();
            let tracked = sessions::tests::lease_race_host(&root, 1, "column");
            let untracked = sessions::tests::lease_race_host(&root, 9, "column");
            let mut runs = sessions::acquire_generation_sessions(
                &root,
                LaunchMode::Workspace,
                "component-1",
                &[],
                None,
                Instant::now() + SESSION_CLEANUP_TIMEOUT,
            )
            .unwrap();
            let extra = runs.pop().unwrap();
            state.sessions = runs;
            let scope = column_scope(&mut state, &root, "p1");
            assert_eq!(scope.untracked.len(), 1);
            assert_eq!(
                super::stop_consented_runs(&mut state, &scope, vec![extra])
                    .unwrap_err()
                    .code,
                "column-close-failed"
            );
            assert_eq!(state.sessions.len(), 1);
            // The next request runs immediately; the peer still owns the old lease until after Busy.
            if stop {
                assert_eq!(
                    stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap(),
                    ["terminal-1", "terminal-9"]
                );
            } else {
                assert!(matches!(
                    column_request(&mut state, &root, "p1", Some(&consent_of(&scope))).unwrap(),
                    super::Response::Ended(_)
                ));
            }
            assert!(state.sessions.is_empty());
            assert_eq!(tracked.finish(), ["leased", "stop-refused", "stopped"]);
            assert_eq!(
                untracked.finish(),
                [
                    "leased",
                    "stop-refused",
                    "busy",
                    "refused-closed",
                    "old-ended",
                    "leased",
                    "stopped"
                ]
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_untracked_column_stops_stay_discoverable_and_keep_the_supervisor_available() {
        for keep_pane in [true, false] {
            let root = temporary_directory();
            let (mut state, _) = finishing_window(&root);
            if !keep_pane {
                let logs = (0..8).fold(root.join("logs"), |path, _| path.join("l".repeat(120)));
                state.log = DiagnosticLog::open_in(Ok(logs), "g1-test", "eon");
            }
            let endpoint = root.join("terminal-9.sock");
            let extra = start_column_run(&state, &root, "terminal-9", &endpoint);
            let extra_pid = sessions::tests::orbit_pid(&extra);
            let extra_run = extra.run_id().to_owned();
            let context = format!(
                "keep_pane={keep_pane}, Orbit pid {extra_pid}, run {extra_run}, endpoint {}",
                endpoint.display()
            );
            let scope = column_scope(&mut state, &root, "p1");
            assert_eq!(scope.untracked.len(), 1, "{context}: {scope:?}");
            assert_eq!(scope.untracked[0].run, extra_run, "{context}");
            sessions::tests::release_lease(extra);
            let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
            let acquired = sessions::acquire_generation_sessions(
                &root,
                LaunchMode::Workspace,
                "component-1",
                &state.sessions,
                state.log.orbit_directory().ok(),
                deadline,
            )
            .unwrap_or_else(|error| panic!("{context}: discover and acquire before Stop: {error}"));
            // SAFETY: freezes only the test-owned host after acquiring its lease.
            assert_eq!(unsafe { libc::kill(extra_pid, libc::SIGSTOP) }, 0);
            let pane_pid = sessions::tests::orbit_pid(&state.sessions[0]);
            if keep_pane {
                // SAFETY: freezes the other host started by this test.
                assert_eq!(unsafe { libc::kill(pane_pid, libc::SIGSTOP) }, 0);
                // An unanswered poll prevents sending Stop to the surviving pane.
                assert!(sessions::send_program_status(&mut state.sessions[0], deadline).unwrap());
            }
            let Err(error) = super::stop_consented_runs(&mut state, &scope, acquired) else {
                panic!("{context}: Stop of frozen Orbit unexpectedly succeeded");
            };
            assert_eq!(error.code, "column-close-failed", "{context}: {error:?}");
            assert!(error.detail.contains("terminal-9"), "{context}: {error:?}");
            assert!(
                eon_workspace_protocol::v43::encode_response(&super::Response::Failure(
                    error.clone()
                ))
                .is_ok(),
                "the close failure must fit EONW even with a long Orbit log path"
            );
            assert_eq!(state.sessions.len(), usize::from(keep_pane), "{context}");
            assert!(
                !state
                    .sessions
                    .iter()
                    .any(|session| session.run_id() == extra_run),
                "{context}: failed untracked run stayed in the supervisor's sessions"
            );
            if keep_pane {
                assert!(state.stopping.is_none());
                let retry = column_scope(&mut state, &root, "p1");
                assert_eq!(
                    retry.untracked, scope.untracked,
                    "{context}: retry must still list the live run"
                );
                // SAFETY: resumes the test-owned host frozen above.
                assert_eq!(unsafe { libc::kill(pane_pid, libc::SIGCONT) }, 0);
            } else {
                assert!(
                    state.stopping.is_some(),
                    "an empty window stays for an explicit Stop"
                );
                assert!(error.detail.contains("eon window stop "), "{error:?}");
                assert_eq!(
                    super::consented_close_response(&mut state, &root, "g1-test", None)
                        .unwrap_err()
                        .code,
                    "column-close-failed",
                    "a retry cannot answer Ended while the run lives"
                );
            }
            // SAFETY: resumes the test-owned host frozen above.
            assert_eq!(
                unsafe { libc::kill(extra_pid, libc::SIGCONT) },
                0,
                "{context}: resume failed: {}",
                std::io::Error::last_os_error()
            );
            let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
            // Wait for Orbit to finish changing its record and sockets, leaving discovery to reap it.
            let exited_start = loop {
                let process = crate::platform::process(extra_pid as u32)
                    .unwrap_or_else(|error| panic!("{context}: observe resumed Orbit: {error}"))
                    .unwrap_or_else(|| panic!("{context}: child vanished before discovery"));
                if process.ended {
                    break process.start;
                }
                assert!(
                    Instant::now() < deadline,
                    "{context}: resumed Orbit did not exit after {error:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            let untracked = sessions::untracked_runs(
                &root,
                LaunchMode::Workspace,
                "component-1",
                &state.sessions,
                deadline,
            )
            .unwrap_or_else(|error| panic!("{context}: discover after Orbit exit: {error}"));
            assert!(
                untracked.is_empty(),
                "{context}: discovery still lists {:?}",
                untracked
                    .iter()
                    .map(|run| &run.identity)
                    .collect::<Vec<_>>()
            );
            let record = endpoint.with_extension("sock.record");
            assert!(
                !record
                    .try_exists()
                    .unwrap_or_else(|error| panic!("{context}: inspect record: {error}")),
                "{context}: discovery left {}",
                record.display()
            );
            assert!(
                crate::platform::process(extra_pid as u32)
                    .unwrap_or_else(|error| panic!("{context}: inspect reaping: {error}"))
                    .is_none_or(|process| process.start != exited_start),
                "{context}: discovery did not reap the ended child"
            );
            if keep_pane {
                assert!(
                    sessions::session_finished(&mut state.sessions[0])
                        .unwrap()
                        .is_none()
                );
                let (response, ended) = super::dispatch_control_request(
                    super::Request {
                        id: super::request_id(),
                        caller: None,
                        action: Action::Workspace(WorkspaceAction::Inspect),
                    },
                    &mut state,
                    &crate::fixtures::inputs(),
                    &root,
                    &root,
                    "g1-test",
                );
                assert!(!ended);
                assert!(matches!(
                    response,
                    crate::control::ControlResponse::Workspace(super::Response::Snapshot(_))
                ));
                assert!(
                    matches!(column_request(&mut state, &root, "p1", Some(&consent_of(&scope))).unwrap(),
                    super::Response::Confirm(current) if current.untracked.is_empty())
                );
            }
            stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
            drop(state);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn work_started_after_a_column_scope_is_stopped_by_its_consent() {
        let root = temporary_directory();
        let (mut state, gate) = finishing_window(&root);
        sessions::stop_managed_sessions(&mut state.sessions, SESSION_CLEANUP_TIMEOUT).unwrap();
        state.sessions.clear();
        let mut shell = leased_orbit(
            &root,
            &format!("set -m; read -r _ < '{}'; sleep 300", gate.display()),
        );
        shell.default_shell = true;
        state.sessions.push(shell);
        let deadline = Instant::now() + Duration::from_secs(10);
        let idle = loop {
            state.tend(Instant::now());
            let scope = column_scope(&mut state, &root, "p1");
            if !scope.terminals[0].busy {
                break scope;
            }
            assert!(Instant::now() < deadline, "shell never reported idle");
            std::thread::sleep(Duration::from_millis(20));
        };
        fs::write(gate, "\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            state.tend(Instant::now());
            if column_scope(&mut state, &root, "p1").terminals[0].busy {
                break;
            }
            assert!(Instant::now() < deadline, "job never took the foreground");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(
            column_request(&mut state, &root, "p1", Some(&consent_of(&idle))).unwrap(),
            super::Response::Ended(_)
        ));
        assert!(state.sessions.is_empty());
        drop(state);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sidebar_actions_in_off_answer_as_eon_c31_says() {
        let root = temporary_directory();
        let (mut state, _) = finishing_window(&root);
        state.attention = Some(crate::attention::Attention::new(false));
        for (action, code) in [
            (Action::SetSidebar { open: true }, Some("unavailable")),
            (Action::ToggleSidebar, Some("unavailable")),
            (Action::SetSidebar { open: false }, None),
        ] {
            let (response, _) = super::dispatch_control_request(
                super::Request {
                    id: super::request_id(),
                    caller: None,
                    action,
                },
                &mut state,
                &crate::fixtures::inputs(),
                &root,
                &root,
                "g1-test",
            );
            match (code, response) {
                (
                    Some(code),
                    crate::control::ControlResponse::Workspace(super::Response::Failure(error)),
                ) => assert_eq!(error.code, code),
                (
                    None,
                    crate::control::ControlResponse::Workspace(super::Response::Snapshot(snapshot)),
                ) => assert_eq!(snapshot.sidebar, eon_workspace_protocol::v43::Sidebar::Off),
                _ => panic!("unexpected sidebar result"),
            }
        }
        state.workspaces = None;
        state.attention = None;
        for action in [
            Action::SetSidebar { open: true },
            Action::SetSidebar { open: false },
            Action::ToggleSidebar,
        ] {
            let (response, _) = super::dispatch_control_request(
                super::Request {
                    id: super::request_id(),
                    caller: None,
                    action,
                },
                &mut state,
                &crate::fixtures::inputs(),
                &root,
                &root,
                "g1-test",
            );
            assert!(
                matches!(response, crate::control::ControlResponse::Workspace(super::Response::Failure(error)) if error.code == "session-unavailable")
            );
        }
        stop_generation_sessions(&mut state, &root, LaunchMode::Terminal).unwrap();
        drop(state);
    }

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

    // Environment-sensitive startup runs in its own process, never changing the test runner.
    #[test]
    fn c38_supervisor_startup_process() {
        let Some(root) = std::env::var_os("EON_C38_STARTUP_ROOT").map(PathBuf::from) else {
            return;
        };
        let mut inputs = crate::fixtures::inputs();
        if root.join("venus").exists() {
            inputs.venus = root.join("venus");
        }
        let generation =
            std::env::var("EON_C38_GENERATION").unwrap_or_else(|_| "component-1".into());
        let log = DiagnosticLog::open_in(Ok(root.join("logs")), "component-1", "eon");
        let lock = super::lock_supervisor_lifecycle(&root.join("startup.lock")).unwrap();
        let mode = if std::env::var_os("EON_C38_BASE").is_some() {
            LaunchMode::Terminal
        } else {
            LaunchMode::Workspace
        };
        let result = super::supervise(
            &inputs,
            &root,
            inputs.defaults.terminal.clone(),
            &super::window_bar(mode, &root, &inputs, None).unwrap(),
            Some(inputs.defaults.popups.clone()),
            None,
            lock,
            &root.join("a/b"),
            &[],
            None,
            &generation,
            mode,
            "",
            false,
            &log,
            super::TERMINAL_START_TIMEOUT,
        );
        fs::write(
            root.join("startup-result"),
            match result {
                Ok(status) => format!("exit {status}"),
                Err(error) => error,
            },
        )
        .unwrap();
    }

    // Fresh launches retain committed metadata; eonbase does not register.
    #[test]
    fn c38_supervisor_startup_gates_recovery_and_retains_identity() {
        use crate::session_registry::Registry;
        for case in ["config", "launch", "base", "busy"] {
            let root = temporary_directory();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let runtime = root.join("a/b");
            fs::create_dir_all(&runtime).unwrap();
            let registry = Registry::in_test_directory(root.join("state/eon/sessions"));
            let release = if case == "busy" {
                registry.create(None).unwrap();
                let lock = fs::File::open(root.join("state/eon/sessions/registry.lock")).unwrap();
                lock.lock().unwrap();
                let marker = root.join("state/eon/sessions/.tmp-test-retrying");
                Some(std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while !marker.exists() {
                        assert!(Instant::now() < deadline, "startup did not retry busy");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    lock.unlock().unwrap();
                }))
            } else {
                None
            };
            if case == "config" {
                fs::write(root.join("config.toml"), "[shell]\ncommand = []\n").unwrap();
            }
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "supervisor::tests::c38_supervisor_startup_process",
                    "--nocapture",
                ])
                .env("EON_C38_STARTUP_ROOT", &root)
                .env("HOME", &root)
                .env("XDG_STATE_HOME", root.join("state"))
                .env("EON_RUNTIME_DIR", root.join("runtime"))
                .current_dir(&root)
                .stdin(std::process::Stdio::null());
            if case == "base" {
                command.env("EON_C38_BASE", "1");
            }
            if case == "busy" {
                command.env("EON_C38_OBSERVE_BUSY_RETRY", "1");
            }
            let output = command.output().unwrap();
            if let Some(release) = release {
                release.join().unwrap();
            }
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let error = fs::read_to_string(root.join("startup-result")).unwrap();
            match case {
                "config" => {
                    assert!(error.contains("shell.command"), "{error}");
                    assert!(registry.list().unwrap().is_empty());
                }
                "launch" | "busy" => {
                    let records = registry.list().unwrap();
                    assert_eq!(records.len(), if case == "busy" { 2 } else { 1 });
                    assert!(error.contains(&records.last().unwrap().id), "{error}");
                }
                "base" => assert!(registry.list().unwrap().is_empty()),
                _ => unreachable!(),
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn c38_restart_keeps_live_terminals_reachable_when_registry_is_unavailable() {
        use crate::{control, session_registry::Registry};
        const GENERATION: &str = "g1-00000000000000000000000000000001";
        for case in ["corrupt", "permissions", "held", "state-root", "worker"] {
            let root = temporary_directory();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let runtime = root.join("a/b");
            fs::create_dir_all(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            let registry_root = root.join("state/eon/sessions");
            let registry = Registry::in_test_directory(registry_root.clone());
            registry.create(None).unwrap();
            let held = if case == "held" {
                let lock = fs::File::open(registry_root.join("registry.lock")).unwrap();
                lock.lock().unwrap();
                Some(lock)
            } else {
                None
            };
            if case == "corrupt" {
                fs::write(registry_root.join("allocator.json"), b"{").unwrap();
            }
            if case == "permissions" {
                fs::set_permissions(&registry_root, fs::Permissions::from_mode(0o755)).unwrap();
            }
            script(&root.join("venus"), "#!/bin/sh\nexec sleep 60\n");
            let mut inputs = crate::fixtures::inputs();
            inputs.orbit = std::env::var_os("ORBIT_BIN").unwrap().into();
            let log = DiagnosticLog::open_in(Ok(root.join("orbit-logs")), GENERATION, "eon");
            let live = super::start_orbit(
                &inputs,
                &root,
                &runtime.join("orbit.sock"),
                "terminal-1",
                GENERATION,
                &root,
                &[
                    "sh".into(),
                    "-c".into(),
                    "printf 'recovered-terminal\\n'; exec sleep 60".into(),
                ],
                None,
                &log,
                Instant::now() + Duration::from_secs(5),
                &mut sessions::FailedStarts::default(),
            )
            .unwrap();
            let pid = orbit_pid(&live);
            sessions::tests::release_lease(live);
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "supervisor::tests::c38_supervisor_startup_process",
                    "--nocapture",
                ])
                .env("EON_C38_STARTUP_ROOT", &root)
                .env("HOME", &root)
                .env("EON_C38_GENERATION", GENERATION)
                .env("XDG_STATE_HOME", root.join("state"))
                .env("EON_RUNTIME_DIR", root.join("runtime"))
                .current_dir(&root)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if case == "state-root" {
                command
                    .env("HOME", "relative")
                    .env("XDG_STATE_HOME", "relative");
            }
            if case == "worker" {
                command.env("EON_C38_FAIL_WORKER", "1");
            }
            let mut child = command.spawn().unwrap();
            let socket = runtime.join("eon.sock");
            let deadline = Instant::now() + Duration::from_secs(10);
            let snapshot = loop {
                if socket.exists()
                    && let Ok(control::ControlResponse::Workspace(super::Response::Snapshot(
                        snapshot,
                    ))) =
                        control::send_action(&socket, Action::Workspace(WorkspaceAction::Inspect))
                    && snapshot.workspaces[0].registration_error.is_some()
                {
                    break snapshot;
                }
                if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                    let _ = child.kill();
                    let output = child.wait_with_output().unwrap();
                    // SAFETY: this is the exact Orbit PID started by this test.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, std::ptr::null_mut(), 0);
                    }
                    panic!(
                        "{case}: recovery unavailable: {} {}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(snapshot.workspaces[0].id, "s1");
            assert_eq!(snapshot.workspaces[0].persistent_id, None);
            if case == "state-root" {
                assert!(
                    snapshot.workspaces[0]
                        .registration_error
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("cannot resolve session state directory")
                );
            }
            if case == "worker" {
                assert!(
                    snapshot.workspaces[0]
                        .registration_error
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("worker spawn failure")
                );
            }
            let pane = snapshot.tabs[0].panes().next().unwrap();
            assert_eq!(pane.session, "terminal-1");
            assert_eq!(
                pane.endpoint,
                runtime.join("orbit.sock").as_os_str().as_encoded_bytes()
            );
            assert!(matches!(
                control::send_action(
                    &socket,
                    Action::ReadTerminal {
                        target: "p1".into()
                    }
                )
                .unwrap(),
                control::ControlResponse::Workspace(super::Response::Text(_))
            ));
            assert!(matches!(
                control::send_action(
                    &socket,
                    Action::Workspace(WorkspaceAction::NewWorkspace {
                        name: None,
                        focus: false
                    })
                )
                .unwrap(),
                control::ControlResponse::Workspace(super::Response::Failure(_))
            ));
            let stopped =
                control::send_stop_on(UnixStream::connect(&socket).unwrap(), GENERATION).unwrap();
            assert!(matches!(
                stopped,
                control::StopAnswer::Reply(eon_workspace_protocol::stop::Reply::Stopped { .. })
            ));
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            // SAFETY: reap the exact child started here, after the supervisor confirmed Stop.
            unsafe {
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
            drop(held);
            if case == "corrupt" {
                assert_eq!(
                    fs::read(registry_root.join("allocator.json")).unwrap(),
                    b"{"
                );
            }
            if case == "permissions" {
                assert_eq!(fs::metadata(&registry_root).unwrap().mode() & 0o7777, 0o755);
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    fn window_agents() -> crate::managed_environment::Agents {
        crate::managed_environment::Agents {
            terminal_access: crate::managed_environment::TerminalAccess::Window,
            send_enter_delay_ms: 300,
            send_typing_quiet_ms: 2000,
        }
    }

    #[test]
    fn c38_registered_recovery_retries_and_publishes_the_confirmed_same_member() {
        use crate::session_registry::{Operation, Registry, Worker};
        use eon_workspace_protocol::v43::{Change, SubscriptionMessage, WorkspaceChange};
        let (root, inputs, mut state, old_peer) = subscribed_window();
        drop(old_peer);
        let registry_root = root.join("registry");
        let registry = Registry::in_test_directory(registry_root.clone());
        registry.create(None).unwrap();
        state.workspaces = Some(
            WorkspaceSet::registered_recovered(
                Some(registry.clone()),
                None,
                crate::workspace::IdAllocator::new(root.clone(), 0).unwrap(),
                root.clone(),
                Some(root.clone()),
                vec![(
                    1,
                    Session {
                        id: "terminal-1".into(),
                        endpoint: root.join("orbit.sock"),
                    },
                )],
                inputs.defaults.popups.clone(),
            )
            .unwrap(),
        );
        state.subscribers = crate::subscriptions::Subscribers::new("component-1");
        let before = state.workspace_snapshot_in(None).unwrap();
        let Dispatched::Subscribe(subscriber) = super::dispatch(
            super::Incoming::Request(super::Request {
                id: "recovered-subscribe".into(),
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
            panic!("subscription refused");
        };
        let (stream, mut peer) = UnixStream::pair().unwrap();
        state
            .subscribers
            .attach(stream, *subscriber, Instant::now());
        let initial = subscription_messages(&mut peer);
        assert!(
            matches!(&initial[0], SubscriptionMessage::Subscribed(subscribed)
            if subscribed.snapshot.as_ref().unwrap().workspaces[0].workspace.persistent_id.is_none())
        );
        let lock = fs::File::open(registry_root.join("registry.lock")).unwrap();
        lock.lock().unwrap();
        let mut worker = Worker::start(registry.clone()).unwrap();
        worker.submit(Operation::Register).unwrap();
        state.registry_worker = Some(worker);
        state.registry_pending = Some(super::RegistryPending::Recovery("s1".into()));
        std::thread::sleep(Duration::from_millis(100));
        state.poll_registry(&inputs, &root, &root, "component-1");
        assert!(
            state.workspace_snapshot_in(None).unwrap().workspaces[0]
                .persistent_id
                .is_none()
        );
        drop(lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.registry_pending.is_some() {
            assert!(
                Instant::now() < deadline,
                "recovered registration did not retry"
            );
            state.poll_registry(&inputs, &root, &root, "component-1");
            state.end_turn(Instant::now());
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = state.workspace_snapshot_in(None).unwrap();
        let id = registry.list().unwrap().last().unwrap().id.clone();
        assert_eq!(
            after.workspaces[0].persistent_id.as_deref(),
            Some(id.as_str())
        );
        assert_eq!(after.workspaces[0].registration_error, None);
        assert_eq!(after.workspace, before.workspace);
        assert_eq!(after.tabs, before.tabs);
        let messages = subscription_messages(&mut peer);
        assert!(messages.iter().any(|message| matches!(message,
            SubscriptionMessage::Event(event) if matches!(&event.change,
                Change::Workspace { change: WorkspaceChange::Registered, workspace }
                    if workspace.workspace.id == "s1" && workspace.workspace.persistent_id.as_deref() == Some(id.as_str())))), "{messages:?}");
        drop((peer, state));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn c38_fresh_start_services_lifecycle_and_stop_before_registration_finishes() {
        use crate::{
            control,
            session_registry::{Fault, Operation, Registry, Worker},
        };
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let registry_root = root.join("registry");
        let mut registry = Registry::in_test_directory(registry_root.clone());
        registry.create(None).unwrap();
        registry.inject(Some(Fault::WaitBeforeRecordPublication));
        let mut worker = Worker::start(registry.clone()).unwrap();
        worker.submit(Operation::Create { name: None }).unwrap();
        let socket = root.join("eon.sock");
        let listener = control::ControlListener::bind(&socket).unwrap();
        let generation = "g1-00000000000000000000000000000001";
        let starting = eon_workspace_protocol::v43::Runtime {
            venus: None,
            generation: generation.into(),
            name: None,
            started_at: 1,
            eon_version: inputs.version.into(),
            workspace_protocol: eon_workspace_protocol::v43::VERSION,
            component_report: inputs.component_report().unwrap().into(),
            sessions: Vec::new(),
            attach: eon_workspace_protocol::v43::Availability {
                available: false,
                reason: "registering".into(),
            },
            stop: eon_workspace_protocol::v43::Availability {
                available: true,
                reason: "starting".into(),
            },
            busy: None,
        };
        let supervisor = std::thread::spawn(move || {
            super::await_initial_registration(&listener, &mut worker, &starting)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !registry_root.join(".tmp-test-waiting").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let control::ControlResponse::Lifecycle(
            eon_workspace_protocol::v43::LifecycleResponse::Runtime(status),
        ) = control::send_action(&socket, Action::Workspace(WorkspaceAction::InspectRuntime))
            .unwrap()
        else {
            panic!("starting runtime inspection failed");
        };
        assert!(!status.attach.available);
        assert!(status.stop.available);
        assert!(status.sessions.is_empty());
        let awaiting = super::probe_supervisor(&inputs, &socket, generation)
            .err()
            .expect("starting supervisor is not attachable yet");
        assert_eq!(
            awaiting.kind,
            control::EndpointFailureKind::Starting,
            "a competing launch must retry the still-starting supervisor"
        );
        assert!(matches!(
            control::send_stop_on(UnixStream::connect(socket).unwrap(), generation).unwrap(),
            control::StopAnswer::Reply(eon_workspace_protocol::stop::Reply::Stopped { .. })
        ));
        assert_eq!(supervisor.join().unwrap().unwrap(), None);
        fs::write(registry_root.join(".tmp-test-release"), b"release").unwrap();
        while registry.list().is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(registry.list().unwrap().len(), 2);
        assert!(!root.join("orbit.sock.record").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn c38_pending_registry_io_services_inspection_and_stop_and_discards_late_creation() {
        use crate::{
            control,
            session_registry::{Fault, Registry, Worker},
        };
        use eon_workspace_protocol::v43::{Request, encode_request};
        use std::io::Write;
        let (root, inputs, mut state, peer) = subscribed_window();
        let registry_root = root.join("registry");
        let mut registry = Registry::in_test_directory(registry_root.clone());
        let registration = registry.create(None).unwrap();
        state.workspaces = Some(
            WorkspaceSet::registered_pending(
                Some(registry.clone()),
                registration,
                crate::workspace::IdAllocator::new(root.clone(), 0).unwrap(),
                root.clone(),
                Some(root.clone()),
                inputs.defaults.popups.clone(),
                true,
                |_, _| Ok(Default::default()),
                |_| Ok(()),
            )
            .unwrap(),
        );
        registry.inject(Some(Fault::WaitBeforeRecordPublication));
        state.registry_worker = Some(Worker::start(registry.clone()).unwrap());
        let before = state.workspaces.as_ref().unwrap().snapshot();
        let listener = control::ControlListener::bind(&root.join("eon.sock")).unwrap();
        let request = Request {
            id: "paused-create".into(),
            caller: None,
            action: Action::Workspace(WorkspaceAction::NewWorkspace {
                name: None,
                focus: true,
            }),
        };
        let mut client = UnixStream::connect(root.join("eon.sock")).unwrap();
        client
            .write_all(&encode_request(&request).unwrap())
            .unwrap();
        let super::Accepted::Deferred(stream) = listener
            .accept(|incoming| {
                super::dispatch(
                    incoming,
                    &mut state,
                    &inputs,
                    &root,
                    &root,
                    "g1-00000000000000000000000000000001",
                )
            })
            .unwrap()
        else {
            panic!("registry request did not defer");
        };
        if let Some(super::RegistryPending::Action {
            stream: pending, ..
        }) = &mut state.registry_pending
        {
            *pending = Some(stream);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !registry_root.join(".tmp-test-waiting").exists() {
            assert!(
                Instant::now() < deadline,
                "worker did not reach the publication barrier"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let replay = super::dispatch(
            super::Incoming::Request(request),
            &mut state,
            &inputs,
            &root,
            &root,
            "g1-00000000000000000000000000000001",
        );
        assert!(
            matches!(replay, Dispatched::Reply(control::ControlResponse::Workspace(super::Response::Failure(error)), false) if error.code == "duplicate-request")
        );
        // The first accept returned with the file lock still held; another real client is serviced.
        let socket = root.join("eon.sock");
        let inspect = std::thread::spawn(move || {
            control::send_action(&socket, Action::Workspace(WorkspaceAction::Inspect))
        });
        loop {
            let accepted = listener
                .accept(|incoming| {
                    super::dispatch(
                        incoming,
                        &mut state,
                        &inputs,
                        &root,
                        &root,
                        "g1-00000000000000000000000000000001",
                    )
                })
                .unwrap();
            if !matches!(accepted, super::Accepted::None) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(
            inspect.join().unwrap().unwrap(),
            control::ControlResponse::Workspace(super::Response::Snapshot(_))
        ));
        let socket = root.join("eon.sock");
        let stop = std::thread::spawn(move || {
            control::send_stop_on(
                UnixStream::connect(socket).unwrap(),
                "g1-00000000000000000000000000000001",
            )
        });
        loop {
            let accepted = listener
                .accept(|incoming| {
                    super::dispatch(
                        incoming,
                        &mut state,
                        &inputs,
                        &root,
                        &root,
                        "g1-00000000000000000000000000000001",
                    )
                })
                .unwrap();
            if matches!(accepted, super::Accepted::Replied { shutdown: true }) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(
            stop.join().unwrap().unwrap(),
            control::StopAnswer::Reply(eon_workspace_protocol::stop::Reply::Stopped { .. })
        ));
        assert!(state.stopping.is_some());
        fs::write(registry_root.join(".tmp-test-release"), b"release").unwrap();
        while state.registry_pending.is_some() {
            assert!(Instant::now() < deadline);
            state.poll_registry(&inputs, &root, &root, "g1-00000000000000000000000000000001");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(state.sessions.is_empty());
        assert_eq!(state.workspaces.as_ref().unwrap().snapshot(), before);
        assert_eq!(
            registry.list().unwrap().len(),
            2,
            "only retained metadata may finish after Stop"
        );
        drop((client, listener, peer, state));
        fs::remove_dir_all(root).unwrap();
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
            |_, _| Ok(Default::default()),
            |_| Ok(()),
        )
        .unwrap();
        let mut state = SupervisorState {
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: Vec::new(),
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
        let mut message = vec![0; eon_workspace_protocol::v43::HEADER_BYTES];
        peer.read_exact(&mut message).unwrap();
        message.resize(
            eon_workspace_protocol::v43::declared_message_len(&message).unwrap(),
            0,
        );
        peer.read_exact(&mut message[eon_workspace_protocol::v43::HEADER_BYTES..])
            .unwrap();
        let eon_workspace_protocol::v43::SubscriptionMessage::Subscribed(subscribed) =
            eon_workspace_protocol::v43::decode_subscription_message(&message).unwrap()
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

        let missing = eon_workspace_protocol::v43::Filter {
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
        state.stopping = Some(StopReason::StopRequired);
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
        state.stopping = None;
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

    /// A deferred registry action completes on a later turn; the tending
    /// change is published while it waits, then the confirmed rename follows.
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
                |_, _| Ok(Default::default()),
                |_| Ok(()),
            )
            .unwrap();
        let client = rename_client(&root, peer);
        serve_turn(&control, &mut state, &inputs, &root);
        let (_, held) = client.join().unwrap();
        assert_eq!(
            renames(&held),
            [Some("tended".to_owned()), Some("renamed".to_owned())],
            "{held:?}"
        );
        drop(control);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    /// A pending window with one unfiltered subscriber that has read its
    /// snapshot, and that subscriber's end of the connection.
    fn subscribed_window() -> (PathBuf, crate::cli::Inputs, SupervisorState, UnixStream) {
        use eon_workspace_protocol::v43::SubscriptionMessage;
        let root = temporary_directory();
        let inputs = crate::fixtures::inputs();
        let workspace = WorkspaceSet::pending(
            root.clone(),
            root.clone(),
            Some(root.clone()),
            inputs.defaults.popups.clone(),
            |_, _| Ok(Default::default()),
            |_| Ok(()),
        )
        .unwrap();
        let mut state = SupervisorState {
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: Some(
                crate::session_registry::Worker::start(workspace.registry().clone()).unwrap(),
            ),
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: Vec::new(),
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
        Vec<eon_workspace_protocol::v43::SubscriptionMessage>,
    )> {
        use eon_workspace_protocol::v43::{
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
        if let super::Accepted::Deferred(stream) = accepted {
            if let Some(super::RegistryPending::Action {
                stream: pending, ..
            }) = &mut state.registry_pending
            {
                *pending = Some(stream);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while state.registry_pending.is_some() {
                assert!(Instant::now() < deadline, "registry did not complete");
                state.poll_registry(inputs, root, root, "component-1");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// The names of the `workspace-renamed` events in `messages`.
    fn renames(
        messages: &[eon_workspace_protocol::v43::SubscriptionMessage],
    ) -> Vec<Option<String>> {
        use eon_workspace_protocol::v43::{Change, SubscriptionMessage, WorkspaceChange};
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
    ) -> Vec<eon_workspace_protocol::v43::SubscriptionMessage> {
        use eon_workspace_protocol::v43::{declared_message_len, decode_subscription_message};
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
        use eon_workspace_protocol::v43::{AttentionState, Change, SubscriptionMessage};
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![session],
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: Some(crate::attention::Attention::new(true)),
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        let Dispatched::Subscribe(subscriber) = super::dispatch(
            super::Incoming::Request(super::Request {
                id: "subscribe-1".into(),
                caller: None,
                action: Action::Subscribe {
                    filter: eon_workspace_protocol::v43::Filter {
                        kinds: vec![eon_workspace_protocol::v43::EventKind::Attention],
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

    #[cfg(target_os = "linux")]
    struct FinishingWindow {
        state: SupervisorState,
        root: PathBuf,
    }

    #[cfg(target_os = "linux")]
    impl std::ops::Deref for FinishingWindow {
        type Target = SupervisorState;

        fn deref(&self) -> &Self::Target {
            &self.state
        }
    }

    #[cfg(target_os = "linux")]
    impl std::ops::DerefMut for FinishingWindow {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.state
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for FinishingWindow {
        fn drop(&mut self) {
            drop(self.state.venus.take());
            if let Err(error) =
                sessions::stop_managed_sessions(&mut self.state.sessions, SESSION_CLEANUP_TIMEOUT)
            {
                eprintln!("fixture cleanup {}: {error}", self.root.display());
            }
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!("fixture directory cleanup {}: {error}", self.root.display());
            }
        }
    }

    /// A full-Eon window whose agent reports a finish for each FIFO pulse.
    #[cfg(target_os = "linux")]
    fn finishing_window(root: &Path) -> (FinishingWindow, PathBuf) {
        let state = SupervisorState {
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions: Vec::new(),
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: Some(crate::attention::Attention::new(true)),
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };
        let mut fixture = FinishingWindow {
            state,
            root: root.to_path_buf(),
        };
        let gate = root.join("gate");
        let ready = root.join("agent.ready");
        assert!(
            Command::new("mkfifo")
                .arg(&gate)
                .status()
                .unwrap()
                .success()
        );
        // Retain a writer so a pulse's close cannot make the next read end the agent.
        fixture
            .state
            .sessions
            .push(sessions::tests::leased_orbit_with_program(
                root,
                &format!(
                    "set -eu; exec 3<> '{}'; : > '{}'; \
                 while read -r _ <&3; do \
                 printf '\\033]7501;state=working:app=claude-code\\007'; \
                 printf '\\033]7501;state=done:app=claude-code\\007'; done",
                    gate.display(),
                    ready.display()
                ),
                Path::new(&std::env::var_os("ORBIT_BIN").unwrap()),
            ));
        // Orbit Ready precedes the agent opening its FIFO; wait for that open before pulses.
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture agent did not open {}: {}",
                gate.display(),
                sessions::tests::lease_diagnostics(&fixture.state.sessions[0])
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let session = &fixture.state.sessions[0];
        fixture.state.workspaces = Some(
            WorkspaceSet::with_recovered_sessions(
                root.to_path_buf(),
                root.to_path_buf(),
                Some(root.to_path_buf()),
                vec![(
                    1,
                    Session {
                        id: session.id.clone(),
                        endpoint: session.endpoint.clone(),
                    },
                )],
                crate::fixtures::inputs().defaults.popups.clone(),
            )
            .unwrap(),
        );
        (fixture, gate)
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

    #[cfg(target_os = "linux")]
    fn reload_venus(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::write(
            &path,
            "#!/bin/sh\nprintf 'ready-v1focus:1\\n'\nexec cat >/dev/null\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_same_build_rereads_config_and_rolls_back_to_accepted_arguments() {
        for reject_connected in [false, true] {
            let root = temporary_directory();
            let (mut fixture, _gate) = finishing_window(&root);
            let mut inputs = crate::fixtures::inputs();
            inputs.venus = root.join("venus");
            fs::write(
                &inputs.venus,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >\"$0.$$.args\"\n\
                 if [ -e \"$0.reject-connected\" ]; then\n\
                   for arg; do [ \"$arg\" != connected ] || exit 9; done\n\
                 fi\nprintf 'ready-v1focus:1\\n'\nexec cat >/dev/null\n",
            )
            .unwrap();
            fs::set_permissions(&inputs.venus, fs::Permissions::from_mode(0o700)).unwrap();
            let backend = root.join("codexbar");
            script(&backend, "printf broken");
            let initial_widgets: &[&str] = if reject_connected {
                &["codex_quota_5h"]
            } else {
                &[]
            };
            fixture.state.quotas = Some(super::quota::Quotas::start(&backend, initial_widgets));
            let quota_failure = |state: &SupervisorState| {
                let mut snapshot = state.workspaces.as_ref().unwrap().snapshot();
                state.quotas.as_ref().unwrap().apply(&mut snapshot);
                snapshot.codex_quota_failure
            };
            let wait_for_quota = |state: &SupervisorState| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while quota_failure(state).is_none() {
                    assert!(Instant::now() < deadline, "quota provider did not start");
                    std::thread::sleep(Duration::from_millis(5));
                }
            };
            if reject_connected {
                wait_for_quota(&fixture.state);
            }
            let config = root.join("config.toml");
            fs::write(
                &config,
                format!("[panels]\nheading = 'notch'\n[bar]\nwidgets = {initial_widgets:?}\n"),
            )
            .unwrap();
            let terminal = crate::managed_environment::terminal_presentation(
                &root,
                &inputs.defaults,
                Some(&fixture.state.log),
            )
            .unwrap();
            let command = venus_command(
                &inputs,
                &inputs.venus,
                &root,
                &root.join("eon.sock"),
                LaunchMode::Workspace,
                &terminal,
                &window_bar(LaunchMode::Workspace, &root, &inputs, None).unwrap(),
                &fixture.state.application_id,
                fixture.state.decorations,
                true,
                true,
                true,
            );
            fixture.state.venus = Some(
                super::PresentationProcess::start(
                    command,
                    true,
                    fixture.state.workspace_snapshot(),
                    super::PRESENTATION_ADMISSION_TIMEOUT,
                    &mut fixture.state.sessions,
                    &fixture.state.log,
                )
                .unwrap(),
            );
            let arguments =
                |pid| fs::read_to_string(root.join(format!("venus.{pid}.args"))).unwrap();
            let old_pid = fixture.state.venus.as_ref().unwrap().child.id();
            let accepted_arguments = arguments(old_pid);
            assert!(accepted_arguments.contains("--panel-heading\nnotch\n"));
            let layout = fixture.state.workspaces.as_ref().unwrap().snapshot();
            let run = fixture.state.sessions[0].run_id().to_owned();
            let host = orbit_pid(&fixture.state.sessions[0]);
            if reject_connected {
                fs::write(root.join("venus.reject-connected"), "").unwrap();
            }
            let widgets: &[&str] = if reject_connected {
                &[]
            } else {
                &["codex_quota_5h"]
            };
            fs::write(
                &config,
                format!("[panels]\nheading = 'connected'\n[bar]\nwidgets = {widgets:?}\n"),
            )
            .unwrap();
            let result = super::reload_presentation(
                &mut fixture.state,
                &inputs,
                &root,
                &root,
                &inputs.venus,
            );
            if reject_connected {
                assert_eq!(result.unwrap_err().code, "reload-presentation-failed");
            } else {
                result.unwrap();
            }
            let reloaded = fixture.state.venus.as_ref().unwrap();
            let new_pid = reloaded.child.id();
            assert_ne!(new_pid, old_pid);
            assert_eq!(reloaded.executable, inputs.venus);
            if reject_connected {
                assert_eq!(arguments(new_pid), accepted_arguments);
            } else {
                assert!(arguments(new_pid).contains("--panel-heading\nconnected\n"));
            }
            wait_for_quota(&fixture.state);
            let attempts: Vec<_> = fs::read_dir(&root)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "args")
                })
                .map(|path| fs::read_to_string(path).unwrap())
                .collect();
            assert_eq!(attempts.len(), if reject_connected { 3 } else { 2 });
            assert!(
                attempts
                    .iter()
                    .any(|args| args.contains("--panel-heading\nconnected\n"))
            );
            for invalid in [
                "broken = [",
                "[panels]\nheading = 'invalid'",
                "[shell]\ncommand = []",
            ] {
                fs::write(&config, invalid).unwrap();
                let error = super::reload_presentation(
                    &mut fixture.state,
                    &inputs,
                    &root,
                    &root,
                    &inputs.venus,
                )
                .unwrap_err();
                assert_eq!(error.code, "reload-refused", "{invalid}: {error:?}");
                assert!(error.detail.contains(config.to_str().unwrap()), "{error:?}");
                assert_eq!(fixture.state.venus.as_ref().unwrap().child.id(), new_pid);
                assert!(quota_failure(&fixture.state).is_some());
            }
            if !reject_connected {
                fs::write(
                    &config,
                    "[panels]\nheading = 'connected'\n[bar]\nwidgets = []\n",
                )
                .unwrap();
                super::reload_presentation(
                    &mut fixture.state,
                    &inputs,
                    &root,
                    &root,
                    &inputs.venus,
                )
                .unwrap();
                assert!(quota_failure(&fixture.state).is_none());
            }
            assert_eq!(fixture.state.sessions[0].run_id(), run);
            assert_eq!(orbit_pid(&fixture.state.sessions[0]), host);
            assert_eq!(
                fixture.state.workspaces.as_ref().unwrap().snapshot(),
                layout
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_keeps_terminals_and_layout_and_restores_the_accepted_venus() {
        use std::io::Write;
        let root = temporary_directory();
        let (mut fixture, _gate) = finishing_window(&root);
        fs::write(root.join("config.toml"), "[terminal]\nfont_size = 16\n").unwrap();
        let mut inputs = crate::fixtures::inputs();
        inputs.venus = reload_venus(&root, "venus-a");
        let newer = reload_venus(&root, "venus-b");
        let (mut focus, control) = attach_reporting_venus(&mut fixture.state);
        fixture.state.venus.as_mut().unwrap().executable = inputs.venus.clone();
        focus.write_all(b"focus:1\n").unwrap();
        let before = fixture.state.workspaces.as_ref().unwrap().snapshot();
        let run = fixture.state.sessions[0].run_id().to_owned();
        let host_pid = orbit_pid(&fixture.state.sessions[0]);
        let old_pid = fixture.state.venus.as_ref().unwrap().child.id();
        super::reload_presentation(&mut fixture.state, &inputs, &root, &root, &newer).unwrap();
        drop(control);
        assert_eq!(fixture.state.venus.as_ref().unwrap().executable, newer);
        assert_ne!(fixture.state.venus.as_ref().unwrap().child.id(), old_pid);
        assert_eq!(fixture.state.sessions[0].run_id(), run);
        assert_eq!(
            fixture.state.workspaces.as_ref().unwrap().snapshot(),
            before
        );
        let accepted_pid = fixture.state.venus.as_ref().unwrap().child.id();
        super::reload_presentation(&mut fixture.state, &inputs, &root, &root, &newer).unwrap();
        assert_ne!(
            fixture.state.venus.as_ref().unwrap().child.id(),
            accepted_pid
        );
        drop(fixture.state.venus.take());
        let (response, ended) = super::dispatch_control_request(
            super::Request {
                id: "reattach-after-reload".into(),
                caller: None,
                action: Action::Workspace(WorkspaceAction::Present { workspace: true }),
            },
            &mut fixture.state,
            &inputs,
            &root,
            &root,
            "component-1",
        );
        assert!(!ended);
        assert!(
            matches!(response, super::ControlResponse::Lifecycle(eon_workspace_protocol::v43::LifecycleResponse::Runtime(info)) if info.venus.as_deref() == newer.to_str())
        );
        let failed = super::reload_presentation(
            &mut fixture.state,
            &inputs,
            &root,
            &root,
            &root.join("absent"),
        )
        .unwrap_err();
        assert_eq!(failed.code, "reload-presentation-failed");
        assert_eq!(fixture.state.venus_path.as_ref(), Some(&newer));
        assert_eq!(fixture.state.sessions[0].run_id(), run);
        assert_eq!(
            fixture.state.workspaces.as_ref().unwrap().snapshot(),
            before
        );
        // An unavailable last accepted artifact falls back to this supervisor's own Venus.
        fs::remove_file(&newer).unwrap();
        let failed = super::reload_presentation(
            &mut fixture.state,
            &inputs,
            &root,
            &root,
            &root.join("absent"),
        )
        .unwrap_err();
        assert_eq!(failed.code, "reload-presentation-failed");
        assert_eq!(fixture.state.venus_path.as_ref(), Some(&inputs.venus));
        fs::remove_file(&inputs.venus).unwrap();
        let failed = super::reload_presentation(
            &mut fixture.state,
            &inputs,
            &root,
            &root,
            &root.join("absent"),
        )
        .unwrap_err();
        assert_eq!(failed.code, "reload-rollback-failed");
        assert!(fixture.state.venus.is_none());
        assert_eq!(fixture.state.sessions[0].run_id(), run);
        assert_eq!(orbit_pid(&fixture.state.sessions[0]), host_pid);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_refuses_unknown_or_unfocused_and_keeps_pending_pickers() {
        use std::io::Write;
        let (root, inputs, mut state, _subscriber) = subscribed_window();
        let (notices, shown) = crate::notification::Notices::record_shows();
        state.notices = notices;
        let mut attention = crate::attention::Attention::new(true);
        attention.handled(&Action::SetSidebar { open: false }, None, &[], false, false);
        state.attention = Some(attention);
        let (mut focus, control) = attach_reporting_venus(&mut state);
        let target = reload_venus(&root, "venus-b");
        let before = state.workspaces.as_ref().unwrap().snapshot();
        assert!(before.tabs[0].pending);
        let original = state.venus.as_ref().unwrap().child.id();
        for report in [None, Some(b"focus:0\n")] {
            if let Some(report) = report {
                focus.write_all(report).unwrap();
            }
            assert_eq!(
                super::reload_presentation(&mut state, &inputs, &root, &root, &target)
                    .unwrap_err()
                    .code,
                "reload-refused"
            );
            assert_eq!(state.venus.as_ref().unwrap().child.id(), original);
        }
        focus.write_all(b"focus:1\n").unwrap();
        super::reload_presentation(&mut state, &inputs, &root, &root, &target).unwrap();
        drop(control);
        assert_eq!(state.workspaces.as_ref().unwrap().snapshot(), before);
        assert_eq!(
            state.attention.as_ref().unwrap().sidebar(),
            eon_workspace_protocol::v43::Sidebar::Closed
        );
        assert!(shown().is_none());
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_crash_or_failed_reattach_returns_to_the_own_build() {
        use eon_workspace_protocol::v43::LifecycleResponse;
        use std::{io::Write, os::unix::fs::OpenOptionsExt, thread};
        for outcome in [9, 0] {
            let root = temporary_directory();
            let (mut fixture, _agent_gate) = finishing_window(&root);
            fs::write(root.join("config.toml"), "[terminal]\nfont_size = 16\n").unwrap();
            let mut inputs = crate::fixtures::inputs();
            inputs.venus = reload_venus(&root, "own-venus");
            let command = venus_command(
                &inputs,
                &inputs.venus,
                &root,
                &root.join("eon.sock"),
                super::LaunchMode::Workspace,
                &inputs.defaults.terminal,
                &inputs.defaults.bar(),
                "eon",
                false,
                true,
                true,
                true,
            );
            fixture.state.venus = Some(
                super::PresentationProcess::start(
                    command,
                    true,
                    fixture.state.workspace_snapshot(),
                    super::PRESENTATION_ADMISSION_TIMEOUT,
                    &mut fixture.state.sessions,
                    &fixture.state.log,
                )
                .unwrap(),
            );
            let gate = root.join("venus-exit");
            assert!(
                Command::new("mkfifo")
                    .arg(&gate)
                    .status()
                    .unwrap()
                    .success()
            );
            let failed = root.join("fail-admission");
            let newer = root.join("newer-venus");
            fs::write(&newer, format!("#!/bin/sh\n[ ! -e '{}' ] || exit 7\nprintf 'ready-v1focus:1\\n'\nread -r _ <'{}'\nexit {outcome}\n", failed.display(), gate.display())).unwrap();
            fs::set_permissions(&newer, fs::Permissions::from_mode(0o700)).unwrap();
            let layout = fixture.state.workspaces.as_ref().unwrap().snapshot();
            let run = fixture.state.sessions[0].run_id().to_owned();
            let host = orbit_pid(&fixture.state.sessions[0]);
            super::reload_presentation(&mut fixture.state, &inputs, &root, &root, &newer).unwrap();
            assert_eq!(fixture.state.venus_path.as_ref(), Some(&newer));
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&gate)
                {
                    Ok(mut writer) => {
                        writer.write_all(b"exit\n").unwrap();
                        break;
                    }
                    Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                        assert!(
                            Instant::now() < deadline,
                            "Venus did not reach its exit gate"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("cannot release Venus: {error}"),
                }
            }
            while fixture
                .state
                .venus
                .as_mut()
                .unwrap()
                .exited()
                .unwrap()
                .is_none()
            {
                assert!(Instant::now() < deadline, "Venus did not exit");
                thread::sleep(Duration::from_millis(5));
            }
            assert!(super::reap_presentation(&mut fixture.state, &inputs, &root).unwrap());
            let present = |state: &mut SupervisorState| {
                super::dispatch_control_request(
                    super::Request {
                        id: super::request_id(),
                        caller: None,
                        action: Action::Workspace(WorkspaceAction::Present { workspace: true }),
                    },
                    state,
                    &inputs,
                    &root,
                    &root,
                    "component-1",
                )
            };
            if outcome == 0 {
                assert_eq!(
                    fixture.state.venus_path.as_ref(),
                    Some(&newer),
                    "clean detach keeps the accepted build"
                );
                fs::write(&failed, "fail").unwrap();
                let (response, ended) = present(&mut fixture.state);
                assert!(!ended);
                assert!(matches!(
                    response,
                    super::ControlResponse::Lifecycle(LifecycleResponse::Failure(_))
                ));
            }
            assert!(
                fixture.state.venus_path.is_none(),
                "failed presentation must not pin a broken reload target"
            );
            let (response, ended) = present(&mut fixture.state);
            assert!(!ended);
            assert!(
                matches!(response, super::ControlResponse::Lifecycle(LifecycleResponse::Runtime(info)) if info.venus.as_deref() == inputs.venus.to_str())
            );
            assert_eq!(fixture.state.sessions[0].run_id(), run);
            assert_eq!(orbit_pid(&fixture.state.sessions[0]), host);
            assert_eq!(
                fixture.state.workspaces.as_ref().unwrap().snapshot(),
                layout
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_attachment_process() {
        use orbit_protocol::session::{self as orbs, ClientMessage, ServerMessage};
        use std::io::{Read, Write};
        use std::thread;
        let Some(endpoint) = std::env::var_os("EON_RELOAD_ATTACH_ENDPOINT") else {
            return;
        };
        let result = std::env::var_os("EON_RELOAD_ATTACH_RESULT").unwrap();
        let mut startup = vec![0; eon_workspace_protocol::v43::HEADER_BYTES];
        std::io::stdin().read_exact(&mut startup).unwrap();
        let length = eon_workspace_protocol::v43::declared_message_len(&startup).unwrap();
        startup.resize(length, 0);
        std::io::stdin()
            .read_exact(&mut startup[eon_workspace_protocol::v43::HEADER_BYTES..])
            .unwrap();
        assert!(matches!(
            eon_workspace_protocol::v43::decode_response(&startup).unwrap(),
            eon_workspace_protocol::v43::Response::Snapshot(_)
        ));
        let read = |stream: &mut UnixStream| {
            let mut bytes = vec![0; orbs::HEADER_BYTES];
            stream.read_exact(&mut bytes).unwrap();
            let length = orbs::server_message_len(&bytes).unwrap().unwrap();
            bytes.resize(length, 0);
            stream.read_exact(&mut bytes[orbs::HEADER_BYTES..]).unwrap();
            orbs::decode_server_message(&bytes).unwrap()
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            assert!(
                Instant::now() < deadline,
                "rollback Venus was never admitted"
            );
            let mut stream = UnixStream::connect(&endpoint).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .write_all(&orbs::encode_client_message(&ClientMessage::Hello).unwrap())
                .unwrap();
            match read(&mut stream) {
                ServerMessage::Attached => break stream,
                ServerMessage::Busy => thread::sleep(Duration::from_millis(5)),
                answer => panic!("unexpected attachment answer: {answer:?}"),
            }
        };
        let marker = format!("reload-{}", std::process::id());
        stream
            .write_all(
                &orbs::encode_client_message(&ClientMessage::Paste(marker.as_bytes().to_vec()))
                    .unwrap(),
            )
            .unwrap();
        loop {
            if let ServerMessage::Frame(frame) = read(&mut stream)
                && frame.rows.iter().any(|row| {
                    row.cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .contains(&marker)
                })
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "rollback input never reached the PTY"
            );
        }
        fs::write(result, std::process::id().to_string()).unwrap();
        std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_rolls_back_spawned_candidates_that_fail_admission() {
        use std::io::Write;
        use std::{os::unix::fs::OpenOptionsExt, thread};
        let root = temporary_directory();
        let (mut fixture, _gate) = finishing_window(&root);
        let mut inputs = crate::fixtures::inputs();
        let attached = root.join("attached");
        let executable = std::env::current_exe().unwrap();
        inputs.venus = root.join("attaching-venus");
        fs::write(&inputs.venus, format!(
            "#!/bin/sh\nprintf 'ready-v1focus:1\\n'\nEON_RELOAD_ATTACH_ENDPOINT='{}' EON_RELOAD_ATTACH_RESULT='{}' exec '{}' --exact supervisor::tests::reload_attachment_process --nocapture >/dev/null\n",
            fixture.state.sessions[0].endpoint.display(), attached.display(), executable.display()
        )).unwrap();
        fs::set_permissions(&inputs.venus, fs::Permissions::from_mode(0o700)).unwrap();
        let command = venus_command(
            &inputs,
            &inputs.venus,
            &root,
            &root.join("eon.sock"),
            super::LaunchMode::Workspace,
            &inputs.defaults.terminal,
            &inputs.defaults.bar(),
            "eon",
            false,
            true,
            true,
            true,
        );
        fixture.state.venus = Some(
            super::PresentationProcess::start(
                command,
                true,
                fixture.state.workspace_snapshot(),
                super::PRESENTATION_ADMISSION_TIMEOUT,
                &mut fixture.state.sessions,
                &fixture.state.log,
            )
            .unwrap(),
        );
        let wait_for_attachment = |pid: u32| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while fs::read_to_string(&attached).ok().as_deref() != Some(&pid.to_string()) {
                assert!(
                    Instant::now() < deadline,
                    "Venus {pid} did not attach and echo input"
                );
                thread::sleep(Duration::from_millis(5));
            }
        };
        wait_for_attachment(fixture.state.venus.as_ref().unwrap().child.id());
        let layout = fixture.state.workspaces.as_ref().unwrap().snapshot();
        let run = fixture.state.sessions[0].run_id().to_owned();
        let host = orbit_pid(&fixture.state.sessions[0]);
        for (name, failure) in [
            ("exit", "exit 9"),
            ("malformed", "printf notready; exec cat >/dev/null"),
        ] {
            let gate = root.join(format!("{name}-gate"));
            assert!(
                Command::new("mkfifo")
                    .arg(&gate)
                    .status()
                    .unwrap()
                    .success()
            );
            let started = root.join(format!("{name}-pid"));
            let candidate = root.join(name);
            fs::write(
                &candidate,
                format!(
                    "#!/bin/sh\nprintf '%s' \"$$\" >'{}'\nread -r _ <'{}'\n{failure}\n",
                    started.display(),
                    gate.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700)).unwrap();
            let old_pid = fixture.state.venus.as_ref().unwrap().child.id();
            fs::remove_file(&attached).unwrap();
            let release = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let pid = loop {
                    if let Some(pid) = fs::read_to_string(&started)
                        .ok()
                        .and_then(|value| value.parse::<u32>().ok())
                    {
                        break pid;
                    }
                    assert!(Instant::now() < deadline, "candidate did not spawn");
                    thread::sleep(Duration::from_millis(5));
                };
                assert!(crate::platform::process(old_pid).unwrap().is_none());
                loop {
                    match fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&gate)
                    {
                        Ok(mut writer) => {
                            writer.write_all(b"fail\n").unwrap();
                            break;
                        }
                        Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                            assert!(
                                Instant::now() < deadline,
                                "candidate never waited at admission"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("cannot release admission gate: {error}"),
                    }
                }
                pid
            });
            let error =
                super::reload_presentation(&mut fixture.state, &inputs, &root, &root, &candidate)
                    .unwrap_err();
            let candidate_pid = release.join().unwrap();
            assert_eq!(error.code, "reload-presentation-failed");
            assert!(crate::platform::process(candidate_pid).unwrap().is_none());
            let restored = fixture.state.venus.as_ref().unwrap();
            assert_ne!(restored.child.id(), old_pid);
            assert_eq!(restored.executable, inputs.venus);
            wait_for_attachment(restored.child.id());
            assert_eq!(fixture.state.sessions[0].run_id(), run);
            assert_eq!(orbit_pid(&fixture.state.sessions[0]), host);
            assert_eq!(
                fixture.state.workspaces.as_ref().unwrap().snapshot(),
                layout
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_refuses_a_backlog_or_partial_native_focus_report() {
        use std::io::Write;
        let (root, inputs, mut state, _subscriber) = subscribed_window();
        let (mut focus, control) = attach_reporting_venus(&mut state);
        let target = reload_venus(&root, "venus-b");
        let original = state.venus.as_ref().unwrap().child.id();
        let before = state.workspaces.as_ref().unwrap().snapshot();
        // Both old checks would stop on focus:1, leaving the final loss unread.
        focus.write_all(&b"focus:0\nfocus:1\n".repeat(100)).unwrap();
        focus.write_all(b"focus:0\n").unwrap();
        for _ in 0..3 {
            assert_eq!(
                super::reload_presentation(&mut state, &inputs, &root, &root, &target)
                    .unwrap_err()
                    .code,
                "reload-refused"
            );
            assert_eq!(state.venus.as_ref().unwrap().child.id(), original);
        }
        focus.write_all(b"focus:1\nfocus:").unwrap();
        assert_eq!(
            super::reload_presentation(&mut state, &inputs, &root, &root, &target)
                .unwrap_err()
                .code,
            "reload-refused"
        );
        assert_eq!(state.workspaces.as_ref().unwrap().snapshot(), before);
        drop(control);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reload_validates_each_protocol_and_the_immutable_target_before_retirement() {
        let inputs = crate::fixtures::inputs();
        let versions = *inputs.presentation_protocols.as_ref().unwrap();
        for index in 0..3 {
            let mut wrong = versions;
            wrong[index] += 1;
            assert!(
                super::validate_reload(&inputs, Path::new("/absent"), wrong)
                    .unwrap_err()
                    .contains(["EONW", "ORBS", "ORBF"][index])
            );
        }
        let root = temporary_directory();
        let path = reload_venus(&root, "venus");
        assert!(
            super::validate_reload(&inputs, &path, versions)
                .unwrap_err()
                .contains("/nix/store")
        );
        let bash = Command::new("bash")
            .args(["-c", "command -v bash"])
            .output()
            .unwrap();
        let bash = fs::canonicalize(String::from_utf8(bash.stdout).unwrap().trim()).unwrap();
        super::validate_reload(&inputs, &bash, versions).unwrap();
        let noncanonical = bash
            .parent()
            .unwrap()
            .join(".")
            .join(bash.file_name().unwrap());
        assert!(super::validate_reload(&inputs, &noncanonical, versions).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    /// The pane's attention and the window's queue, as Inspect shows them.
    #[cfg(target_os = "linux")]
    fn inspected(
        state: &mut SupervisorState,
    ) -> (
        Option<eon_workspace_protocol::v43::Attention>,
        Vec<eon_workspace_protocol::v43::QueueEntry>,
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
            super::ControlResponse::Workspace(eon_workspace_protocol::v43::Response::Snapshot(
                snapshot,
            )) => snapshot,
            super::ControlResponse::Workspace(eon_workspace_protocol::v43::Response::Failure(
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
    #[track_caller]
    fn raise_finish(
        state: &mut SupervisorState,
        gate: &Path,
    ) -> eon_workspace_protocol::v43::AttentionState {
        use eon_workspace_protocol::v43::AttentionState;
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let changed = |attention: &Option<eon_workspace_protocol::v43::Attention>| {
            attention
                .as_ref()
                .and_then(|attention| attention.record.as_ref())
                .map(|record| record.changed_at)
        };
        let shown = |state: &mut SupervisorState| {
            let snapshot = state.workspace_snapshot_in(None).unwrap();
            snapshot.tabs[0].panes().next().unwrap().attention.clone()
        };
        let before_attention = shown(state);
        let before = changed(&before_attention);
        let before_diagnostics = state.attention.as_ref().unwrap().diagnostics();
        fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(gate)
            .and_then(|mut writer| writer.write_all(b"\n"))
            .unwrap_or_else(|error| {
                panic!(
                    "cannot pulse {} at {}: {error}; before={before_attention:?}; poll={before_diagnostics}; native focus={:?}; leases={:?}",
                    gate.display(),
                    std::panic::Location::caller(),
                    state.venus.as_ref().map(|venus| venus.focus.focused),
                    state.sessions.iter().map(sessions::tests::lease_diagnostics).collect::<Vec<_>>()
                )
            });
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
            assert!(
                Instant::now() < deadline,
                "no finish within 20 s at {}, gate {}; before={before_attention:?}; before poll={before_diagnostics}; last={attention:?}; native focus={:?}; leases={:?}; poll={}",
                std::panic::Location::caller(),
                gate.display(),
                state.venus.as_ref().map(|venus| venus.focus.focused),
                state
                    .sessions
                    .iter()
                    .map(sessions::tests::lease_diagnostics)
                    .collect::<Vec<_>>(),
                state.attention.as_ref().unwrap().diagnostics()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// EON-C31: attachment alone does not see a raise; the selected
    /// terminal's finish waits for its window's native focus.
    #[cfg(target_os = "linux")]
    #[test]
    fn selected_raise_waits_for_native_window_focus() {
        use eon_workspace_protocol::v43::AttentionState::{Finished, Idle};
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
        use eon_workspace_protocol::v43::AttentionState::{Finished, Idle};
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

        assert!(state.venus.is_some() && state.stopping.is_none());
        assert_eq!(
            (state.sessions.len(), state.sessions[0].run_id()),
            (1, run.as_str())
        );
        stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap();
        drop(state);
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![session],
            venus: Some(venus),
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices,
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
            |_, _| Ok(Default::default()),
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: Some(workspace),
            sessions: vec![pair],
            venus: Some(venus),
            venus_path: None,
            initial_child: ["/bin/sh", "-c", "exec sleep 30"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices,
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
                line.contains("terminal terminal-1 ended without a terminal result (Orbit status 137); Orbit exited without publishing a terminal result")
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
            elapsed < SESSION_CLEANUP_TIMEOUT + Duration::from_millis(1500),
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
        // A supervisor that recovers the run rebuilds its log path.
        sessions::tests::release_lease(session);
        let (sessions, _) = sessions::recover_sessions(
            &root,
            LaunchMode::Workspace,
            "component-1",
            Some(&orbit_logs),
            Instant::now() + SESSION_CLEANUP_TIMEOUT,
        )
        .unwrap_or_else(|error| panic!("recover {}: {error}", orbit_log.display()));
        assert_eq!(sessions[0].orbit_log.as_ref(), Some(&orbit_log));
        kill_orbit_worker(orbit);
        let mut state = SupervisorState {
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
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
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions: vec![session],
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
            log: DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon"),
        };

        let error = stop_generation_sessions(&mut state, &root, LaunchMode::Workspace).unwrap_err();
        assert!(
            error.contains("terminal-1: terminal owner exited without completing Stop")
                && error.ends_with(&format!("; Orbit log: {}", orbit_log.display())),
            "{error}"
        );
        assert!(state.stopping.is_some());
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
                    line.contains("terminal terminal-1 ended without a terminal result (Orbit status 137); Orbit exited without publishing a terminal result")
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
                &mut sessions::FailedStarts::default(),
                &mut Vec::new(),
                &mut Sockets::default(),
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
            sessions::tests::release_lease(session);
            let (mut recovered, pair) = sessions::recover_sessions(
                &root,
                LaunchMode::Workspace,
                "component-1",
                None,
                Instant::now() + SESSION_CLEANUP_TIMEOUT,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "recover {} with pending={pending}: {error}",
                    endpoint.display()
                )
            });
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
        use eon_workspace_protocol::v43::{
            Column, Pane, PopupGeometry, Response, Snapshot, Tab, encode_response,
        };
        use std::time::{Duration, Instant};

        let root = temporary_directory();
        let snapshot = Snapshot {
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v43::WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
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
                            tag: None,
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
            google_quota: None,
            google_quota_failure: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v43::Sidebar::Open,
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
                &inputs.venus,
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
                assert_eq!(
                    launch_input(&inputs, LaunchMode::Workspace, "--eon-release-stage"),
                    ["alpha"]
                );
                assert_eq!(versions, [inputs.version]);
            } else {
                assert!(versions.is_empty());
                assert!(
                    launch_input(&inputs, LaunchMode::Workspace, "--eon-release-stage").is_empty()
                );
            }
            for flag in ["--bar-items", "--eon-version", "--eon-release-stage"] {
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
        for (stage, expected) in [
            (crate::ReleaseStage::Alpha, "alpha"),
            (crate::ReleaseStage::Beta, "beta"),
            (crate::ReleaseStage::Stable, "stable"),
        ] {
            inputs.release_stage = stage;
            assert_eq!(
                launch_input(&inputs, LaunchMode::Workspace, "--eon-release-stage"),
                [expected]
            );
        }
        // Load levels and the disk path reach Venus with the chip they belong to, and never a
        // terminal window. `~` names the supervisor's HOME.
        let levels = |inputs: &crate::Inputs, mode| {
            [
                "--ram-notice-percent",
                "--ram-warning-percent",
                "--ram-critical-percent",
                "--cpu-notice-percent",
                "--cpu-warning-percent",
                "--cpu-critical-percent",
                "--cpu-average-seconds",
                "--cpu-temperature-warning-celsius",
                "--cpu-temperature-critical-celsius",
                "--disk-notice-percent",
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
            [
                "60", "75", "90", "60", "75", "90", "30", "85", "95", "60", "75", "90", home
            ]
        );
        fs::write(
            root.join("config.toml"),
            "[bar]\nwidgets = ['ram']\nram_notice_percent = 1\nram_warning_percent = 1\nram_critical_percent = 1\n",
        )
        .unwrap();
        assert_eq!(
            levels(&inputs, LaunchMode::Workspace),
            ["1", "1", "1", "", "", "", "", "", "", "", "", "", ""]
        );
        fs::write(
            root.join("config.toml"),
            "[bar]\nwidgets = ['cpu']\ncpu_notice_percent = 100\ncpu_critical_percent = 100\ncpu_warning_percent = 100\ncpu_average_seconds = 600\ncpu_temperature_warning_celsius = 30\ncpu_temperature_critical_celsius = 120\n",
        )
        .unwrap();
        assert_eq!(
            levels(&inputs, LaunchMode::Workspace),
            [
                "", "", "", "100", "100", "100", "600", "30", "120", "", "", "", ""
            ]
        );
        for (path, measured) in [
            ("~/work/eon", format!("{home}/work/eon")),
            ("/srv/data", "/srv/data".to_owned()),
        ] {
            fs::write(
                root.join("config.toml"),
                format!(
                    "[bar]\nwidgets = ['disk']\ndisk_notice_percent = 1\ndisk_warning_percent = 1\n\
                     disk_critical_percent = 100\ndisk_path = '{path}'\n"
                ),
            )
            .unwrap();
            assert_eq!(
                levels(&inputs, LaunchMode::Workspace),
                [
                    "", "", "", "", "", "", "", "", "", "1", "1", "100", &measured
                ],
                "{path}"
            );
        }
        assert_eq!(
            levels(&inputs, LaunchMode::Terminal),
            ["", "", "", "", "", "", "", "", "", "", "", "", ""]
        );
        for (chip, warning, expected) in [
            ("cpu", 95, ["60", "95", "95"]),
            ("ram", 50, ["50", "50", "90"]),
            ("disk", 92, ["60", "92", "92"]),
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[bar]\nwidgets = ['{chip}']\n{chip}_warning_percent = {warning}\n"),
            )
            .unwrap();
            let actual = ["notice", "warning", "critical"].map(|tier| {
                launch_input(
                    &inputs,
                    LaunchMode::Workspace,
                    &format!("--{chip}-{tier}-percent"),
                )[0]
                .to_string_lossy()
                .into_owned()
            });
            assert_eq!(actual, expected, "{chip}");
        }
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
                "[bar]\nram_warning_percent = 95\nram_critical_percent = 90\n",
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
                "[bar]\ndisk_warning_percent = 96\ndisk_critical_percent = 90\n",
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
                &inputs.venus,
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
                    &inputs.venus,
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
                    &inputs.venus,
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
                &inputs.venus,
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
                    &inputs.venus,
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
                        &inputs.venus,
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
                    &inputs.venus,
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
                &inputs.venus,
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
    fn configured_panel_heading_reaches_workspace_launches_and_rereads_on_reopen() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        // The reopen path uses this same resolver and command projection.
        for (source, expected) in [
            ("", "connected"),
            ("[panels]\n", "connected"),
            ("[panels]\nheading = 'connected'\n", "connected"),
            ("[panels]\nheading = 'notch'\n", "notch"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &inputs.venus,
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
                    .filter(|pair| pair[0] == "--panel-heading")
                    .map(|pair| pair[1])
                    .collect::<Vec<_>>();
                if mode == LaunchMode::Workspace {
                    assert_eq!(values, [expected]);
                } else {
                    assert!(values.is_empty());
                }
            }
        }
        for value in [
            "'inline'",
            "'Connected'",
            "true",
            "0",
            "'notch'\nheading = 'connected'",
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[panels]\nheading = {value}\n"),
            )
            .unwrap();
            let error =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap_err();
            assert!(error.contains("heading"), "{error}");
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
                    &inputs.venus,
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
    fn configured_local_host_detection_reaches_every_launch() {
        let inputs = crate::fixtures::inputs();
        let root = temporary_directory();
        for (source, expected) in [
            ("", "true"),
            ("[terminal]\ndetect_local_hosts = false\n", "false"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let terminal =
                crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap();
            for mode in [LaunchMode::Workspace, LaunchMode::Terminal] {
                let command = venus_command(
                    &inputs,
                    &inputs.venus,
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
                    .filter(|pair| pair[0] == "--detect-local-hosts")
                    .map(|pair| pair[1])
                    .collect::<Vec<_>>();
                assert_eq!(values, [expected], "{source}");
            }
        }
        fs::write(
            root.join("config.toml"),
            "[terminal]\ndetect_local_hosts = \"false\"\n",
        )
        .unwrap();
        assert!(
            crate::managed_environment::terminal_presentation(&root, &inputs.defaults, None)
                .unwrap_err()
                .contains("detect_local_hosts")
        );
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
            &inputs.venus,
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
                "--detect-local-hosts",
                "true",
                "--cpu-notice-percent",
                "60",
                "--cpu-warning-percent",
                "75",
                "--cpu-critical-percent",
                "90",
                "--cpu-average-seconds",
                "30",
                "--cpu-temperature-warning-celsius",
                "85",
                "--cpu-temperature-critical-celsius",
                "95",
                "--bar-items",
                "cpu",
                "--panel-heading",
                "connected",
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
            &inputs.venus,
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
                "--detect-local-hosts",
                "true",
                "/runtime/orbit.sock",
            ]
            .map(OsString::from)
        );

        let undecorated = venus_command(
            &inputs,
            &inputs.venus,
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
                "--detect-local-hosts",
                "true",
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

    fn end_status_test_state(
        sessions: Vec<sessions::RunningSession>,
        log_root: &Path,
    ) -> SupervisorState {
        SupervisorState {
            failed_starts: sessions::FailedStarts::default(),
            registry_worker: None,
            registry_pending: None,
            started_at: 1,
            window: None,
            running: None,
            component_generation: "component-1".into(),
            application_id: String::new(),
            decorations: false,
            workspaces: None,
            sessions,
            venus: None,
            venus_path: None,
            initial_child: Vec::new(),
            initial_status: None,
            stopping: None,
            quotas: None,
            attention: None,
            pane_labels: super::super::pane_labels::Labels::default(),
            agents: window_agents(),
            notices: crate::notification::Notices::default(),
            subscribers: crate::subscriptions::Subscribers::new("g1-test"),
            scrollback: crate::scrollback::Snapshots::new(Path::new("/unused")),
            editor_sockets: Sockets::default(),
            log: DiagnosticLog::open_in(Ok(log_root.to_path_buf()), "g1-test", "eon"),
        }
    }

    fn read_logs(log_root: &Path) -> String {
        fs::read_dir(log_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| fs::read_to_string(path).unwrap())
            .collect()
    }

    #[test]
    fn a_signalled_initial_command_sets_initial_status_and_logs_signal() {
        let root = temporary_directory();
        let go = root.join("go");
        let session = leased_orbit(
            &root,
            &format!(
                "while [ ! -e '{}' ]; do sleep 0.02; done; kill -TERM $$",
                go.display()
            ),
        );
        let orbit = orbit_pid(&session).cast_unsigned();
        fs::write(&go, "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !crate::platform::process(orbit)
            .unwrap()
            .is_some_and(|process| process.ended)
        {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let record_bytes = fs::read(sessions::tests::record(&session)).unwrap();
        let record = orbit_protocol::management::decode_record(&record_bytes).unwrap();
        assert!(
            matches!(
                record,
                orbit_protocol::management::Record::Tombstone(
                    orbit_protocol::management::Tombstone {
                        reason: orbit_protocol::management::TerminationReason::NaturalExit,
                        outcome: orbit_protocol::management::ProcessOutcome::Signal(libc::SIGTERM),
                        ..
                    }
                )
            ),
            "{record:?}"
        );

        let log_root = root.join("logs");
        let mut state = end_status_test_state(vec![session], &log_root);
        let inputs = crate::fixtures::inputs();
        assert_eq!(reap_finished_sessions(&mut state, &inputs, &root), Ok(()));
        assert!(state.sessions.is_empty());
        assert_eq!(state.initial_status, Some(128 + libc::SIGTERM));
        let log = read_logs(&log_root);
        assert!(
            log.lines().any(|line| {
                line.contains(&format!(
                    "terminal terminal-1 ended with status {} (signal {})",
                    128 + libc::SIGTERM,
                    libc::SIGTERM
                ))
            }),
            "{log}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_orbit_killed_by_signal_reports_status_one_and_logs_orbit_signal() {
        let root = temporary_directory();
        let session = leased_orbit(&root, "exec sleep 30");
        let orbit_log = session.orbit_log.clone().unwrap();
        let orbit = orbit_pid(&session);
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(unsafe { libc::kill(orbit, libc::SIGKILL) }, 0);

        let log_root = root.join("logs");
        let mut state = end_status_test_state(vec![session], &log_root);
        let inputs = crate::fixtures::inputs();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.sessions.is_empty() {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            assert_eq!(reap_finished_sessions(&mut state, &inputs, &root), Ok(()));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(state.initial_status, Some(1));
        let log = read_logs(&log_root);
        assert!(
            log.lines().any(|line| {
                line.contains(&format!(
                    "terminal terminal-1 ended without a terminal result (Orbit signal {}, status {}); Orbit exited without publishing a terminal result",
                    libc::SIGKILL,
                    128 + libc::SIGKILL
                )) && line.ends_with(&format!("; Orbit log: {}", orbit_log.display()))
            }),
            "{log}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_recovered_orbit_without_known_exit_logs_unknown_status_and_reports_one() {
        let root = temporary_directory();
        let launcher = root.join("orbit-grandchild");
        script(
            &launcher,
            "#!/bin/sh\n\"$ORBIT_BIN\" \"$@\" &\nwait \"$!\"\n",
        );
        let session = sessions::tests::leased_orbit_with_program(&root, "exec sleep 30", &launcher);
        let sh_pid = orbit_pid(&session);
        let orbit = children(sh_pid)
            .into_iter()
            .next()
            .expect("launcher spawned Orbit");
        let orbit_logs = root.join("launch-logs/orbit");
        let orbit_log = orbit_logs.join(format!("component-1-terminal-1-{}.log", session.run_id()));
        sessions::tests::release_lease(session);
        let (sessions, _) = sessions::recover_sessions(
            &root,
            LaunchMode::Workspace,
            "component-1",
            Some(&orbit_logs),
            Instant::now() + SESSION_CLEANUP_TIMEOUT,
        )
        .unwrap();
        kill_orbit_worker(orbit);

        let log_root = root.join("logs");
        let mut state = end_status_test_state(sessions, &log_root);
        let inputs = crate::fixtures::inputs();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.sessions.is_empty() {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            assert_eq!(reap_finished_sessions(&mut state, &inputs, &root), Ok(()));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(state.initial_status, Some(1));
        let log = read_logs(&log_root);
        assert!(
            log.lines().any(|line| {
                line.contains(
                    "terminal terminal-1 ended with unknown status; Orbit exited without publishing a terminal result",
                ) && line.ends_with(&format!("; Orbit log: {}", orbit_log.display()))
            }),
            "{log}"
        );
        let _ = rustix::process::waitpid(
            rustix::process::Pid::from_raw(sh_pid),
            rustix::process::WaitOptions::empty(),
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_signalled_program_reconciled_during_close_logs_its_signal() {
        let root = temporary_directory();
        let go = root.join("go");
        let session = leased_orbit(
            &root,
            &format!(
                "while [ ! -e '{}' ]; do sleep 0.02; done; kill -TERM $$",
                go.display()
            ),
        );
        let orbit = orbit_pid(&session).cast_unsigned();
        fs::write(&go, "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !crate::platform::process(orbit)
            .unwrap()
            .is_some_and(|process| process.ended)
        {
            assert!(std::time::Instant::now() < deadline, "Orbit never exited");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let log_root = root.join("logs");
        let log = DiagnosticLog::open_in(Ok(log_root.clone()), "g1-test", "eon");
        let mut sessions = vec![session];
        assert_eq!(
            stop_owned_session(&mut sessions, "terminal-1", &log),
            Ok(Some(128 + libc::SIGTERM))
        );
        assert!(sessions.is_empty());
        let supervisor_log = read_logs(&log_root);
        assert!(
            supervisor_log.lines().any(|line| {
                line.contains(&format!(
                    "terminal terminal-1 ended with status {} (signal {})",
                    128 + libc::SIGTERM,
                    libc::SIGTERM
                ))
            }),
            "{supervisor_log}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
