mod events;

use super::{
    control::{
        ControlResponse, EndpointFailure, EndpointFailureKind, Error, failure, report_failure,
        send_action_from, send_action_from_until, validate_action, write_stdout,
    },
    generation::{
        Ask, WorkspaceTarget, attach_generation, confirm, current_generation, generations_command,
        resolve_workspace, stop_generation, valid_generation,
    },
    managed_environment,
    running::running_notice,
    sessions::{TAB_VARIABLE, TERMINAL_VARIABLE, terminal_session},
    supervisor::{
        self, LaunchMode, LaunchOptions, WORKSPACE_APPLICATION_ID, WORKSPACE_DECORATIONS,
        configuration_directory, launch_current, prepare_configuration, runtime_directory,
    },
    windows,
    workspace::{
        caller_output, failure_human, human as human_output, json as json_output, json_escape,
        keys_human, keys_json, list_human, list_json, registration_json, resolve_target,
        tab_number, terminal_editing_json, terminal_history_json, terminal_sent_json,
        terminal_text_json,
    },
};
use eon_workspace_protocol::v50::{
    Action, AttentionState, CloseConsent, CloseEnd, CloseScope, Direction, FILES_ENTRY, Failure,
    InputLock, InvokeIntent, MAX_HISTORY_TEXT_BYTES, MAX_PASTE_BYTES, Pane, PopupTarget, Response,
    Side, Snapshot, Tab, TargetId, TerminalHistory, TerminalInput, VERSION, WorkspaceAction,
    WorkspaceInfo, WorkspaceTarget as SessionTarget, entry_popup, pane_label_character,
    valid_caller, validate_paste,
};
use std::{
    cmp::Ordering,
    env,
    ffi::{OsStr, OsString},
    fmt::Write as _,
    io::{IsTerminal, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const EON_USAGE: &str = "invalid command or arguments\nusage: eon [COMMAND]\nRun `eon --help` for commands and examples.";
const EONBASE_USAGE: &str = "usage: eonbase [--no-decorations] [--application-id ID] -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json]";

/// Immutable product release metadata supplied by the composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseStage {
    Alpha,
    Beta,
    Stable,
}

impl ReleaseStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Alpha => "alpha",
            Self::Beta => "beta",
            Self::Stable => "stable",
        }
    }
}

pub struct Inputs {
    /// Original supported launch inputs, or why this invocation is not an Update target.
    /// Ineligibility never fails an ordinary command or launch.
    pub launch: Result<super::launch_environment::LaunchInputs, String>,
    /// Eon's installed association reader for a trusted candidate wrapper.
    pub target_launch_facts: fn(&Path) -> Result<super::launch_environment::LaunchFacts, String>,
    #[cfg(test)]
    pub(crate) update_store: PathBuf,
    pub release_stage: ReleaseStage,
    pub version: &'static str,
    /// The agent skill this build ships, which `eon --skill` prints.
    pub skill: &'static str,
    /// This build's whole component graph, which supervisors report and launches compare.
    pub component_report: Result<String, String>,
    /// Venus's exact EONW, ORBS and ORBF requirements from the component graph.
    pub presentation_protocols: Result<[u16; 3], String>,
    /// The components this composition includes, which `eon versions` prints:
    /// `component_report` without the tools the package leaves out.
    pub composed_components: Result<String, String>,
    pub assembly: &'static [u8],
    pub defaults: managed_environment::Defaults,
    /// Defaults for the separate canonical-account EON-C43 host policy.
    pub slot_policy: super::slots::SlotPolicy,
    pub orbit: PathBuf,
    pub venus: PathBuf,
    pub anima: PathBuf,
    /// The Eon icon's image, passed to Venus unread; none when Eon has no icon.
    pub icon: Option<PathBuf>,
    pub codexbar: PathBuf,
    pub managed: managed_environment::ManagedPrograms,
}

impl Inputs {
    pub(crate) fn supervisor_command(&self) -> Result<Command, String> {
        let mut command = match &self.launch {
            Ok(launch) => launch.command(launch.facts()),
            Err(_) => Command::new(env::current_exe().map_err(|error| error.to_string())?),
        };
        super::launch_environment::remove_launch_markers(&mut command);
        let display = super::launch_environment::WAYLAND_SOCKET;
        if cfg!(target_os = "linux")
            && let Some(value) = env::var_os(display)
        {
            command.env(display, value);
        }
        Ok(command)
    }

    pub(crate) fn component_report(&self) -> Result<&str, String> {
        self.component_report.as_deref().map_err(Clone::clone)
    }
}

/// The executable name of the Workspaces picker, whose own popup key confirms its highlighted
/// row (EON-C33).
pub(crate) const WORKSPACE_PICKER: &str = "eon-sessions";

