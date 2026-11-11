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
        LaunchMode, LaunchOptions, configuration_directory, launch_current, prepare_configuration,
        runtime_directory,
    },
    windows,
    workspace::{
        caller_output, failure_human, human as human_output, json as json_output, json_escape,
        list_human, list_json, registration_json, resolve_target, tab_number,
        terminal_editing_json, terminal_history_json, terminal_sent_json, terminal_text_json,
    },
};
use eon_workspace_protocol::v40::{
    Action, AttentionState, CloseConsent, CloseEnd, CloseScope, Direction, Failure,
    MAX_HISTORY_TEXT_BYTES, MAX_PASTE_BYTES, Pane, PopupTarget, Response, Side, Snapshot, Tab,
    TargetId, TerminalHistory, TerminalInput, VERSION, WorkspaceAction, WorkspaceInfo,
    WorkspaceTarget as SessionTarget, valid_caller, validate_paste,
};
use std::{
    env,
    ffi::{OsStr, OsString},
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

pub struct Inputs {
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
    pub orbit: PathBuf,
    pub venus: PathBuf,
    pub anima: PathBuf,
    /// The Eon icon's image, passed to Venus unread; none when Eon has no icon.
    pub icon: Option<PathBuf>,
    pub codexbar: PathBuf,
    pub managed: managed_environment::ManagedPrograms,
}

impl Inputs {
    pub(crate) fn component_report(&self) -> Result<&str, String> {
        self.component_report.as_deref().map_err(Clone::clone)
    }
}

/// The executable name of the Workspaces picker, whose own popup key confirms its highlighted
/// row (EON-C33).
pub(crate) const WORKSPACE_PICKER: &str = "eon-sessions";

pub fn run(inputs: Inputs) -> (&'static str, Result<i32, Error>) {
    let _trace = super::diagnostics::BenchTrace::start();
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments: Vec<OsString> = arguments.collect();
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
        // A caught no-op resets on exec, letting the product handle terminal
        // interrupts while the runner survives to retain its result.
        extern "C" fn interrupt(_: libc::c_int) {}
        let handler = interrupt as *const () as libc::sighandler_t;
        let previous_int = unsafe { libc::signal(libc::SIGINT, handler) };
        if previous_int == libc::SIG_ERR {
            return Err(format!(
                "cannot retain update results: {}",
                std::io::Error::last_os_error()
            ));
        }
        let previous_quit = unsafe { libc::signal(libc::SIGQUIT, handler) };
        let status = if previous_quit == libc::SIG_ERR {
            Err(std::io::Error::last_os_error())
        } else {
            // Use the popup's PATH and inherit its terminal for product-owned consent.
            let status = Command::new("eon").args(arguments).status();
            unsafe { libc::signal(libc::SIGQUIT, previous_quit) };
            status
        };
        unsafe { libc::signal(libc::SIGINT, previous_int) };
        status
            .map_err(|error| format!("cannot run eon update: {error}"))?
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
    let mut terminal = std::mem::MaybeUninit::<libc::termios>::uninit();
    let terminal = (unsafe { libc::tcgetattr(libc::STDIN_FILENO, terminal.as_mut_ptr()) } == 0)
        .then(|| unsafe { terminal.assume_init() });
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
                unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
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
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &terminal) };
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
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let ControlResponse::Workspace(Response::Snapshot(snapshot)) = send_action_from_until(
            socket,
            None,
            Action::Workspace(WorkspaceAction::Inspect),
            deadline,
        )
        .ok()?
        else {
            return None;
        };
        let running = snapshot.running?;
        if let Some(others) = running.others {
            return running_notice(&others, running.omitted);
        }
        let remaining = deadline.checked_duration_since(Instant::now())?;
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
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
            false,
            "eon",
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
                false,
                "eon",
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
            Ok(windows::reload_window(inputs, id, false)?)
        }
        [command, action, id, flag]
            if command == "window" && action == "reload" && flag == "--json" =>
        {
            Ok(windows::reload_window(inputs, id, true)?)
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
                false,
                "eon",
                &options,
            )
        }
        [command, ..] if control_command(command) => {
            let (arguments, target) = parse_workspace_target_arguments(&arguments)?;
            if let Some(send) = parse_pane_send_arguments(&arguments)? {
                return send_terminal(inputs, send, target);
            }
            if let Some((popup, json)) = parse_popup_hide_arguments(&arguments)? {
                return Ok(hide_popup(inputs, popup, json, target)?);
            }
            if let Some((verb, json)) = parse_sidebar_arguments(&arguments)? {
                return Ok(sidebar(inputs, verb, json, target)?);
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
        "session", "tab", "pane", "column", "popup", "sidebar", "edit", "focus", "--window",
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
    let path = configuration_directory().map(|root| root.join(managed_environment::CONFIG_FILE));
    let result = match &path {
        Ok(path) => managed_environment::read_configuration(path)
            .map_err(|detail| ("unreadable-configuration", detail))
            .and_then(|source| {
                managed_environment::validate_configuration_text(&source, path, &inputs.defaults)
                    .map_err(|detail| ("invalid-configuration", detail))
            }),
        Err(detail) => Err(("unreadable-configuration", detail.clone())),
    };
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
        ],
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
                "Reload Venus in a focused window",
            ),
            (
                "window stop ID|previous|all [--json]",
                "End a window, the old-version",
            ),
            ("", "ones, or all"),
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
            ("edit [--json] -- FILE", "Open FILE in a new editor pane"),
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
                "pane rename PANE NAME [--json]",
                "Pin a pane name; empty clears",
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
                "pane send PANE|POPUP [--enter] [--force]",
                "Paste TEXT, then Enter if asked",
            ),
            (
                "  [--json] [--stdin | -- TEXT]",
                "TEXT up to 64 KiB, or Enter alone",
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
                "focus PANE|left|right|up|down [--json]",
                "Focus a pane or navigate",
            ),
        ],
    ),
    (
        "Tools & information",
        &[
            ("anima [STYLE] [CHILD OPTIONS...]", "Play an animation here"),
            ("versions", "Show exact component versions"),
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
    (!help.is_empty())
        .then(|| help + "Run `eon --help` for the window flag, notes and exit status.\n")
}

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
Window IDs, as eon window ls shows them: default, another build's or an
independent window's ID, or any unique prefix of 4+ characters.
Window flag: --window ID, before any --.
Explicit targets still validate EON_TERMINAL; independent windows use
attach's rule.
Results name generation, window (null for default), and session.
SESSION is a session ID (s1, s2, ...) or its exact name.
Inspect/switch/rename/close accept --id ID instead of a SESSION target.
ID is the full persistent_id, scoped to the selected window.
session registry reads metadata without a window; it does not show liveness.
session new --name NAME names the new session; created gives its local ID,
created_persistent_id its durable identity. Uncertain creation needs inspection,
not an automatic retry.
With --no-focus, session new keeps the shown session, and either
pane create keeps the selected pane and any popup.
pane read, history and send reach hidden sessions; [agents]
terminal_access governs all three. pane send --enter waits
[agents] send_enter_delay_ms first.
pane history --edit opens the history in a Scrollback editor popup,
only in the shown session's active tab.
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
        if let Some(status) = confirm(
            &format!(
                "Close session {id}{}? This {}stops every terminal in it. [y/N] ",
                target
                    .name
                    .as_ref()
                    .map_or_else(String::new, |name| format!(" ({})", name.escape_debug())),
                if listed.len() == 1 {
                    "quits the window and "
                } else {
                    ""
                }
            ),
            &format!("cancelled; session {id} stays open\n"),
        )? {
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
    let creation = match action {
        Action::Workspace(WorkspaceAction::NewWorkspace { .. }) => Some(Creation::Workspace),
        Action::Workspace(WorkspaceAction::CreateTab) | Action::LaunchTab { .. } => {
            Some(Creation::Tab)
        }
        Action::Workspace(WorkspaceAction::CreatePane { .. })
        | Action::CreateCommandPane { .. }
        | Action::Edit { .. } => Some(Creation::Pane),
        _ => None,
    };
    let explicit_inspection = matches!(
        action,
        Action::Workspace(WorkspaceAction::InspectWorkspace { .. })
    );
    let inspecting = !list
        && (explicit_inspection || matches!(action, Action::Workspace(WorkspaceAction::Inspect)));
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
                    "generation {generation}\nwindow {}\nsession {}\n{caller}{}{}{}",
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
    let action = |input| Action::SendTerminal {
        target: send.target.clone(),
        input,
        force: send.force,
    };
    let resolve = || {
        let caller = calling_terminal()?;
        validate_action(caller.clone(), action(TerminalInput::Enter))
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
    let request =
        |input| match send_action_from(Ok(&resolved.socket), caller.clone(), action(input)) {
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
        None => match request(TerminalInput::Enter) {
            Ok(sent) => (sent, None),
            Err(failure) => return Ok(report_failure(&failure, json)?),
        },
        Some(text) => match request(TerminalInput::Paste(text)) {
            Err(failure) => return Ok(report_failure(&failure, json)?),
            Ok(pasted) if send.enter => {
                thread::sleep(Duration::from_millis(pasted.enter_delay_ms.into()));
                (pasted, request(TerminalInput::Enter).err())
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
    // SAFETY: stat holds only integers, so all zeros is a valid value.
    let mut status = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: fstat fills the valid local stat buffer.
    if unsafe { libc::fstat(libc::STDIN_FILENO, &mut status) } != 0 {
        return false;
    }
    match status.st_mode & libc::S_IFMT {
        libc::S_IFREG => {
            // SAFETY: a zero move from the current offset reads it without changing it.
            let offset = unsafe { libc::lseek(libc::STDIN_FILENO, 0, libc::SEEK_CUR) };
            offset >= 0 && status.st_size > offset
        }
        libc::S_IFIFO | libc::S_IFSOCK => {
            let mut input = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: poll reads and writes the one valid local pollfd. It returns early when
            // bytes arrive or the writer has ended; its result only bounds the wait.
            unsafe { libc::poll(&mut input, 1, STDIN_DATA_WAIT_MS) };
            let mut waiting: libc::c_int = 0;
            // SAFETY: FIONREAD writes the waiting byte count into the valid local int.
            let read = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::FIONREAD, &mut waiting) };
            read == 0 && waiting > 0
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
        return failure(code, format!("{}; run `eon` first", error.detail));
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
    let tab_terminals = |tab: &eon_workspace_protocol::v40::Tab| -> Vec<(String, String)> {
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
        |name| format!("closes session {}", name.escape_debug()),
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
        |name| format!("cancelled; session {} stays open\n", name.escape_debug()),
    );
    (question, cancelled)
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
        if argument == "--name" || argument == "--dir" {
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
    if let Some(action) = parse_command_pane_arguments(arguments)? {
        return Ok(action);
    }
    if let Some(action) = parse_tab_create_arguments(arguments)? {
        return Ok(action);
    }
    let (mut json, mut focus) = (false, true);
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
            if !focus {
                return Err(Error::usage(EON_USAGE));
            }
            focus = false;
        } else {
            values.push(argument);
        }
    }
    let action = match values.as_slice() {
        ["pane", "create"] => Action::Workspace(WorkspaceAction::CreatePane { focus }),
        _ if !focus => return Err(Error::usage(EON_USAGE)),
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
                "eon focus takes a pane; switch to tab {tab} with `eon tab switch {tab}`"
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
    let mut values = Vec::new();
    for argument in arguments {
        let value = argument
            .to_str()
            .ok_or_else(|| Error::usage("pane rename requires UTF-8"))?;
        match value {
            "--" if !literal => literal = true,
            "--json" if !literal && !json => json = true,
            "--clear" if !literal && !clear => clear = true,
            value if !literal && value.starts_with('-') => return Err(Error::usage(EON_USAGE)),
            _ => values.push(value),
        }
    }
    let (pane, name) = match values.as_slice() {
        [pane] if clear => (*pane, None),
        [pane, name] if !clear => (*pane, Some(name.trim()).filter(|name| !name.is_empty())),
        _ => return Err(Error::usage(EON_USAGE)),
    };
    if let Some(name) = name {
        eon_workspace_protocol::v40::validate_pane_name(name)
            .map_err(|error| Error::usage(error.to_string()))?;
    }
    Ok((
        Action::RenamePane {
            pane: pane.into(),
            name: name.map(str::to_owned),
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

/// `pane create [--json] [--no-focus] -- COMMAND [ARGS...]`, flags in either
/// order; every argument after `--` passes through unchanged as the new pane's
/// argv.
fn parse_command_pane_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    if command != "pane" || operation != "create" {
        return Ok(None);
    }
    let Some(separator) = rest.iter().position(|argument| argument == "--") else {
        return Ok(None);
    };
    let (mut json, mut focus) = (false, true);
    for flag in &rest[..separator] {
        match flag.to_str() {
            Some("--json") if !json => json = true,
            Some("--no-focus") if focus => focus = false,
            _ => return Ok(None),
        }
    }
    let argv = &rest[separator + 1..];
    if argv.first().is_none_or(|program| program.is_empty()) {
        return Err(Error::usage("eon pane create -- needs a command"));
    }
    Ok(Some((
        Action::CreateCommandPane {
            argv: argv
                .iter()
                .map(|argument| argument.as_bytes().to_vec())
                .collect(),
            focus,
        },
        json,
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

#[cfg(test)]
mod parity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_close_steps_follow_runtimes_answer() {
        use eon_workspace_protocol::v40::{CloseTerminal, TerminalTarget, UntrackedRun};
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
            matches!(parse_control_arguments(&args).unwrap().0, Action::CreateCommandPane { argv, .. } if argv == [b"echo".to_vec(), b"--window".to_vec(), b"literal".to_vec()])
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
                        name: expected.map(str::to_owned)
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
                Action::CreateCommandPane {
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
        let unfocused = Action::Workspace(WorkspaceAction::CreatePane { focus: false });
        for (arguments, expected) in [
            (
                &["pane", "create"][..],
                (
                    Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
                    false,
                ),
            ),
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
            (
                &["pane", "--json", "create"],
                (
                    Action::Workspace(WorkspaceAction::CreatePane { focus: true }),
                    true,
                ),
            ),
            (&["--no-focus", "pane", "create"], (unfocused, false)),
            (
                &["pane", "create", "--no-focus", "--", "sh"],
                (
                    Action::CreateCommandPane {
                        argv: vec![b"sh".to_vec()],
                        focus: false,
                    },
                    false,
                ),
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
        use eon_workspace_protocol::v40::{
            BusyTerminal, BusyTerminals, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey,
            Sidebar, Tab, TargetId, TerminalTarget,
        };
        let pane = |id: &str, session: &str| Pane {
            id: id.into(),
            label: id.into(),
            pinned_name: None,
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
        use eon_workspace_protocol::v40::{
            BusyTerminal, BusyTerminals, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey,
            Sidebar, Tab, TargetId, TerminalTarget,
        };
        let mut snap = Snapshot {
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
}