pub fn run(inputs: Inputs) -> (&'static str, Result<i32, Error>) {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments: Vec<OsString> = arguments.collect();
    if Path::new(&invocation).file_name() == Some(OsStr::new("eon"))
        && arguments
            .first()
            .is_some_and(|flag| flag == supervisor::VALIDATE_UPDATE_TARGET)
    {
        return (
            "eon",
            supervisor::validate_update_target(&inputs, &arguments[1..]),
        );
    }
    if super::update::serves_update()
        && Path::new(&invocation).file_name() == Some(OsStr::new("eon"))
        && arguments
            .first()
            .is_some_and(|flag| flag == supervisor::RESUME_UPDATE)
    {
        return ("eon", supervisor::resume_update(&inputs, &arguments[1..]));
    }
    let _trace = super::diagnostics::BenchTrace::start();
    match Path::new(&invocation).file_name().and_then(OsStr::to_str) {
        Some("eonbase") => ("eonbase", execute_eonbase(&inputs, arguments)),
        Some("eon-commands") => ("eon-commands", Ok(command_runner(&inputs))),
        Some(WORKSPACE_PICKER) => (WORKSPACE_PICKER, Ok(workspace_picker(&inputs))),
        // Other names stay Eon: Nix's wrapped executable relaunches itself under its own file
        // name for independent windows.
        _ => match managed_environment::tool(&invocation) {
            Some(tool) => (
                "eon",
                launch_managed(&inputs, tool, arguments).map_err(Error::from),
            ),
            None => ("eon", execute(&inputs, arguments)),
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunnerOutput {
    /// The popup closes once the command succeeds.
    Close,
    /// The popup waits for Enter so its output stays readable.
    Keep,
}

/// The Commands popup's curated list. Each entry runs an existing `eon` command as the popup's
/// caller, so the CLI keeps owning validation, targets and confirmations. `TAB` stands for the
/// popup's own tab, `PANE` for its tab's selected pane and `THIS` for its own session.
/// `NAME` is typed in a name box, where an empty name
/// clears. `SESSION` is chosen in the Sessions popup's picker, run inside Commands: a query that
/// matches no session runs `session new --name QUERY` instead, and a cancel runs nothing.
/// Pane moves are absent because Eon refuses them while a popup is open.
const RUNNER_COMMANDS: &[(&str, &[&str], RunnerOutput)] = {
    use RunnerOutput::{Close, Keep};
    &[
        ("New tab", &["tab", "create"], Close),
        ("New pane", &["pane", "create"], Close),
        ("Rename pane", &["pane", "rename", "PANE", "NAME"], Close),
        (
            "Clear pane name",
            &["pane", "rename", "PANE", "--clear"],
            Close,
        ),
        ("New window", &["window", "new"], Close),
        ("Close tab", &["tab", "close", "TAB"], Close),
        ("Quit this window", &["quit"], Close),
        ("Move tab left", &["tab", "move", "left"], Close),
        ("Move tab right", &["tab", "move", "right"], Close),
        ("Switch session", &["session", "switch", "SESSION"], Close),
        ("New session", &["session", "new"], Close),
        ("Rename session", &["session", "rename", "NAME"], Close),
        ("Close this session", &["session", "close", "THIS"], Close),
        ("Show session", &["session"], Keep),
        ("List windows", &["window", "ls"], Keep),
        (
            "Stop old-version windows",
            &["window", "stop", "previous"],
            Keep,
        ),
        ("Show versions", &["versions"], Keep),
        ("Show bug report", &["report"], Keep),
        ("Check for updates", &["update", "--check"], Keep),
        ("Install update", &["update", "--install"], Keep),
        ("Show config path", &["config", "path"], Keep),
        ("Show command help", &["--help"], Keep),
    ]
};

fn runner_arguments<'a>(arguments: &[&'a str], tab: &'a str) -> Vec<&'a str> {
    arguments
        .iter()
        .map(|argument| match *argument {
            "TAB" => tab,
            _ => argument,
        })
        .collect()
}

/// The picker's lines: index, label and the exact `eon` command each runs.
fn runner_list(tab: &str) -> String {
    RUNNER_COMMANDS
        .iter()
        .enumerate()
        .map(|(index, (label, arguments, _))| {
            let command = runner_arguments(arguments, tab).join(" ");
            format!("{index}\t{label:<26}eon {command}\n")
        })
        .collect()
}

/// Runs the Commands popup and keeps a listing or any failure on screen until Enter.
fn command_runner(inputs: &Inputs) -> i32 {
    let (status, kept) = pick_and_run_command(inputs).unwrap_or_else(|error| {
        eprintln!("eon-commands: {error}");
        (1, RunnerOutput::Keep)
    });
    if kept == RunnerOutput::Keep || status != 0 {
        wait_for_enter();
    }
    status
}

/// Runs the Workspaces popup and keeps any failure on screen until Enter.
fn workspace_picker(inputs: &Inputs) -> i32 {
    let status = pick_and_run_workspace(inputs).unwrap_or_else(|error| {
        eprintln!("eon-sessions: {error}");
        1
    });
    if status != 0 {
        wait_for_enter();
    }
    status
}

fn wait_for_enter() {
    print!("\nPress Enter to close");
    let _ = std::io::stdout().flush();
    let _ = std::io::stdin().read_line(&mut String::new());
}

fn pick_and_run_command(inputs: &Inputs) -> Result<(i32, RunnerOutput), String> {
    let tab = env::var(TAB_VARIABLE).map_err(|_| "runs only in an Eon popup".to_string())?;
    let Some(output) = run_picker(
        packaged_fzf("Command > ", "Enter Run · Esc/Ctrl+C Cancel")
            .args(["--delimiter=\t", "--with-nth=2.."]),
        &runner_list(&tab),
        "command",
    )?
    else {
        return Ok((0, RunnerOutput::Close));
    };
    let (_, arguments, kept) = String::from_utf8_lossy(&output)
        .split('\t')
        .next()
        .and_then(|index| index.trim().parse::<usize>().ok())
        .and_then(|index| RUNNER_COMMANDS.get(index))
        .ok_or("command picker returned an invalid selection")?;
    let mut arguments: Vec<OsString> = runner_arguments(arguments, &tab)
        .into_iter()
        .map(OsString::from)
        .collect();
    if arguments.iter().any(|argument| argument == "SESSION") {
        return Ok((pick_and_run_workspace(inputs)?, *kept));
    }
    if arguments
        .iter()
        .any(|argument| argument == "PANE" || argument == "THIS")
    {
        let snapshot = caller_snapshot(inputs)?;
        let pane = snapshot
            .tabs
            .iter()
            .find(|candidate| candidate.id == tab)
            .and_then(|tab| tab.selected_pane.as_deref());
        for argument in &mut arguments {
            if argument == "PANE" {
                *argument = pane.ok_or("this tab has no pane to name")?.into();
            } else if argument == "THIS" {
                *argument = snapshot.workspace.as_str().into();
            }
        }
    }
    if arguments.last().is_some_and(|argument| argument == "NAME") {
        let subject = if arguments[0] == "pane" {
            "Pane"
        } else {
            "Session"
        };
        let output = packaged_fzf(
            &format!("{subject} name > "),
            "Enter Set · Empty Clear · Esc/Ctrl+C Cancel",
        )
        .args(["--bind=enter:print-query", "--print0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .map_err(|error| format!("cannot read the name: {error}"))?;
        if matches!(output.status.code(), Some(1 | 130)) {
            return Ok((0, RunnerOutput::Close));
        }
        if !output.status.success() {
            return Err("name box failed".into());
        }
        let name = parse_name_output(&output.stdout)?;
        arguments.pop();
        arguments.push("--".into());
        arguments.push(name.into());
    }
    let status = if arguments[0] == "update" {
        crate::platform::update_status(Command::new("eon").args(arguments))?
            .code()
            .unwrap_or(1)
    } else if arguments[0] == "report" {
        // The product answers `eon report`, as it does `eon update`.
        Command::new("eon")
            .args(arguments)
            .status()
            .map_err(|error| format!("cannot run eon report: {error}"))?
            .code()
            .unwrap_or(1)
    } else {
        run_as_caller(inputs, arguments)
    };
    Ok((status, *kept))
}

/// Runs the workspace picker as the popup's caller: Enter on a row, including on an empty query,
/// shows that workspace, Enter on a non-blank query that matches no row creates a workspace of
/// that name, Ctrl+X on a row closes that workspace and lists the rest again, and Escape or
/// Ctrl+C does nothing.
fn pick_and_run_workspace(inputs: &Inputs) -> Result<i32, String> {
    loop {
        let snapshot = caller_snapshot(inputs)?;
        let Some(output) = run_picker(
            packaged_fzf(
                "Session > ",
                "Enter Switch · Ctrl+X Close · No match: Enter Create · Esc/Ctrl+C Cancel",
            )
            .args([
                "--exact",
                "--delimiter=\t",
                "--with-nth=2..",
                "--nth=1",
                "--print-query",
                "--expect=ctrl-x",
                "--bind=enter:accept-or-print-query",
            ]),
            &workspace_rows(&snapshot.workspaces, &snapshot.active_workspace),
            "session",
        )?
        else {
            return Ok(0);
        };
        let output = String::from_utf8_lossy(&output);
        let Some(arguments) = workspace_pick_arguments(&output) else {
            return Ok(0);
        };
        // Runtime would read a departed row's sN as another workspace's name, so a row switches
        // or closes only while its sN is still live; the check and the action are not atomic.
        if arguments[1] != "new"
            && !caller_snapshot(inputs)?
                .workspaces
                .iter()
                .any(|workspace| workspace.id == arguments[2])
        {
            let detail = format!("session {} is not live", arguments[2]);
            return report_failure(&failure("unknown-session", detail), false);
        }
        let status = run_as_caller(
            inputs,
            arguments.iter().copied().map(OsString::from).collect(),
        );
        if let Some(status) = workspace_pick_end(&arguments, status, &snapshot.workspace, || {
            caller_snapshot(inputs).ok().map(|after| after.workspaces)
        }) {
            return Ok(status);
        }
    }
}

/// How the workspace picker ends after running `arguments` with `status`, or None to list
/// again. Only a close that succeeded, including a declined one, lists again. A close of the
/// picker's own workspace lists again only while `listed()`, the caller's fresh list (None when
/// it cannot be read), still holds it; once it has closed the picker ends with it.
fn workspace_pick_end(
    arguments: &[&str],
    status: i32,
    own: &str,
    listed: impl FnOnce() -> Option<Vec<WorkspaceInfo>>,
) -> Option<i32> {
    if arguments[1] != "close" || status != 0 {
        return Some(status);
    }
    if arguments[2] != own {
        return None;
    }
    match listed() {
        Some(workspaces) if workspaces.iter().any(|workspace| workspace.id == own) => None,
        _ => Some(0),
    }
}

/// The workspace picker's rows: ID, label, tab count and a needs-input marker, most recently
/// shown first with the shown workspace last and marked, so Enter on an empty query returns to
/// the previous one (EON-C33).
fn workspace_rows(workspaces: &[WorkspaceInfo], shown: &str) -> String {
    let mut rows: Vec<_> = workspaces.iter().collect();
    rows.sort_by_key(|workspace| (workspace.id == shown, workspace.recency));
    rows.into_iter()
        .map(|workspace| {
            format!(
                "{}\t{}\t{} tab{}{}{}\n",
                workspace.id,
                workspace.name.as_deref().unwrap_or(&workspace.id),
                workspace.tab_count,
                if workspace.tab_count == 1 { "" } else { "s" },
                if workspace.attention == Some(AttentionState::NeedsInput) {
                    " · needs input"
                } else {
                    ""
                },
                if workspace.id == shown {
                    " · shown"
                } else {
                    ""
                },
            )
        })
        .collect()
}

/// The `eon` arguments for the workspace picker's output: its query line, then the key that
/// accepted (empty for Enter) and the chosen row when one matched. Ctrl+X closes the row's
/// workspace and Enter shows it; a query that matched no row names a new workspace; a blank one
/// runs nothing.
fn workspace_pick_arguments(output: &str) -> Option<Vec<&str>> {
    let mut lines = output.lines();
    let query = lines.next().unwrap_or_default();
    let key = lines.next();
    match lines.next() {
        Some(row) => Some(vec![
            "session",
            if key == Some("ctrl-x") {
                "close"
            } else {
                "switch"
            },
            row.split('\t').next()?,
        ]),
        None if key.is_none() && !query.trim().is_empty() => {
            Some(vec!["session", "new", "--name", query])
        }
        None => None,
    }
}

/// `eon agent`'s quota facts (EON-C40), in the session JSON's projection. Each provider's
/// snapshot and failure come from the calling terminal's window, which gets at most half the
/// time left; a provider that window does not supply, because it cannot answer or has neither
/// for it, comes from the quota store windows share. `quota_source` names each provider's
/// source (`window ID` or `store`) and `quota_unreadable` why no source has one.
pub fn quota_facts_json(inputs: &Inputs, deadline: Instant) -> Result<String, String> {
    let now = Instant::now();
    let window = caller_window(
        inputs,
        now + (deadline.saturating_duration_since(now) / 2).min(super::control::CONTROL_TIMEOUT),
    );
    let store = super::quota::store_directory();
    let mut sources = serde_json::Map::new();
    let mut unreadable = serde_json::Map::new();
    // A window that has a provider's snapshot or failure supplies it; the store fills the rest.
    let mut fill = |provider: &str, supplied: fn(&Snapshot) -> bool| {
        let from_window = match &window {
            Ok((snapshot, label)) if supplied(snapshot) => {
                sources.insert(provider.into(), format!("window {label}").into());
                return None;
            }
            Ok((_, label)) => format!("Eon window {label} has no {provider} facts"),
            Err(detail) => detail.clone(),
        };
        let stored = store
            .as_deref()
            .ok_or_else(|| "no Eon state directory (XDG_STATE_HOME or HOME)".to_owned())
            .and_then(|store| super::quota::stored_facts(store, provider, deadline));
        match stored {
            Ok(stored) => {
                sources.insert(provider.into(), "store".into());
                Some(stored)
            }
            Err(from_store) => {
                unreadable.insert(
                    provider.into(),
                    format!("window: {from_window}; store: {from_store}").into(),
                );
                None
            }
        }
    };
    let codex_stored = fill("codex", |snapshot| {
        snapshot.codex_quota.is_some() || snapshot.codex_quota_failure.is_some()
    });
    let claude_stored = fill("claude", |snapshot| {
        snapshot.claude_quota.is_some() || snapshot.claude_quota_failure.is_some()
    });
    let snapshot = window.as_ref().ok().map(|(snapshot, _)| snapshot);
    let codex = match codex_stored {
        Some((quota, failure)) => (Some(quota), failure),
        None => (
            snapshot.and_then(|snapshot| snapshot.codex_quota.clone()),
            snapshot.and_then(|snapshot| snapshot.codex_quota_failure),
        ),
    };
    let claude = match claude_stored {
        Some((quota, failure)) => (Some(super::quota::claude_quota(quota)), failure),
        None => (
            snapshot.and_then(|snapshot| snapshot.claude_quota.clone()),
            snapshot.and_then(|snapshot| snapshot.claude_quota_failure),
        ),
    };
    Ok(super::workspace::quota_facts_json(
        (codex.0.as_ref(), codex.1),
        (claude.0.as_ref(), claude.1),
        sources,
        unreadable,
    ))
}

/// The calling terminal's window's session snapshot and full ID, inspected by `deadline`.
fn caller_window(inputs: &Inputs, deadline: Instant) -> Result<(Snapshot, String), String> {
    let caller = calling_terminal().map_err(|error| error.detail)?;
    let target = WorkspaceTarget {
        deadline: Some(deadline),
        ..WorkspaceTarget::default()
    };
    let resolved =
        resolve_workspace(inputs, target, caller.as_deref()).map_err(|error| error.detail)?;
    match send_action_from_until(
        &resolved.socket,
        caller,
        Action::Workspace(WorkspaceAction::Inspect),
        deadline,
    ) {
        Ok(ControlResponse::Workspace(Response::Snapshot(snapshot))) => {
            Ok((snapshot, resolved.label))
        }
        Ok(ControlResponse::Workspace(Response::Failure(error))) => Err(error.detail),
        Ok(_) => Err("Eon returned the wrong result for session inspection".into()),
        Err(error) => Err(unreachable_supervisor(error, &resolved.label).detail),
    }
}

/// The popup caller's own session, as `eon session` reports it.
fn caller_snapshot(inputs: &Inputs) -> Result<Snapshot, String> {
    let caller = calling_terminal().map_err(|error| error.detail)?;
    let resolved = resolve_workspace(inputs, WorkspaceTarget::default(), caller.as_deref())
        .map_err(|error| error.detail)?;
    inspect(&resolved.socket, caller.as_deref(), &resolved.label).map_err(|error| error.detail)
}

/// Runs an `eon` command in-process as the popup's caller and returns its exit status.
fn run_as_caller(inputs: &Inputs, arguments: Vec<OsString>) -> i32 {
    match execute(inputs, arguments) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("eon: {}", error.message);
            i32::from(error.status)
        }
    }
}

/// Runs a packaged picker over `list` and returns what it printed, or None when it was
/// cancelled.
fn run_picker(picker: &mut Command, list: &str, noun: &str) -> Result<Option<Vec<u8>>, String> {
    let mut picker = picker
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot launch packaged {noun} picker: {error}"))?;
    picker
        .stdin
        .take()
        .expect("picker stdin is piped")
        .write_all(list.as_bytes())
        .map_err(|error| format!("cannot list {noun}s: {error}"))?;
    let output = picker
        .wait_with_output()
        .map_err(|error| format!("cannot read the {noun} picker: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(Some(output.stdout)),
        // No match, Escape and Ctrl+C all cancel.
        Some(1 | 130) => Ok(None),
        _ => Err(format!(
            "packaged {noun} picker exited with status {}",
            output.status
        )),
    }
}

fn parse_name_output(output: &[u8]) -> Result<&str, String> {
    let text = output
        .strip_suffix(&[0])
        .filter(|text| !text.contains(&0))
        .ok_or("name box returned malformed text")?;
    std::str::from_utf8(text).map_err(|_| "name is not UTF-8".into())
}

fn play_startup_anima(inputs: &Inputs, style: &OsStr, seconds: u64) -> Result<(), String> {
    let terminal = rustix::termios::tcgetattr(std::io::stdin()).ok();
    let result = (|| {
        let mut child = Command::new(&inputs.anima)
            .arg(style)
            .arg("--duration-seconds")
            .arg(seconds.to_string())
            .spawn()
            .map_err(|error| format!("cannot launch pinned executable: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(seconds + 2);
        loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("exited with status {status}"))
                };
            }
            if Instant::now() >= deadline {
                let _ = rustix::process::kill_process(
                    rustix::process::Pid::from_child(&child),
                    rustix::process::Signal::TERM,
                );
                let grace = Instant::now() + Duration::from_millis(250);
                while Instant::now() < grace {
                    if child
                        .try_wait()
                        .map_err(|error| error.to_string())?
                        .is_some()
                    {
                        return Err("timed out".into());
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    if result.is_err() {
        if let Some(terminal) = terminal {
            let _ = rustix::termios::tcsetattr(
                std::io::stdin(),
                rustix::termios::OptionalActions::Now,
                &terminal,
            );
        }
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"\x1b[?1049l");
        let _ = stdout.flush();
    }
    result
}

/// The packaged fzf with the look Commands and Workspaces share, ignoring the user's
/// own fzf options.
fn packaged_fzf(prompt: &str, footer: &str) -> Command {
    let mut fzf = Command::new("fzf");
    fzf.args([
        "--bind=ctrl-z:ignore,btab:up",
        "--cycle",
        "--info=inline",
        "--layout=reverse",
        "--border=none",
        "--footer-border=none",
        "--color=footer:-1",
    ])
    .arg(format!("--prompt={prompt}"))
    .arg(format!("--footer={footer}"))
    .env_remove("FZF_DEFAULT_OPTS")
    .env_remove("FZF_DEFAULT_OPTS_FILE")
    .stderr(Stdio::inherit());
    fzf
}

fn startup_notice(socket: &Path) -> Option<String> {
    let tab_id = env::var(TAB_VARIABLE).ok()?;
    let caller = env::var(TERMINAL_VARIABLE).ok()?;
    let terminal = terminal_session(&caller)?;
    poll_startup_notice(&tab_id, terminal, |deadline| {
        send_action_from_until(
            socket,
            None,
            Action::Workspace(WorkspaceAction::Inspect),
            deadline,
        )
    })
}

fn poll_startup_notice(
    tab_id: &str,
    terminal: &str,
    mut inspect: impl FnMut(Instant) -> Result<ControlResponse, EndpointFailure>,
) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut admitted = false;
    loop {
        let snapshot = match inspect(deadline) {
            Ok(ControlResponse::Workspace(Response::Snapshot(snapshot))) => snapshot,
            Err(error)
                if error.kind == EndpointFailureKind::Unreachable && Instant::now() >= deadline =>
            {
                break;
            }
            _ => return None,
        };
        // A request held during bootstrap may be answered after Venus dismisses the picker.
        if snapshot.active_tab != tab_id {
            return None;
        }
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id && tab.pending)?;
        if !tab.popups.iter().any(|popup| {
            tab.selected_popup.as_ref() == Some(&popup.id)
                && popup.entry == FILES_ENTRY
                && popup.session == terminal
        }) {
            return None;
        }
        let running = snapshot.running?;
        admitted = true;
        if let Some(others) = running.others {
            return Some(running_notice(&others, running.omitted).unwrap_or_default());
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    admitted.then(String::new)
}

fn launch_managed(
    inputs: &Inputs,
    tool: managed_environment::Tool,
    arguments: Vec<OsString>,
) -> Result<i32, String> {
    // A name this build cannot run refuses before anything is prepared.
    managed_environment::program(tool, &inputs.managed)?;
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut arguments = arguments;
    let mut notice = None;
    if tool == managed_environment::Tool::Files {
        // The startup pair plays Anima in its terminal before Yazi starts, and
        // its quick search names the other running windows (EON-C18).
        while let Some(flag) = arguments.first().and_then(|flag| flag.to_str()) {
            match (flag, arguments.as_slice()) {
                ("--eon-anima", [_, style, seconds, ..]) => {
                    let seconds = seconds
                        .to_str()
                        .and_then(|value| value.parse().ok())
                        .filter(|value| (1..=30).contains(value))
                        .ok_or("invalid internal Anima duration")?;
                    if let Err(error) = play_startup_anima(inputs, style, seconds) {
                        eprintln!("eon: startup Anima: {error}; continuing to quick search");
                    }
                    arguments.drain(..3);
                }
                ("--eon-notice", [_, socket, ..]) => {
                    notice = startup_notice(Path::new(socket));
                    arguments.drain(..2);
                }
                _ => break,
            }
        }
    }
    let mut command =
        managed_environment::command(tool, &inputs.managed, &config, &arguments, &inputs.defaults)?;
    if let Some(notice) = notice {
        command.env(managed_environment::FILES_NOTICE_VARIABLE, notice);
    }
    let program = command.get_program().to_string_lossy().into_owned();
    super::diagnostics::bench_trace("exec", "managed-tool");
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

/// Splits a full-Eon launch's leading `--dir DIR` and `--no-anima`, in
/// either order and each at most once, from what follows them (EON-C18). DIR
/// is neither empty nor `--`, as for `tab create --dir`.
fn parse_launch_options(arguments: &[OsString]) -> Result<(LaunchOptions, &[OsString]), Error> {
    let mut options = LaunchOptions::default();
    let mut rest = arguments;
    loop {
        match rest {
            [flag, ..] if flag == "--dir" && options.directory.is_none() => {
                let [_, directory, tail @ ..] = rest else {
                    return Err(Error::usage(EON_USAGE));
                };
                if directory.is_empty() || directory == "--" {
                    return Err(Error::usage(EON_USAGE));
                }
                options.directory = Some(std::path::absolute(directory).map_err(|error| {
                    Error::from(format!("cannot resolve {directory:?}: {error}"))
                })?);
                rest = tail;
            }
            [flag, tail @ ..] if flag == "--no-anima" && !options.no_anima => {
                options.no_anima = true;
                rest = tail;
            }
            _ => return Ok((options, rest)),
        }
    }
}

fn parse_quit_arguments(arguments: &[OsString]) -> Result<Option<(bool, bool)>, Error> {
    let [command, flags @ ..] = arguments else {
        return Ok(None);
    };
    if command != "quit" {
        return Ok(None);
    }
    let (mut all, mut json) = (false, false);
    for flag in flags {
        if flag == "--all" && !all {
            all = true;
        } else if flag == "--json" && !json {
            json = true;
        } else {
            return Err(Error::usage(EON_USAGE));
        }
    }
    Ok(Some((all, json)))
}

/// The bulk Stop `eon window stop previous|all` names.
fn bulk(which: &OsString) -> windows::Bulk {
    if which == "previous" {
        windows::Bulk::Previous
    } else {
        windows::Bulk::All
    }
}

fn execute(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, Error> {
    if let Some(error) = renamed_command(&arguments) {
        return Err(error);
    }
    if let Some(result) = command_help(inputs.version, &arguments) {
        return result;
    }
    if let [command, operation, rest @ ..] = arguments.as_slice()
        && command == "session"
        && operation == "registry"
    {
        return registry_command(rest);
    }
    if arguments.first().is_some_and(|word| word == "events")
        || (arguments.first().is_some_and(|word| word == "--window")
            && arguments.get(2).is_some_and(|word| word == "events"))
    {
        return events::execute(inputs, &arguments);
    }
    if let Some((all, json)) = parse_quit_arguments(&arguments)? {
        return if all {
            windows::stop_windows(inputs, windows::Bulk::Quit, json)
        } else {
            windows::quit(inputs, json, &arguments)
        };
    }
    let result = match arguments.as_slice() {
        [] => launch_current(
            inputs,
            LaunchMode::Workspace,
            &[],
            true,
            WORKSPACE_DECORATIONS,
            WORKSPACE_APPLICATION_ID,
            &LaunchOptions::default(),
        ),
        [flag, ..] if flag == "--dir" || flag == "--no-anima" => {
            let (options, []) = parse_launch_options(&arguments)? else {
                return Err(Error::usage(EON_USAGE));
            };
            launch_current(
                inputs,
                LaunchMode::Workspace,
                &[],
                true,
                WORKSPACE_DECORATIONS,
                WORKSPACE_APPLICATION_ID,
                &options,
            )
        }
        [command, action, rest @ ..] if command == "window" && action == "new" => {
            let (options, []) = parse_launch_options(rest)? else {
                return Err(Error::usage(EON_USAGE));
            };
            windows::new_window(inputs, &options)
        }
        [command, action] if command == "window" && action == "ls" => {
            windows::list_windows(inputs, false)
        }
        [command, action, flag] if command == "window" && action == "ls" && flag == "--json" => {
            windows::list_windows(inputs, true)
        }
        [command, action] if command == "window" && action == "attach" => {
            Ok(windows::attach_window(inputs, None)?)
        }
        [command, action, id] if command == "window" && action == "attach" => {
            Ok(windows::attach_window(inputs, Some(id))?)
        }
        [command, action, id] if command == "window" && action == "reload" => {
            Ok(windows::reload_window(inputs, id, false, &arguments)?)
        }
        [command, action, id, flag]
            if command == "window" && action == "reload" && flag == "--json" =>
        {
            Ok(windows::reload_window(inputs, id, true, &arguments)?)
        }
        [command, action, which]
            if command == "window"
                && action == "stop"
                && (which == "previous" || which == "all") =>
        {
            Ok(windows::stop_windows(inputs, bulk(which), false)?)
        }
        [command, action, which, flag]
            if command == "window"
                && action == "stop"
                && (which == "previous" || which == "all")
                && flag == "--json" =>
        {
            Ok(windows::stop_windows(inputs, bulk(which), true)?)
        }
        [command, action, id] if command == "window" && action == "stop" => {
            Ok(windows::stop_window(inputs, id, false)?)
        }
        [command, action, id, flag]
            if command == "window" && action == "stop" && flag == "--json" =>
        {
            Ok(windows::stop_window(inputs, id, true)?)
        }
        [command, child @ ..] if command == "anima" => {
            super::diagnostics::bench_trace("exec", "anima");
            let error = Command::new(&inputs.anima)
                .args(child)
                .env("YAZELIX_SCREEN_COMMAND_NAME", "eon anima")
                .exec();
            Err(format!("cannot launch Anima: {error}"))
        }
        [command, rest @ ..] if command == "run" => {
            let (options, child) = match parse_launch_options(rest)? {
                (options, []) => (options, &[][..]),
                (options, [separator, child @ ..]) if separator == "--" && !child.is_empty() => {
                    (options, child)
                }
                _ => return Err(Error::usage(EON_USAGE)),
            };
            launch_current(
                inputs,
                LaunchMode::Workspace,
                child,
                false,
                WORKSPACE_DECORATIONS,
                WORKSPACE_APPLICATION_ID,
                &options,
            )
        }
        [command, ..] if control_command(command) => {
            let (arguments, target) = parse_workspace_target_arguments(&arguments)?;
            if let Some(send) = parse_pane_send_arguments(&arguments)? {
                return send_terminal(inputs, send, target);
            }
            if let Some(show) = parse_popup_show_arguments(&arguments)? {
                return Ok(show_popup(inputs, show, target)?);
            }
            if let Some((popup, json)) = parse_popup_hide_arguments(&arguments)? {
                return Ok(hide_popup(inputs, popup, json, target)?);
            }
            if let Some((verb, json)) = parse_sidebar_arguments(&arguments)? {
                return Ok(sidebar(inputs, verb, json, target)?);
            }
            if let Some(json) = parse_keys_arguments(&arguments)? {
                return Ok(keys(inputs, json, target)?);
            }
            if let Some((action, json, yes)) = parse_close_arguments(&arguments)? {
                return Ok(match action {
                    Action::CloseColumn { pane, .. } => {
                        close_column(inputs, pane, json, yes, target)?
                    }
                    action => control(inputs, action, json, false, target, yes)?,
                });
            }
            let (action, json) = parse_control_arguments(&arguments)?;
            control(
                inputs,
                action,
                json,
                lists_sessions(&arguments),
                target,
                false,
            )
        }
        [flag] if flag == "--skill" => {
            write_stdout(inputs.skill)?;
            Ok(0)
        }
        [flag] if flag == "-V" || flag == "--version" => {
            write_stdout(format!("eon {}\n", inputs.version))?;
            Ok(0)
        }
        [command] if command == "versions" => {
            write_stdout(versions(inputs)?)?;
            Ok(0)
        }
        [command, action] if command == "config" && action == "path" => print_config_path(true),
        [command, action] if command == "config" && action == "dir" => print_config_path(false),
        [command, action, flag]
            if command == "config" && action == "schema" && flag == "--json" =>
        {
            write_stdout(managed_environment::print_schema(&inputs.defaults)?)?;
            Ok(0)
        }
        [command, action] if command == "config" && action == "check" => {
            check_configuration(inputs, false)
        }
        [command, action, flag] if command == "config" && action == "check" && flag == "--json" => {
            check_configuration(inputs, true)
        }
        [command, action] if command == "config" && action == "defaults" => {
            print_config_defaults(inputs, false)
        }
        [command, action, flag]
            if command == "config" && action == "defaults" && flag == "--json" =>
        {
            print_config_defaults(inputs, true)
        }
        _ => return Err(Error::usage(EON_USAGE)),
    };
    Ok(result?)
}

/// `eon versions`'s output: Eon's version and generation, EONW's version and the components'.
pub(super) fn versions(inputs: &Inputs) -> Result<String, String> {
    Ok(format!(
        "eon {} {}\neonw {}\n{}\n",
        inputs.version,
        current_generation(inputs)?,
        VERSION,
        inputs
            .composed_components
            .as_deref()
            .map_err(Clone::clone)?
    ))
}

fn registry_command(arguments: &[OsString]) -> Result<i32, Error> {
    use super::session_registry::{Failure as RegistryFailure, Registry, validate_id};

    let (mut id, mut json) = (None, false);
    for argument in arguments {
        match argument.to_str() {
            Some("--json") if !json => json = true,
            Some(value) if id.is_none() && !value.starts_with('-') => {
                validate_id(value).map_err(|error| Error::usage(error.to_string()))?;
                id = Some(value);
            }
            _ => return Err(Error::usage(EON_USAGE)),
        }
    }
    let records = Registry::new()
        .map_err(RegistryFailure::into_control)
        .and_then(|registry| {
            if let Some(id) = id {
                registry
                    .lookup(id)
                    .map_err(RegistryFailure::into_control)?
                    .map(|record| vec![record])
                    .ok_or_else(|| {
                        failure(
                            "unknown-session",
                            format!("session registration {id} is missing"),
                        )
                    })
            } else {
                registry.list().map_err(RegistryFailure::into_control)
            }
        });
    let records = match records {
        Ok(records) => records,
        Err(error) => return Ok(report_failure(&error, json)?),
    };
    write_stdout(if json {
        let mut values: Vec<_> = records
            .iter()
            .map(|record| serde_json::json!({"persistent_id": record.id, "name": record.name}))
            .collect();
        let value = if id.is_some() {
            values.remove(0)
        } else {
            serde_json::Value::Array(values)
        };
        format!("{value}\n")
    } else {
        records
            .iter()
            .map(|record| {
                format!(
                    "persistent_id={} name={}\n",
                    record.id,
                    record
                        .name
                        .as_ref()
                        .map_or_else(|| "none".into(), |name| format!("{name:?}")),
                )
            })
            .collect()
    })?;
    Ok(0)
}

/// Whether control arguments that parsed as an inspection are `session ls`, with `--json`
/// anywhere after `session`.
fn lists_sessions(arguments: &[OsString]) -> bool {
    arguments
        .iter()
        .filter(|argument| *argument != "--json")
        .eq(["session", "ls"])
}

/// The usage failure for a removed workspace command, naming the session command that
/// replaced it. It is a message, not an alias: the command still fails.
fn renamed_command(arguments: &[OsString]) -> Option<Error> {
    let new = match arguments.first()?.to_str()? {
        "workspace" => "session",
        "workspaces" => "session ls",
        _ => return None,
    };
    let old = arguments[0].to_string_lossy();
    Some(Error::usage(format!(
        "invalid command or arguments\n`eon {old}` is now `eon {new}`\nRun `eon --help` for commands and examples."
    )))
}

/// The first words `execute` hands to `parse_control_arguments`.
fn control_command(command: &OsStr) -> bool {
    [
        "session", "tab", "pane", "column", "popup", "sidebar", "edit", "files", "focus", "keys",
        "lock", "unlock", "--window",
    ]
    .iter()
    .any(|word| command == *word)
}

/// Creates the configuration root and prints its settings file, or without `file` the root.
fn print_config_path(file: bool) -> Result<i32, String> {
    let root = configuration_directory()?;
    prepare_configuration(&root)?;
    let path = if file {
        root.join(managed_environment::CONFIG_FILE)
    } else {
        root
    };
    write_stdout(format!("{}\n", path.display()))?;
    Ok(0)
}

fn check_configuration(inputs: &Inputs, json: bool) -> Result<i32, String> {
    let (path, result) = managed_environment::check_configuration(&inputs.defaults);
    let valid = result.is_ok();
    if json {
        let error = result
            .err()
            .map(|(code, detail)| serde_json::json!({"code": code, "detail": detail}));
        let output = serde_json::json!({
            "path": path.as_ref().ok().map(|path| path.to_string_lossy()),
            "valid": valid,
            "error": error,
        });
        write_stdout(format!("{output}\n"))?;
    } else {
        result.map_err(|(_, detail)| detail)?;
        write_stdout(format!("Valid Eon configuration: {}\n", path?.display()))?;
    }
    Ok(if valid { 0 } else { 1 })
}

fn print_config_defaults(inputs: &Inputs, json: bool) -> Result<i32, String> {
    let output = managed_environment::print_defaults(&inputs.defaults, json)?;
    std::io::stdout()
        .lock()
        .write_all(output.as_bytes())
        .map_err(|error| format!("cannot write stdout: {error}"))?;
    Ok(0)
}

/// `eon --help`'s command rows by section, as (command, description). A row whose command is
/// empty or starts with spaces continues the row above it.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Start",
        &[
            ("eon [--dir DIR] [--no-anima]", "Open or present the window"),
            ("run [--dir DIR] [--no-anima]", "Start a fresh window"),
            ("  [-- COMMAND...]", "with an exact first command"),
            ("  --dir DIR", "First tab in DIR: no quick search,"),
            ("", "no startup animation"),
            ("  --no-anima", "Skip the startup animation"),
            (
                "tutor [list|N|next|--reset]",
                "Print lessons and save progress",
            ),
            ("  [--json]", "Or tutor --help for details"),
        ],
    ),
    #[cfg(target_os = "linux")]
    (
        "Agents",
        &[("agent --help", "Choose and run coding agents")],
    ),
    (
        "Windows",
        &[
            ("quit [--all] [--json]", "End this window (or every window)"),
            ("window ls [--json]", "Every running window: ID, name,"),
            ("", "version, started, terminals,"),
            ("", "compatible or not"),
            (
                "window new [--dir DIR] [--no-anima]",
                "Open an independent window",
            ),
            ("window attach [ID]", "Reopen a window (no ID: default)"),
            (
                "window reload ID [--json]",
                "Reread config and replace Venus",
            ),
            ("", "in a focused window"),
            (
                "window stop ID|previous|all [--json]",
                "End a window, the old-version",
            ),
            ("", "ones, or all"),
            ("keys [--json]", "Show effective shortcuts for"),
            ("", "this window"),
            ("lock [--json]", "Pass shortcuts to the terminal"),
            ("unlock [--json]", "Restore this window's shortcuts"),
        ],
    ),
    (
        "Sessions",
        &[
            ("session ls [--json]", "List this window's sessions"),
            ("session [--json]", "Inspect the session"),
            ("session --id ID [--json]", "Inspect an exact window member"),
            (
                "session registry [ID] [--json]",
                "Read this user's registrations",
            ),
            ("events [--json]", "Subscribe to this window's events"),
            (
                "  --session SESSION --tab TAB --target ID",
                "Repeatable filters (AND across,",
            ),
            ("  --kind KIND --state STATE", "OR within each flag)"),
            ("  --after STREAM:SEQ", "Resume, or take a fresh snapshot"),
            (
                "session new [--json] [--no-focus]",
                "Add a session and show it",
            ),
            ("session switch SESSION [--json]", "Show session SESSION"),
            (
                "session rename [SESSION] NAME [--json]",
                "Name a session; empty clears",
            ),
            (
                "session close SESSION [--json]",
                "Stop its terminals and remove it",
            ),
        ],
    ),
    (
        "Tabs & panes",
        &[
            ("tab create [--json]", "Choose a directory for a new tab"),
            ("tab create --dir DIR", "Start a tab without quick search"),
            (
                "  [--json] [--no-focus] [-- COMMAND...]",
                "COMMAND needs an Eon terminal",
            ),
            (
                "tab close TAB [--yes] [--json]",
                "Close any tab of this session",
            ),
            (
                "tab directory TAB [--json] -- DIRECTORY",
                "Set the tab's launch directory",
            ),
            ("tab switch TAB [--json]", "Make TAB the active tab"),
            ("tab move left|right [--json]", "Move the active tab"),
            (
                "pane create [--json] [--no-focus]",
                "Add a pane to the active tab",
            ),
            (
                "pane create [--json] -- COMMAND...",
                "Run COMMAND in a new pane",
            ),
            (
                "pane create --tab TAB ...",
                "Add the pane to tab TAB instead",
            ),
            ("edit [--json] -- FILE", "Open FILE in Editor or a pane"),
            (
                "files reveal PATH [--tab TAB] [--json]",
                "Show PATH in the tab's Files",
            ),
            ("", "Uses the caller's tab, or the"),
            ("", "active tab outside Eon."),
            ("", "Use -- before an option-like path."),
            (
                "pane close PANE [--yes] [--json]",
                "Close any pane of this session",
            ),
            (
                "column close PANE --yes [--json]",
                "Close the column (Stack) that",
            ),
            ("", "holds a pane; without --yes, list"),
            ("", "what it would stop"),
            (
                "pane rename PANE NAME [--tag TAG] [--json]",
                "Pin a name and optionally tag",
            ),
            (
                "pane rename PANE --clear [--json]",
                "Restore automatic pane naming",
            ),
            (
                "pane move up|down|left|right [--json]",
                "Move the selected pane",
            ),
            (
                "pane read PANE|POPUP [--json]",
                "Print a pane's or popup's text",
            ),
            (
                "pane history PANE|POPUP [--json] [--edit]",
                "Print a pane's or popup's history",
            ),
            (
                "pane send PANE|POPUP|focused [--enter]",
                "Paste TEXT, then Enter if asked",
            ),
            (
                "  [--force] [--json] [--stdin | -- TEXT]",
                "TEXT up to 64 KiB, or Enter alone",
            ),
            (
                "popup show ENTRY [--tab TAB] [--json]",
                "Start a popup in a tab",
            ),
            (
                "popup hide POPUP [--json]",
                "Hide a popup as its window would",
            ),
            (
                "sidebar show|hide|toggle [--json]",
                "Show or hide the agent sidebar",
            ),
            (
                "focus ID|left|right|up|down [--json]",
                "Focus a pane, popup or navigate",
            ),
        ],
    ),
    (
        "Tools & information",
        &[
            ("anima [STYLE] [CHILD OPTIONS...]", "Play an animation here"),
            ("versions", "Show exact component versions"),
            ("doctor [--json]", "Check the local Eon setup"),
            ("report", "Print a bug report to paste"),
            (
                "update --check [--json]",
                "Check this install for an update",
            ),
            (
                "update --check --profile ABS --entry NAME",
                "Check that profile entry",
            ),
            (
                "update --check --prefix ABS",
                "Check that managed Linux prefix",
            ),
            (
                "update --install --prefix ABS [--yes]",
                "Install a signed Linux release",
            ),
            (
                "update --install [--yes] [--json]",
                "Update one profile entry or prefix",
            ),
            ("config path", "Create the config root and print"),
            ("", "its config.toml path"),
            ("config dir", "Create and print the config root"),
            ("config defaults [--json]", "Print config.toml defaults"),
            ("config schema --json", "Describe configuration keys"),
            ("config check [--json]", "Check config.toml offline"),
            ("--skill", "Print this build's agent skill"),
            ("-V, --version", "Print this build's Eon version"),
            ("-h, --help", "Show this help; after a command,"),
            ("", "only that command's lines"),
        ],
    ),
];

/// Answers `-h` or `--help` alone or after one or two command words: `eon FAMILY --help` and
/// `eon FAMILY COMMAND --help` print the rows of `eon --help` whose command starts with those
/// words, and words no row starts with are invalid. Operands, as in `run -- --help`, and
/// Anima's own help are not help here.
fn command_help(version: &str, arguments: &[OsString]) -> Option<Result<i32, Error>> {
    let (flag, words) = arguments.split_last()?;
    // Command words are lowercase, as `window`; `--`, IDs and the `-h,` row are not.
    let command_word = |word: &OsString| {
        word.to_str().is_some_and(|word| {
            word.starts_with(|first: char| first.is_ascii_lowercase())
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        })
    };
    if !(flag == "-h" || flag == "--help")
        || words.len() > 2
        || words.first().is_some_and(|word| word == "anima")
        || !words.iter().all(command_word)
    {
        return None;
    }
    let color = std::io::stdout().is_terminal()
        && env::var_os("TERM").as_deref() != Some(OsStr::new("dumb"))
        && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let help = match words {
        [] => Some(help_text(version, color)),
        // The bare `eon` row is not a family.
        [family, ..] if family == "eon" => None,
        _ => command_help_text(words, color),
    };
    Some(match help {
        Some(help) => write_stdout(help).map(|()| 0).map_err(Error::from),
        None => Err(Error::usage(EON_USAGE)),
    })
}

fn help_colors(color: bool) -> (&'static str, &'static str, &'static str) {
    if color {
        ("\x1b[1;35m", "\x1b[36m", "\x1b[0m")
    } else {
        ("", "", "")
    }
}

fn help_rows<'a>(rows: impl IntoIterator<Item = &'a (&'a str, &'a str)>, color: bool) -> String {
    let (_, command, reset) = help_colors(color);
    rows.into_iter()
        .map(|(text, description)| {
            let words = text.trim_start();
            let indent = &text[..text.len() - words.len()];
            // A row longer than the column keeps one space before its description.
            let padding = 44usize.saturating_sub(text.len()).max(1);
            if words.is_empty() {
                format!("  {:padding$}{description}\n", "")
            } else {
                format!(
                    "  {indent}{command}{words}{reset}{:padding$}{description}\n",
                    ""
                )
            }
        })
        .collect()
}

/// The rows whose command starts with `words`, with their continuations, under their
/// sections' headings, or `None` when no row does.
fn command_help_text(words: &[OsString], color: bool) -> Option<String> {
    let (heading, _, reset) = help_colors(color);
    let mut help = String::new();
    for (title, rows) in HELP_SECTIONS {
        let mut selected = false;
        let rows = help_rows(
            rows.iter().filter(|(text, _)| {
                if !text.is_empty() && !text.starts_with(' ') {
                    let mut row = text.split(' ');
                    selected = words
                        .iter()
                        .all(|word| row.next().is_some_and(|row| word == row));
                }
                selected
            }),
            color,
        );
        if !rows.is_empty() {
            help += &format!("{heading}{title}{reset}\n{rows}\n");
        }
    }
    (!help.is_empty()).then(|| {
        if words
            .first()
            .is_some_and(|word| word == "popup" || word == "focus")
        {
            help += POPUP_HELP;
        }
        help + "Run `eon --help` for the window flag, notes and exit status.\n"
    })
}

const POPUP_HELP: &str = "popup show starts an entry only when its tab has no instance of it.
Use focus uN for an existing popup and popup hide uN to dismiss it.
--tab keeps the active tab and shown session; omission uses the active tab.
Popup focus selects its tab; neither command moves desktop focus.
Neither command replaces a transient popup; hide it explicitly when asked.
Scrollback show/focus obeys [agents] terminal_access, even without a caller.
";

fn help_text(version: &str, color: bool) -> String {
    let (heading, command, reset) = help_colors(color);
    let sections: String = HELP_SECTIONS
        .iter()
        .map(|(title, rows)| format!("\n{heading}{title}{reset}\n{}", help_rows(*rows, color)))
        .collect();
    let examples = help_rows(
        &[
            ("eon run -- eon-nu -c 'print hello'", "Start with a command"),
            ("eon window ls --json", "Inspect windows as JSON"),
            ("eon anima --help", "Browse animation styles/options"),
        ],
        color,
    );
    format!(
        "{heading}Eon{reset} {version}  ·  Terminal sessions. Persistent terminals.

{heading}Usage{reset}  {command}eon [COMMAND]{reset}
{sections}
{heading}Examples{reset}
{examples}
Alt+Shift+D detaches a window; closing it or eon quit ends it.
Inside Eon, session commands use the calling terminal's supervisor/session.
Pane creation and moves, and tab moves, retain their calling-tab rules.
Window IDs, as eon window ls shows them: default, a generation's hex ID,
an independent window's ID, or any unique prefix of 4+ characters.
Window flag: --window ID, before any --.
default uses this build's running window, otherwise the only running window
in the default namespace; if several older windows run, choose an ID.
Explicit targets still validate EON_TERMINAL; independent windows use
attach's rule.
To address another window, clear EON_TERMINAL and EON_TAB or run outside Eon.
Session commands, events and reload use the window's build when EONW differs.
Results name generation, created_build, active_build, window (null for default),
and session.
SESSION is a session ID (s1, s2, ...) or its exact name.
Inspect/switch/rename/close accept --id ID instead of a SESSION target.
ID is the full persistent_id, scoped to the selected window.
session registry reads metadata without a window; it does not show liveness.
session new --name NAME names the new session; created gives its local ID,
created_persistent_id its durable identity. Uncertain creation needs inspection,
not an automatic retry.
With --no-focus, session new keeps the shown session, and either
pane create keeps the selected pane and any popup.
pane create --tab TAB adds the pane to TAB; the active tab and shown
session stay as they are.
pane read, history and send reach hidden sessions; [agents]
terminal_access governs all three. pane send --enter waits
[agents] send_enter_delay_ms first.
edit uses the packaged Helix Editor popup by default; set [editor]
open_in = \"pane\" for a new pane.
pane history --edit opens the history in a Scrollback editor popup,
only in the shown session's active tab.
{POPUP_HELP}\
pane send focused explicitly targets the person's keyboard-focused terminal.
It refuses unknown or unfocused windows and missing terminal focus.
--force cannot bypass these focus checks.
With focused --enter, Enter requires the same focused terminal after the delay.
pane send refuses while a person is typing; --force bypasses the typing check.
Where shown, {command}--json{reset} prints JSON and never asks.
Busy tab/pane closes require {command}--yes{reset} with {command}--json{reset};
a column close always requires {command}--yes{reset} and reads no stdin.
Tab/pane/column close IDs resolve in the caller's session,
otherwise the shown one.
Tab/pane confirmation without a terminal needs yes or exits 1 (not-confirmed).
Exit status: 0 success, 1 failed, 2 invalid command, 3 target not found,
4 Eon unreachable, 5 person typing (nothing sent), 130 interrupted.
Agent? Run {command}eon --skill{reset} to learn how to work in Eon.
"
    )
}

fn execute_eonbase(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, Error> {
    if let Some(code) = lifecycle_command(inputs, &arguments)? {
        return Ok(code);
    }
    let result = match arguments.as_slice() {
        [separator, child @ ..] if separator == "--" && !child.is_empty() => launch_current(
            inputs,
            LaunchMode::Terminal,
            child,
            true,
            true,
            "eonbase",
            &LaunchOptions::default(),
        ),
        [flag, separator, child @ ..]
            if flag == "--no-decorations" && separator == "--" && !child.is_empty() =>
        {
            launch_current(
                inputs,
                LaunchMode::Terminal,
                child,
                true,
                false,
                "eonbase",
                &LaunchOptions::default(),
            )
        }
        [flag, application_id, separator, child @ ..]
            if flag == "--application-id" && separator == "--" && !child.is_empty() =>
        {
            launch_current(
                inputs,
                LaunchMode::Terminal,
                child,
                true,
                true,
                application_id_argument(application_id)?,
                &LaunchOptions::default(),
            )
        }
        [decorations, flag, application_id, separator, child @ ..]
            if decorations == "--no-decorations"
                && flag == "--application-id"
                && separator == "--"
                && !child.is_empty() =>
        {
            launch_current(
                inputs,
                LaunchMode::Terminal,
                child,
                true,
                false,
                application_id_argument(application_id)?,
                &LaunchOptions::default(),
            )
        }
        _ => return Err(Error::usage(EONBASE_USAGE)),
    };
    Ok(result?)
}

fn application_id_argument(argument: &OsStr) -> Result<&str, Error> {
    let value = argument
        .to_str()
        .ok_or_else(|| Error::usage("Eon application identities must be UTF-8"))?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(Error::usage(format!(
            "invalid Eon application identity {value:?}"
        )));
    }
    Ok(value)
}

/// `eonbase attach [GENERATION]`, `generations [--json]` and
/// `stop GENERATION [--json]` (EON-C12); `eon` has none of them.
fn lifecycle_command(inputs: &Inputs, arguments: &[OsString]) -> Result<Option<i32>, Error> {
    let root = runtime_directory("eonbase");
    Ok(Some(match arguments {
        [command] if command == "attach" => attach_generation(inputs, &root, None, None)?,
        [command, generation] if command == "attach" => {
            attach_generation(inputs, &root, Some(generation_argument(generation)?), None)?
        }
        [command] if command == "generations" => generations_command(inputs, false, "eonbase")?,
        [command, flag] if command == "generations" && flag == "--json" => {
            generations_command(inputs, true, "eonbase")?
        }
        [command, generation] if command == "stop" => stop_generation(
            inputs,
            &root,
            generation_argument(generation)?,
            false,
            &Ask::Stop,
        )?,
        [command, generation, flag] | [command, flag, generation]
            if command == "stop" && flag == "--json" =>
        {
            stop_generation(
                inputs,
                &root,
                generation_argument(generation)?,
                true,
                &Ask::Stop,
            )?
        }
        _ => return Ok(None),
    }))
}

fn generation_argument(argument: &OsStr) -> Result<&str, Error> {
    let generation = argument
        .to_str()
        .ok_or_else(|| Error::usage("Eon generation identities must be UTF-8"))?;
    if !valid_generation(generation) {
        return Err(Error::usage(format!(
            "invalid Eon generation identity {generation:?}"
        )));
    }
    Ok(generation)
}

fn control(
    inputs: &Inputs,
    mut action: Action,
    json: bool,
    list: bool,
    target: WorkspaceTarget,
    yes: bool,
) -> Result<i32, String> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return report_failure(&error, json),
    };
    if let Err(error) = validate_action(caller.clone(), action.clone()) {
        return report_failure(&failure("malformed-action", error.detail), json);
    }
    let resolved = match resolve_workspace(inputs, target, caller.as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => return report_failure(&error, json),
    };
    let generation = &resolved.generation;
    let socket = &resolved.socket;
    let mut confirmed_connection = None;
    // Every tab or pane close inspects first, even with --json or --yes, and
    // asks about its own busy or unknown terminals and a final close (EON-C10).
    if let Some(target_label) = close_target_label(&action) {
        let snapshot = match inspect(socket, caller.as_deref(), &resolved.label) {
            Ok(snapshot) => snapshot,
            Err(error) => return report_failure(&error, json),
        };
        let has_caller = caller.is_some();
        let mut question = match close_question(&snapshot, &action, has_caller, false) {
            Ok(question) => question,
            Err(error) => return report_failure(&error, json),
        };
        // The idle final warning asks only a person at a terminal, and the
        // setting is read only when it changes the question.
        if !yes && !json && std::io::stdin().is_terminal() {
            let warned = match close_question(&snapshot, &action, has_caller, true) {
                Ok(warned) => warned,
                Err(error) => return report_failure(&error, json),
            };
            if warned != question
                && managed_environment::warning_close_last_tab(
                    &configuration_directory()?,
                    &inputs.defaults,
                    None,
                )?
            {
                question = warned;
            }
        }
        if let Some((question, cancelled)) = question
            && !yes
        {
            if json {
                return report_failure(
                    &failure(
                        "not-confirmed",
                        format!("close of {target_label} requires confirmation; rerun with --yes"),
                    ),
                    true,
                );
            }
            if let Some(status) = confirm(&question, &cancelled)? {
                return Ok(status);
            }
        }
    }
    // Confirm the resolved member, then send its exact persistent identity.
    if let Action::Workspace(WorkspaceAction::CloseWorkspace { workspace }) = &mut action
        && !json
    {
        let endpoint = super::control::socket_identity(socket)?;
        let listed = match inspect(socket, caller.as_deref(), &resolved.label) {
            Ok(snapshot) => snapshot.workspaces,
            Err(error) => return report_failure(&error, json),
        };
        let target = match resolve_target(
            listed.iter().map(|item| {
                (
                    item.id.as_str(),
                    item.name.as_deref(),
                    item.persistent_id.as_deref(),
                )
            }),
            workspace,
        ) {
            Ok(index) => &listed[index],
            Err(error) => return report_failure(&error, json),
        };
        let id = &target.id;
        let (question, cancelled) =
            session_close_prompt(id, target.name.as_deref(), listed.len() == 1);
        if let Some(status) = confirm(&question, &cancelled)? {
            return Ok(status);
        }
        if endpoint.is_none() || super::control::socket_identity(socket)? != endpoint {
            return report_failure(
                &failure("unknown-session", "supervisor changed during confirmation"),
                json,
            );
        }
        let confirmed =
            match inspect(socket, caller.as_deref(), &resolved.label).and_then(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .find_map(|item| confirmed_close_target(target, item))
                    .ok_or_else(|| failure("unknown-session", format!("session {id} is not live")))
            }) {
                Ok(target) => target,
                Err(error) => return report_failure(&error, json),
            };
        *workspace = confirmed;
        let stream = match super::control::connect_control(socket) {
            Ok(stream) => stream,
            Err(error) => {
                return report_failure(&unreachable_supervisor(error, &resolved.label), json);
            }
        };
        if super::control::socket_identity(socket)? != endpoint {
            return report_failure(
                &failure("unknown-session", "supervisor changed during confirmation"),
                json,
            );
        }
        confirmed_connection = Some(stream);
    }
    let editing = matches!(action, Action::Edit { .. });
    let creation = match action {
        Action::Workspace(WorkspaceAction::NewWorkspace { .. }) => Some(Creation::Workspace),
        Action::Workspace(WorkspaceAction::CreateTab) | Action::LaunchTab { .. } => {
            Some(Creation::Tab)
        }
        Action::CreatePane { .. } | Action::Edit { .. } => Some(Creation::Pane),
        _ => None,
    };
    let explicit_inspection = matches!(
        action,
        Action::Workspace(WorkspaceAction::InspectWorkspace { .. })
    );
    let inspecting = !list
        && (explicit_inspection || matches!(action, Action::Workspace(WorkspaceAction::Inspect)));
    // Opening Editor can stop a transient caller before the reply names its tab.
    let edited_tab = if editing && let Some(session) = caller.as_deref().and_then(terminal_session)
    {
        let before = match inspect(socket, caller.as_deref(), &resolved.label) {
            Ok(snapshot) => snapshot,
            Err(error) => return report_failure(&error, json),
        };
        let Some(tab) = before.tabs.iter().find(|tab| {
            tab.panes().any(|pane| pane.session == session)
                || tab.popups.iter().any(|popup| popup.session == session)
        }) else {
            return report_failure(
                &failure(
                    "unknown-caller",
                    format!("terminal {session} is not in window {}", resolved.label),
                ),
                json,
            );
        };
        Some(tab.id.clone())
    } else {
        None
    };
    let response = match match confirmed_connection {
        Some(stream) => super::control::send_action_on(stream, caller.clone(), action),
        None => send_action_from(Ok(socket), caller.clone(), action),
    } {
        Ok(response) => response,
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            return report_failure(&failure("malformed-action", error.detail), json);
        }
        Err(error) => {
            return report_failure(&unreachable_supervisor(error, &resolved.label), json);
        }
    };
    let window = window_json(resolved.window.as_deref());
    match response {
        ControlResponse::Workspace(Response::Snapshot(snapshot)) => {
            let edit_popup = editing
                && snapshot.tabs.iter().any(|tab| {
                    let target = tab.id == edited_tab.as_deref().unwrap_or(&snapshot.active_tab);
                    target
                        && tab.popups.iter().any(|popup| {
                            popup.entry == "editor"
                                && tab.selected_popup.as_deref() == Some(&popup.id)
                        })
                });
            let member = snapshot
                .workspaces
                .iter()
                .find(|item| item.id == snapshot.workspace)
                .expect("a snapshot projects a live session");
            let persistent_id = member.persistent_id.as_deref().unwrap_or("null");
            let caller = if inspecting {
                // Explicit inspection describes the target, not the caller's session.
                let caller_snapshot = if explicit_inspection && caller.is_some() {
                    match inspect(socket, caller.as_deref(), &resolved.label) {
                        Ok(snapshot) => Some(snapshot),
                        Err(error) => return report_failure(&error, json),
                    }
                } else {
                    None
                };
                caller_output(
                    caller_snapshot.as_ref().unwrap_or(&snapshot),
                    caller.as_deref().and_then(terminal_session),
                    json,
                )
            } else {
                String::new()
            };
            // The reply projects the caller's workspace, where a tab or pane
            // creation acts. sN, tN and pN identities rise window-wide and are
            // never reused, and the supervisor builds the reply in the same
            // request step that creates one, so the highest is the new one.
            let terminal = |tab: &Tab, pane: Option<&Pane>| {
                format!(
                    "{{\"tab\":\"{}\",\"pane\":{}}}",
                    json_escape(&tab.id),
                    pane.map_or_else(
                        || "null".into(),
                        |pane| format!("\"{}\"", json_escape(&pane.id))
                    )
                )
            };
            // The JSON value, and the human line only a new workspace prints.
            // A pending tab has no pane until its directory is picked.
            let created_workspace = match creation {
                Some(Creation::Workspace) => newest(&snapshot.workspaces, 's', |item| &item.id),
                _ => None,
            };
            let created_id = match created_workspace.map(created_persistent_id).transpose() {
                Ok(id) => id,
                Err(error) => return report_failure(&error, json),
            };
            let (created, created_line) = match creation {
                Some(Creation::Workspace) => created_workspace
                    .zip(created_id)
                    .map(|(item, persistent_id)| {
                        (
                            format!("\"{}\"", json_escape(&item.id)),
                            format!(
                                "created {}\ncreated_persistent_id {}\n",
                                item.id, persistent_id
                            ),
                        )
                    })
                    .unzip(),
                Some(Creation::Tab) => (
                    newest(&snapshot.tabs, 't', |tab| &tab.id)
                        .map(|tab| terminal(tab, newest(tab.panes(), 'p', |pane| &pane.id))),
                    None,
                ),
                Some(Creation::Pane) if edit_popup => (None, None),
                Some(Creation::Pane) => (
                    newest(
                        snapshot
                            .tabs
                            .iter()
                            .flat_map(|tab| tab.panes().map(move |pane| (tab, pane))),
                        'p',
                        |(_, pane)| &pane.id,
                    )
                    .map(|(tab, pane)| terminal(tab, Some(pane))),
                    None,
                ),
                None => (None, None),
            };
            write_stdout(if json {
                format!(
                    "{{\"generation\":\"{}\",\"window\":{window},\"session\":\"{}\",{},{caller}{}{}{}",
                    json_escape(generation),
                    json_escape(&snapshot.workspace),
                    registration_json(member),
                    created.map_or_else(String::new, |value| format!("\"created\":{value},")),
                    created_id.map_or_else(String::new, |id| format!(
                        "\"created_persistent_id\":\"{}\",",
                        json_escape(id)
                    )),
                    &if list {
                        list_json(&snapshot)
                    } else {
                        json_output(&snapshot)
                    }[1..]
                )
            } else {
                format!(
                    "generation {generation}\ncreated_build {}\nactive_build {}\nwindow {}\nsession {}\n{caller}{}{}{}",
                    snapshot.created_build,
                    snapshot.active_build,
                    resolved.window.as_deref().unwrap_or("default"),
                    snapshot.workspace,
                    if list {
                        String::new()
                    } else {
                        format!(
                            "persistent_id {persistent_id}\n{}",
                            member.registration_error.as_ref().map_or_else(
                                String::new,
                                |error| format!(
                                    "registration_error {}: {}\n",
                                    error.code,
                                    super::workspace::terminal_text(&error.detail)
                                )
                            )
                        )
                    },
                    created_line.unwrap_or_default(),
                    if list {
                        list_human(&snapshot)
                    } else {
                        human_output(&snapshot)
                    }
                )
            })?;
            Ok(0)
        }
        ControlResponse::Workspace(Response::Failure(failure)) => report_failure(&failure, json),
        ControlResponse::Workspace(Response::Ended(ended)) => {
            write_stdout(if json {
                format!(
                    "{{\"ended\":{{\"generation\":\"{}\"}},\"generation\":\"{}\",\"window\":{window}}}\n",
                    json_escape(&ended.generation),
                    json_escape(generation),
                )
            } else {
                format!("Eon window {} closed\n", resolved.label)
            })?;
            Ok(0)
        }
        ControlResponse::Lifecycle(_) => {
            Err("Eon supervisor returned a lifecycle result for a session action".into())
        }
        ControlResponse::Workspace(Response::Text(read)) => {
            write_stdout(if json {
                format!(
                    "{{\"generation\":\"{}\",\"window\":{window},{}",
                    json_escape(generation),
                    &terminal_text_json(&read)[1..]
                )
            } else {
                format!("{}\n", read.text)
            })?;
            Ok(0)
        }
        ControlResponse::Workspace(Response::History(history)) => {
            write_stdout(if json {
                format!(
                    "{{\"generation\":\"{}\",\"window\":{window},{}",
                    json_escape(generation),
                    &terminal_history_json(&history)[1..]
                )
            } else {
                format!("{}\n", history.text)
            })?;
            if let Some(notice) = history_notice(&history).filter(|_| !json) {
                eprintln!("{notice}");
            }
            Ok(0)
        }
        // `eon pane history --edit` prints nothing but its JSON (EON-C35).
        ControlResponse::Workspace(Response::Editing(editing)) => {
            if json {
                write_stdout(format!(
                    "{{\"generation\":\"{}\",\"window\":{window},{}",
                    json_escape(generation),
                    &terminal_editing_json(&editing)[1..]
                ))?;
            }
            Ok(0)
        }
        // A second scope means consent no longer covers the current close.
        ControlResponse::Workspace(Response::Confirm(scope)) => {
            let CloseStep::Refuse(error) = column_close_step(&scope, 1, yes) else {
                unreachable!()
            };
            report_column_refusal(&scope, &error, json)
        }
        // Only `eon pane send` asks for this, through `send_terminal`.
        ControlResponse::Workspace(Response::Sent(_)) => {
            Err("Eon supervisor returned a terminal result for a session action".into())
        }
    }
}

/// What the 8 MiB bound left out of `eon pane history`'s text (EON-C35),
/// when it left out anything.
fn history_notice(history: &TerminalHistory) -> Option<String> {
    history_left_out(history.omitted_lines, history.cut_bytes).map(|left_out| {
        format!(
            "eon: the history is over its {} MiB bound; left out {left_out}",
            MAX_HISTORY_TEXT_BYTES / (1024 * 1024)
        )
    })
}

/// The earliest lines and the newest line's start that ORB-C17's history
/// bound left out, in words, when it left out anything.
pub(crate) fn history_left_out(omitted_lines: u32, cut_bytes: u32) -> Option<String> {
    let lines = match omitted_lines {
        0 => None,
        1 => Some("the earliest line".to_owned()),
        lines => Some(format!("the {lines} earliest lines")),
    };
    let cut = (cut_bytes > 0).then(|| format!("the first {cut_bytes} bytes of the newest line"));
    match (lines, cut) {
        (None, None) => None,
        (Some(part), None) | (None, Some(part)) => Some(part),
        (Some(lines), Some(cut)) => Some(format!("{lines} and {cut}")),
    }
}

/// What a workspace action's reply names in `created`.
#[derive(Clone, Copy)]
enum Creation {
    Workspace,
    Tab,
    Pane,
}

fn created_persistent_id(workspace: &WorkspaceInfo) -> Result<&str, Failure> {
    workspace.persistent_id.as_deref().ok_or_else(|| {
        failure(
            "unrepresentable-state",
            "new session reply has no confirmed persistent ID; inspect before retrying",
        )
    })
}

fn confirmed_close_target(
    previous: &WorkspaceInfo,
    current: &WorkspaceInfo,
) -> Option<SessionTarget> {
    if previous.id != current.id
        || previous
            .persistent_id
            .as_ref()
            .is_some_and(|id| current.persistent_id.as_ref() != Some(id))
    {
        return None;
    }
    Some(current.persistent_id.as_ref().map_or_else(
        || SessionTarget::Selector(current.id.clone()),
        |id| SessionTarget::PersistentId(id.clone()),
    ))
}

/// The item with the highest `prefix`N identity, which is the newest: Runtime
/// allocates sN, tN and pN rising window-wide and never reuses one.
fn newest<T>(
    items: impl IntoIterator<Item = T>,
    prefix: char,
    id: impl Fn(&T) -> &String,
) -> Option<T> {
    items.into_iter().max_by_key(|item| {
        id(item)
            .strip_prefix(prefix)
            .and_then(|n| n.parse::<u64>().ok())
    })
}

/// A result's `window`: its ID, or null for the default namespace.
fn window_json(window: Option<&str>) -> String {
    window.map_or_else(|| "null".into(), |id| format!("\"{}\"", json_escape(id)))
}

/// `eon pane send` (EON-C36): checks TEXT before anything is sent, pastes it,
/// then waits the window's Enter delay and sends Enter when asked. Empty TEXT
/// counts as none.
fn send_terminal(inputs: &Inputs, send: PaneSend, target: WorkspaceTarget) -> Result<i32, Error> {
    let json = send.json;
    let action = |input, expected_terminal| Action::SendTerminal {
        target: if send.target == "focused" {
            eon_workspace_protocol::v50::SendTarget::Focused { expected_terminal }
        } else {
            eon_workspace_protocol::v50::SendTarget::Id(send.target.clone())
        },
        input,
        force: send.force,
    };
    let resolve = || {
        let caller = calling_terminal()?;
        validate_action(caller.clone(), action(TerminalInput::Enter, None))
            .map_err(|error| failure("malformed-action", error.detail))?;
        let resolved = resolve_workspace(inputs, target.clone(), caller.as_deref())?;
        Ok::<_, Failure>((caller, resolved))
    };
    // Standard input belongs to the build that a routed send runs as, so the
    // target is known before it is read (EON-C11, Routing).
    let mut resolved_first = None;
    let text = match send.text {
        SendText::None if stdin_holds_data() => {
            let detail = "standard input holds text, so nothing was sent; add --stdin to send \
                          that text, or give empty input to press Enter alone: </dev/null, or \
                          null | eon ... in Nushell";
            return Ok(report_failure(&failure("unread-stdin", detail), json)?);
        }
        SendText::None => Vec::new(),
        SendText::Operand(text) => text.into_vec(),
        SendText::Stdin => {
            match resolve() {
                Ok(resolved) => resolved_first = Some(resolved),
                Err(error) => return Ok(report_failure(&error, json)?),
            }
            let mut text = Vec::new();
            std::io::stdin()
                .lock()
                .take(MAX_PASTE_BYTES as u64 + 1)
                .read_to_end(&mut text)
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            text
        }
    };
    if text.is_empty() && !send.enter {
        return Err(Error::usage(
            "eon pane send needs TEXT or --enter\nRun `eon --help` for commands and examples.",
        ));
    }
    let text = if text.is_empty() {
        None
    } else if text.len() > MAX_PASTE_BYTES {
        let detail =
            format!("TEXT is over the {MAX_PASTE_BYTES}-byte limit; send a file's path instead");
        return Ok(report_failure(&failure("input-too-large", detail), json)?);
    } else {
        match String::from_utf8(text) {
            Ok(text) if validate_paste(&text).is_ok() => Some(text),
            _ => {
                let detail = "TEXT must be UTF-8 without control characters other than tab, \
                              line feed and carriage return";
                return Ok(report_failure(&failure("invalid-text", detail), json)?);
            }
        }
    };
    let (caller, resolved) = match resolved_first.map_or_else(resolve, Ok) {
        Ok(resolved) => resolved,
        Err(error) => return Ok(report_failure(&error, json)?),
    };
    let request = |input, expected| match send_action_from(
        Ok(&resolved.socket),
        caller.clone(),
        action(input, expected),
    ) {
        Ok(ControlResponse::Workspace(Response::Sent(sent))) => Ok(sent),
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => Err(failure),
        Ok(_) => Err(failure(
            "unrepresentable-state",
            "Eon returned the wrong result for a terminal send",
        )),
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            Err(failure("malformed-action", error.detail))
        }
        Err(error) => Err(unreachable_supervisor(error, &resolved.label)),
    };
    // No TEXT means --enter alone. The Enter goes after the window's delay, so
    // the program takes the whole paste first; once the paste is in, an Enter
    // failure leaves it there.
    let bytes = text.as_ref().map_or(0, String::len);
    let (sent, mut enter_failure) = match text {
        None => match request(TerminalInput::Enter, None) {
            Ok(sent) => (sent, None),
            Err(failure) => return Ok(report_failure(&failure, json)?),
        },
        Some(text) => match request(TerminalInput::Paste(text), None) {
            Err(failure) => return Ok(report_failure(&failure, json)?),
            Ok(pasted) if send.enter => {
                thread::sleep(Duration::from_millis(pasted.enter_delay_ms.into()));
                let enter_failure =
                    request(TerminalInput::Enter, Some(pasted.target.terminal.clone())).err();
                (pasted, enter_failure)
            }
            Ok(pasted) => (pasted, None),
        },
    };
    if let Some(failure) = &mut enter_failure
        && failure.code == "person-typing"
    {
        failure.detail = "a person is typing in the target terminal; wait for a quiet interval, \
                          then send --enter alone without resending the text"
            .into();
    }
    let enter = send.enter && enter_failure.is_none();
    if json {
        write_stdout(format!(
            "{{\"generation\":\"{}\",\"window\":{},{}",
            json_escape(&resolved.generation),
            window_json(resolved.window.as_deref()),
            &terminal_sent_json(&sent, bytes, enter, enter_failure.as_ref())[1..]
        ))?;
    } else if let Some(failure) = &enter_failure {
        eprintln!(
            "{}the text was pasted; Enter was not sent",
            failure_human(failure)
        );
    }
    Ok(if enter_failure.is_some() { 1 } else { 0 })
}

/// How long `pane send` without TEXT gives an open, empty pipe on standard input to receive
/// bytes. A writer such as `echo X |` starts beside Eon and usually has written before Eon looks;
/// under load average 22 its bytes took at most 11 ms to arrive, and an open pipe that stays
/// empty, as agents' shells give commands, costs this wait once.
// Fixed, not configured: eon-zuidu records why (rule 6), the measurement and the residual gap.
const STDIN_DATA_WAIT_MS: libc::c_int = 100;

/// Whether standard input already holds bytes `pane send` would ignore without `--stdin`: a
/// regular file with bytes past its offset, or a pipe or socket with bytes waiting, or arriving
/// within `STDIN_DATA_WAIT_MS`. Terminals, devices such as /dev/null, and ended inputs are not
/// read or waited on.
fn stdin_holds_data() -> bool {
    let input = std::io::stdin();
    let Ok(status) = rustix::fs::fstat(&input) else {
        return false;
    };
    match rustix::fs::FileType::from_raw_mode(status.st_mode) {
        rustix::fs::FileType::RegularFile => {
            rustix::fs::seek(&input, rustix::fs::SeekFrom::Current(0))
                .is_ok_and(|offset| u64::try_from(status.st_size).is_ok_and(|size| size > offset))
        }
        rustix::fs::FileType::Fifo | rustix::fs::FileType::Socket => {
            let mut ready = [rustix::event::PollFd::new(
                &input,
                rustix::event::PollFlags::IN,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: i64::from(STDIN_DATA_WAIT_MS / 1000),
                tv_nsec: i64::from(STDIN_DATA_WAIT_MS % 1000) * 1_000_000,
            };
            // The result only bounds the wait; bytes may arrive before it ends.
            let _ = rustix::event::poll(&mut ready, Some(&timeout));
            rustix::io::ioctl_fionread(&input).is_ok_and(|waiting| waiting > 0)
        }
        _ => false,
    }
}

/// A supervisor that stopped answering, in window `window` (its full ID).
fn unreachable_supervisor(error: EndpointFailure, window: &str) -> Failure {
    let dead = error.kind == EndpointFailureKind::Dead;
    if env::var_os(TERMINAL_VARIABLE).is_none() {
        let code = if dead {
            "missing-supervisor"
        } else {
            "supervisor-unavailable"
        };
        return failure(
            code,
            format!(
                "Eon window {window}: {}; run `eon window ls` to inspect running windows",
                error.detail
            ),
        );
    }
    failure(
        "supervisor-unavailable",
        format!(
            "Eon window {window}: {}; use the terminal's own `eon` from its PATH",
            error.detail
        ),
    )
}

/// How a question, cancel note or refusal names a tab or pane close's target,
/// and `None` for any other action.
fn close_target_label(action: &Action) -> Option<String> {
    match action {
        Action::Workspace(WorkspaceAction::CloseTab { tab }) => Some(format!("tab {tab}")),
        Action::ClosePane { pane } => Some(format!("pane {pane}")),
        _ => None,
    }
}

/// The question and cancel note a tab or pane close asks, from the latest
/// inspection, or `None` when its terminals are known idle and no final
/// warning applies. It fails as Runtime would for a target that is not live
/// in the inspected workspace or that a covering popup protects; Runtime
/// still enforces both after the question (EON-C10).
fn close_question(
    snapshot: &Snapshot,
    action: &Action,
    has_caller: bool,
    warn_final: bool,
) -> Result<Option<(String, String)>, Failure> {
    let Some(target_label) = close_target_label(action) else {
        return Ok(None);
    };
    let tab_terminals = |tab: &eon_workspace_protocol::v50::Tab| -> Vec<(String, String)> {
        tab.panes()
            .map(|pane| (pane.session.clone(), pane.label.clone()))
            .chain(tab.popups.iter().map(|popup| {
                let label = snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.id == popup.entry)
                    .map_or_else(|| popup.entry.clone(), |entry| entry.label.clone());
                (popup.session.clone(), label)
            }))
            .collect()
    };
    let (tab, affected, is_final) = match action {
        Action::Workspace(WorkspaceAction::CloseTab { tab: id }) => {
            let tab = snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == *id)
                .ok_or_else(|| failure("unknown-tab", format!("tab {id} is not live")))?;
            (tab, tab_terminals(tab), snapshot.tabs.len() == 1)
        }
        Action::ClosePane { pane: id } => {
            let (tab, pane) = snapshot
                .tabs
                .iter()
                .find_map(|tab| {
                    tab.panes()
                        .find(|pane| pane.id == *id)
                        .map(|pane| (tab, pane))
                })
                .ok_or_else(|| failure("unknown-pane", format!("pane {id} is not live")))?;
            let last_pane = tab.panes().count() == 1;
            if tab.selected_popup.is_some() {
                if !has_caller {
                    return Err(failure(
                        "unavailable",
                        "close the popup before closing a pane",
                    ));
                }
                if last_pane {
                    return Err(failure(
                        "unavailable",
                        "close the popup before closing the tab's last pane",
                    ));
                }
            }
            // The final tab's last pane closes as that tab does, popups included.
            let is_final = snapshot.tabs.len() == 1 && last_pane;
            let affected = if is_final {
                tab_terminals(tab)
            } else {
                vec![(pane.session.clone(), pane.label.clone())]
            };
            (tab, affected, is_final)
        }
        _ => return Ok(None),
    };
    // A missing or capped busy list cannot show the target idle.
    let busy = match &snapshot.busy {
        Some(busy) if busy.omitted == 0 => {
            let running = busy
                .terminals
                .iter()
                .filter(|item| {
                    affected
                        .iter()
                        .any(|(session, _)| *session == item.target.terminal)
                })
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>();
            (!running.is_empty())
                .then(|| format!("{} still running: {}", running.len(), running.join(", ")))
        }
        _ => Some(format!(
            "busy state unknown: {}",
            affected
                .iter()
                .map(|(_, label)| label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    };
    if is_final && (busy.is_some() || warn_final) {
        // The close removes the workspace the reply projects while others remain (EON-C33).
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == snapshot.workspace)
            .filter(|_| snapshot.workspaces.len() >= 2)
            .map(|workspace| workspace.name.as_deref().unwrap_or(&workspace.id));
        let terminals = tab
            .panes()
            .map(|pane| pane.label.clone())
            .chain(tab.popups.iter().map(|popup| popup.entry.clone()))
            .collect::<Vec<_>>();
        return Ok(Some(final_close_prompt(
            &terminals,
            workspace,
            busy.as_deref(),
        )));
    }
    Ok(busy.map(|busy| {
        let count = affected.len();
        (
            format!(
                "Close {target_label}? This stops {count} terminal{}; {busy}. [y/N] ",
                if count == 1 { "" } else { "s" }
            ),
            format!("cancelled; {target_label} stays open\n"),
        )
    }))
}

/// Formats a user-visible label (such as a session name) for a single-line terminal prompt using
/// pinned Unicode 18.0.0 tables (independent of `std`'s compiler-versioned `escape_debug`):
///
/// - Always escapes backslash and quotes (`\`, `"`, `'`), every control and invisible format
///   control rejected by `pane_label_character` (C0/C1/DEL controls, soft hyphen, Arabic/Syriac/
///   Kaithi/Egyptian/Shorthand/Musical format controls including `U+1BCA0..=U+1BCA3`, bidi
///   controls/isolates, zero-width spaces/joiners, BOM, interlinear annotations, tags), non-ASCII
///   Unicode `White_Space` (including `U+2028`/`U+2029` line/paragraph separators and NBSP),
///   additional default-ignorable, private-use, or noncharacter scalars (`U+034F`, Hangul fillers
///   `U+115F..=U+1160`/`U+3164`/`U+FFA0`, Mongolian free variation selectors
///   `U+180B..=U+180D`/`U+180F`, non-emoji variation selectors `U+FE00..=U+FE0D`, default-ignorable
///   ranges `U+FFF0..=U+FFF8` and `U+E0000..=U+E0FFF`, private-use `Co`, and Unicode
///   noncharacters), and every unassigned (`Cn`) scalar in Unicode 18.0.0 wherever it appears.
/// - Escapes combining marks and extenders across scripts (pinned Unicode 18.0.0
///   `Grapheme_Extend` — all `Mn`, `Me`, extending `Mc`, emoji presentation selectors
///   `U+FE0E`/`U+FE0F`, and halfwidth Katakana sound marks `U+FF9E..=U+FF9F`, excluding the
///   always-escaped invisibles above) when they have no visible non-space base before them (at
///   start of text, after `' '`, or after an escaped character); keeps them raw after a visible
///   base so decomposed names (`Cafe\u{0301}`, `\u{0915}\u{093C}`), Japanese voiced marks, and
///   emoji presentation selectors stay readable.
/// - Leaves other assigned visible Unicode 18.0.0 text (precomposed and decomposed names, CJK,
///   Hangul, emoji) unchanged.
fn prompt_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut has_base = false;
    for ch in text.chars() {
        if is_prompt_always_escaped(ch) {
            push_escaped_char(&mut out, ch);
            has_base = false;
        } else if is_prompt_combining_mark(ch) {
            if has_base {
                out.push(ch);
            } else {
                push_escaped_char(&mut out, ch);
            }
        } else {
            out.push(ch);
            has_base = ch != ' ';
        }
    }
    out
}

fn is_prompt_always_escaped(ch: char) -> bool {
    matches!(ch, '\\' | '"' | '\'')
        || !pane_label_character(ch)
        || is_prompt_non_ascii_whitespace(ch)
        || is_prompt_extra_invisible(ch)
        || is_unicode_18_unassigned(ch)
}

fn is_prompt_non_ascii_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

fn is_prompt_extra_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{034F}'
            | '\u{115F}'..='\u{1160}'
            | '\u{180B}'..='\u{180D}'
            | '\u{180F}'
            | '\u{3164}'
            | '\u{E000}'..='\u{F8FF}'
            | '\u{FDD0}'..='\u{FDEF}'
            | '\u{FE00}'..='\u{FE0D}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{E0000}'..='\u{E0FFF}'
            | '\u{F0000}'..='\u{FFFFD}'
            | '\u{100000}'..='\u{10FFFD}'
    ) || (ch as u32 & 0xFFFE) == 0xFFFE
}

fn is_unicode_18_unassigned(ch: char) -> bool {
    let code = ch as u32;
    match code {
        0x0000..=0x0377 => false,
        0x0378..=0x3FFFD => in_codepoint_ranges(code, UNICODE_18_0_UNASSIGNED_PLANES_0_3),
        0xE0001
        | 0xE0020..=0xE007F
        | 0xE0100..=0xE01EF
        | 0xF0000..=0xFFFFD
        | 0x100000..=0x10FFFD => false,
        _ => true,
    }
}

fn is_prompt_combining_mark(ch: char) -> bool {
    let code = ch as u32;
    code >= 0x0300 && in_codepoint_ranges(code, UNICODE_18_0_GRAPHEME_EXTEND)
}

fn in_codepoint_ranges(code: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .binary_search_by(|&(lo, hi)| {
            if code < lo {
                Ordering::Greater
            } else if code > hi {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '\0' => out.push_str(r"\0"),
        '\t' => out.push_str(r"\t"),
        '\r' => out.push_str(r"\r"),
        '\n' => out.push_str(r"\n"),
        '\\' => out.push_str(r"\\"),
        '"' => out.push_str(r#"\""#),
        '\'' => out.push_str(r"\'"),
        _ => {
            let _ = write!(out, "\\u{{{:x}}}", ch as u32);
        }
    }
}

/// The `eon session close` question and its cancel note, keeping decomposed session names
/// readable while escaping characters that could spoof the prompt.
fn session_close_prompt(id: &str, name: Option<&str>, quits_window: bool) -> (String, String) {
    let label = name.map_or_else(String::new, |name| format!(" ({})", prompt_text(name)));
    let quits = if quits_window {
        "quits the window and "
    } else {
        ""
    };
    (
        format!("Close session {id}{label}? This {quits}stops every terminal in it. [y/N] "),
        format!("cancelled; session {id} stays open\n"),
    )
}

/// The final-close question and its cancel note: the window's, or EON-C33's
/// wording for the workspace the close removes while others remain, with the
/// busy clause of a busy or unknown final close.
fn final_close_prompt(
    terminals: &[String],
    workspace: Option<&str>,
    busy: Option<&str>,
) -> (String, String) {
    let ends = workspace.map_or_else(
        || "quits the window".into(),
        |name| format!("closes session {}", prompt_text(name)),
    );
    let busy = busy.map_or_else(String::new, |busy| format!("; {busy}"));
    let question = format!(
        "Close the last tab? This {ends} and stops {} terminal{} [{}]{busy}. [y/N] ",
        terminals.len(),
        if terminals.len() == 1 { "" } else { "s" },
        terminals.join(", ")
    );
    let cancelled = workspace.map_or_else(
        || "cancelled; the window stays open\n".into(),
        |name| format!("cancelled; session {} stays open\n", prompt_text(name)),
    );
    (question, cancelled)
}

fn show_popup(inputs: &Inputs, show: PopupShow, target: WorkspaceTarget) -> Result<i32, String> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return report_failure(&error, show.json),
    };
    let snapshot = match resolve_workspace(inputs, target.clone(), caller.as_deref())
        .and_then(|resolved| inspect(&resolved.socket, caller.as_deref(), &resolved.label))
    {
        Ok(snapshot) => snapshot,
        Err(error) => return report_failure(&error, show.json),
    };
    let tab = show.tab.as_deref().unwrap_or(&snapshot.active_tab);
    if snapshot.entries.iter().any(|item| item.id == show.entry)
        && let Some(popup) = snapshot
            .tabs
            .iter()
            .find(|item| item.id == tab)
            .and_then(|tab| entry_popup(tab, &show.entry))
    {
        return report_failure(
            &failure(
                "unavailable",
                format!(
                    "popup entry {} is already live as {1}; use eon focus {1}",
                    show.entry, popup.id,
                ),
            ),
            show.json,
        );
    }
    control(
        inputs,
        show.action(&snapshot.active_tab),
        show.json,
        false,
        target,
        false,
    )
}

/// `eon popup hide` (EON-C10): dismisses popup `uN` as its owning window would,
/// reading the owning tab from the workspace first.
fn hide_popup(
    inputs: &Inputs,
    popup: String,
    json: bool,
    target: WorkspaceTarget,
) -> Result<i32, String> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return report_failure(&error, json),
    };
    let snapshot = match resolve_workspace(inputs, target.clone(), caller.as_deref())
        .and_then(|resolved| inspect(&resolved.socket, caller.as_deref(), &resolved.label))
    {
        Ok(snapshot) => snapshot,
        Err(error) => return report_failure(&error, json),
    };
    let Some(tab) = snapshot
        .tabs
        .iter()
        .find(|tab| tab.popups.iter().any(|item| item.id == popup))
    else {
        let detail = format!("popup {} is not live in this session", popup.escape_debug());
        return report_failure(&failure("unknown-popup", detail), json);
    };
    let action = Action::DismissPopup(PopupTarget {
        tab: tab.id.clone(),
        instance: popup,
    });
    control(inputs, action, json, false, target, false)
}

/// `eon sidebar`'s verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarVerb {
    Show,
    Hide,
    Toggle,
}

/// State-naming commands set the Runtime-owned value in one request (EON-C10).
fn sidebar(
    inputs: &Inputs,
    verb: SidebarVerb,
    json: bool,
    target: WorkspaceTarget,
) -> Result<i32, String> {
    let action = match verb {
        SidebarVerb::Show => Action::SetSidebar { open: true },
        SidebarVerb::Hide => Action::SetSidebar { open: false },
        SidebarVerb::Toggle => Action::ToggleSidebar,
    };
    control(inputs, action, json, false, target, false)
}

/// Inspects the addressed window and projects its effective keymap (`EON-C10`).
fn keys(inputs: &Inputs, json: bool, target: WorkspaceTarget) -> Result<i32, String> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return report_failure(&error, json),
    };
    let resolved = match resolve_workspace(inputs, target, caller.as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => return report_failure(&error, json),
    };
    let snapshot = match inspect(&resolved.socket, caller.as_deref(), &resolved.label) {
        Ok(snapshot) => snapshot,
        Err(error) => return report_failure(&error, json),
    };
    let number_keys = match configuration_directory()
        .map_err(|detail| ("unreadable-configuration", detail))
        .and_then(|root| {
            managed_environment::read_number_keys(
                &root.join(managed_environment::CONFIG_FILE),
                &inputs.defaults,
            )
        }) {
        Ok(enabled) => enabled,
        Err((code, detail)) => return report_failure(&failure(code, detail), json),
    };
    let keymap = eon_workspace_protocol::shortcuts::effective_keymap(
        eon_workspace_protocol::shortcuts::Platform::CURRENT,
        &snapshot.entries,
        snapshot.sidebar,
        snapshot.running.is_some(),
        number_keys,
        snapshot.released_shortcuts,
        (&snapshot.input_lock, &inputs.defaults.popups.input_lock),
    );
    let output = if json {
        keys_json(
            &resolved.generation,
            resolved.window.as_deref(),
            &keymap,
            snapshot.input_locked,
        )
    } else {
        keys_human(
            &resolved.generation,
            resolved.window.as_deref(),
            &keymap,
            snapshot.input_locked,
        )
    };
    write_stdout(output)?;
    Ok(0)
}

#[derive(Debug, PartialEq)]
enum CloseStep {
    Consent,
    Refuse(Failure),
}

fn column_close_step(scope: &CloseScope, round: usize, yes: bool) -> CloseStep {
    if round == 0 && yes {
        return CloseStep::Consent;
    }
    let (code, detail) = if round == 0 {
        (
            "not-confirmed",
            format!(
                "close of the column of pane {} requires confirmation; rerun with --yes",
                scope.pane
            ),
        )
    } else {
        (
            "close-changed",
            format!(
                "what closing the column of pane {} stops changed before it ran; nothing was closed; rerun",
                scope.pane
            ),
        )
    };
    CloseStep::Refuse(failure(code, detail))
}

fn close_column(
    inputs: &Inputs,
    pane: String,
    json: bool,
    yes: bool,
    target: WorkspaceTarget,
) -> Result<i32, String> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return report_failure(&error, json),
    };
    let action = Action::CloseColumn {
        pane: pane.clone(),
        consent: None,
    };
    if let Err(error) = validate_action(caller.clone(), action.clone()) {
        return report_failure(&failure("malformed-action", error.detail), json);
    }
    let resolved = match resolve_workspace(inputs, target.clone(), caller.as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => return report_failure(&error, json),
    };
    let response = match send_action_from(Ok(&resolved.socket), caller, action) {
        Ok(response) => response,
        Err(error) => return report_failure(&unreachable_supervisor(error, &resolved.label), json),
    };
    match response {
        ControlResponse::Workspace(Response::Confirm(scope)) => {
            match column_close_step(&scope, 0, yes) {
                CloseStep::Consent => control(
                    inputs,
                    Action::CloseColumn {
                        pane,
                        consent: Some(CloseConsent {
                            generation: scope.generation,
                            runs: scope
                                .terminals
                                .into_iter()
                                .map(|terminal| terminal.run)
                                .chain(scope.untracked.into_iter().map(|run| run.run))
                                .collect(),
                            end: scope.end,
                        }),
                    },
                    json,
                    false,
                    target,
                    true,
                ),
                CloseStep::Refuse(error) => report_column_refusal(&scope, &error, json),
            }
        }
        ControlResponse::Workspace(Response::Failure(error)) => report_failure(&error, json),
        _ => Err("Eon returned the wrong result for column close scope".into()),
    }
}

fn column_close_human(scope: &CloseScope, changed: bool) -> String {
    let count = scope.terminals.len() + scope.untracked.len();
    let consequence = match scope.end {
        CloseEnd::Column => "stop".into(),
        CloseEnd::Session => format!("close session {} and stop", scope.session),
        CloseEnd::Window => "quit the window and stop".into(),
    };
    let mut output = format!(
        "{}Closing the column of pane {} would {}{consequence} {count} terminal{}:\n",
        if changed {
            "The column changed before it was closed. "
        } else {
            ""
        },
        scope.pane,
        if changed { "now " } else { "" },
        if count == 1 { "" } else { "s" }
    );
    for terminal in &scope.terminals {
        let (kind, id) = match &terminal.target.id {
            TargetId::Pane(id) => ("pane", id),
            TargetId::Popup(id) => ("popup", id),
        };
        output.push_str(&format!(
            "  {kind} {id}    {}{}\n",
            terminal.name,
            if terminal.busy { "    running" } else { "" }
        ));
    }
    for run in &scope.untracked {
        output.push_str(&format!("  {}    in no tab    running\n", run.terminal));
    }
    output.push_str("Nothing was closed.\n");
    output
}

fn column_close_json(scope: &CloseScope, error: &Failure) -> serde_json::Value {
    use serde_json::json;
    let terminals: Vec<_> = scope
        .terminals
        .iter()
        .map(|terminal| {
            let (pane, popup) = match &terminal.target.id {
                TargetId::Pane(id) => (Some(id), None),
                TargetId::Popup(id) => (None, Some(id)),
            };
            json!({ "session": terminal.target.workspace, "tab": terminal.target.tab,
            "pane": pane, "popup": popup, "terminal": terminal.target.terminal,
            "run": terminal.run, "name": terminal.name, "busy": terminal.busy })
        })
        .collect();
    let untracked: Vec<_> = scope
        .untracked
        .iter()
        .map(|run| json!({ "terminal": run.terminal, "run": run.run }))
        .collect();
    json!({ "error": { "code": error.code, "detail": error.detail }, "close": {
        "generation": scope.generation, "pane": scope.pane, "end": match scope.end {
            CloseEnd::Column => "column", CloseEnd::Session => "session", CloseEnd::Window => "window"
        }, "session": scope.session, "terminals": terminals, "untracked": untracked } })
}

fn report_column_refusal(scope: &CloseScope, error: &Failure, json: bool) -> Result<i32, String> {
    if json {
        write_stdout(format!("{}\n", column_close_json(scope, error)))?;
        Ok(1)
    } else {
        eprint!(
            "{}",
            column_close_human(scope, error.code == "close-changed")
        );
        report_failure(error, false)
    }
}

/// The session `eon session` reports: the caller's, else the shown one.
fn inspect(socket: &Path, caller: Option<&str>, window: &str) -> Result<Snapshot, Failure> {
    match send_action_from(
        Ok(socket),
        caller.map(str::to_owned),
        Action::Workspace(WorkspaceAction::Inspect),
    ) {
        Ok(ControlResponse::Workspace(Response::Snapshot(snapshot))) => Ok(snapshot),
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => Err(failure),
        Ok(_) => Err(failure(
            "unrepresentable-state",
            "Eon returned the wrong result for session inspection",
        )),
        Err(error) => Err(unreachable_supervisor(error, window)),
    }
}

/// Workspace control always validates the exact terminal named by EON_TERMINAL.
pub(super) fn calling_terminal() -> Result<Option<String>, Failure> {
    let Some(caller) = env::var_os(TERMINAL_VARIABLE) else {
        return Ok(None);
    };
    caller
        .to_str()
        .filter(|caller| valid_caller(caller))
        .map(|caller| Some(caller.into()))
        .ok_or_else(|| {
            failure(
                "unknown-caller",
                format!("{TERMINAL_VARIABLE} does not name an Eon terminal"),
            )
        })
}

fn parse_workspace_target_arguments(
    arguments: &[OsString],
) -> Result<(Vec<OsString>, WorkspaceTarget), Error> {
    let mut target = WorkspaceTarget {
        command: arguments.to_vec(),
        ..WorkspaceTarget::default()
    };
    let mut values = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            values.push(argument.clone());
            values.extend(arguments.cloned());
            break;
        }
        // Named operands stay literal, even `--window`.
        if argument == "--name" || argument == "--dir" || argument == "--tag" {
            values.push(argument.clone());
            values.extend(arguments.next().cloned());
            continue;
        }
        if argument != "--window" {
            values.push(argument.clone());
            continue;
        }
        if target.window.is_some() {
            return Err(Error::usage(EON_USAGE));
        }
        let value = arguments.next().ok_or_else(|| Error::usage(EON_USAGE))?;
        target.window = Some(windows::window_id_argument(value)?.to_owned());
    }
    Ok((values, target))
}

/// `pane send TARGET [--enter] [--force] [--stdin] [--json] [-- TEXT]` (EON-C36).
#[derive(Debug, PartialEq)]
struct PaneSend {
    target: String,
    text: SendText,
    enter: bool,
    force: bool,
    json: bool,
}

#[derive(Debug, PartialEq)]
enum SendText {
    None,
    Operand(OsString),
    Stdin,
}

fn parse_pane_send_arguments(arguments: &[OsString]) -> Result<Option<PaneSend>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    if command != "pane" || operation != "send" {
        return Ok(None);
    }
    let (options, operands) = match rest.iter().position(|argument| argument == "--") {
        Some(separator) => (&rest[..separator], Some(&rest[separator + 1..])),
        None => (rest, None),
    };
    let mut target = None;
    let (mut enter, mut force, mut stdin, mut json) = (false, false, false, false);
    for option in options {
        let flag = match option.to_str() {
            Some("--enter") => &mut enter,
            Some("--force") => &mut force,
            Some("--stdin") => &mut stdin,
            Some("--json") => &mut json,
            Some(id) if target.is_none() && !id.starts_with('-') => {
                target = Some(id.to_owned());
                continue;
            }
            _ => return Err(Error::usage(EON_USAGE)),
        };
        if std::mem::replace(flag, true) {
            return Err(Error::usage(EON_USAGE));
        }
    }
    let text = match (operands, stdin) {
        (None, false) => SendText::None,
        (None, true) => SendText::Stdin,
        (Some([text]), false) => SendText::Operand(text.clone()),
        _ => return Err(Error::usage(EON_USAGE)),
    };
    Ok(Some(PaneSend {
        target: target.ok_or_else(|| Error::usage(EON_USAGE))?,
        text,
        enter,
        force,
        json,
    }))
}

/// An exact tab, pane or column close, with --yes and --json at most once.
fn parse_close_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool, bool)>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    let kind = match (command.to_str(), operation.to_str()) {
        (Some(kind @ ("tab" | "pane" | "column")), Some("close")) => kind,
        _ => return Ok(None),
    };
    let (mut json, mut yes) = (false, false);
    let mut target = None;
    for argument in rest {
        let argument = argument
            .to_str()
            .ok_or_else(|| Error::usage("Eon actions require UTF-8 arguments"))?;
        match argument {
            "--json" if !json => json = true,
            "--yes" if !yes => yes = true,
            id if target.is_none() && !id.starts_with('-') => target = Some(id),
            _ => return Err(Error::usage(EON_USAGE)),
        }
    }
    let Some(target) = target else {
        return Err(Error::usage(EON_USAGE));
    };
    let action = if kind == "tab" {
        Action::Workspace(WorkspaceAction::CloseTab { tab: target.into() })
    } else if kind == "column" {
        Action::CloseColumn {
            pane: target.into(),
            consent: None,
        }
    } else {
        Action::ClosePane {
            pane: target.into(),
        }
    };
    Ok(Some((action, json, yes)))
}

fn parse_control_arguments(arguments: &[OsString]) -> Result<(Action, bool), Error> {
    if arguments.first().is_some_and(|value| value == "pane")
        && arguments.get(1).is_some_and(|value| value == "rename")
    {
        return parse_pane_rename_arguments(&arguments[2..]);
    }
    if let Some(action) = parse_workspace_arguments(arguments)? {
        return Ok(action);
    }
    if let Some(action) = parse_tab_directory_arguments(arguments)? {
        return Ok(action);
    }
    if let Some(action) = parse_edit_arguments(arguments)? {
        return Ok(action);
    }
    if let [command, operation, rest @ ..] = arguments
        && command == "files"
        && operation == "reveal"
    {
        return parse_files_reveal_arguments(rest);
    }
    if let Some(action) = parse_pane_create_arguments(arguments)? {
        return Ok(action);
    }
    if let Some(action) = parse_tab_create_arguments(arguments)? {
        return Ok(action);
    }
    // `--no-focus` belongs to the pane, tab and session creation parsers above.
    let mut json = false;
    let mut values = Vec::new();
    for argument in arguments {
        let argument = argument
            .to_str()
            .ok_or_else(|| Error::usage("Eon actions require UTF-8 arguments"))?;
        if argument == "--json" {
            if json {
                return Err(Error::usage(EON_USAGE));
            }
            json = true;
        } else if argument == "--no-focus" {
            return Err(Error::usage(EON_USAGE));
        } else {
            values.push(argument);
        }
    }
    let action = match values.as_slice() {
        ["lock"] => Action::InputLock(InputLock::Lock),
        ["unlock"] => Action::InputLock(InputLock::Unlock),
        ["session"] | ["session", "ls"] => Action::Workspace(WorkspaceAction::Inspect),
        ["tab", "create"] => Action::Workspace(WorkspaceAction::CreateTab),
        ["tab", "switch", tab] if tab_number(tab).is_some() => {
            Action::Workspace(WorkspaceAction::FocusId((*tab).into()))
        }
        ["tab", "move", "left"] => Action::Workspace(WorkspaceAction::Move(Direction::Left)),
        ["tab", "move", "right"] => Action::Workspace(WorkspaceAction::Move(Direction::Right)),
        ["pane", "read", target] => Action::ReadTerminal {
            target: (*target).into(),
        },
        ["pane", "history", target] if *target != "--edit" => Action::ReadTerminalHistory {
            target: (*target).into(),
        },
        ["pane", "history", target, "--edit"] | ["pane", "history", "--edit", target]
            if *target != "--edit" =>
        {
            Action::EditTerminalHistory {
                target: (*target).into(),
            }
        }
        ["pane", "move", "up"] => Action::Workspace(WorkspaceAction::Move(Direction::Up)),
        ["pane", "move", "down"] => Action::Workspace(WorkspaceAction::Move(Direction::Down)),
        ["pane", "move", "left"] => Action::MovePaneSideways(Side::Left),
        ["pane", "move", "right"] => Action::MovePaneSideways(Side::Right),
        ["focus", "left"] => Action::Workspace(WorkspaceAction::Focus(Direction::Left)),
        ["focus", "right"] => Action::Workspace(WorkspaceAction::Focus(Direction::Right)),
        ["focus", "up"] => Action::Workspace(WorkspaceAction::Focus(Direction::Up)),
        ["focus", "down"] => Action::Workspace(WorkspaceAction::Focus(Direction::Down)),
        ["focus", tab] if tab_number(tab).is_some() => {
            return Err(Error::usage(format!(
                "eon focus takes a pane or popup; switch to tab {tab} with `eon tab switch {tab}`"
            )));
        }
        ["focus", id] => Action::Workspace(WorkspaceAction::FocusId((*id).into())),
        _ => return Err(Error::usage(EON_USAGE)),
    };
    Ok((action, json))
}

fn parse_pane_rename_arguments(arguments: &[OsString]) -> Result<(Action, bool), Error> {
    let mut json = false;
    let mut clear = false;
    let mut literal = false;
    let mut tag = None;
    let mut values = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let value = argument
            .to_str()
            .ok_or_else(|| Error::usage("pane rename requires UTF-8"))?;
        match value {
            "--" if !literal => literal = true,
            "--json" if !literal && !json => json = true,
            "--clear" if !literal && !clear => clear = true,
            "--tag" if !literal && tag.is_none() => {
                tag = Some(
                    arguments
                        .next()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| Error::usage("--tag requires a UTF-8 tag"))?
                        .trim(),
                );
            }
            value if !literal && value.starts_with('-') => return Err(Error::usage(EON_USAGE)),
            _ => values.push(value),
        }
    }
    let (pane, name) = match values.as_slice() {
        [pane] if clear && tag.is_none() => (*pane, None),
        [pane, name] if !clear => (
            *pane,
            Some(name.trim()).filter(|name| !name.is_empty() || tag.is_some()),
        ),
        _ => return Err(Error::usage(EON_USAGE)),
    };
    eon_workspace_protocol::v50::validate_pane_rename(name, tag)
        .map_err(|error| Error::usage(error.to_string()))?;
    Ok((
        Action::RenamePane {
            pane: pane.into(),
            name: name.map(str::to_owned),
            tag: tag.map(str::to_owned),
        },
        json,
    ))
}

/// `session new|switch|rename|close`. SESSION is a `sN` or a name, which the
/// runtime resolves; after `--`, operands may start with `-`. Names are
/// trimmed, as pane names are, and an empty rename clears the name.
fn parse_workspace_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let [command, arguments @ ..] = arguments else {
        return Ok(None);
    };
    if command != "session" {
        return Ok(None);
    }
    let (operation, rest) = match arguments {
        [operation, rest @ ..]
            if matches!(
                operation.to_str(),
                Some("new" | "switch" | "rename" | "close")
            ) =>
        {
            (operation.to_str().unwrap(), rest)
        }
        _ if arguments
            .iter()
            .take_while(|argument| *argument != "--")
            .any(|argument| argument == "--id") =>
        {
            ("inspect", arguments)
        }
        _ => return Ok(None),
    };
    let (mut json, mut focus, mut name, mut literal) = (false, true, None, false);
    let mut id = None;
    let mut values = Vec::new();
    let mut rest = rest.iter();
    while let Some(argument) = rest.next() {
        let value = argument
            .to_str()
            .ok_or_else(|| Error::usage("Eon session names must be UTF-8"))?;
        match value {
            "--" if !literal => literal = true,
            "--json" if !literal && !json => json = true,
            "--no-focus" if !literal && focus && operation == "new" => focus = false,
            "--name" if !literal && name.is_none() && operation == "new" => {
                name = Some(
                    rest.next()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| Error::usage(EON_USAGE))?
                        .trim(),
                );
            }
            "--id" if !literal && id.is_none() && operation != "new" => {
                let value = rest
                    .next()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| Error::usage(EON_USAGE))?;
                super::session_registry::validate_id(value)
                    .map_err(|error| Error::usage(error.to_string()))?;
                id = Some(SessionTarget::PersistentId(value.into()));
            }
            value if !literal && value.starts_with('-') => return Err(Error::usage(EON_USAGE)),
            _ => values.push(value),
        }
    }
    let action = match (operation, id, values.as_slice()) {
        ("inspect", Some(workspace), []) => WorkspaceAction::InspectWorkspace { workspace },
        ("new", None, []) => WorkspaceAction::NewWorkspace {
            name: name.map(str::to_owned),
            focus,
        },
        ("switch", Some(workspace), []) => WorkspaceAction::SwitchWorkspace { workspace },
        ("switch", None, [workspace]) => WorkspaceAction::SwitchWorkspace {
            workspace: SessionTarget::Selector((*workspace).into()),
        },
        ("rename", Some(workspace), [name]) => WorkspaceAction::RenameWorkspace {
            workspace: Some(workspace),
            name: name.trim().into(),
        },
        ("rename", None, [name]) => WorkspaceAction::RenameWorkspace {
            workspace: None,
            name: name.trim().into(),
        },
        ("rename", None, [workspace, name]) => WorkspaceAction::RenameWorkspace {
            workspace: Some(SessionTarget::Selector((*workspace).into())),
            name: name.trim().into(),
        },
        ("close", Some(workspace), []) => WorkspaceAction::CloseWorkspace { workspace },
        ("close", None, [workspace]) => WorkspaceAction::CloseWorkspace {
            workspace: SessionTarget::Selector((*workspace).into()),
        },
        _ => return Err(Error::usage(EON_USAGE)),
    };
    Ok(Some((Action::Workspace(action), json)))
}

fn parse_tab_directory_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let (tab, directory, json) = match arguments {
        [command, operation, tab, separator, directory]
            if command == "tab" && operation == "directory" && separator == "--" =>
        {
            (tab, directory, false)
        }
        [command, operation, tab, flag, separator, directory]
            if command == "tab"
                && operation == "directory"
                && flag == "--json"
                && separator == "--" =>
        {
            (tab, directory, true)
        }
        [command, operation, ..] if command == "tab" && operation == "directory" => {
            return Err(Error::usage(EON_USAGE));
        }
        _ => return Ok(None),
    };
    let tab = tab
        .to_str()
        .ok_or_else(|| Error::usage("Eon tab identities must be UTF-8"))?;
    Ok(Some((
        Action::Workspace(WorkspaceAction::SetTabDirectory {
            tab: tab.into(),
            directory: directory.as_os_str().as_bytes().to_vec(),
        }),
        json,
    )))
}

#[derive(Debug, PartialEq, Eq)]
struct PopupShow {
    entry: String,
    tab: Option<String>,
    json: bool,
}

impl PopupShow {
    fn action(&self, active_tab: &str) -> Action {
        Action::InvokePopup {
            tab: self.tab.as_deref().unwrap_or(active_tab).into(),
            entry: self.entry.clone(),
            expected_instance: None,
            intent: InvokeIntent::Show,
        }
    }
}

fn parse_popup_show_arguments(arguments: &[OsString]) -> Result<Option<PopupShow>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    if command != "popup" || operation != "show" {
        return Ok(None);
    }
    let (mut entry, mut tab, mut json) = (None, None, false);
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let word = word
            .to_str()
            .ok_or_else(|| Error::usage("Eon popup arguments must be UTF-8"))?;
        match word {
            "--json" if !json => json = true,
            "--tab" if tab.is_none() => {
                let value = words
                    .next()
                    .and_then(|word| word.to_str())
                    .filter(|value| tab_number(value).is_some())
                    .ok_or_else(|| Error::usage(EON_USAGE))?;
                tab = Some(value.into());
            }
            _ if !word.starts_with('-') && entry.is_none() => entry = Some(word.into()),
            _ => return Err(Error::usage(EON_USAGE)),
        }
    }
    let show = PopupShow {
        entry: entry.ok_or_else(|| Error::usage(EON_USAGE))?,
        tab,
        json,
    };
    validate_action(None, show.action("t1")).map_err(|error| Error::usage(error.detail))?;
    Ok(Some(show))
}

/// `popup hide POPUP [--json]`.
fn parse_popup_hide_arguments(arguments: &[OsString]) -> Result<Option<(String, bool)>, Error> {
    let (popup, json) = match arguments {
        [command, operation, popup] if command == "popup" && operation == "hide" => (popup, false),
        [command, operation, popup, flag]
            if command == "popup" && operation == "hide" && flag == "--json" =>
        {
            (popup, true)
        }
        [command, ..] if command == "popup" => return Err(Error::usage(EON_USAGE)),
        _ => return Ok(None),
    };
    let popup = popup
        .to_str()
        .ok_or_else(|| Error::usage("Eon popup identities must be UTF-8"))?;
    // An option where POPUP belongs, as in `popup hide --json`, is a usage error.
    if popup.starts_with('-') {
        return Err(Error::usage(EON_USAGE));
    }
    Ok(Some((popup.into(), json)))
}

/// `sidebar show|hide|toggle [--json]`.
fn parse_sidebar_arguments(arguments: &[OsString]) -> Result<Option<(SidebarVerb, bool)>, Error> {
    let (verb, json) = match arguments {
        [command, verb] if command == "sidebar" => (verb, false),
        [command, verb, flag] if command == "sidebar" && flag == "--json" => (verb, true),
        [command, ..] if command == "sidebar" => return Err(Error::usage(EON_USAGE)),
        _ => return Ok(None),
    };
    let verb = match verb.to_str() {
        Some("show") => SidebarVerb::Show,
        Some("hide") => SidebarVerb::Hide,
        Some("toggle") => SidebarVerb::Toggle,
        _ => return Err(Error::usage(EON_USAGE)),
    };
    Ok(Some((verb, json)))
}

/// `keys [--json]`.
fn parse_keys_arguments(arguments: &[OsString]) -> Result<Option<bool>, Error> {
    match arguments {
        [command] if command == "keys" => Ok(Some(false)),
        [command, flag] if command == "keys" && flag == "--json" => Ok(Some(true)),
        [command, ..] if command == "keys" => Err(Error::usage(EON_USAGE)),
        _ => Ok(None),
    }
}

/// `edit [--json] -- FILE`, with a relative FILE resolved against the
/// caller's working directory.
fn parse_edit_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let (file, json) = match arguments {
        [command, separator, file] if command == "edit" && separator == "--" => (file, false),
        [command, flag, separator, file]
            if command == "edit" && flag == "--json" && separator == "--" =>
        {
            (file, true)
        }
        [command, ..] if command == "edit" => return Err(Error::usage(EON_USAGE)),
        _ => return Ok(None),
    };
    if file.is_empty() {
        return Err(Error::usage("eon edit needs a file"));
    }
    let file = std::path::absolute(file)
        .map_err(|error| Error::from(format!("cannot resolve {file:?}: {error}")))?;
    Ok(Some((
        Action::Edit {
            file: file.into_os_string().into_vec(),
        },
        json,
    )))
}

fn parse_files_reveal_arguments(arguments: &[OsString]) -> Result<(Action, bool), Error> {
    let (mut path, mut tab, mut json) = (None, None, false);
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        match word.to_str() {
            Some("--json") if !json => json = true,
            Some("--tab") if tab.is_none() => {
                let value = words
                    .next()
                    .and_then(|word| word.to_str())
                    .filter(|value| tab_number(value).is_some())
                    .ok_or_else(|| Error::usage(EON_USAGE))?;
                tab = Some(value.into());
            }
            Some("--") if path.is_none() => {
                let [operand] = words.as_slice() else {
                    return Err(Error::usage(EON_USAGE));
                };
                path = Some(operand);
                break;
            }
            Some(value) if path.is_none() && !value.starts_with('-') => path = Some(word),
            None => return Err(Error::usage("eon files reveal requires a UTF-8 path")),
            _ => return Err(Error::usage(EON_USAGE)),
        }
    }
    let path = path
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::usage("eon files reveal needs a path"))?;
    let path = std::path::absolute(path)
        .map_err(|error| Error::from(format!("cannot resolve {path:?}: {error}")))?
        .into_os_string()
        .into_string()
        .map_err(|_| Error::usage("eon files reveal requires a UTF-8 path"))?;
    let action = Action::RevealFiles { tab, path };
    validate_action(None, action.clone()).map_err(|error| Error::usage(error.detail))?;
    Ok((action, json))
}

/// `pane create [--json] [--no-focus] [--tab TAB] [-- COMMAND [ARGS...]]`,
/// flags anywhere before `--`, each at most once; every argument after `--`
/// passes through unchanged as the new pane's argv.
fn parse_pane_create_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let separator = arguments.iter().position(|argument| argument == "--");
    let (head, argv) = match separator {
        Some(separator) => (&arguments[..separator], &arguments[separator + 1..]),
        None => (arguments, &[][..]),
    };
    let (mut words, mut json, mut unfocused, mut tabs) = (Vec::new(), 0, 0, Vec::new());
    let mut head = head.iter();
    while let Some(argument) = head.next() {
        match argument.to_str() {
            Some("--json") => json += 1,
            Some("--no-focus") => unfocused += 1,
            Some("--tab") => tabs.push(head.next().and_then(|value| value.to_str())),
            _ => words.push(argument),
        }
    }
    if words != ["pane", "create"] {
        return Ok(None);
    }
    let tab = match tabs.as_slice() {
        [] => None,
        [Some(tab)] if tab_number(tab).is_some() => Some((*tab).to_owned()),
        _ => return Err(Error::usage(EON_USAGE)),
    };
    if json > 1 || unfocused > 1 {
        return Err(Error::usage(EON_USAGE));
    }
    if separator.is_some() && argv.first().is_none_or(|program| program.is_empty()) {
        return Err(Error::usage("eon pane create -- needs a command"));
    }
    Ok(Some((
        Action::CreatePane {
            tab,
            argv: argv
                .iter()
                .map(|argument| argument.as_bytes().to_vec())
                .collect(),
            focus: unfocused == 0,
        },
        json == 1,
    )))
}

/// Directory-based tab creation leaves bare `tab create` on the pending pair.
fn parse_tab_create_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    if command != "tab" || operation != "create" {
        return Ok(None);
    }
    let (mut json, mut focus) = (false, true);
    let mut directory = None;
    let mut argv = Vec::new();
    let mut flags = rest.iter();
    while let Some(flag) = flags.next() {
        match flag.to_str() {
            Some("--json") if !json => json = true,
            Some("--no-focus") if focus => focus = false,
            Some("--dir") if directory.is_none() => {
                directory = Some(
                    flags
                        .next()
                        .filter(|value| !value.is_empty() && *value != "--")
                        .ok_or_else(|| Error::usage(EON_USAGE))?,
                );
            }
            Some("--") => {
                let command = flags.as_slice();
                if command.first().is_none_or(|program| program.is_empty()) {
                    return Err(Error::usage("eon tab create -- needs a command"));
                }
                argv = command
                    .iter()
                    .map(|argument| argument.as_bytes().to_vec())
                    .collect();
                break;
            }
            _ => return Err(Error::usage(EON_USAGE)),
        }
    }
    let Some(directory) = directory else {
        return if focus && argv.is_empty() {
            Ok(None)
        } else {
            Err(Error::usage(EON_USAGE))
        };
    };
    let directory = std::path::absolute(directory)
        .map_err(|error| Error::from(format!("cannot resolve {directory:?}: {error}")))?;
    Ok(Some((
        Action::LaunchTab {
            directory: directory.into_os_string().into_vec(),
            argv,
            focus,
        },
        json,
    )))
}

/// Pinned Unicode 18.0.0 `General_Category = Cn` (unassigned) ranges in planes 0..=3
/// (`U+0378..=U+3FFFD`, excluding UTF-16 surrogates).
#[rustfmt::skip]
const UNICODE_18_0_UNASSIGNED_PLANES_0_3: &[(u32, u32)] = &[
    (0x0378, 0x0379), (0x0380, 0x0383), (0x038B, 0x038B), (0x038D, 0x038D),
    (0x03A2, 0x03A2), (0x0530, 0x0530), (0x0557, 0x0557), (0x0590, 0x0590),
    (0x05CA, 0x05CF), (0x05EB, 0x05EE), (0x05F5, 0x05FF), (0x070E, 0x070E),
    (0x074B, 0x074C), (0x07B2, 0x07BF), (0x07FB, 0x07FC), (0x082E, 0x082F),
    (0x083F, 0x083F), (0x085C, 0x085D), (0x085F, 0x085F), (0x086B, 0x086F),
    (0x0892, 0x0896), (0x0984, 0x0984), (0x098D, 0x098E), (0x0991, 0x0992),
    (0x09A9, 0x09A9), (0x09B1, 0x09B1), (0x09B3, 0x09B5), (0x09BA, 0x09BB),
    (0x09C5, 0x09C6), (0x09C9, 0x09CA), (0x09CF, 0x09D6), (0x09D8, 0x09DB),
    (0x09DE, 0x09DE), (0x09E4, 0x09E5), (0x09FF, 0x0A00), (0x0A04, 0x0A04),
    (0x0A0B, 0x0A0E), (0x0A11, 0x0A12), (0x0A29, 0x0A29), (0x0A31, 0x0A31),
    (0x0A34, 0x0A34), (0x0A37, 0x0A37), (0x0A3A, 0x0A3B), (0x0A3D, 0x0A3D),
    (0x0A43, 0x0A46), (0x0A49, 0x0A4A), (0x0A4E, 0x0A50), (0x0A52, 0x0A58),
    (0x0A5D, 0x0A5D), (0x0A5F, 0x0A65), (0x0A77, 0x0A80), (0x0A84, 0x0A84),
    (0x0A8E, 0x0A8E), (0x0A92, 0x0A92), (0x0AA9, 0x0AA9), (0x0AB1, 0x0AB1),
    (0x0AB4, 0x0AB4), (0x0ABA, 0x0ABB), (0x0AC6, 0x0AC6), (0x0ACA, 0x0ACA),
    (0x0ACE, 0x0ACF), (0x0AD1, 0x0ADF), (0x0AE4, 0x0AE5), (0x0AF2, 0x0AF8),
    (0x0B00, 0x0B00), (0x0B04, 0x0B04), (0x0B0D, 0x0B0E), (0x0B11, 0x0B12),
    (0x0B29, 0x0B29), (0x0B31, 0x0B31), (0x0B34, 0x0B34), (0x0B3A, 0x0B3B),
    (0x0B45, 0x0B46), (0x0B49, 0x0B4A), (0x0B4E, 0x0B52), (0x0B58, 0x0B5B),
    (0x0B5E, 0x0B5E), (0x0B64, 0x0B65), (0x0B78, 0x0B81), (0x0B84, 0x0B84),
    (0x0B8B, 0x0B8D), (0x0B91, 0x0B91), (0x0B96, 0x0B98), (0x0B9B, 0x0B9B),
    (0x0B9D, 0x0B9D), (0x0BA0, 0x0BA2), (0x0BA5, 0x0BA7), (0x0BAB, 0x0BAD),
    (0x0BBA, 0x0BBD), (0x0BC3, 0x0BC5), (0x0BC9, 0x0BC9), (0x0BCE, 0x0BCF),
    (0x0BD1, 0x0BD6), (0x0BD8, 0x0BE5), (0x0BFB, 0x0BFF), (0x0C0D, 0x0C0D),
    (0x0C11, 0x0C11), (0x0C29, 0x0C29), (0x0C3A, 0x0C3B), (0x0C45, 0x0C45),
    (0x0C49, 0x0C49), (0x0C4E, 0x0C54), (0x0C57, 0x0C57), (0x0C5B, 0x0C5B),
    (0x0C5E, 0x0C5F), (0x0C64, 0x0C65), (0x0C70, 0x0C76), (0x0C8D, 0x0C8D),
    (0x0C91, 0x0C91), (0x0CA9, 0x0CA9), (0x0CB4, 0x0CB4), (0x0CBA, 0x0CBB),
    (0x0CC5, 0x0CC5), (0x0CC9, 0x0CC9), (0x0CCE, 0x0CD4), (0x0CD7, 0x0CDB),
    (0x0CDF, 0x0CDF), (0x0CE4, 0x0CE5), (0x0CF0, 0x0CF0), (0x0CF4, 0x0CFF),
    (0x0D0D, 0x0D0D), (0x0D11, 0x0D11), (0x0D45, 0x0D45), (0x0D49, 0x0D49),
    (0x0D50, 0x0D53), (0x0D64, 0x0D65), (0x0D80, 0x0D80), (0x0D84, 0x0D84),
    (0x0D97, 0x0D99), (0x0DB2, 0x0DB2), (0x0DBC, 0x0DBC), (0x0DBE, 0x0DBF),
    (0x0DC7, 0x0DC9), (0x0DCB, 0x0DCE), (0x0DD5, 0x0DD5), (0x0DD7, 0x0DD7),
    (0x0DE0, 0x0DE5), (0x0DF0, 0x0DF1), (0x0DF5, 0x0E00), (0x0E3B, 0x0E3E),
    (0x0E5C, 0x0E80), (0x0E83, 0x0E83), (0x0E85, 0x0E85), (0x0E8B, 0x0E8B),
    (0x0EA4, 0x0EA4), (0x0EA6, 0x0EA6), (0x0EBE, 0x0EBF), (0x0EC5, 0x0EC5),
    (0x0EC7, 0x0EC7), (0x0ECF, 0x0ECF), (0x0EDA, 0x0EDB), (0x0EE0, 0x0EFF),
    (0x0F48, 0x0F48), (0x0F6D, 0x0F70), (0x0F98, 0x0F98), (0x0FBD, 0x0FBD),
    (0x0FCD, 0x0FCD), (0x0FDB, 0x0FFF), (0x10C6, 0x10C6), (0x10C8, 0x10CC),
    (0x10CE, 0x10CF), (0x1249, 0x1249), (0x124E, 0x124F), (0x1257, 0x1257),
    (0x1259, 0x1259), (0x125E, 0x125F), (0x1289, 0x1289), (0x128E, 0x128F),
    (0x12B1, 0x12B1), (0x12B6, 0x12B7), (0x12BF, 0x12BF), (0x12C1, 0x12C1),
    (0x12C6, 0x12C7), (0x12D7, 0x12D7), (0x1311, 0x1311), (0x1316, 0x1317),
    (0x135B, 0x135C), (0x137D, 0x137F), (0x139A, 0x139F), (0x13F6, 0x13F7),
    (0x13FE, 0x13FF), (0x169D, 0x169F), (0x16F9, 0x16FF), (0x1716, 0x171E),
    (0x1737, 0x173F), (0x1754, 0x175F), (0x176D, 0x176D), (0x1771, 0x1771),
    (0x1774, 0x177F), (0x17DE, 0x17DF), (0x17EA, 0x17EF), (0x17FA, 0x17FF),
    (0x181A, 0x181F), (0x1879, 0x187F), (0x18AB, 0x18AF), (0x18F6, 0x18FF),
    (0x191F, 0x191F), (0x192C, 0x192F), (0x193C, 0x193F), (0x1941, 0x1943),
    (0x196E, 0x196F), (0x1975, 0x197F), (0x19AC, 0x19AF), (0x19CA, 0x19CF),
    (0x19DB, 0x19DD), (0x1A1C, 0x1A1D), (0x1A5F, 0x1A5F), (0x1A7D, 0x1A7E),
    (0x1A8A, 0x1A8F), (0x1A9A, 0x1A9F), (0x1AAE, 0x1AAF), (0x1AF1, 0x1AFF),
    (0x1B4D, 0x1B4D), (0x1BF4, 0x1BFB), (0x1C38, 0x1C3A), (0x1C4A, 0x1C4C),
    (0x1C8B, 0x1C8F), (0x1CBB, 0x1CBC), (0x1CC8, 0x1CCF), (0x1CFB, 0x1CFF),
    (0x1F16, 0x1F17), (0x1F1E, 0x1F1F), (0x1F46, 0x1F47), (0x1F4E, 0x1F4F),
    (0x1F58, 0x1F58), (0x1F5A, 0x1F5A), (0x1F5C, 0x1F5C), (0x1F5E, 0x1F5E),
    (0x1F7E, 0x1F7F), (0x1FB5, 0x1FB5), (0x1FC5, 0x1FC5), (0x1FD4, 0x1FD5),
    (0x1FDC, 0x1FDC), (0x1FF0, 0x1FF1), (0x1FF5, 0x1FF5), (0x1FFF, 0x1FFF),
    (0x2065, 0x2065), (0x2072, 0x2073), (0x20C5, 0x20CF), (0x20F1, 0x20FF),
    (0x218C, 0x218F), (0x242A, 0x243F), (0x244B, 0x245F), (0x2B74, 0x2B75),
    (0x2CF4, 0x2CF8), (0x2D26, 0x2D26), (0x2D28, 0x2D2C), (0x2D2E, 0x2D2F),
    (0x2D68, 0x2D6E), (0x2D71, 0x2D7E), (0x2D97, 0x2D9F), (0x2DA7, 0x2DA7),
    (0x2DAF, 0x2DAF), (0x2DB7, 0x2DB7), (0x2DBF, 0x2DBF), (0x2DC7, 0x2DC7),
    (0x2DCF, 0x2DCF), (0x2DD7, 0x2DD7), (0x2DDF, 0x2DDF), (0x2E5E, 0x2E5F),
    (0x2E64, 0x2E7F), (0x2E9A, 0x2E9A), (0x2EF4, 0x2EFF), (0x2FD6, 0x2FEF),
    (0x3040, 0x3040), (0x3097, 0x3098), (0x3100, 0x3104), (0x3130, 0x3130),
    (0x318F, 0x318F), (0x31E6, 0x31EE), (0x321F, 0x321F), (0xA48D, 0xA48F),
    (0xA4C7, 0xA4CF), (0xA62C, 0xA63F), (0xA6F8, 0xA6FF), (0xA7DE, 0xA7E1),
    (0xA7E3, 0xA7F0), (0xA82D, 0xA82F), (0xA83A, 0xA83F), (0xA878, 0xA87F),
    (0xA8C6, 0xA8CD), (0xA8DA, 0xA8DF), (0xA954, 0xA95E), (0xA97D, 0xA97F),
    (0xA9CE, 0xA9CE), (0xA9DA, 0xA9DD), (0xA9FF, 0xA9FF), (0xAA37, 0xAA3F),
    (0xAA4E, 0xAA4F), (0xAA5A, 0xAA5B), (0xAAC3, 0xAADA), (0xAAF7, 0xAB00),
    (0xAB07, 0xAB08), (0xAB0F, 0xAB10), (0xAB17, 0xAB1F), (0xAB27, 0xAB27),
    (0xAB2F, 0xAB2F), (0xAB6E, 0xAB6F), (0xABEE, 0xABEF), (0xABFA, 0xABFF),
    (0xD7A4, 0xD7AF), (0xD7C7, 0xD7CA), (0xD7FC, 0xD7FF), (0xFA6E, 0xFA6F),
    (0xFADA, 0xFAFF), (0xFB07, 0xFB12), (0xFB18, 0xFB1C), (0xFB37, 0xFB37),
    (0xFB3D, 0xFB3D), (0xFB3F, 0xFB3F), (0xFB42, 0xFB42), (0xFB45, 0xFB45),
    (0xFDD0, 0xFDEF), (0xFE1A, 0xFE1F), (0xFE53, 0xFE53), (0xFE67, 0xFE67),
    (0xFE6C, 0xFE6F), (0xFE75, 0xFE75), (0xFEFD, 0xFEFE), (0xFF00, 0xFF00),
    (0xFFBF, 0xFFC1), (0xFFC8, 0xFFC9), (0xFFD0, 0xFFD1), (0xFFD8, 0xFFD9),
    (0xFFDD, 0xFFDF), (0xFFE7, 0xFFE7), (0xFFEF, 0xFFF8), (0xFFFE, 0xFFFF),
    (0x1000C, 0x1000C), (0x10027, 0x10027), (0x1003B, 0x1003B), (0x1003E, 0x1003E),
    (0x1004E, 0x1004F), (0x1005E, 0x1007F), (0x100FB, 0x100FF), (0x10103, 0x10106),
    (0x10134, 0x10136), (0x1018F, 0x1018F), (0x1019D, 0x1019F), (0x101A1, 0x101CF),
    (0x101FE, 0x1027F), (0x1029D, 0x1029F), (0x102D1, 0x102DF), (0x102FC, 0x102FF),
    (0x10324, 0x1032C), (0x1034B, 0x1034F), (0x1037B, 0x1037F), (0x1039E, 0x1039E),
    (0x103C4, 0x103C7), (0x103D6, 0x103FF), (0x1049E, 0x1049F), (0x104AA, 0x104AF),
    (0x104D4, 0x104D7), (0x104FC, 0x104FF), (0x10528, 0x1052F), (0x10564, 0x1056E),
    (0x1057B, 0x1057B), (0x1058B, 0x1058B), (0x10593, 0x10593), (0x10596, 0x10596),
    (0x105A2, 0x105A2), (0x105B2, 0x105B2), (0x105BA, 0x105BA), (0x105BD, 0x105BF),
    (0x105F4, 0x105FF), (0x10737, 0x1073F), (0x10756, 0x1075F), (0x10768, 0x1077F),
    (0x10786, 0x10786), (0x107B1, 0x107B1), (0x107C0, 0x107FF), (0x10806, 0x10807),
    (0x10809, 0x10809), (0x10836, 0x10836), (0x10839, 0x1083B), (0x1083D, 0x1083E),
    (0x10856, 0x10856), (0x1089F, 0x108A6), (0x108B0, 0x108DF), (0x108F3, 0x108F3),
    (0x108F6, 0x108FA), (0x1091C, 0x1091E), (0x1093A, 0x1093E), (0x1095A, 0x1097F),
    (0x109B8, 0x109BB), (0x109D0, 0x109D1), (0x10A04, 0x10A04), (0x10A07, 0x10A0B),
    (0x10A14, 0x10A14), (0x10A18, 0x10A18), (0x10A36, 0x10A37), (0x10A3B, 0x10A3E),
    (0x10A49, 0x10A4F), (0x10A59, 0x10A5F), (0x10AA0, 0x10ABF), (0x10AE7, 0x10AEA),
    (0x10AF7, 0x10AFF), (0x10B36, 0x10B38), (0x10B56, 0x10B57), (0x10B73, 0x10B77),
    (0x10B92, 0x10B98), (0x10B9D, 0x10BA8), (0x10BB0, 0x10BFF), (0x10C49, 0x10C7F),
    (0x10CB3, 0x10CBF), (0x10CF3, 0x10CF9), (0x10D28, 0x10D2F), (0x10D3A, 0x10D3F),
    (0x10D66, 0x10D68), (0x10D86, 0x10D8D), (0x10D90, 0x10E5F), (0x10E7F, 0x10E7F),
    (0x10EAA, 0x10EAA), (0x10EAE, 0x10EAF), (0x10EB2, 0x10EC1), (0x10EC8, 0x10EC8),
    (0x10EEF, 0x10EEF), (0x10F28, 0x10F2F), (0x10F5A, 0x10F6F), (0x10F8A, 0x10FAF),
    (0x10FCC, 0x10FDF), (0x10FF7, 0x10FFF), (0x1104E, 0x11051), (0x11076, 0x1107E),
    (0x110C3, 0x110CC), (0x110CE, 0x110CF), (0x110E9, 0x110EF), (0x110FA, 0x110FF),
    (0x11135, 0x11135), (0x11148, 0x1114F), (0x11177, 0x1117F), (0x111E0, 0x111E0),
    (0x111F5, 0x111FF), (0x11212, 0x11212), (0x11242, 0x1127F), (0x11287, 0x11287),
    (0x11289, 0x11289), (0x1128E, 0x1128E), (0x1129E, 0x1129E), (0x112AA, 0x112AF),
    (0x112EB, 0x112EF), (0x112FA, 0x112FF), (0x11304, 0x11304), (0x1130D, 0x1130E),
    (0x11311, 0x11312), (0x11329, 0x11329), (0x11331, 0x11331), (0x11334, 0x11334),
    (0x1133A, 0x1133A), (0x11345, 0x11346), (0x11349, 0x1134A), (0x1134E, 0x1134F),
    (0x11351, 0x11356), (0x11358, 0x1135C), (0x11364, 0x11365), (0x1136D, 0x1136F),
    (0x11375, 0x1137F), (0x1138A, 0x1138A), (0x1138C, 0x1138D), (0x1138F, 0x1138F),
    (0x113B6, 0x113B6), (0x113C1, 0x113C1), (0x113C3, 0x113C4), (0x113C6, 0x113C6),
    (0x113CB, 0x113CB), (0x113D6, 0x113D6), (0x113D9, 0x113E0), (0x113E3, 0x113FF),
    (0x1145C, 0x1145C), (0x11462, 0x1147F), (0x114C8, 0x114CF), (0x114DA, 0x1157F),
    (0x115B6, 0x115B7), (0x115DE, 0x115FF), (0x11645, 0x1164F), (0x1165A, 0x1165F),
    (0x1166D, 0x1167F), (0x116BA, 0x116BF), (0x116CA, 0x116CF), (0x116E4, 0x116FF),
    (0x1171B, 0x1171C), (0x1172C, 0x1172F), (0x11747, 0x117FF), (0x1183C, 0x1189F),
    (0x118F3, 0x118FE), (0x11907, 0x11908), (0x1190A, 0x1190B), (0x11914, 0x11914),
    (0x11917, 0x11917), (0x11936, 0x11936), (0x11939, 0x1193A), (0x11947, 0x1194F),
    (0x1195A, 0x1199F), (0x119A8, 0x119A9), (0x119D8, 0x119D9), (0x119E5, 0x119FF),
    (0x11A48, 0x11A4F), (0x11AA3, 0x11AAF), (0x11AF9, 0x11AFF), (0x11B0B, 0x11B5F),
    (0x11B68, 0x11BBF), (0x11BE2, 0x11BEF), (0x11BFA, 0x11BFF), (0x11C09, 0x11C09),
    (0x11C37, 0x11C37), (0x11C46, 0x11C4F), (0x11C6D, 0x11C6F), (0x11C90, 0x11C91),
    (0x11CA8, 0x11CA8), (0x11CB7, 0x11CFF), (0x11D07, 0x11D07), (0x11D0A, 0x11D0A),
    (0x11D37, 0x11D39), (0x11D3B, 0x11D3B), (0x11D3E, 0x11D3E), (0x11D48, 0x11D4F),
    (0x11D5A, 0x11D5F), (0x11D66, 0x11D66), (0x11D69, 0x11D69), (0x11D8F, 0x11D8F),
    (0x11D92, 0x11D92), (0x11D99, 0x11D9F), (0x11DAA, 0x11DAF), (0x11DDC, 0x11DDF),
    (0x11DEA, 0x11DEF), (0x11DF2, 0x11EDF), (0x11EF9, 0x11EFF), (0x11F11, 0x11F11),
    (0x11F3B, 0x11F3D), (0x11F5B, 0x11FAF), (0x11FB1, 0x11FBF), (0x11FF2, 0x11FFE),
    (0x1239A, 0x123FF), (0x12544, 0x1254F), (0x12687, 0x12F8F), (0x12FF3, 0x12FFF),
    (0x13456, 0x1345F), (0x143FB, 0x143FF), (0x14647, 0x160FF), (0x1613A, 0x167FF),
    (0x16A39, 0x16A3F), (0x16A5F, 0x16A5F), (0x16A6A, 0x16A6D), (0x16ABF, 0x16ABF),
    (0x16ACA, 0x16ACF), (0x16AEE, 0x16AEF), (0x16AF6, 0x16AFF), (0x16B46, 0x16B4F),
    (0x16B5A, 0x16B5A), (0x16B62, 0x16B62), (0x16B78, 0x16B7C), (0x16B90, 0x16D3F),
    (0x16D7A, 0x16E3F), (0x16E9B, 0x16E9F), (0x16EB9, 0x16EBA), (0x16ED4, 0x16EFF),
    (0x16F4B, 0x16F4E), (0x16F88, 0x16F8E), (0x16FA0, 0x16FDF), (0x16FE5, 0x16FEF),
    (0x16FF7, 0x16FFF), (0x18CDB, 0x18CFE), (0x18D21, 0x18D7F), (0x18DF3, 0x18DFF),
    (0x19192, 0x1919F), (0x191D3, 0x1AFEF), (0x1AFF4, 0x1AFF4), (0x1AFFC, 0x1AFFC),
    (0x1AFFF, 0x1AFFF), (0x1B129, 0x1B131), (0x1B133, 0x1B14F), (0x1B153, 0x1B154),
    (0x1B156, 0x1B163), (0x1B169, 0x1B16F), (0x1B2FC, 0x1BBFF), (0x1BC6B, 0x1BC6F),
    (0x1BC7D, 0x1BC7F), (0x1BC89, 0x1BC8F), (0x1BC9A, 0x1BC9B), (0x1BCA4, 0x1CBFF),
    (0x1CCFD, 0x1CCFF), (0x1CEB4, 0x1CEB9), (0x1CED1, 0x1CED1), (0x1CED5, 0x1CEDC),
    (0x1CEFE, 0x1CEFF), (0x1CF2E, 0x1CF2F), (0x1CF47, 0x1CF4F), (0x1CFC4, 0x1CFFF),
    (0x1D0F6, 0x1D0FF), (0x1D246, 0x1D24F), (0x1D282, 0x1D2BF), (0x1D2D4, 0x1D2DF),
    (0x1D2F4, 0x1D2FF), (0x1D357, 0x1D35F), (0x1D379, 0x1D3FF), (0x1D455, 0x1D455),
    (0x1D49D, 0x1D49D), (0x1D4A0, 0x1D4A1), (0x1D4A3, 0x1D4A4), (0x1D4A7, 0x1D4A8),
    (0x1D4AD, 0x1D4AD), (0x1D4BA, 0x1D4BA), (0x1D4BC, 0x1D4BC), (0x1D4C4, 0x1D4C4),
    (0x1D506, 0x1D506), (0x1D50B, 0x1D50C), (0x1D515, 0x1D515), (0x1D51D, 0x1D51D),
    (0x1D53A, 0x1D53A), (0x1D53F, 0x1D53F), (0x1D545, 0x1D545), (0x1D547, 0x1D549),
    (0x1D551, 0x1D551), (0x1D6A7, 0x1D6A7), (0x1D7CC, 0x1D7CD), (0x1DA8C, 0x1DA9A),
    (0x1DAA0, 0x1DAA0), (0x1DAB0, 0x1DAFF), (0x1DB1D, 0x1DEFF), (0x1DF82, 0x1DF8F),
    (0x1DF97, 0x1DFCC), (0x1E007, 0x1E007), (0x1E019, 0x1E01A), (0x1E022, 0x1E022),
    (0x1E025, 0x1E025), (0x1E02B, 0x1E02F), (0x1E06E, 0x1E08E), (0x1E090, 0x1E0FF),
    (0x1E12D, 0x1E12F), (0x1E13E, 0x1E13F), (0x1E14A, 0x1E14D), (0x1E150, 0x1E28F),
    (0x1E2AF, 0x1E2BF), (0x1E2FA, 0x1E2FE), (0x1E300, 0x1E4CF), (0x1E4FA, 0x1E5CF),
    (0x1E5FB, 0x1E5FE), (0x1E600, 0x1E6BF), (0x1E6DF, 0x1E6DF), (0x1E6F6, 0x1E6FD),
    (0x1E700, 0x1E7DF), (0x1E7E7, 0x1E7E7), (0x1E7EC, 0x1E7EC), (0x1E7EF, 0x1E7EF),
    (0x1E7FF, 0x1E7FF), (0x1E8C5, 0x1E8C6), (0x1E8D7, 0x1E8FF), (0x1E94C, 0x1E94F),
    (0x1E95A, 0x1E95D), (0x1E960, 0x1EC70), (0x1ECB5, 0x1ED00), (0x1ED3E, 0x1EDFF),
    (0x1EE04, 0x1EE04), (0x1EE20, 0x1EE20), (0x1EE23, 0x1EE23), (0x1EE25, 0x1EE26),
    (0x1EE28, 0x1EE28), (0x1EE33, 0x1EE33), (0x1EE38, 0x1EE38), (0x1EE3A, 0x1EE3A),
    (0x1EE3C, 0x1EE41), (0x1EE43, 0x1EE46), (0x1EE48, 0x1EE48), (0x1EE4A, 0x1EE4A),
    (0x1EE4C, 0x1EE4C), (0x1EE50, 0x1EE50), (0x1EE53, 0x1EE53), (0x1EE55, 0x1EE56),
    (0x1EE58, 0x1EE58), (0x1EE5A, 0x1EE5A), (0x1EE5C, 0x1EE5C), (0x1EE5E, 0x1EE5E),
    (0x1EE60, 0x1EE60), (0x1EE63, 0x1EE63), (0x1EE65, 0x1EE66), (0x1EE6B, 0x1EE6B),
    (0x1EE73, 0x1EE73), (0x1EE78, 0x1EE78), (0x1EE7D, 0x1EE7D), (0x1EE7F, 0x1EE7F),
    (0x1EE8A, 0x1EE8A), (0x1EE9C, 0x1EEA0), (0x1EEA4, 0x1EEA4), (0x1EEAA, 0x1EEAA),
    (0x1EEBC, 0x1EEEF), (0x1EEF2, 0x1EFFF), (0x1F02C, 0x1F02F), (0x1F094, 0x1F09F),
    (0x1F0AF, 0x1F0B0), (0x1F0C0, 0x1F0C0), (0x1F0D0, 0x1F0D0), (0x1F0F6, 0x1F0FF),
    (0x1F1AF, 0x1F1E5), (0x1F203, 0x1F20F), (0x1F23C, 0x1F23F), (0x1F249, 0x1F24F),
    (0x1F252, 0x1F25F), (0x1F266, 0x1F2FF), (0x1F6DA, 0x1F6DB), (0x1F6ED, 0x1F6EF),
    (0x1F6FD, 0x1F6FF), (0x1F7DC, 0x1F7DF), (0x1F7EC, 0x1F7EF), (0x1F80C, 0x1F80F),
    (0x1F848, 0x1F84F), (0x1F85A, 0x1F85F), (0x1F888, 0x1F88F), (0x1F8AE, 0x1F8AF),
    (0x1F8BC, 0x1F8BF), (0x1F8C2, 0x1F8CF), (0x1F8D9, 0x1F8FF), (0x1FA58, 0x1FA5F),
    (0x1FA6E, 0x1FA6F), (0x1FA7D, 0x1FA7F), (0x1FAC7, 0x1FAC7), (0x1FAC9, 0x1FACB),
    (0x1FADE, 0x1FADE), (0x1FAEC, 0x1FAEE), (0x1FAFB, 0x1FAFF), (0x1FB93, 0x1FB93),
    (0x1FBFB, 0x1FFFF), (0x2A6E0, 0x2A6FF), (0x2B81F, 0x2B81F), (0x2CEAE, 0x2CEAF),
    (0x2EBE1, 0x2EBEF), (0x2EE5E, 0x2F7FF), (0x2FA1E, 0x2FFFF), (0x3134B, 0x3134F),
    (0x3347A, 0x3CFFF), (0x3FC40, 0x3FFFD),
];

/// Pinned Unicode 18.0.0 `Grapheme_Extend = Yes` ranges (all `Mn`, `Me`, extending `Mc`,
/// variation selectors `U+FE0E..=U+FE0F`, and halfwidth Katakana sound marks `U+FF9E..=U+FF9F`).
#[rustfmt::skip]
const UNICODE_18_0_GRAPHEME_EXTEND: &[(u32, u32)] = &[
    (0x0300, 0x036F), (0x0483, 0x0489), (0x0591, 0x05BD), (0x05BF, 0x05BF),
    (0x05C1, 0x05C2), (0x05C4, 0x05C5), (0x05C7, 0x05C9), (0x0610, 0x061A),
    (0x064B, 0x065F), (0x0670, 0x0670), (0x06D6, 0x06DC), (0x06DF, 0x06E4),
    (0x06E7, 0x06E8), (0x06EA, 0x06ED), (0x0711, 0x0711), (0x0730, 0x074A),
    (0x07A6, 0x07B0), (0x07EB, 0x07F3), (0x07FD, 0x07FD), (0x0816, 0x0819),
    (0x081B, 0x0823), (0x0825, 0x0827), (0x0829, 0x082D), (0x0859, 0x085B),
    (0x0897, 0x089F), (0x08CA, 0x08E1), (0x08E3, 0x0902), (0x093A, 0x093A),
    (0x093C, 0x093C), (0x0941, 0x0948), (0x094D, 0x094D), (0x0951, 0x0957),
    (0x0962, 0x0963), (0x0981, 0x0981), (0x09BC, 0x09BC), (0x09BE, 0x09BE),
    (0x09C1, 0x09C4), (0x09CD, 0x09CD), (0x09D7, 0x09D7), (0x09E2, 0x09E3),
    (0x09FE, 0x09FE), (0x0A01, 0x0A02), (0x0A3C, 0x0A3C), (0x0A41, 0x0A42),
    (0x0A47, 0x0A48), (0x0A4B, 0x0A4D), (0x0A51, 0x0A51), (0x0A70, 0x0A71),
    (0x0A75, 0x0A75), (0x0A81, 0x0A82), (0x0ABC, 0x0ABC), (0x0AC1, 0x0AC5),
    (0x0AC7, 0x0AC8), (0x0ACD, 0x0ACD), (0x0AE2, 0x0AE3), (0x0AFA, 0x0AFF),
    (0x0B01, 0x0B01), (0x0B3C, 0x0B3C), (0x0B3E, 0x0B3F), (0x0B41, 0x0B44),
    (0x0B4D, 0x0B4D), (0x0B53, 0x0B57), (0x0B62, 0x0B63), (0x0B82, 0x0B82),
    (0x0BBE, 0x0BBE), (0x0BC0, 0x0BC0), (0x0BCD, 0x0BCD), (0x0BD7, 0x0BD7),
    (0x0C00, 0x0C00), (0x0C04, 0x0C04), (0x0C3C, 0x0C3C), (0x0C3E, 0x0C40),
    (0x0C46, 0x0C48), (0x0C4A, 0x0C4D), (0x0C55, 0x0C56), (0x0C62, 0x0C63),
    (0x0C81, 0x0C81), (0x0CBC, 0x0CBC), (0x0CBF, 0x0CC0), (0x0CC2, 0x0CC2),
    (0x0CC6, 0x0CC8), (0x0CCA, 0x0CCD), (0x0CD5, 0x0CD6), (0x0CE2, 0x0CE3),
    (0x0D00, 0x0D01), (0x0D3B, 0x0D3C), (0x0D3E, 0x0D3E), (0x0D41, 0x0D44),
    (0x0D4D, 0x0D4D), (0x0D57, 0x0D57), (0x0D62, 0x0D63), (0x0D81, 0x0D81),
    (0x0DCA, 0x0DCA), (0x0DCF, 0x0DCF), (0x0DD2, 0x0DD4), (0x0DD6, 0x0DD6),
    (0x0DDF, 0x0DDF), (0x0E31, 0x0E31), (0x0E34, 0x0E3A), (0x0E47, 0x0E4E),
    (0x0EB1, 0x0EB1), (0x0EB4, 0x0EBC), (0x0EC8, 0x0ECE), (0x0F18, 0x0F19),
    (0x0F35, 0x0F35), (0x0F37, 0x0F37), (0x0F39, 0x0F39), (0x0F71, 0x0F7E),
    (0x0F80, 0x0F84), (0x0F86, 0x0F87), (0x0F8D, 0x0F97), (0x0F99, 0x0FBC),
    (0x0FC6, 0x0FC6), (0x102D, 0x1030), (0x1032, 0x1037), (0x1039, 0x103A),
    (0x103D, 0x103E), (0x1058, 0x1059), (0x105E, 0x1060), (0x1071, 0x1074),
    (0x1082, 0x1082), (0x1085, 0x1086), (0x108D, 0x108D), (0x109D, 0x109D),
    (0x135D, 0x135F), (0x1712, 0x1715), (0x1732, 0x1734), (0x1752, 0x1753),
    (0x1772, 0x1773), (0x17B4, 0x17B5), (0x17B7, 0x17BD), (0x17C6, 0x17C6),
    (0x17C9, 0x17D3), (0x17DD, 0x17DD), (0x180B, 0x180D), (0x180F, 0x180F),
    (0x1885, 0x1886), (0x18A9, 0x18A9), (0x1920, 0x1922), (0x1927, 0x1928),
    (0x1932, 0x1932), (0x1939, 0x193B), (0x1A17, 0x1A18), (0x1A1B, 0x1A1B),
    (0x1A56, 0x1A56), (0x1A58, 0x1A5E), (0x1A60, 0x1A60), (0x1A62, 0x1A62),
    (0x1A65, 0x1A6C), (0x1A73, 0x1A7C), (0x1A7F, 0x1A7F), (0x1AB0, 0x1AF0),
    (0x1B00, 0x1B03), (0x1B34, 0x1B3D), (0x1B42, 0x1B44), (0x1B6B, 0x1B73),
    (0x1B80, 0x1B81), (0x1BA2, 0x1BA5), (0x1BA8, 0x1BAD), (0x1BE6, 0x1BE6),
    (0x1BE8, 0x1BE9), (0x1BED, 0x1BED), (0x1BEF, 0x1BF3), (0x1C2C, 0x1C33),
    (0x1C36, 0x1C37), (0x1CD0, 0x1CD2), (0x1CD4, 0x1CE0), (0x1CE2, 0x1CE8),
    (0x1CED, 0x1CED), (0x1CF4, 0x1CF4), (0x1CF8, 0x1CF9), (0x1DC0, 0x1DFF),
    (0x200C, 0x200C), (0x20D0, 0x20F0), (0x2CEF, 0x2CF1), (0x2D7F, 0x2D7F),
    (0x2DE0, 0x2DFF), (0x302A, 0x302F), (0x3099, 0x309A), (0xA66F, 0xA672),
    (0xA674, 0xA67D), (0xA69E, 0xA69F), (0xA6F0, 0xA6F1), (0xA802, 0xA802),
    (0xA806, 0xA806), (0xA80B, 0xA80B), (0xA825, 0xA826), (0xA82C, 0xA82C),
    (0xA8C4, 0xA8C5), (0xA8E0, 0xA8F1), (0xA8FF, 0xA8FF), (0xA926, 0xA92D),
    (0xA947, 0xA951), (0xA953, 0xA953), (0xA980, 0xA982), (0xA9B3, 0xA9B3),
    (0xA9B6, 0xA9B9), (0xA9BC, 0xA9BD), (0xA9C0, 0xA9C0), (0xA9E5, 0xA9E5),
    (0xAA29, 0xAA2E), (0xAA31, 0xAA32), (0xAA35, 0xAA36), (0xAA43, 0xAA43),
    (0xAA4C, 0xAA4C), (0xAA7C, 0xAA7C), (0xAAB0, 0xAAB0), (0xAAB2, 0xAAB4),
    (0xAAB7, 0xAAB8), (0xAABE, 0xAABF), (0xAAC1, 0xAAC1), (0xAAEC, 0xAAED),
    (0xAAF6, 0xAAF6), (0xABE5, 0xABE5), (0xABE8, 0xABE8), (0xABED, 0xABED),
    (0xFB1E, 0xFB1E), (0xFE00, 0xFE0F), (0xFE20, 0xFE2F), (0xFF9E, 0xFF9F),
    (0x101FD, 0x101FD), (0x102E0, 0x102E0), (0x10376, 0x1037A), (0x10A01, 0x10A03),
    (0x10A05, 0x10A06), (0x10A0C, 0x10A0F), (0x10A38, 0x10A3A), (0x10A3F, 0x10A3F),
    (0x10AE5, 0x10AE6), (0x10D24, 0x10D27), (0x10D69, 0x10D6D), (0x10EAB, 0x10EAC),
    (0x10ECB, 0x10ECF), (0x10EF0, 0x10EFF), (0x10F46, 0x10F50), (0x10F82, 0x10F85),
    (0x11001, 0x11001), (0x11038, 0x11046), (0x11070, 0x11070), (0x11073, 0x11074),
    (0x1107F, 0x11081), (0x110B3, 0x110B6), (0x110B9, 0x110BA), (0x110C2, 0x110C2),
    (0x11100, 0x11102), (0x11127, 0x1112B), (0x1112D, 0x11134), (0x11173, 0x11173),
    (0x11180, 0x11181), (0x111B6, 0x111BE), (0x111C0, 0x111C0), (0x111C9, 0x111CC),
    (0x111CF, 0x111CF), (0x1122F, 0x11231), (0x11234, 0x11237), (0x1123E, 0x1123E),
    (0x11241, 0x11241), (0x112DF, 0x112DF), (0x112E3, 0x112EA), (0x11300, 0x11301),
    (0x1133B, 0x1133C), (0x1133E, 0x1133E), (0x11340, 0x11340), (0x1134D, 0x1134D),
    (0x11357, 0x11357), (0x11366, 0x1136C), (0x11370, 0x11374), (0x113B8, 0x113B8),
    (0x113BB, 0x113C0), (0x113C2, 0x113C2), (0x113C5, 0x113C5), (0x113C7, 0x113C9),
    (0x113CE, 0x113D0), (0x113D2, 0x113D2), (0x113E1, 0x113E2), (0x11438, 0x1143F),
    (0x11442, 0x11444), (0x11446, 0x11446), (0x1145E, 0x1145E), (0x114B0, 0x114B0),
    (0x114B3, 0x114B8), (0x114BA, 0x114BA), (0x114BD, 0x114BD), (0x114BF, 0x114C0),
    (0x114C2, 0x114C3), (0x115AF, 0x115AF), (0x115B2, 0x115B5), (0x115BC, 0x115BD),
    (0x115BF, 0x115C0), (0x115DC, 0x115DD), (0x11633, 0x1163A), (0x1163D, 0x1163D),
    (0x1163F, 0x11640), (0x116AB, 0x116AB), (0x116AD, 0x116AD), (0x116B0, 0x116B7),
    (0x1171D, 0x1171D), (0x1171F, 0x1171F), (0x11722, 0x11725), (0x11727, 0x1172B),
    (0x1182F, 0x11837), (0x11839, 0x1183A), (0x11930, 0x11930), (0x1193B, 0x1193E),
    (0x11943, 0x11943), (0x119D4, 0x119D7), (0x119DA, 0x119DB), (0x119E0, 0x119E0),
    (0x11A01, 0x11A0A), (0x11A33, 0x11A38), (0x11A3B, 0x11A3E), (0x11A47, 0x11A47),
    (0x11A51, 0x11A56), (0x11A59, 0x11A5B), (0x11A8A, 0x11A96), (0x11A98, 0x11A99),
    (0x11B60, 0x11B60), (0x11B62, 0x11B64), (0x11B66, 0x11B66), (0x11C30, 0x11C36),
    (0x11C38, 0x11C3D), (0x11C3F, 0x11C3F), (0x11C92, 0x11CA7), (0x11CAA, 0x11CB0),
    (0x11CB2, 0x11CB3), (0x11CB5, 0x11CB6), (0x11D31, 0x11D36), (0x11D3A, 0x11D3A),
    (0x11D3C, 0x11D3D), (0x11D3F, 0x11D45), (0x11D47, 0x11D47), (0x11D90, 0x11D91),
    (0x11D95, 0x11D95), (0x11D97, 0x11D97), (0x11DF0, 0x11DF0), (0x11EF3, 0x11EF4),
    (0x11F00, 0x11F01), (0x11F36, 0x11F3A), (0x11F40, 0x11F42), (0x11F5A, 0x11F5A),
    (0x13440, 0x13440), (0x13447, 0x13455), (0x1611E, 0x16129), (0x1612D, 0x1612F),
    (0x16AF0, 0x16AF4), (0x16B30, 0x16B36), (0x16F4F, 0x16F4F), (0x16F8F, 0x16F92),
    (0x16FE4, 0x16FE4), (0x16FF0, 0x16FF1), (0x1BC9D, 0x1BC9E), (0x1CF00, 0x1CF2D),
    (0x1CF30, 0x1CF46), (0x1D127, 0x1D128), (0x1D165, 0x1D169), (0x1D16D, 0x1D172),
    (0x1D17B, 0x1D182), (0x1D185, 0x1D18B), (0x1D1AA, 0x1D1AD), (0x1D242, 0x1D244),
    (0x1D250, 0x1D252), (0x1D25B, 0x1D25C), (0x1D25F, 0x1D25F), (0x1D280, 0x1D281),
    (0x1DA00, 0x1DA36), (0x1DA3B, 0x1DA6C), (0x1DA75, 0x1DA75), (0x1DA84, 0x1DA84),
    (0x1DA9B, 0x1DA9F), (0x1DAA1, 0x1DAAF), (0x1E000, 0x1E006), (0x1E008, 0x1E018),
    (0x1E01B, 0x1E021), (0x1E023, 0x1E024), (0x1E026, 0x1E02A), (0x1E08F, 0x1E08F),
    (0x1E130, 0x1E136), (0x1E2AE, 0x1E2AE), (0x1E2EC, 0x1E2EF), (0x1E4EC, 0x1E4EF),
    (0x1E5EE, 0x1E5EF), (0x1E6E3, 0x1E6E3), (0x1E6E6, 0x1E6E6), (0x1E6EE, 0x1E6EF),
    (0x1E6F5, 0x1E6F5), (0x1E8D0, 0x1E8D6), (0x1E944, 0x1E94A), (0xE0020, 0xE007F),
    (0xE0100, 0xE01EF),
];

#[cfg(test)]
mod parity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_notice_fails_closed_if_supervision_is_lost_while_waiting_for_peers() {
        use crate::workspace::{WorkspaceSet, tests};
        use eon_workspace_protocol::v50::{RunningWindows, WindowEntry, WindowState};
        let root = supervisor::temporary_directory();
        let workspace = WorkspaceSet::pending(
            root.join("runtime"),
            root.clone(),
            None,
            tests::catalog(),
            tests::prepared,
            |_| Ok(()),
        )
        .unwrap();
        let mut snapshot = workspace.snapshot();
        snapshot.running = Some(RunningWindows {
            this: WindowEntry {
                window: None,
                generation: String::new(),
                created_build: None,
                active_build: None,
                name: None,
                started_at: None,
                live_terminals: None,
                state: WindowState::Live,
            },
            others: None,
            omitted: 0,
        });
        let terminal = &snapshot.tabs[0].popups[0].session;
        for failure in [
            Ok(ControlResponse::Workspace(Response::Failure(failure(
                "stop-incomplete",
                "test stopping",
            )))),
            Err(EndpointFailure::new(EndpointFailureKind::Dead, "test dead")),
            Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "test corrupt",
            )),
        ] {
            let mut replies = [
                Ok(ControlResponse::Workspace(Response::Snapshot(
                    snapshot.clone(),
                ))),
                failure,
            ]
            .into_iter();
            assert_eq!(
                poll_startup_notice(&snapshot.active_tab, terminal, |_| replies.next().unwrap()),
                None
            );
            assert!(replies.next().is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_capture_survives_supervisor_self_launch_without_old_authority() {
        let facts = crate::LaunchFacts::decode(
            b"EON-LAUNCH-1\0/target/bin/eon\0/target/bin/.eon-wrapped\0prefix\0TERMINFO_DIRS\0/target/info\0",
            Path::new("/target/bin/eon"), Path::new("/target/bin/.eon-wrapped"),
        ).unwrap();
        let mut inputs = crate::fixtures::inputs();
        inputs.launch = crate::LaunchInputs::capture(
            &facts,
            vec!["eon".into(), "window".into(), "new".into()],
            [
                (
                    "EON_LAUNCH_FACTS".into(),
                    "/target/share/eon/launch-environment".into(),
                ),
                (
                    "EON_LAUNCH_SAVED_TERMINFO_DIRS".into(),
                    "+/user/info".into(),
                ),
                ("TERMINFO_DIRS".into(), "/target/info:/user/info".into()),
                ("EON_ROUTED".into(), "old".into()),
                ("EON_TERMINAL".into(), "old".into()),
            ],
        );
        let command = inputs.supervisor_command().unwrap();
        assert_eq!(command.get_program(), "/target/bin/eon");
        assert_eq!(command.get_args().count(), 0);
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "TERMINFO_DIRS"
                    && value == Some(OsStr::new("/user/info")))
        );
        for name in [
            "EON_ROUTED",
            "EON_ATTACH_DELEGATED",
            "EON_TERMINAL",
            "EON_TAB",
        ] {
            assert!(
                command
                    .get_envs()
                    .all(|(key, value)| key != name || value.is_none())
            );
        }
    }

    #[test]
    fn failed_endpoint_suggests_discovery_instead_of_another_launch() {
        for kind in [EndpointFailureKind::Dead, EndpointFailureKind::Unreachable] {
            let error = unreachable_supervisor(EndpointFailure::new(kind, "test endpoint"), "8644");
            assert!(error.detail.contains("Eon window 8644"), "{}", error.detail);
            assert!(error.detail.contains("eon window ls"), "{}", error.detail);
            assert!(!error.detail.contains("run `eon`"), "{}", error.detail);
            assert_eq!(
                error.code,
                if kind == EndpointFailureKind::Dead {
                    "missing-supervisor"
                } else {
                    "supervisor-unavailable"
                }
            );
        }
    }

    #[test]
    fn launch_capture_parent_environment_is_not_forwarded() {
        if env::var_os("EON_CAPTURE_TEST_CHILD").is_none() {
            let output = Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cli::tests::launch_capture_parent_environment_is_not_forwarded",
                    "--nocapture",
                ])
                .env("EON_CAPTURE_TEST_CHILD", "1")
                .env("EON_PARENT_ONLY", "must disappear")
                .env("EON_ROUTED", "old")
                .env("WAYLAND_SOCKET", "17")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        let facts = crate::LaunchFacts::decode(
            b"EON-LAUNCH-1\0/bin/sh\0/bin/.sh-wrapped\0",
            Path::new("/bin/sh"),
            Path::new("/bin/.sh-wrapped"),
        )
        .unwrap();
        let mut inputs = crate::fixtures::inputs();
        inputs.launch = crate::LaunchInputs::capture(
            &facts,
            vec!["eon".into()],
            [
                (
                    "EON_LAUNCH_FACTS".into(),
                    "/share/eon/launch-environment".into(),
                ),
                ("WAYLAND_SOCKET".into(), "99".into()),
            ],
        );
        let output = inputs
            .supervisor_command()
            .unwrap()
            .args([
                "-c",
                "test -z \"${EON_PARENT_ONLY+x}\" && test -z \"${EON_ROUTED+x}\" && printf \"clean $WAYLAND_SOCKET\"",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        // Linux replays no display number; a fresh supervisor gets the caller's.
        let expected: &[u8] = if cfg!(target_os = "linux") {
            b"clean 17"
        } else {
            b"clean 99"
        };
        assert_eq!(output.stdout, expected);
        inputs.launch = Err("unwrapped".into());
        let command = inputs.supervisor_command().unwrap();
        let display = command
            .get_envs()
            .find(|(name, _)| *name == "WAYLAND_SOCKET")
            .and_then(|(_, value)| value);
        assert_eq!(
            display,
            cfg!(target_os = "linux").then_some(OsStr::new("17"))
        );
    }

    #[test]
    fn column_close_steps_follow_runtimes_answer() {
        use eon_workspace_protocol::v50::{CloseTerminal, TerminalTarget, UntrackedRun};
        let mut scope = CloseScope {
            generation: "g1".into(),
            pane: "p3".into(),
            end: CloseEnd::Column,
            session: "work".into(),
            terminals: vec![CloseTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p3".into()),
                    terminal: "terminal-1".into(),
                },
                run: "run-a".into(),
                name: "fish".into(),
                busy: false,
            }],
            untracked: Vec::new(),
        };
        assert_eq!(column_close_step(&scope, 0, true), CloseStep::Consent);
        for (round, yes, code) in [
            (0, false, "not-confirmed"),
            (1, true, "close-changed"),
            (1, false, "close-changed"),
        ] {
            let CloseStep::Refuse(error) = column_close_step(&scope, round, yes) else {
                panic!("must refuse")
            };
            assert_eq!(error.code, code);
        }
        assert!(column_close_human(&scope, false).contains("would stop 1 terminal:"));
        scope.terminals[0].busy = true;
        scope.end = CloseEnd::Session;
        assert!(
            column_close_human(&scope, false)
                .contains("would close session work and stop 1 terminal:")
        );
        scope.end = CloseEnd::Window;
        scope.untracked.push(UntrackedRun {
            terminal: "terminal-9".into(),
            run: "run-b".into(),
        });
        let text = column_close_human(&scope, true);
        for part in [
            "The column changed before it was closed.",
            "would now quit the window and stop 2 terminals:",
            "pane p3    fish    running",
            "terminal-9    in no tab    running",
            "Nothing was closed.",
        ] {
            assert!(text.contains(part), "{text}");
        }
        let value = column_close_json(&scope, &failure("not-confirmed", "refused"));
        assert_eq!(value["close"]["generation"], "g1");
        assert_eq!(value["close"]["terminals"][0]["run"], "run-a");
        assert_eq!(
            value["close"]["terminals"][0]["popup"],
            serde_json::Value::Null
        );
        assert_eq!(value["close"]["untracked"][0]["run"], "run-b");
        for args in [
            vec!["column", "close", "p3"],
            vec!["column", "close", "--json", "p3", "--yes"],
        ] {
            let (action, json, yes) =
                parse_close_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                    .unwrap()
                    .unwrap();
            assert_eq!(
                action,
                Action::CloseColumn {
                    pane: "p3".into(),
                    consent: None
                }
            );
            assert_eq!(
                (json, yes),
                (args.contains(&"--json"), args.contains(&"--yes"))
            );
        }
        for args in [
            vec!["column", "close"],
            vec!["column", "close", "p3", "p4"],
            vec!["column", "close", "p3", "--yes", "--yes"],
            vec!["column", "close", "p3", "--json", "--json"],
        ] {
            assert!(
                parse_close_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                    .is_err()
            );
        }
    }

    #[test]
    fn c38_creation_requires_an_id_and_close_accepts_late_registration() {
        let pending = WorkspaceInfo {
            id: "s1".into(),
            persistent_id: None,
            registration_error: None,
            name: None,
            tab_count: 1,
            attention: None,
            recency: 0,
        };
        let error = created_persistent_id(&pending).unwrap_err();
        assert_eq!(error.code, "unrepresentable-state");
        assert!(error.detail.contains("inspect before retrying"));
        let mut confirmed = pending.clone();
        confirmed.persistent_id = Some("confirmed".into());
        assert_eq!(created_persistent_id(&confirmed).unwrap(), "confirmed");
        assert_eq!(
            confirmed_close_target(&pending, &confirmed),
            Some(SessionTarget::PersistentId("confirmed".into()))
        );
        assert_eq!(
            confirmed_close_target(&pending, &pending),
            Some(SessionTarget::Selector("s1".into()))
        );
        assert_eq!(confirmed_close_target(&confirmed, &pending), None);
        let mut other = confirmed.clone();
        other.id = "s2".into();
        assert_eq!(confirmed_close_target(&pending, &other), None);
        other.id = "s1".into();
        other.persistent_id = Some("different".into());
        assert_eq!(confirmed_close_target(&confirmed, &other), None);
    }

    #[test]
    fn help_flags_that_are_operands_or_anima_options_reach_their_commands() {
        for arguments in [
            &["run", "--", "--help"][..],
            &["edit", "--", "-h"],
            &["anima", "--help"],
            &["anima", "aquarium", "-h"],
            &["pane", "send", "p1", "--", "--help"],
            &["session", "rename", "--", "-h"],
            &["session", "switch", "--", "--help"],
            &["pane", "rename", "p1", "--", "--help"],
            &["pane", "p1", "--help"],
            &["-h,", "--help"],
        ] {
            let arguments: Vec<OsString> = arguments.iter().map(OsString::from).collect();
            assert!(command_help("0", &arguments).is_none(), "{arguments:?}");
        }
        let send = ["pane", "send", "p1", "--", "--help"].map(OsString::from);
        let send = parse_pane_send_arguments(&send).unwrap().unwrap();
        assert!(matches!(send.text, SendText::Operand(text) if text == "--help"));
    }

    #[test]
    fn a_help_row_longer_than_the_column_keeps_one_space() {
        let long = "x".repeat(50);
        assert_eq!(
            help_rows(&[(long.as_str(), "d")], false),
            format!("  {long} d\n")
        );
        assert_eq!(
            help_rows(&[("eon", "d")], false),
            format!("  eon{}d\n", " ".repeat(41))
        );
    }

    #[test]
    fn final_close_prompt_names_the_workspace_it_closes() {
        let terminals = ["p1".to_owned(), "agent".to_owned()];
        assert_eq!(
            final_close_prompt(&terminals[..1], None, None),
            (
                "Close the last tab? This quits the window and stops 1 terminal [p1]. [y/N] "
                    .to_owned(),
                "cancelled; the window stays open\n".to_owned()
            )
        );
        assert_eq!(
            final_close_prompt(&terminals, Some("my \"notes\""), None),
            (
                "Close the last tab? This closes session my \\\"notes\\\" and stops 2 \
                 terminals [p1, agent]. [y/N] "
                    .to_owned(),
                "cancelled; session my \\\"notes\\\" stays open\n".to_owned()
            )
        );
        // Decomposed Unicode names stay readable, while control and ANSI/bidi spoofing stays escaped.
        assert_eq!(
            final_close_prompt(&terminals[..1], Some("Cafe\u{0301}"), None),
            (
                "Close the last tab? This closes session Cafe\u{0301} and stops 1 terminal \
                 [p1]. [y/N] "
                    .to_owned(),
                "cancelled; session Cafe\u{0301} stays open\n".to_owned()
            )
        );
        assert_eq!(
            final_close_prompt(&terminals[..1], Some("a\n\x1b[31m\u{202E}b"), None),
            (
                "Close the last tab? This closes session a\\n\\u{1b}[31m\\u{202e}b and stops 1 \
                 terminal [p1]. [y/N] "
                    .to_owned(),
                "cancelled; session a\\n\\u{1b}[31m\\u{202e}b stays open\n".to_owned()
            )
        );
        assert_eq!(
            session_close_prompt("s2", Some("Cafe\u{0301}"), false),
            (
                "Close session s2 (Cafe\u{0301})? This stops every terminal in it. [y/N] "
                    .to_owned(),
                "cancelled; session s2 stays open\n".to_owned()
            )
        );
        // Invisible format controls (e.g. U+1BCA0) and leading non-Latin combining marks
        // (e.g. Devanagari nukta U+093C followed by an accent) stay escaped in prompts.
        assert_eq!(
            session_close_prompt("s2", Some("a\u{1BCA0}b"), false),
            (
                "Close session s2 (a\\u{1bca0}b)? This stops every terminal in it. [y/N] "
                    .to_owned(),
                "cancelled; session s2 stays open\n".to_owned()
            )
        );
        assert_eq!(
            final_close_prompt(&terminals[..1], Some("a\u{1BCA0}b"), None),
            (
                "Close the last tab? This closes session a\\u{1bca0}b and stops 1 terminal \
                 [p1]. [y/N] "
                    .to_owned(),
                "cancelled; session a\\u{1bca0}b stays open\n".to_owned()
            )
        );
        assert_eq!(
            session_close_prompt("s2", Some("\u{093C}\u{0301}foo"), false),
            (
                "Close session s2 (\\u{93c}\\u{301}foo)? This stops every terminal in it. [y/N] "
                    .to_owned(),
                "cancelled; session s2 stays open\n".to_owned()
            )
        );
        // Unassigned (Cn) scalars stay escaped in prompts even after a visible base.
        assert_eq!(
            session_close_prompt("s2", Some("a\u{0378}b"), false),
            (
                "Close session s2 (a\\u{378}b)? This stops every terminal in it. [y/N] ".to_owned(),
                "cancelled; session s2 stays open\n".to_owned()
            )
        );
        assert_eq!(
            final_close_prompt(&terminals[..1], Some("a\u{0378}b"), None),
            (
                "Close the last tab? This closes session a\\u{378}b and stops 1 terminal \
                 [p1]. [y/N] "
                    .to_owned(),
                "cancelled; session a\\u{378}b stays open\n".to_owned()
            )
        );
    }

    #[test]
    fn prompt_text_keeps_readable_names_and_escapes_spoofing_and_unattached_marks() {
        // Precomposed, decomposed, CJK, Hangul, Devanagari, Arabic, Hebrew, Thai, and emoji
        // names with attached combining marks stay readable.
        for readable in [
            "Cafe\u{0301}",
            "e\u{0301}\u{0308}",
            "中文",
            "日本語",
            "か\u{3099}",
            "ｶ\u{FF9E}ﾊ\u{FF9F}",
            "한국어",
            "\u{0915}\u{093C}",
            "\u{0628}\u{064E}",
            "\u{05D0}\u{05B0}",
            "\u{0E01}\u{0E31}",
            "\u{0995}\u{09BE}",
            "🚀",
            "\u{1F44D}\u{1F3FD}",
            "\u{2764}\u{FE0F}",
            "#\u{FE0E}",
            "1\u{FE0F}\u{20E3}",
        ] {
            assert_eq!(prompt_text(readable), readable, "{readable:?}");
        }

        // Controls, newlines, ANSI/C1 escapes, quotes, backslash, and non-ASCII spaces stay escaped.
        assert_eq!(
            prompt_text("my \"notes\" \\ O'Brien"),
            r#"my \"notes\" \\ O\'Brien"#
        );
        assert_eq!(
            prompt_text("evil\nline\r\t\0\x1b[31m\u{009B}2K\u{2028}split\u{2029}para\u{00A0}sp"),
            r"evil\nline\r\t\0\u{1b}[31m\u{9b}2K\u{2028}split\u{2029}para\u{a0}sp"
        );

        // Every invisible format control recognized by `pane_label_character` (including
        // U+1BCA0..=U+1BCA3) plus extra invisible/private-use/noncharacter scalars stay escaped
        // even after a visible base.
        let invisible_ranges = [
            0x00AD..=0x00AD,
            0x034F..=0x034F,
            0x0600..=0x0605,
            0x061C..=0x061C,
            0x06DD..=0x06DD,
            0x070F..=0x070F,
            0x0890..=0x0891,
            0x08E2..=0x08E2,
            0x115F..=0x1160,
            0x17B4..=0x17B5,
            0x180B..=0x180F,
            0x200B..=0x200F,
            0x202A..=0x202E,
            0x2060..=0x206F, // includes bidi isolates U+2066..=U+2069
            0x3164..=0x3164,
            0xE000..=0xE000,
            0xFDD0..=0xFDD0,
            0xFE00..=0xFE0D,
            0xFEFF..=0xFEFF,
            0xFFA0..=0xFFA0,
            0xFFF0..=0xFFFB,
            0xFFFE..=0xFFFF,
            0x110BD..=0x110BD,
            0x110CD..=0x110CD,
            0x13430..=0x1343F,
            0x1BCA0..=0x1BCA3,
            0x1D173..=0x1D17A,
            0xE0001..=0xE0001,
            0xE0020..=0xE007F,
            0xE0100..=0xE0100,
        ];
        for range in invisible_ranges {
            for code in range {
                let ch = char::from_u32(code).unwrap();
                assert_eq!(
                    prompt_text(&format!("a{ch}b")),
                    format!("a\\u{{{code:x}}}b"),
                    "U+{code:04X} must be escaped after a base"
                );
            }
        }

        // Unattached combining marks across scripts (Latin, Devanagari nukta U+093C, Arabic,
        // Hebrew, Thai, Bengali, Japanese, keycap, halfwidth Katakana extenders, and emoji
        // variation selectors) stay escaped and cannot serve as a base for following marks.
        assert_eq!(prompt_text("\u{0301}leading"), r"\u{301}leading");
        assert_eq!(prompt_text("a \u{0301}"), r"a \u{301}");
        assert_eq!(prompt_text("a\n\u{0301}"), r"a\n\u{301}");
        assert_eq!(prompt_text("'\u{0301}"), r"\'\u{301}");
        assert_eq!(prompt_text("\u{093C}a"), r"\u{93c}a");
        assert_eq!(prompt_text("\u{093C}\u{0301}a"), r"\u{93c}\u{301}a");
        assert_eq!(prompt_text("a \u{093C}\u{0301}"), r"a \u{93c}\u{301}");
        assert_eq!(prompt_text("\u{064E}\u{0301}"), r"\u{64e}\u{301}");
        assert_eq!(prompt_text("\u{05B0}\u{0301}"), r"\u{5b0}\u{301}");
        assert_eq!(prompt_text("\u{0E31}\u{0301}"), r"\u{e31}\u{301}");
        assert_eq!(prompt_text("\u{09BE}\u{0301}"), r"\u{9be}\u{301}");
        assert_eq!(prompt_text("\u{3099}a"), r"\u{3099}a");
        assert_eq!(prompt_text("\u{20E3}a"), r"\u{20e3}a");
        assert_eq!(prompt_text("\u{FE0E}a"), r"\u{fe0e}a");
        assert_eq!(prompt_text("\u{FE0F}a"), r"\u{fe0f}a");
        assert_eq!(prompt_text("a \u{FE0F}"), r"a \u{fe0f}");
        assert_eq!(prompt_text("\u{FF9E}\u{0301}"), r"\u{ff9e}\u{301}");
        assert_eq!(prompt_text("\u{FF9F}\u{0301}"), r"\u{ff9f}\u{301}");
        assert_eq!(prompt_text("a\u{034F}\u{0301}"), r"a\u{34f}\u{301}");
        assert_eq!(prompt_text("a\u{1BCA0}\u{0301}"), r"a\u{1bca0}\u{301}");

        // Unassigned (Cn) scalars are always escaped — standalone, after a visible base, and
        // blocking attachment of following combining marks — using the pinned Unicode 18.0.0 table.
        assert_eq!(prompt_text("\u{0378}"), r"\u{378}");
        assert_eq!(prompt_text("a\u{0378}b"), r"a\u{378}b");
        assert_eq!(prompt_text("a\u{0378}\u{0301}"), r"a\u{378}\u{301}");
        assert_eq!(prompt_text("a\u{0379}b"), r"a\u{379}b");
        assert_eq!(prompt_text("a\u{3FFFE}b"), r"a\u{3fffe}b");
        assert_eq!(prompt_text("a\u{40000}b"), r"a\u{40000}b");

        // Assigned-vs-unassigned boundaries under pinned Unicode 18.0.0 (independent of std's
        // Unicode version): U+0377/U+037A bracket unassigned U+0378..=U+0379; U+0558 (newly
        // assigned in Unicode 18.0.0) stays raw and serves as a visible base while adjacent
        // unassigned U+0557 stays escaped everywhere; U+05C8 (newly added Unicode 18.0.0
        // Grapheme_Extend mark) escapes when unattached and attaches raw after a base.
        assert_eq!(prompt_text("\u{0377}"), "\u{0377}");
        assert_eq!(prompt_text("a\u{0377}b"), "a\u{0377}b");
        assert_eq!(prompt_text("\u{037A}"), "\u{037A}");
        assert_eq!(prompt_text("\u{0556}"), "\u{0556}");
        assert_eq!(prompt_text("\u{0557}"), r"\u{557}");
        assert_eq!(prompt_text("a\u{0557}b"), r"a\u{557}b");
        assert_eq!(prompt_text("a\u{0557}\u{0301}"), r"a\u{557}\u{301}");
        assert_eq!(prompt_text("\u{0558}"), "\u{0558}");
        assert_eq!(prompt_text("a\u{0558}b"), "a\u{0558}b");
        assert_eq!(prompt_text("\u{0558}\u{0301}"), "\u{0558}\u{0301}");
        assert_eq!(prompt_text("\u{05C8}a"), r"\u{5c8}a");
        assert_eq!(prompt_text("\u{05C8}\u{0301}"), r"\u{5c8}\u{301}");
        assert_eq!(prompt_text("a\u{05C8}"), "a\u{05C8}");

        // Every non-ASCII Unicode White_Space scalar stays escaped even after a visible base.
        for code in [
            0x0085u32, 0x00A0, 0x1680, 0x2000, 0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000,
        ] {
            let ch = char::from_u32(code).unwrap();
            assert_eq!(
                prompt_text(&format!("a{ch}b")),
                format!("a\\u{{{code:x}}}b"),
                "U+{code:04X} non-ASCII whitespace must be escaped after a base"
            );
        }

        // Pinned Unicode 18.0.0 range tables remain sorted, disjoint, coalesced, and surrogate-free.
        for (name, table) in [
            (
                "UNICODE_18_0_UNASSIGNED_PLANES_0_3",
                UNICODE_18_0_UNASSIGNED_PLANES_0_3,
            ),
            ("UNICODE_18_0_GRAPHEME_EXTEND", UNICODE_18_0_GRAPHEME_EXTEND),
        ] {
            assert!(!table.is_empty(), "{name} must not be empty");
            for pair in table.windows(2) {
                assert!(
                    pair[0].0 <= pair[0].1,
                    "{name} inverted range {:?}",
                    pair[0]
                );
                assert!(
                    pair[0].1 + 1 < pair[1].0,
                    "{name} unsorted or uncoalesced ranges {:?} and {:?}",
                    pair[0],
                    pair[1]
                );
            }
            let last = table.last().unwrap();
            assert!(last.0 <= last.1, "{name} inverted tail range {last:?}");
            for &(lo, hi) in table {
                assert!(
                    hi < 0xD800 || lo > 0xDFFF,
                    "{name} range {lo:#x}..={hi:#x} overlaps UTF-16 surrogates"
                );
            }
        }
    }

    #[test]
    fn workspace_target_flags_stop_at_the_operand_separator() {
        let generation = "g1-0123456789abcdef0123456789abcdef";
        let parse = |args: &[&str]| {
            parse_workspace_target_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        let (args, target) = parse(&[
            "pane", "--window", "0000abcd", "create", "--", "echo", "--window", "literal",
        ])
        .unwrap();
        assert_eq!(target.window.as_deref(), Some("0000abcd"));
        assert_eq!(
            args,
            ["pane", "create", "--", "echo", "--window", "literal"].map(OsString::from)
        );
        assert!(
            matches!(parse_control_arguments(&args).unwrap().0, Action::CreatePane { argv, .. } if argv == [b"echo".to_vec(), b"--window".to_vec(), b"literal".to_vec()])
        );
        // `--window` takes window IDs and prefixes of 4+ characters (EON-C23).
        for id in ["default", "defa", "abcd", &generation[3..]] {
            let (_, target) = parse(&["session", "--window", id]).unwrap();
            assert_eq!(target.window.as_deref(), Some(id));
        }
        for args in [
            vec!["session", "--window"],
            vec!["session", "--window", "bad/id"],
            vec!["session", "--window", "abc"],
            vec!["session", "--window", "ABCD"],
            vec!["session", "--window", generation],
            vec!["session", "--window", "0000abcd", "--window", "0000abcd"],
        ] {
            assert_eq!(parse(&args).err().unwrap().status, 2, "{args:?}");
        }
        // `--generation` is no target flag: it stays an operand, which no
        // workspace command takes.
        let (args, target) = parse(&["session", "--generation", generation]).unwrap();
        assert!(target.window.is_none());
        assert_eq!(
            parse_control_arguments(&args).err().unwrap().status,
            2,
            "{args:?}"
        );
        assert!(!control_command(OsStr::new("--generation")));
    }

    #[test]
    fn pane_rename_preserves_literal_names_and_validates_before_connecting() {
        let parse = |args: &[&str]| {
            parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        for (args, expected, json) in [
            (
                vec!["pane", "rename", "p1", "  Review  ", "--json"],
                Some("Review"),
                true,
            ),
            (vec!["pane", "rename", "p1", "--clear"], None, false),
            (vec!["pane", "rename", "p1", "   "], None, false),
            (
                vec!["pane", "rename", "p1", "--json", "--", "--clear"],
                Some("--clear"),
                true,
            ),
            (
                vec!["pane", "rename", "p1", "--", "--json"],
                Some("--json"),
                false,
            ),
        ] {
            assert_eq!(
                parse(&args).unwrap(),
                (
                    Action::RenamePane {
                        pane: "p1".into(),
                        name: expected.map(str::to_owned),
                        tag: None,
                    },
                    json
                )
            );
        }
        for args in [
            vec!["pane", "rename", "p1"],
            vec!["pane", "rename", "p1", "name", "--clear"],
            vec!["pane", "rename", "p1", "bad\u{202e}name"],
            vec!["pane", "rename", "p1", "bad\nname"],
            vec!["pane", "rename", "p1", "--clear", "--clear"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2);
        }
        assert_eq!(
            parse(&["pane", "rename", "p1", &"x".repeat(81)])
                .unwrap_err()
                .status,
            2
        );
        assert_eq!(parse_name_output(b"--clear\0").unwrap(), "--clear");
        assert_eq!(parse_name_output(b"\0").unwrap(), "");
        assert!(parse_name_output(b"name\0extra\0").is_err());
        assert!(parse_name_output(b"name\n").is_err());
    }

    #[test]
    fn pane_tag_parser_handles_literal_values_and_empty_names() {
        let parse = |args: &[&str]| {
            let (args, target) = parse_workspace_target_arguments(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
            )?;
            assert!(target.window.is_none());
            parse_control_arguments(&args)
        };
        for tag in ["work", "", "-x", "--json", "--window", "--", "--tag"] {
            assert_eq!(
                parse(&["pane", "rename", "p1", "", "--tag", tag, "--json"]).unwrap(),
                (
                    Action::RenamePane {
                        pane: "p1".into(),
                        name: Some("".into()),
                        tag: Some(tag.into())
                    },
                    true
                )
            );
            assert_eq!(
                parse(&["pane", "rename", "--tag", tag, "--", "p1", "--clear"]).unwrap(),
                (
                    Action::RenamePane {
                        pane: "p1".into(),
                        name: Some("--clear".into()),
                        tag: Some(tag.into())
                    },
                    false
                )
            );
        }
        for args in [
            vec!["pane", "rename", "p1", "name", "--tag"],
            vec!["pane", "rename", "p1", "name", "--tag", "x", "--tag", "y"],
            vec!["pane", "rename", "p1", "--clear", "--tag", "x"],
            vec!["pane", "rename", "p1", "--tag", "x"],
            vec!["pane", "rename", "p1", "name", "--tag", "bad\u{200b}tag"],
            vec!["pane", "rename", "p1", "--", "name", "--tag", "x"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2, "{args:?}");
        }
    }

    #[test]
    fn quit_parser_takes_only_all_and_json_once() {
        let parse = |args: &[&str]| {
            parse_quit_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        for (args, expected) in [
            (vec!["quit"], (false, false)),
            (vec!["quit", "--json"], (false, true)),
            (vec!["quit", "--all"], (true, false)),
            (vec!["quit", "--all", "--json"], (true, true)),
            (vec!["quit", "--json", "--all"], (true, true)),
        ] {
            assert_eq!(parse(&args).unwrap(), Some(expected));
        }
        for args in [
            vec!["quit", "x"],
            vec!["quit", "--all", "--all"],
            vec!["quit", "--json", "--json"],
            vec!["quit", "--window", "12345678"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2);
        }
        assert_eq!(parse(&["windows"]).unwrap(), None);
        assert_eq!(parse(&[]).unwrap(), None);
    }

    #[test]
    fn command_runner_workspace_entries_parse_as_eon_commands() {
        assert!(
            runner_list("t2")
                .lines()
                .any(|line| line.ends_with("Quit this window          eon quit"))
        );
        // The longest label keeps a gap before its command.
        assert!(
            runner_list("t2")
                .lines()
                .any(|line| line.ends_with("Stop old-version windows  eon window stop previous"))
        );
        for (label, arguments, _) in RUNNER_COMMANDS {
            let arguments = runner_arguments(arguments, "t2");
            assert!(!arguments.contains(&"TAB"), "{label}");
            // The rest stand for what the popup inspects, picks or reads, as the runner fills them.
            let arguments: Vec<OsString> = arguments
                .into_iter()
                .flat_map(|argument| match argument {
                    "PANE" => vec!["p1"],
                    "THIS" | "SESSION" => vec!["s2"],
                    "NAME" => vec!["--", "--json"],
                    _ => vec![argument],
                })
                .map(OsString::from)
                .collect();
            if arguments[0] == "quit" {
                assert_eq!(
                    parse_quit_arguments(&arguments).unwrap(),
                    Some((false, false))
                );
            } else if ["session", "tab", "pane"].contains(&arguments[0].to_str().unwrap()) {
                let close = matches!(parse_close_arguments(&arguments), Ok(Some(_)));
                assert!(
                    close || parse_control_arguments(&arguments).is_ok(),
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn workspace_picker_lists_by_recency_with_the_shown_last_and_maps_its_result() {
        let workspace =
            |id: &str, name: Option<&str>, tab_count, recency, attention| WorkspaceInfo {
                id: id.into(),
                persistent_id: Some(id.into()),
                registration_error: None,
                name: name.map(Into::into),
                tab_count,
                attention,
                recency,
            };
        // Shown s1, then s3, then s2: the previous one leads, needs input only marks.
        assert_eq!(
            workspace_rows(
                &[
                    workspace("s1", Some("work"), 2, 2, Some(AttentionState::NeedsInput)),
                    workspace("s2", None, 1, 0, Some(AttentionState::NeedsInput)),
                    workspace("s3", Some("notes"), 3, 1, Some(AttentionState::Finished)),
                ],
                "s2",
            ),
            "s3\tnotes\t3 tabs\ns1\twork\t2 tabs · needs input\n\
             s2\ts2\t1 tab · needs input · shown\n"
        );
        // fzf prints the query, the --expect key (empty for Enter), then the row.
        assert_eq!(
            workspace_pick_arguments("wo\n\ns1\twork\t2 tabs\n"),
            Some(vec!["session", "switch", "s1"])
        );
        assert_eq!(
            workspace_pick_arguments("\n\ns2\ts2\t1 tab · shown\n"),
            Some(vec!["session", "switch", "s2"])
        );
        assert_eq!(
            workspace_pick_arguments("wo\nctrl-x\ns1\twork\t2 tabs\n"),
            Some(vec!["session", "close", "s1"])
        );
        // fzf exits 1 on Ctrl+X without a row, so it never prints this.
        assert_eq!(workspace_pick_arguments("zz\nctrl-x\n"), None);
        // Closing another workspace always lists again, and the next listing reports any
        // failure; closing the picker's own s1 lists again only while s1 is still live (it was
        // declined); a failure or any other command ends the picker.
        let live = |ids: &[&str]| {
            Some(
                ids.iter()
                    .map(|id| workspace(id, None, 1, 0, None))
                    .collect::<Vec<_>>(),
            )
        };
        let close = |id| ["session", "close", id];
        assert_eq!(workspace_pick_end(&close("s1"), 0, "s1", || None), Some(0));
        assert_eq!(
            workspace_pick_end(&close("s1"), 0, "s1", || live(&["s2"])),
            Some(0)
        );
        assert_eq!(
            workspace_pick_end(&close("s1"), 0, "s1", || live(&["s1"])),
            None
        );
        assert_eq!(
            workspace_pick_end(&close("s2"), 0, "s1", || live(&["s1"])),
            None
        );
        assert_eq!(workspace_pick_end(&close("s2"), 0, "s1", || None), None);
        assert_eq!(
            workspace_pick_end(&close("s2"), 1, "s1", || live(&["s1"])),
            Some(1)
        );
        assert_eq!(
            workspace_pick_end(&["session", "switch", "s2"], 0, "s1", || live(&["s1"])),
            Some(0)
        );
        assert_eq!(
            workspace_pick_arguments(" --fresh \n"),
            Some(vec!["session", "new", "--name", " --fresh "])
        );
        assert_eq!(workspace_pick_arguments("  \n"), None);
        assert_eq!(workspace_pick_arguments(""), None);
    }

    #[test]
    fn popup_show_parser_accepts_only_create_arguments() {
        let parse = |args: &[&str]| {
            parse_popup_show_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        let show = parse(&["popup", "show", "files"]).unwrap().unwrap();
        assert_eq!(
            show.action("t3"),
            Action::InvokePopup {
                tab: "t3".into(),
                entry: "files".into(),
                expected_instance: None,
                intent: InvokeIntent::Show,
            }
        );
        for args in [
            vec!["popup", "show", "files", "--tab", "t2", "--json"],
            vec!["popup", "show", "--json", "--tab", "t2", "files"],
        ] {
            let show = parse(&args).unwrap().unwrap();
            assert!(show.json);
            assert_eq!(
                show.action("t1"),
                Action::InvokePopup {
                    tab: "t2".into(),
                    entry: "files".into(),
                    expected_instance: None,
                    intent: InvokeIntent::Show,
                }
            );
        }
        for args in [
            vec!["popup", "show"],
            vec!["popup", "show", "--json"],
            vec!["popup", "show", ""],
            vec!["popup", "show", "two words"],
            vec!["popup", "show", "files", "extra"],
            vec!["popup", "show", "files", "--tab"],
            vec!["popup", "show", "files", "--tab", "t0"],
            vec!["popup", "show", "files", "--tab", "t02"],
            vec!["popup", "show", "files", "--tab", "p2"],
            vec!["popup", "show", "files", "--tab", "t2", "--tab", "t3"],
            vec!["popup", "show", "files", "--json", "--json"],
            vec!["popup", "show", "files", "--no-focus"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2, "{args:?}");
        }
        assert_eq!(
            parse(&["popup", "show", &"x".repeat(65)])
                .unwrap_err()
                .status,
            2
        );
        let invalid = [
            OsString::from("popup"),
            OsString::from("show"),
            OsString::from_vec(vec![0xff]),
        ];
        assert_eq!(parse_popup_show_arguments(&invalid).unwrap_err().status, 2);
        assert_eq!(parse(&["popup", "hide", "u2"]).unwrap(), None);
        assert_eq!(parse(&["session"]).unwrap(), None);
        for words in [vec!["popup"], vec!["popup", "show"], vec!["focus"]] {
            let args = words.iter().map(OsString::from).collect::<Vec<_>>();
            let help = command_help_text(&args, false).unwrap();
            assert!(help.contains(POPUP_HELP), "{help}");
        }
        assert!(help_text("0", false).contains(POPUP_HELP));
    }

    #[test]
    fn popup_hide_parser_takes_one_popup_and_json_once() {
        let parse = |args: &[&str]| {
            parse_popup_hide_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["popup", "hide", "u2"]).unwrap(),
            Some(("u2".into(), false))
        );
        assert_eq!(
            parse(&["popup", "hide", "u2", "--json"]).unwrap(),
            Some(("u2".into(), true))
        );
        for args in [
            vec!["popup", "hide", "--json"],
            vec!["popup", "hide", "-u2"],
            vec!["popup", "hide"],
            vec!["popup", "hide", "u2", "--json", "--json"],
            vec!["popup", "show", "u2"],
            vec!["popup"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2, "{args:?}");
        }
        assert_eq!(parse(&["pane", "read", "u2"]).unwrap(), None);
    }

    #[test]
    fn sidebar_parser_takes_one_verb_and_json_once() {
        let parse = |args: &[&str]| {
            parse_sidebar_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        for (word, verb) in [
            ("show", SidebarVerb::Show),
            ("hide", SidebarVerb::Hide),
            ("toggle", SidebarVerb::Toggle),
        ] {
            assert_eq!(parse(&["sidebar", word]).unwrap(), Some((verb, false)));
            assert_eq!(
                parse(&["sidebar", word, "--json"]).unwrap(),
                Some((verb, true))
            );
        }
        for args in [
            vec!["sidebar"],
            vec!["sidebar", "--json"],
            vec!["sidebar", "open"],
            vec!["sidebar", "hide", "show"],
            vec!["sidebar", "--json", "hide"],
            vec!["sidebar", "hide", "--json", "--json"],
        ] {
            assert_eq!(parse(&args).unwrap_err().status, 2, "{args:?}");
        }
        assert_eq!(parse(&["popup", "hide", "u2"]).unwrap(), None);
    }

    #[test]
    fn tab_directory_parser_preserves_one_opaque_path_argument() {
        let directory = OsString::from_vec(b"/tmp/eon-\xff".to_vec());
        assert_eq!(
            parse_control_arguments(&[
                "tab".into(),
                "directory".into(),
                "t2".into(),
                "--json".into(),
                "--".into(),
                directory,
            ])
            .unwrap(),
            (
                Action::Workspace(WorkspaceAction::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/tmp/eon-\xff".to_vec(),
                }),
                true,
            )
        );
        assert!(
            parse_control_arguments(&[
                "tab".into(),
                "directory".into(),
                "t2".into(),
                "/tmp/eon".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn tab_create_directory_parser_resolves_paths_and_preserves_commands() {
        let parse = |arguments: &[&str]| {
            parse_control_arguments(&arguments.iter().map(OsString::from).collect::<Vec<_>>())
        };
        for (flags, focus, json) in [
            (vec!["--dir", "project"], true, false),
            (
                vec!["--json", "--dir", "project", "--no-focus"],
                false,
                true,
            ),
            (
                vec!["--no-focus", "--json", "--dir", "project"],
                false,
                true,
            ),
            (
                vec!["--dir", "project", "--no-focus", "--json"],
                false,
                true,
            ),
        ] {
            let mut args = vec!["tab", "create"];
            args.extend(flags);
            assert_eq!(
                parse(&args).unwrap(),
                (
                    Action::LaunchTab {
                        directory: env::current_dir()
                            .unwrap()
                            .join("project")
                            .into_os_string()
                            .into_vec(),
                        argv: vec![],
                        focus,
                    },
                    json,
                )
            );
        }
        assert_eq!(
            parse_control_arguments(&[
                "tab".into(),
                "create".into(),
                "--dir".into(),
                OsString::from_vec(b"/tmp/\xff".to_vec()),
                "--".into(),
                "hx".into(),
                OsString::from_vec(b"notes \xff".to_vec()),
                "--json".into(),
            ])
            .unwrap(),
            (
                Action::LaunchTab {
                    directory: b"/tmp/\xff".to_vec(),
                    argv: vec![b"hx".to_vec(), b"notes \xff".to_vec(), b"--json".to_vec()],
                    focus: true,
                },
                false
            ),
        );
        let (args, target) = parse_workspace_target_arguments(
            &[
                "tab", "create", "--window", "0000abcd", "--dir", "/", "--", "hx", "--window",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(target.window.as_deref(), Some("0000abcd"));
        assert_eq!(
            parse_control_arguments(&args).unwrap(),
            (
                Action::LaunchTab {
                    directory: b"/".to_vec(),
                    argv: vec![b"hx".to_vec(), b"--window".to_vec()],
                    focus: true,
                },
                false
            )
        );
        let (args, target) = parse_workspace_target_arguments(
            &["tab", "create", "--dir", "--window"].map(OsString::from),
        )
        .unwrap();
        assert_eq!(target.window, None);
        assert!(parse_control_arguments(&args).is_ok());
        for args in [vec!["tab", "create"], vec!["tab", "create", "--json"]] {
            assert_eq!(
                parse(&args).unwrap(),
                (
                    Action::Workspace(WorkspaceAction::CreateTab),
                    args.contains(&"--json"),
                )
            );
        }
        for args in [
            vec!["--no-focus"],
            vec!["--", "hx"],
            vec!["--dir"],
            vec!["--dir", ""],
            vec!["--dir", "--", "hx"],
            vec!["--dir", "/", "--dir", "/"],
            vec!["--json", "--json", "--dir", "/"],
            vec!["--no-focus", "--no-focus", "--dir", "/"],
            vec!["--unknown"],
            vec!["--workspace", "s1", "--dir", "/"],
        ] {
            let mut command = vec!["tab", "create"];
            command.extend(args);
            assert_eq!(parse(&command).unwrap_err().status, 2, "{command:?}");
        }
        for args in [
            vec!["tab", "create", "--"],
            vec!["tab", "create", "--", ""],
            vec!["tab", "create", "--dir", "/", "--"],
            vec!["tab", "create", "--dir", "/", "--", ""],
        ] {
            let error = parse(&args).unwrap_err();
            assert_eq!(error.status, 2, "{args:?}");
            assert_eq!(
                error.message, "eon tab create -- needs a command",
                "{args:?}"
            );
        }
    }

    #[test]
    fn command_pane_parser_passes_the_argv_through() {
        let prompt = OsString::from_vec(b"prime \xff".to_vec());
        assert_eq!(
            parse_control_arguments(&[
                "pane".into(),
                "create".into(),
                "--json".into(),
                "--".into(),
                "claude".into(),
                prompt,
                "--".into(),
            ])
            .unwrap(),
            (
                Action::CreatePane {
                    tab: None,
                    argv: vec![b"claude".to_vec(), b"prime \xff".to_vec(), b"--".to_vec()],
                    focus: true,
                },
                true,
            )
        );
        let parse = |arguments: &[&str]| {
            let arguments: Vec<OsString> = arguments.iter().map(Into::into).collect();
            parse_control_arguments(&arguments)
        };
        let pane = |tab: Option<&str>, argv: &[&str], focus| Action::CreatePane {
            tab: tab.map(Into::into),
            argv: argv.iter().map(|word| word.as_bytes().to_vec()).collect(),
            focus,
        };
        let unfocused = pane(None, &[], false);
        for (arguments, expected) in [
            (&["pane", "create"][..], (pane(None, &[], true), false)),
            (
                &["pane", "create", "--no-focus"],
                (unfocused.clone(), false),
            ),
            (
                &["pane", "create", "--no-focus", "--json"],
                (unfocused.clone(), true),
            ),
            (
                &["pane", "create", "--json", "--no-focus"],
                (unfocused.clone(), true),
            ),
            (&["pane", "--json", "create"], (pane(None, &[], true), true)),
            (&["--no-focus", "pane", "create"], (unfocused, false)),
            (
                &["pane", "create", "--no-focus", "--", "sh"],
                (pane(None, &["sh"], false), false),
            ),
            // `--tab` names the destination tab; it changes nothing else.
            (
                &["pane", "create", "--tab", "t3"],
                (pane(Some("t3"), &[], true), false),
            ),
            (
                &[
                    "pane",
                    "create",
                    "--tab",
                    "t3",
                    "--no-focus",
                    "--json",
                    "--",
                    "sh",
                    "-c",
                    "sleep 30",
                ],
                (pane(Some("t3"), &["sh", "-c", "sleep 30"], false), true),
            ),
            (
                &[
                    "pane", "create", "--json", "--tab", "t12", "--", "x", "--tab", "t9",
                ],
                (pane(Some("t12"), &["x", "--tab", "t9"], true), true),
            ),
        ] {
            assert_eq!(parse(arguments).unwrap(), expected, "{arguments:?}");
        }
        for arguments in [
            &["pane", "create", "--"][..],
            &["pane", "create", "--", ""],
            &["pane", "create", "--no-focus", "--no-focus"],
            &["pane", "create", "--json", "--json", "--", "sh"],
            &["pane", "create", "sh"],
            &["session", "--no-focus"],
            &["pane", "create", "--tab"],
            &["pane", "create", "--tab", "--", "sh"],
            &["pane", "create", "--tab", "t0"],
            &["pane", "create", "--tab", "t03"],
            &["pane", "create", "--tab", "p3"],
            &["pane", "create", "--tab", "t2", "--tab", "t3"],
            &["pane", "create", "--tab", "t2", "--tab", "t2"],
        ] {
            assert!(parse(arguments).is_err(), "{arguments:?}");
        }
    }

    /// Each launch flag reaches the fresh window once, in either order, and
    /// anything else is invalid before a window starts (EON-C18).
    #[test]
    fn launch_flags_parse_once_in_either_order_and_fail_before_starting() {
        let arguments = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
        let parsed = arguments(&["--no-anima", "--dir", "a b", "--", "x"]);
        let (options, rest) = parse_launch_options(&parsed).unwrap();
        assert_eq!(
            options.directory,
            Some(env::current_dir().unwrap().join("a b"))
        );
        assert!(options.no_anima);
        assert_eq!(rest, arguments(&["--", "x"]));
        let parsed = arguments(&["--dir", "/d", "--no-anima", "--dir", "/e"]);
        let (options, rest) = parse_launch_options(&parsed).unwrap();
        assert_eq!(options.directory.as_deref(), Some(Path::new("/d")));
        assert_eq!(rest, arguments(&["--dir", "/e"]));
        let (options, rest) = parse_launch_options(&[]).unwrap();
        assert!(options.directory.is_none() && !options.no_anima && rest.is_empty());

        let inputs = crate::fixtures::inputs();
        for words in [
            &["--dir"][..],
            &["--dir", "--"],
            &["--dir", ""],
            &["--no-anima", "--dir", "--", "x"],
            &["run", "--dir", "--", "x"],
            &["run", "--dir", ""],
            &["window", "new", "--dir", "--"],
            &["window", "new", "--no-anima", "--dir", ""],
            &["--no-anima", "--no-anima"],
            &["--dir", "/", "--dir", "/"],
            &["--dir", "/", "extra"],
            &["--no-anima", "run"],
            &["run", "--no-anima", "--"],
            &["run", "--dir", "/", "x"],
            &["window", "new", "--json"],
            &["window", "new", "--no-anima", "--", "x"],
        ] {
            let error = execute(&inputs, arguments(words)).unwrap_err();
            assert_eq!(error.status, 2, "{words:?}: {}", error.message);
        }
        let missing = env::temp_dir().join(format!("eon-missing-launch-{}", std::process::id()));
        let missing = missing.to_str().unwrap();
        for words in [
            &["--dir", missing][..],
            &["run", "--dir", missing],
            &["window", "new", "--no-anima", "--dir", missing],
        ] {
            let error = execute(&inputs, arguments(words)).unwrap_err();
            assert_eq!(error.status, 1, "{words:?}");
            assert!(
                error
                    .message
                    .contains("cannot inspect Eon launch directory"),
                "{words:?}: {}",
                error.message
            );
        }
    }

    #[test]
    fn edit_parser_sends_one_absolute_opaque_file() {
        let file = OsString::from_vec(b"/tmp/eon-\xff".to_vec());
        assert_eq!(
            parse_control_arguments(&["edit".into(), "--json".into(), "--".into(), file]).unwrap(),
            (
                Action::Edit {
                    file: b"/tmp/eon-\xff".to_vec(),
                },
                true,
            )
        );
        let (Action::Edit { file }, false) =
            parse_control_arguments(&["edit", "--", "a.txt"].map(Into::into)).unwrap()
        else {
            panic!("edit parses as Edit");
        };
        assert_eq!(
            OsString::from_vec(file),
            env::current_dir().unwrap().join("a.txt").into_os_string()
        );
        for arguments in [
            &["edit", "a.txt"][..],
            &["edit", "--"],
            &["edit", "--", ""],
            &["edit", "--", "a", "b"],
        ] {
            let arguments: Vec<OsString> = arguments.iter().map(Into::into).collect();
            assert!(
                parse_control_arguments(&arguments).is_err(),
                "{arguments:?}"
            );
        }
    }

    #[test]
    fn reveal_parser_keeps_one_path_and_resolves_it_against_the_working_directory() {
        let path = "-雪 'quotes' \"double\"\nnext line";
        let parse = |words: &[&str]| {
            let arguments: Vec<OsString> = words.iter().map(Into::into).collect();
            let (arguments, _) = parse_workspace_target_arguments(&arguments).unwrap();
            parse_control_arguments(&arguments)
        };
        assert_eq!(
            parse(&["files", "reveal", "--tab", "t7", "--json", "--", path]).unwrap(),
            (
                Action::RevealFiles {
                    tab: Some("t7".into()),
                    path: env::current_dir()
                        .unwrap()
                        .join(path)
                        .into_os_string()
                        .into_string()
                        .unwrap(),
                },
                true,
            )
        );
        assert_eq!(
            parse(&["files", "reveal", "/a path", "--json"]).unwrap(),
            (
                Action::RevealFiles {
                    tab: None,
                    path: "/a path".into(),
                },
                true,
            )
        );
        let help = command_help_text(&["files".into(), "reveal".into()], false).unwrap();
        assert!(
            help.contains("files reveal PATH [--tab TAB] [--json]"),
            "{help}"
        );
    }

    #[test]
    fn reveal_parser_refuses_ambiguous_or_unrepresentable_requests() {
        let long_path = format!(
            "/{}",
            "x".repeat(eon_workspace_protocol::v50::MAX_DIRECTORY_BYTES)
        );
        for words in [
            &["files", "reveal"][..],
            &["files", "reveal", "--"],
            &["files", "reveal", "--", ""],
            &["files", "reveal", "-name"],
            &["files", "reveal", "one", "--", "two"],
            &["files", "reveal", "one", "two"],
            &["files", "reveal", "--json", "--json", "name"],
            &["files", "reveal", "--tab", "t1", "--tab", "t7", "name"],
            &["files", "reveal", "--tab", "t01", "name"],
            &["files", "reveal", "name", "--tab"],
            &["files", "reveal", "/nul\0name"],
            &["files", "reveal", &long_path],
        ] {
            let arguments: Vec<OsString> = words.iter().map(Into::into).collect();
            let error = parse_control_arguments(&arguments).unwrap_err();
            assert_eq!(error.status, 2, "{words:?}");
        }
        let arguments = [
            "files".into(),
            "reveal".into(),
            "--".into(),
            OsString::from_vec(b"/non-utf-8-\xff".to_vec()),
        ];
        assert_eq!(parse_control_arguments(&arguments).unwrap_err().status, 2);
    }

    #[test]
    fn reorder_parser_accepts_only_each_workspace_axis() {
        for (scope, name, direction) in [
            ("tab", "left", Direction::Left),
            ("tab", "right", Direction::Right),
            ("pane", "up", Direction::Up),
            ("pane", "down", Direction::Down),
        ] {
            assert_eq!(
                parse_control_arguments(&[scope, "move", name, "--json"].map(Into::into)).unwrap(),
                (Action::Workspace(WorkspaceAction::Move(direction)), true)
            );
        }
        for (name, side) in [("left", Side::Left), ("right", Side::Right)] {
            assert_eq!(
                parse_control_arguments(&["pane", "move", name].map(Into::into)).unwrap(),
                (Action::MovePaneSideways(side), false)
            );
        }
        for name in ["up", "down"] {
            assert!(parse_control_arguments(&["tab", "move", name].map(Into::into)).is_err());
        }
    }

    #[test]
    fn tab_switch_takes_a_tab_and_focus_refers_tabs_to_it() {
        let parse = |args: &[&str]| {
            parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        let focus = |id: &str| Action::Workspace(WorkspaceAction::FocusId(id.into()));
        assert_eq!(
            parse(&["tab", "switch", "t2"]).unwrap(),
            (focus("t2"), false)
        );
        assert_eq!(
            parse(&["tab", "switch", "--json", "t12"]).unwrap(),
            (focus("t12"), true)
        );
        assert_eq!(parse(&["focus", "p2"]).unwrap(), (focus("p2"), false));
        assert_eq!(
            parse(&["focus", "u2", "--json"]).unwrap(),
            (focus("u2"), true)
        );
        for args in [
            &["tab", "switch"][..],
            &["tab", "switch", "p2"],
            &["tab", "switch", "t0"],
            &["tab", "switch", "t02"],
            &["tab", "switch", "s1"],
            &["tab", "switch", "t1", "t2"],
            &["tab", "switch", "t1", "--no-focus"],
        ] {
            let error = parse(args).unwrap_err();
            assert_eq!(
                (error.status, error.message),
                (2, EON_USAGE.into()),
                "{args:?}"
            );
        }
        let error = parse(&["focus", "t2", "--json"]).unwrap_err();
        assert_eq!(error.status, 2);
        assert!(
            error.message.contains("`eon tab switch t2`"),
            "{}",
            error.message
        );
    }

    #[test]
    fn close_parsers_require_one_stable_identity() {
        let parse = |args: &[&str]| {
            parse_close_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
        };
        let tab = |id: &str| Action::Workspace(WorkspaceAction::CloseTab { tab: id.into() });
        let pane = |id: &str| Action::ClosePane { pane: id.into() };
        for (args, expected) in [
            (&["tab", "close", "t2"][..], (tab("t2"), false, false)),
            (&["tab", "close", "t2", "--json"], (tab("t2"), true, false)),
            (&["tab", "close", "t2", "--yes"], (tab("t2"), false, true)),
            (
                &["tab", "close", "--yes", "t2", "--json"],
                (tab("t2"), true, true),
            ),
            (&["pane", "close", "p3"], (pane("p3"), false, false)),
            (&["pane", "close", "p3", "--yes"], (pane("p3"), false, true)),
            (
                &["pane", "close", "--json", "--yes", "p3"],
                (pane("p3"), true, true),
            ),
        ] {
            assert_eq!(parse(args).unwrap(), Some(expected), "{args:?}");
        }
        assert_eq!(parse(&["tab", "create"]).unwrap(), None);
        assert_eq!(parse(&["pane", "create"]).unwrap(), None);
        for args in [
            &["tab", "close"][..],
            &["pane", "close"],
            &["tab", "close", "--yes"],
            &["pane", "close", "--json"],
            &["tab", "close", "t1", "t2"],
            &["pane", "close", "p1", "p2"],
            &["tab", "close", "-t1"],
            &["pane", "close", "--", "p1"],
            &["tab", "close", "t1", "--yes", "--yes"],
            &["pane", "close", "p1", "--json", "--json"],
            &["tab", "close", "t1", "--no-focus"],
            &["pane", "close", "p1", "--no-focus"],
            &["tab", "close", "t1", "-y"],
            &["pane", "close", "p1", "--force"],
            &["tab", "close", "t1", "--generation", "g1"],
        ] {
            let error = parse(args).unwrap_err();
            assert_eq!(
                (error.status, error.message),
                (2, EON_USAGE.into()),
                "{args:?}"
            );
        }
        let non_utf8 = OsString::from_vec(vec![b't', 0xff]);
        assert_eq!(
            parse_close_arguments(&["tab".into(), "close".into(), non_utf8])
                .unwrap_err()
                .status,
            2
        );
        // Only closes take --yes.
        for args in [
            &["tab", "create", "--yes"][..],
            &["tab", "move", "left", "--yes"],
            &["pane", "create", "--yes"],
            &["session", "--yes"],
        ] {
            let arguments = args.iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(parse_close_arguments(&arguments).unwrap(), None, "{args:?}");
            let error = parse_control_arguments(&arguments).unwrap_err();
            assert_eq!(
                (error.status, error.message),
                (2, EON_USAGE.into()),
                "{args:?}"
            );
        }
        // Target flags are taken before the close parser sees its action.
        let (arguments, _) = parse_workspace_target_arguments(
            &["tab", "--window", "0000abcd", "close", "t2", "--yes"].map(OsString::from),
        )
        .unwrap();
        assert_eq!(
            parse_close_arguments(&arguments).unwrap(),
            Some((
                Action::Workspace(WorkspaceAction::CloseTab { tab: "t2".into() }),
                false,
                true
            ))
        );
    }

    #[test]
    fn close_question_is_target_scoped_and_conservative() {
        use eon_workspace_protocol::v50::{
            BusyTerminal, BusyTerminals, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey,
            Sidebar, Tab, TargetId, TerminalTarget,
        };
        let pane = |id: &str, session: &str| Pane {
            id: id.into(),
            label: id.into(),
            pinned_name: None,
            tag: None,
            session: session.into(),
            endpoint: b"/runtime/orbit.sock".to_vec(),
            live: true,
            attention: None,
        };
        let busy_item = |tab: &str, id: TargetId, session: &str, name: &str| BusyTerminal {
            target: TerminalTarget {
                workspace: "s1".into(),
                tab: tab.into(),
                id,
                terminal: session.into(),
            },
            name: name.into(),
        };
        let base_snapshot = || Snapshot {
            focus: None,
            input_lock: crate::fixtures::input_lock(),
            input_locked: false,
            released_shortcuts: Default::default(),
            generation: "g1-00000000000000000000000000000001".into(),
            created_build: "g1-00000000000000000000000000000001".into(),
            active_build: "g1-00000000000000000000000000000001".into(),
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: None,
                tab_count: 2,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: vec![PopupEntry {
                id: "git".into(),
                label: "Git".into(),
                shortcut: PopupKey::Unbound,
            }],
            tabs: vec![
                Tab {
                    id: "t1".into(),
                    directory: b"/".to_vec(),
                    pending: false,
                    columns: vec![Column {
                        panes: vec![pane("p1", "s1")],
                        expanded: "p1".into(),
                    }],
                    selected_pane: Some("p1".into()),
                    popups: Vec::new(),
                    selected_popup: None,
                    attention_rollup: None,
                },
                Tab {
                    id: "t2".into(),
                    directory: b"/".to_vec(),
                    pending: false,
                    columns: vec![Column {
                        panes: vec![pane("p2", "s2"), pane("p3", "s3")],
                        expanded: "p2".into(),
                    }],
                    selected_pane: Some("p2".into()),
                    popups: vec![Popup {
                        id: "u1".into(),
                        entry: "git".into(),
                        session: "s4".into(),
                        endpoint: b"/runtime/orbit.sock".to_vec(),
                        attention: None,
                    }],
                    selected_popup: None,
                    attention_rollup: None,
                },
            ],
            codex_quota: None,
            claude_quota: None,
            google_quota: None,
            google_quota_failure: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            sidebar: Sidebar::Off,
            attention_queue: Vec::new(),
            running: None,
            busy: Some(BusyTerminals {
                terminals: vec![busy_item("t1", TargetId::Pane("p1".into()), "s1", "p1")],
                omitted: 0,
            }),
        };

        let close_t2 = Action::Workspace(WorkspaceAction::CloseTab { tab: "t2".into() });
        let close_p2 = Action::ClosePane { pane: "p2".into() };

        // Known-idle background tab or pane (only t1's s1 is busy) returns Ok(None).
        let snap = base_snapshot();
        assert_eq!(close_question(&snap, &close_t2, true, true), Ok(None));
        assert_eq!(close_question(&snap, &close_p2, false, true), Ok(None));

        // Known-busy background tab or pane returns exact question and cancel note.
        let mut snap = base_snapshot();
        snap.busy = Some(BusyTerminals {
            terminals: vec![
                busy_item("t2", TargetId::Pane("p2".into()), "s2", "p2"),
                busy_item("t2", TargetId::Popup("u1".into()), "s4", "Git"),
            ],
            omitted: 0,
        });
        assert_eq!(
            close_question(&snap, &close_t2, true, true),
            Ok(Some((
                "Close tab t2? This stops 3 terminals; 2 still running: p2, Git. [y/N] ".into(),
                "cancelled; tab t2 stays open\n".into(),
            )))
        );
        assert_eq!(
            close_question(&snap, &close_p2, true, true),
            Ok(Some((
                "Close pane p2? This stops 1 terminal; 1 still running: p2. [y/N] ".into(),
                "cancelled; pane p2 stays open\n".into(),
            )))
        );

        // snapshot.busy == None -> busy state unknown.
        let mut snap = base_snapshot();
        snap.busy = None;
        assert_eq!(
            close_question(&snap, &close_t2, true, false),
            Ok(Some((
                "Close tab t2? This stops 3 terminals; busy state unknown: p2, p3, Git. [y/N] "
                    .into(),
                "cancelled; tab t2 stays open\n".into(),
            )))
        );

        // A capped list (omitted > 0) is unknown for every target, even one it
        // lists as busy, and names all of the target's terminals.
        let mut snap = base_snapshot();
        snap.busy = Some(BusyTerminals {
            terminals: vec![busy_item("t2", TargetId::Pane("p2".into()), "s2", "p2")],
            omitted: 1,
        });
        assert_eq!(
            close_question(&snap, &close_t2, true, false),
            Ok(Some((
                "Close tab t2? This stops 3 terminals; busy state unknown: p2, p3, Git. [y/N] "
                    .into(),
                "cancelled; tab t2 stays open\n".into(),
            )))
        );
        assert_eq!(
            close_question(&snap, &close_p2, true, false),
            Ok(Some((
                "Close pane p2? This stops 1 terminal; busy state unknown: p2. [y/N] ".into(),
                "cancelled; pane p2 stays open\n".into(),
            )))
        );
        // Even a capped list that names only another tab's terminal.
        snap.busy = Some(BusyTerminals {
            terminals: vec![busy_item("t1", TargetId::Pane("p1".into()), "s1", "p1")],
            omitted: 3,
        });
        assert_eq!(
            close_question(
                &snap,
                &Action::ClosePane { pane: "p3".into() },
                false,
                false
            ),
            Ok(Some((
                "Close pane p3? This stops 1 terminal; busy state unknown: p3. [y/N] ".into(),
                "cancelled; pane p3 stays open\n".into(),
            )))
        );

        // The target is found by ID whichever tab is active, and a caller's
        // own busy pane counts when it is the target.
        let mut snap = base_snapshot();
        snap.active_tab = "t2".into();
        assert_eq!(close_question(&snap, &close_t2, false, false), Ok(None));
        assert_eq!(
            close_question(&snap, &Action::ClosePane { pane: "p1".into() }, true, false),
            Ok(Some((
                "Close pane p1? This stops 1 terminal; 1 still running: p1. [y/N] ".into(),
                "cancelled; pane p1 stays open\n".into(),
            )))
        );
        // Other actions ask nothing here.
        assert_eq!(
            close_question(
                &snap,
                &Action::Workspace(WorkspaceAction::CreateTab),
                true,
                true
            ),
            Ok(None)
        );

        // A stale ID, or one in another workspace, which the inspection of
        // the addressed workspace never lists, fails as Runtime does.
        assert_eq!(
            close_question(
                &snap,
                &Action::Workspace(WorkspaceAction::CloseTab { tab: "t9".into() }),
                true,
                false,
            ),
            Err(failure("unknown-tab", "tab t9 is not live"))
        );
        assert_eq!(
            close_question(&snap, &Action::ClosePane { pane: "p9".into() }, true, false),
            Err(failure("unknown-pane", "pane p9 is not live"))
        );

        // A covering popup refuses a pane close without a caller, and the
        // tab's last pane even with one, before any question.
        let mut snap = base_snapshot();
        snap.busy = None;
        snap.tabs[1].selected_popup = Some("u1".into());
        assert_eq!(
            close_question(&snap, &close_p2, false, false),
            Err(failure(
                "unavailable",
                "close the popup before closing a pane"
            ))
        );
        assert_eq!(
            close_question(&snap, &close_p2, true, false),
            Ok(Some((
                "Close pane p2? This stops 1 terminal; busy state unknown: p2. [y/N] ".into(),
                "cancelled; pane p2 stays open\n".into(),
            )))
        );
        snap.tabs[0].selected_popup = Some("u1".into());
        assert_eq!(
            close_question(&snap, &Action::ClosePane { pane: "p1".into() }, true, false),
            Err(failure(
                "unavailable",
                "close the popup before closing the tab's last pane"
            ))
        );
        // A covered tab still closes by its ID.
        assert!(matches!(
            close_question(&snap, &close_t2, false, false),
            Ok(Some(_))
        ));
    }

    #[test]
    fn close_question_combines_busy_and_final_warnings_once() {
        use eon_workspace_protocol::v50::{
            BusyTerminal, BusyTerminals, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey,
            Sidebar, Tab, TargetId, TerminalTarget,
        };
        let mut snap = Snapshot {
            focus: None,
            input_lock: crate::fixtures::input_lock(),
            input_locked: false,
            released_shortcuts: Default::default(),
            generation: "g1-00000000000000000000000000000001".into(),
            created_build: "g1-00000000000000000000000000000001".into(),
            active_build: "g1-00000000000000000000000000000001".into(),
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
            }],
            active_tab: "t1".into(),
            geometry: PopupGeometry {
                side_margin: 0.0,
                vertical_margin: 0.0,
            },
            entries: vec![PopupEntry {
                id: "agent".into(),
                label: "Agent".into(),
                shortcut: PopupKey::Unbound,
            }],
            tabs: vec![Tab {
                id: "t1".into(),
                directory: b"/".to_vec(),
                pending: false,
                columns: vec![Column {
                    panes: vec![Pane {
                        id: "p1".into(),
                        label: "p1".into(),
                        pinned_name: None,
                        tag: None,
                        session: "s1".into(),
                        endpoint: b"/runtime/orbit.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                    expanded: "p1".into(),
                }],
                selected_pane: Some("p1".into()),
                popups: Vec::new(),
                selected_popup: None,
                attention_rollup: None,
            }],
            codex_quota: None,
            claude_quota: None,
            google_quota: None,
            google_quota_failure: None,
            codex_quota_failure: None,
            claude_quota_failure: None,
            sidebar: Sidebar::Off,
            attention_queue: Vec::new(),
            running: None,
            busy: Some(BusyTerminals {
                terminals: Vec::new(),
                omitted: 0,
            }),
        };
        let close_t1 = Action::Workspace(WorkspaceAction::CloseTab { tab: "t1".into() });

        let close_p1 = Action::ClosePane { pane: "p1".into() };

        // An idle final close asks only with the warning, for the tab or its
        // last pane, in the existing words.
        let idle_final = Ok(Some((
            "Close the last tab? This quits the window and stops 1 terminal [p1]. [y/N] ".into(),
            "cancelled; the window stays open\n".into(),
        )));
        assert_eq!(close_question(&snap, &close_t1, true, true), idle_final);
        assert_eq!(close_question(&snap, &close_p1, true, true), idle_final);
        assert_eq!(close_question(&snap, &close_t1, true, false), Ok(None));
        assert_eq!(close_question(&snap, &close_p1, false, false), Ok(None));

        // A busy final close asks once, with the final consequence, whether
        // or not the warning applies; its last pane stops the tab's popups too.
        snap.tabs[0].popups.push(Popup {
            id: "u1".into(),
            entry: "agent".into(),
            session: "s2".into(),
            endpoint: b"/runtime/orbit.sock".to_vec(),
            attention: None,
        });
        snap.busy = Some(BusyTerminals {
            terminals: vec![BusyTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Pane("p1".into()),
                    terminal: "s1".into(),
                },
                name: "p1".into(),
            }],
            omitted: 0,
        });
        let busy_final = Ok(Some((
            "Close the last tab? This quits the window and stops 2 terminals [p1, agent]; \
             1 still running: p1. [y/N] "
                .into(),
            "cancelled; the window stays open\n".into(),
        )));
        for (action, warn_final) in [
            (&close_t1, true),
            (&close_t1, false),
            (&close_p1, true),
            (&close_p1, false),
        ] {
            assert_eq!(
                close_question(&snap, action, true, warn_final),
                busy_final,
                "{action:?} {warn_final}"
            );
        }

        // Unknown busy state, missing or capped, is a busy final close.
        let unknown_final = Ok(Some((
            "Close the last tab? This quits the window and stops 2 terminals [p1, agent]; \
             busy state unknown: p1, Agent. [y/N] "
                .into(),
            "cancelled; the window stays open\n".into(),
        )));
        snap.busy = None;
        assert_eq!(close_question(&snap, &close_t1, true, false), unknown_final);
        snap.busy = Some(BusyTerminals {
            terminals: Vec::new(),
            omitted: 2,
        });
        assert_eq!(close_question(&snap, &close_p1, true, true), unknown_final);

        // While other workspaces remain, the question names the workspace it
        // closes, escaped, and its busy terminals.
        snap.busy = Some(BusyTerminals {
            terminals: vec![BusyTerminal {
                target: TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: TargetId::Popup("u1".into()),
                    terminal: "s2".into(),
                },
                name: "Agent".into(),
            }],
            omitted: 0,
        });
        snap.workspaces = vec![
            WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: Some("my \"notes\"".into()),
                tab_count: 1,
                attention: None,
                recency: 0,
            },
            WorkspaceInfo {
                id: "s2".into(),
                persistent_id: Some("s2".into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency: 1,
            },
        ];
        let workspace_final = Ok(Some((
            "Close the last tab? This closes session my \\\"notes\\\" and stops 2 terminals \
             [p1, agent]; 1 still running: Agent. [y/N] "
                .into(),
            "cancelled; session my \\\"notes\\\" stays open\n".into(),
        )));
        assert_eq!(
            close_question(&snap, &close_t1, true, false),
            workspace_final
        );
        assert_eq!(
            close_question(&snap, &close_p1, true, true),
            workspace_final
        );
        // An unnamed workspace is named by its ID.
        snap.workspaces[0].name = None;
        assert_eq!(
            close_question(&snap, &close_t1, true, true),
            Ok(Some((
                "Close the last tab? This closes session s1 and stops 2 terminals \
                 [p1, agent]; 1 still running: Agent. [y/N] "
                    .into(),
                "cancelled; session s1 stays open\n".into(),
            )))
        );
    }

    #[test]
    fn pane_send_takes_one_target_one_text_source_and_each_flag_once() {
        let parse = |args: &[&str]| {
            parse_pane_send_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                .map_err(|error| error.status)
        };
        let send = |text, enter, json| {
            Ok(Some(PaneSend {
                target: "u2".into(),
                text,
                enter,
                force: false,
                json,
            }))
        };
        assert_eq!(parse(&["pane", "create"]), Ok(None));
        assert_eq!(
            parse(&["pane", "send", "u2", "--", "--enter"]),
            send(SendText::Operand("--enter".into()), false, false)
        );
        assert_eq!(
            parse(&["pane", "send", "--json", "u2", "--enter", "--", ""]),
            send(SendText::Operand("".into()), true, true)
        );
        assert_eq!(
            parse(&["pane", "send", "u2", "--stdin", "--enter"]),
            send(SendText::Stdin, true, false)
        );
        assert_eq!(
            parse(&["pane", "send", "u2", "--enter"]),
            send(SendText::None, true, false)
        );
        for args in [
            &["pane", "send"][..],
            &["pane", "send", "--", "text"],
            &["pane", "send", "u2", "text"],
            &["pane", "send", "u2", "--"],
            &["pane", "send", "u2", "--", "a", "b"],
            &["pane", "send", "u2", "--stdin", "--", "text"],
            &["pane", "send", "u2", "--enter", "--enter"],
            &["pane", "send", "u2", "--force", "--force", "--enter"],
            &["pane", "send", "u2", "--no-focus", "--", "text"],
        ] {
            assert_eq!(parse(args), Err(2), "{args:?}");
        }
    }

    #[test]
    fn pane_read_and_history_take_one_target_and_their_optional_flags() {
        for verb in ["read", "history"] {
            let action = |target: &str| {
                let target = target.into();
                if verb == "read" {
                    Action::ReadTerminal { target }
                } else {
                    Action::ReadTerminalHistory { target }
                }
            };
            assert_eq!(
                parse_control_arguments(&["pane", verb, "u2", "--json"].map(Into::into)).unwrap(),
                (action("u2"), true)
            );
            assert_eq!(
                parse_control_arguments(&["pane", verb, "p1"].map(Into::into)).unwrap(),
                (action("p1"), false)
            );
            for args in [
                &["pane", verb][..],
                &["pane", verb, "p1", "p2"],
                &["pane", verb, "--", "p1"],
                &["pane", verb, "p1", "--no-focus"],
                &["pane", verb, "p1", "--json", "--json"],
            ] {
                assert!(
                    parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                        .is_err(),
                    "{args:?}"
                );
            }
        }
        let edit = |target: &str| Action::EditTerminalHistory {
            target: target.into(),
        };
        for (args, json) in [
            (&["pane", "history", "p1", "--edit"][..], false),
            (&["pane", "history", "--edit", "p1"], false),
            (&["pane", "history", "p1", "--edit", "--json"], true),
            (&["pane", "history", "--json", "p1", "--edit"], true),
        ] {
            assert_eq!(
                parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                    .unwrap(),
                (edit("p1"), json),
                "{args:?}"
            );
        }
        for args in [
            &["pane", "history", "--edit"][..],
            &["pane", "history", "--edit", "--edit"],
            &["pane", "history", "p1", "--edit", "--edit"],
            &["pane", "history", "p1", "p2", "--edit"],
            &["pane", "read", "p1", "--edit"],
        ] {
            assert!(
                parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                    .is_err(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn session_ls_lists_with_json_anywhere_after_session_and_workspace_commands_name_their_replacement()
     {
        let words = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        for args in [
            &["session", "ls"][..],
            &["session", "ls", "--json"],
            &["session", "--json", "ls"],
        ] {
            assert!(super::lists_sessions(&words(args)), "{args:?}");
            assert!(matches!(
                super::parse_control_arguments(&words(args)),
                Ok((Action::Workspace(WorkspaceAction::Inspect), _))
            ));
        }
        for args in [&["session"][..], &["session", "--json"], &["tab", "ls"]] {
            assert!(!super::lists_sessions(&words(args)), "{args:?}");
        }
        for (args, hint) in [
            (&["workspace"][..], "`eon workspace` is now `eon session`"),
            (
                &["workspace", "new", "--json"],
                "`eon workspace` is now `eon session`",
            ),
            (
                &["workspaces", "--json"],
                "`eon workspaces` is now `eon session ls`",
            ),
        ] {
            let error = super::renamed_command(&words(args)).expect("a removed command");
            assert_eq!(error.status, 2);
            assert!(error.message.contains(hint), "{}", error.message);
            assert!(error.message.contains("eon --help"));
        }
        assert!(super::renamed_command(&words(&["session"])).is_none());
    }

    #[test]
    fn workspace_parsers_take_literal_names_and_refuse_stray_flags() {
        // The whole pipeline: target flags first, then the action.
        let parse = |args: &[&str]| {
            let (args, _) = parse_workspace_target_arguments(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
            )?;
            parse_control_arguments(&args)
        };
        let workspace = |action| Ok((Action::Workspace(action), false));
        assert_eq!(
            parse(&["session", "ls", "--json"]).unwrap(),
            (Action::Workspace(WorkspaceAction::Inspect), true)
        );
        assert_eq!(
            parse(&["session", "new", "--no-focus", "--name", "-dash"]).map_err(|_| ()),
            workspace(WorkspaceAction::NewWorkspace {
                name: Some("-dash".into()),
                focus: false,
            })
        );
        assert_eq!(
            parse(&["session", "new", "--name", "--window"]).map_err(|_| ()),
            workspace(WorkspaceAction::NewWorkspace {
                name: Some("--window".into()),
                focus: true,
            })
        );
        assert_eq!(
            parse(&["session", "rename", "--", "--json"]).map_err(|_| ()),
            workspace(WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: "--json".into(),
            })
        );
        assert_eq!(
            parse(&["session", "rename", "s2", "  "]).map_err(|_| ()),
            workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some(SessionTarget::Selector("s2".into())),
                name: String::new(),
            })
        );
        for args in [
            &["session", "new", "extra"][..],
            &["session", "new", "--name"],
            &["session", "switch", "s2", "--no-focus"],
            &["session", "switch"],
            &["session", "close", "s2", "s3"],
            &["session", "close", "--force", "s2"],
            &["session", "rename"],
            &["session", "ls", "--no-focus"],
        ] {
            assert_eq!(parse(args).unwrap_err().status, 2, "{args:?}");
        }
    }

    #[test]
    fn keys_parser_and_help_cover_json_target_and_usage_errors() {
        let parse = |args: &[&str]| -> Result<(Option<bool>, WorkspaceTarget), Error> {
            let (rest, target) = parse_workspace_target_arguments(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
            )?;
            Ok((parse_keys_arguments(&rest)?, target))
        };
        assert_eq!(parse(&["keys"]).unwrap().0, Some(false));
        assert_eq!(parse(&["keys", "--json"]).unwrap().0, Some(true));
        let (json, target) = parse(&["--window", "abcd", "keys", "--json"]).unwrap();
        assert_eq!(json, Some(true));
        assert_eq!(target.window.as_deref(), Some("abcd"));
        let (json, target) = parse(&["keys", "--window", "abcd", "--json"]).unwrap();
        assert_eq!(json, Some(true));
        assert_eq!(target.window.as_deref(), Some("abcd"));
        assert_eq!(parse(&["session"]).unwrap().0, None);

        for bad in [
            &["keys", "extra"][..],
            &["keys", "--json", "--json"],
            &["keys", "--no-focus"],
            &["keys", "--json", "extra"],
        ] {
            let Err(err) = parse(bad) else {
                panic!("expected usage error for {bad:?}");
            };
            assert_eq!(err.status, 2, "{bad:?}");
        }

        let help = command_help_text(&[OsString::from("keys")], false).unwrap();
        assert!(help.contains("keys [--json]"), "{help}");
    }
}
