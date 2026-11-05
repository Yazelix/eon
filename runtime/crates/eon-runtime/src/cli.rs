use super::{
    control::{
        ControlResponse, EndpointFailure, EndpointFailureKind, Error, failure, report_failure,
        send_action_from, send_action_from_until, validate_action, write_stdout,
    },
    generation::{
        Ask, WorkspaceTarget, attach_generation, confirm, current_generation, generations_command,
        resolve_workspace, stop_generation, stop_generations, valid_generation,
    },
    managed_environment,
    running::running_notice,
    sessions::{TAB_VARIABLE, TERMINAL_VARIABLE, terminal_session},
    supervisor::{
        LaunchMode, configuration_directory, launch_current, prepare_configuration,
        runtime_directory,
    },
    windows,
    workspace::{
        caller_output, failure_human, human as human_output, json as json_output, json_escape,
        list_human, list_json, resolve_selector, terminal_history_json, terminal_sent_json,
        terminal_text_json,
    },
};
use eon_workspace_protocol::v33::{
    Action, AttentionState, Direction, Failure, MAX_HISTORY_TEXT_BYTES, MAX_PASTE_BYTES,
    PopupTarget, Response, Side, Snapshot, TerminalHistory, TerminalInput, VERSION,
    WorkspaceAction, WorkspaceInfo, valid_caller, validate_paste,
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
    pub component_report: Result<String, String>,
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
pub(crate) const WORKSPACE_PICKER: &str = "eon-workspaces";

pub fn run(inputs: Inputs) -> (&'static str, Result<i32, Error>) {
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
/// popup's own tab, `PANE` for its tab's selected pane and `WORKSPACE` for its own workspace.
/// `NAME` is typed in a name box, where an empty name
/// clears. `WS` is chosen in the Workspaces popup's picker, run inside Commands: a query that
/// matches no workspace runs `workspace new --name QUERY` instead, and a cancel runs nothing.
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
        ("Switch workspace", &["workspace", "switch", "WS"], Close),
        ("New workspace", &["workspace", "new"], Close),
        ("Rename workspace", &["workspace", "rename", "NAME"], Close),
        (
            "Close this workspace",
            &["workspace", "close", "WORKSPACE"],
            Close,
        ),
        ("Show workspace", &["workspace"], Keep),
        ("List windows", &["windows"], Keep),
        ("List generations", &["generations"], Keep),
        ("Stop older generations", &["stop", "previous"], Keep),
        ("Show versions", &["versions"], Keep),
        ("Show config path", &["config-path"], Keep),
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
            format!("{index}\t{label:<24}eon {command}\n")
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
        eprintln!("eon-workspaces: {error}");
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
    if arguments.iter().any(|argument| argument == "WS") {
        return Ok((pick_and_run_workspace(inputs)?, *kept));
    }
    if arguments
        .iter()
        .any(|argument| argument == "PANE" || argument == "WORKSPACE")
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
            } else if argument == "WORKSPACE" {
                *argument = snapshot.workspace.as_str().into();
            }
        }
    }
    if arguments.last().is_some_and(|argument| argument == "NAME") {
        let subject = if arguments[0] == "pane" {
            "Pane"
        } else {
            "Workspace"
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
    Ok((run_as_caller(inputs, arguments), *kept))
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
                "Workspace > ",
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
            "workspace",
        )?
        else {
            return Ok(0);
        };
        let output = String::from_utf8_lossy(&output);
        let Some(arguments) = workspace_pick_arguments(&output) else {
            return Ok(0);
        };
        // Runtime would read a departed row's wN as another workspace's name, so a row switches
        // or closes only while its wN is still live; the check and the action are not atomic.
        if arguments[1] != "new"
            && !caller_snapshot(inputs)?
                .workspaces
                .iter()
                .any(|workspace| workspace.id == arguments[2])
        {
            let detail = format!("workspace {} is not live", arguments[2]);
            return report_failure(&failure("unknown-workspace", detail), false);
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
            "workspace",
            if key == Some("ctrl-x") {
                "close"
            } else {
                "switch"
            },
            row.split('\t').next()?,
        ]),
        None if key.is_none() && !query.trim().is_empty() => {
            Some(vec!["workspace", "new", "--name", query])
        }
        None => None,
    }
}

/// The popup caller's own workspace, as `eon workspace` reports it.
fn caller_snapshot(inputs: &Inputs) -> Result<Snapshot, String> {
    let caller = calling_terminal().map_err(|error| error.detail)?;
    let resolved = resolve_workspace(inputs, WorkspaceTarget::default(), caller.as_deref())
        .map_err(|error| error.detail)?;
    inspect(&resolved.socket, caller.as_deref(), &resolved.generation).map_err(|error| error.detail)
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
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
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

fn execute(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, Error> {
    if let Some(result) = command_help(inputs.version, &arguments) {
        return result;
    }
    if let Some(code) = lifecycle_command(inputs, &arguments, "eon")? {
        return Ok(code);
    }
    if let Some((all, json)) = parse_quit_arguments(&arguments)? {
        return if all {
            windows::quit_all(inputs, json)
        } else {
            windows::quit(inputs, json)
        };
    }
    let result = match arguments.as_slice() {
        [] => launch_current(inputs, LaunchMode::Workspace, &[], true, false, "eon"),
        [command, action] if command == "window" && action == "new" => windows::new_window(inputs),
        [command, action, id] if command == "window" && action == "attach" => {
            Ok(windows::attach_window(inputs, id)?)
        }
        [command, action, all] if command == "window" && action == "stop" && all == "all" => {
            Ok(windows::stop_namespaces(false, false)?)
        }
        [command, action, all, flag]
            if command == "window" && action == "stop" && all == "all" && flag == "--json" =>
        {
            Ok(windows::stop_namespaces(false, true)?)
        }
        [command, action, id] if command == "window" && action == "stop" => {
            Ok(windows::stop_window(id, false)?)
        }
        [command, action, id, flag]
            if command == "window" && action == "stop" && flag == "--json" =>
        {
            Ok(windows::stop_window(id, true)?)
        }
        [command] if command == "windows" => windows::list_windows(inputs, false),
        [command, flag] if command == "windows" && flag == "--json" => {
            windows::list_windows(inputs, true)
        }
        [command, child @ ..] if command == "anima" => {
            let error = Command::new(&inputs.anima)
                .args(child)
                .env("YAZELIX_SCREEN_COMMAND_NAME", "eon anima")
                .exec();
            Err(format!("cannot launch Anima: {error}"))
        }
        [command] if command == "run" => {
            launch_current(inputs, LaunchMode::Workspace, &[], false, false, "eon")
        }
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            launch_current(inputs, LaunchMode::Workspace, child, false, false, "eon")
        }
        [command, ..] if control_command(command) => {
            let (arguments, target) = parse_workspace_target_arguments(&arguments)?;
            if let Some(send) = parse_pane_send_arguments(&arguments)? {
                return send_terminal(inputs, send, target);
            }
            if let Some((popup, json)) = parse_popup_hide_arguments(&arguments)? {
                return Ok(hide_popup(inputs, popup, json, target)?);
            }
            let (action, json) = parse_control_arguments(&arguments)?;
            let list = arguments.first().is_some_and(|value| value == "workspaces");
            control(inputs, action, json, list, target)
        }
        [flag] if flag == "--skill" => {
            write_stdout(inputs.skill)?;
            Ok(0)
        }
        [command] if command == "versions" => {
            write_stdout(format!(
                "eon {} {}\neonw {}\n{}\n",
                inputs.version,
                current_generation(inputs)?,
                VERSION,
                inputs.component_report()?
            ))?;
            Ok(0)
        }
        [command] if command == "config-path" => print_config_path(true),
        [command] if command == "config-dir" => print_config_path(false),
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

/// The first words `execute` hands to `parse_control_arguments`.
fn control_command(command: &OsStr) -> bool {
    [
        "workspace",
        "workspaces",
        "tab",
        "pane",
        "popup",
        "edit",
        "focus",
        "--generation",
        "--window",
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
            ("eon", "Open or present the workspace"),
            ("run [-- COMMAND...]", "Start a fresh workspace"),
        ],
    ),
    (
        "Windows",
        &[
            ("quit [--json]", "End this window and its terminals"),
            ("quit --all [--json]", "End every window's terminals"),
            ("window new", "Open an independent window"),
            ("window attach ID", "Reopen a window"),
            ("window stop ID [--json]", "End that window's terminals"),
            ("window stop all [--json]", "End every window's terminals"),
            ("windows [--json]", "List independent windows"),
        ],
    ),
    (
        "Attach & stop",
        &[
            ("attach [GENERATION]", "Reattach to current or named work"),
            ("generations [--json]", "List current and older work"),
            (
                "stop GENERATION|previous|all [--json]",
                "End selected terminals",
            ),
        ],
    ),
    (
        "Workspaces",
        &[
            ("workspaces [--json]", "List this window's workspaces"),
            ("workspace [--json]", "Inspect the workspace"),
            (
                "workspace new [--json] [--no-focus]",
                "Add a workspace and show it",
            ),
            ("workspace switch WS [--json]", "Show workspace WS"),
            (
                "workspace rename [WS] NAME [--json]",
                "Name a workspace; empty clears",
            ),
            (
                "workspace close WS [--json]",
                "Stop its terminals and remove it",
            ),
        ],
    ),
    (
        "Tabs & panes",
        &[
            ("tab create [--json]", "Choose a directory for a new tab"),
            (
                "tab create --directory DIRECTORY",
                "Start a tab without quick search",
            ),
            (
                "  [--json] [--no-focus] [-- COMMAND...]",
                "COMMAND needs an Eon terminal",
            ),
            ("tab close TAB [--json]", "Close the active tab"),
            (
                "tab directory TAB [--json] -- DIRECTORY",
                "Set the tab's launch directory",
            ),
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
            ("pane close PANE [--json]", "Close a pane of the active tab"),
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
                "pane history PANE|POPUP [--json]",
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
                "focus ID|left|right|up|down [--json]",
                "Focus by ID or navigate",
            ),
        ],
    ),
    (
        "Tools & information",
        &[
            ("anima [STYLE] [CHILD OPTIONS...]", "Play an animation here"),
            ("versions", "Show exact component versions"),
            ("config-path", "Create the config root and print"),
            ("", "its config.toml path"),
            ("config-dir", "Create and print the config root"),
            ("config defaults [--json]", "Print config.toml defaults"),
            ("--skill", "Print this build's agent skill"),
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
    // Command words are lowercase, as `config-path`; `--`, IDs and the `-h,` row are not.
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
        .then(|| help + "Run `eon --help` for workspace flags, notes and exit status.\n")
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
            ("eon windows --json", "Inspect windows as JSON"),
            ("eon anima --help", "Browse animation styles/options"),
        ],
        color,
    );
    format!(
        "{heading}Eon{reset} {version}  ·  Terminal workspaces. Persistent terminals.

{heading}Usage{reset}  {command}eon [COMMAND]{reset}
{sections}
{heading}Examples{reset}
{examples}
Alt+Shift+D detaches a window; closing it or eon quit ends it.
Inside Eon, workspace commands use the calling terminal's supervisor/workspace.
Tab move and pane commands retain their calling-tab rules.
Workspace flags: --generation GENERATION / --window ID, before any --.
Explicit targets still validate EON_TERMINAL; window-only uses attach's rule.
Results name generation, window (null for default), and workspace.
WS is a workspace ID (w1, w2, ...) or its exact name.
workspace new --name NAME names the new workspace; created gives its ID.
With --no-focus, workspace new keeps the shown workspace, and either
pane create keeps the selected pane and any popup.
pane read, history and send reach hidden workspaces; [agents]
terminal_access governs all three. pane send --enter waits
[agents] send_enter_delay_ms first.
pane send refuses while a person is typing; --force bypasses the typing check.
Where shown, {command}--json{reset} prints JSON and skips confirmation.
Without a terminal, a confirmation needs yes or exits 1 (not-confirmed).
Exit status: 0 success, 1 failed, 2 invalid command, 3 target not found,
4 Eon unreachable, 5 person typing (nothing sent), 130 interrupted.
Agent? Run {command}eon --skill{reset} to learn how to work in Eon.
"
    )
}

fn execute_eonbase(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, Error> {
    if let Some(code) = lifecycle_command(inputs, &arguments, "eonbase")? {
        return Ok(code);
    }
    let result = match arguments.as_slice() {
        [separator, child @ ..] if separator == "--" && !child.is_empty() => {
            launch_current(inputs, LaunchMode::Terminal, child, true, true, "eonbase")
        }
        [flag, separator, child @ ..]
            if flag == "--no-decorations" && separator == "--" && !child.is_empty() =>
        {
            launch_current(inputs, LaunchMode::Terminal, child, true, false, "eonbase")
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

fn lifecycle_command(
    inputs: &Inputs,
    arguments: &[OsString],
    product: &str,
) -> Result<Option<i32>, Error> {
    Ok(Some(match arguments {
        [command] if command == "attach" => attach_generation(inputs, None, product)?,
        [command, generation] if command == "attach" => {
            attach_generation(inputs, Some(generation_argument(generation)?), product)?
        }
        [command] if command == "generations" => generations_command(inputs, false, product)?,
        [command, flag] if command == "generations" && flag == "--json" => {
            generations_command(inputs, true, product)?
        }
        [command, generation] if command == "stop" => {
            stop_argument(inputs, generation, false, product)?
        }
        [command, generation, flag] | [command, flag, generation]
            if command == "stop" && flag == "--json" =>
        {
            stop_argument(inputs, generation, true, product)?
        }
        _ => return Ok(None),
    }))
}

fn stop_argument(
    inputs: &Inputs,
    argument: &OsStr,
    json: bool,
    product: &str,
) -> Result<i32, Error> {
    let root = runtime_directory(product);
    Ok(match argument.to_str() {
        Some("previous") if product == "eon" => {
            stop_generations(inputs, &root, false, json, &Ask::Stop)?
        }
        Some("all") if product == "eon" => stop_generations(inputs, &root, true, json, &Ask::Stop)?,
        _ => stop_generation(
            inputs,
            &root,
            generation_argument(argument)?,
            json,
            &Ask::Stop,
        )?,
    })
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
    let final_close = if !json && std::io::stdin().is_terminal() {
        match final_close_terminals(socket, &action, generation, caller.as_deref()) {
            Ok(terminals) => terminals,
            Err(error) => return report_failure(&error, json),
        }
    } else {
        None
    };
    if let Some((terminals, workspace)) = final_close
        && managed_environment::warning_close_last_tab(
            &configuration_directory()?,
            &inputs.defaults,
            None,
        )?
    {
        let (question, cancelled) = final_close_prompt(&terminals, workspace.as_deref());
        if let Some(status) = confirm(&question, &cancelled)? {
            return Ok(status);
        }
    }
    // Closing a workspace asks as Stop does, about the wN that WS resolves to
    // by Runtime's rule; a WS that resolves to none or several fails unasked.
    // After a yes it checks again that the wN is live before closing it, since
    // Runtime would read a departed wN as another workspace's name; the check
    // and the close are not atomic. v33 lists only
    // the caller's or shown workspace's terminals, so the prompt names none.
    if let Action::Workspace(WorkspaceAction::CloseWorkspace { workspace }) = &mut action
        && !json
    {
        let listed = match inspect(socket, caller.as_deref(), generation) {
            Ok(snapshot) => snapshot.workspaces,
            Err(error) => return report_failure(&error, json),
        };
        let target = match resolve_selector(
            listed
                .iter()
                .map(|item| (item.id.as_str(), item.name.as_deref())),
            workspace,
        ) {
            Ok(index) => &listed[index],
            Err(error) => return report_failure(&error, json),
        };
        let id = &target.id;
        if let Some(status) = confirm(
            &format!(
                "Close workspace {id}{}? This {}stops every terminal in it. [y/N] ",
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
            &format!("cancelled; workspace {id} stays open\n"),
        )? {
            return Ok(status);
        }
        match inspect(socket, caller.as_deref(), generation) {
            Ok(snapshot) if snapshot.workspaces.iter().any(|item| item.id == *id) => {}
            Ok(_) => {
                return report_failure(
                    &failure("unknown-workspace", format!("workspace {id} is not live")),
                    json,
                );
            }
            Err(error) => return report_failure(&error, json),
        }
        *workspace = id.clone();
    }
    let creating = matches!(
        action,
        Action::Workspace(WorkspaceAction::NewWorkspace { .. })
    );
    let inspecting = !list && matches!(action, Action::Workspace(WorkspaceAction::Inspect));
    let response = match send_action_from(Ok(socket), caller.clone(), action) {
        Ok(response) => response,
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            return report_failure(&failure("malformed-action", error.detail), json);
        }
        Err(error) => {
            return report_failure(&unreachable_supervisor(error, generation), json);
        }
    };
    let window = window_json(resolved.window.as_deref());
    match response {
        ControlResponse::Workspace(Response::Snapshot(snapshot)) => {
            let caller = if inspecting {
                caller_output(
                    &snapshot,
                    caller.as_deref().and_then(terminal_session),
                    json,
                )
            } else {
                String::new()
            };
            // The reply projects the caller's workspace. wN identities rise and
            // are never reused, and the supervisor lists the workspaces in the
            // same request step that creates one, so the highest is the new one.
            let created = creating
                .then(|| {
                    snapshot.workspaces.iter().max_by_key(|item| {
                        item.id
                            .strip_prefix('w')
                            .and_then(|n| n.parse::<u64>().ok())
                    })
                })
                .flatten()
                .map(|item| item.id.as_str());
            write_stdout(if json {
                format!(
                    "{{\"generation\":\"{}\",\"window\":{window},\"workspace\":\"{}\",{caller}{}{}",
                    json_escape(generation),
                    json_escape(&snapshot.workspace),
                    created.map_or_else(String::new, |id| format!(
                        "\"created\":\"{}\",",
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
                    "generation {generation}\nwindow {}\nworkspace {}\n{caller}{}{}",
                    resolved.window.as_deref().unwrap_or("default"),
                    snapshot.workspace,
                    created.map_or_else(String::new, |id| format!("created {id}\n")),
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
                format!(
                    "ended generation {}; window {} closed\n",
                    ended.generation,
                    resolved.window.as_deref().unwrap_or("default")
                )
            })?;
            Ok(0)
        }
        ControlResponse::Lifecycle(_) => {
            Err("Eon supervisor returned a lifecycle result for a workspace action".into())
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
        // Only `eon pane send` asks for this, through `send_terminal`.
        ControlResponse::Workspace(Response::Sent(_)) => {
            Err("Eon supervisor returned a terminal result for a workspace action".into())
        }
    }
}

/// What the 8 MiB bound left out of `eon pane history`'s text (EON-C35),
/// when it left out anything.
fn history_notice(history: &TerminalHistory) -> Option<String> {
    let lines = match history.omitted_lines {
        0 => None,
        1 => Some("the earliest line".to_owned()),
        lines => Some(format!("the {lines} earliest lines")),
    };
    let cut = (history.cut_bytes > 0)
        .then(|| format!("the first {} bytes of the newest line", history.cut_bytes));
    let left_out = match (lines, cut) {
        (None, None) => return None,
        (Some(part), None) | (None, Some(part)) => part,
        (Some(lines), Some(cut)) => format!("{lines} and {cut}"),
    };
    Some(format!(
        "eon: the history is over its {} MiB bound; left out {left_out}",
        MAX_HISTORY_TEXT_BYTES / (1024 * 1024)
    ))
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
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return Ok(report_failure(&error, json)?),
    };
    let action = |input| Action::SendTerminal {
        target: send.target.clone(),
        input,
        force: send.force,
    };
    if let Err(error) = validate_action(caller.clone(), action(TerminalInput::Enter)) {
        return Ok(report_failure(
            &failure("malformed-action", error.detail),
            json,
        )?);
    }
    let resolved = match resolve_workspace(inputs, target, caller.as_deref()) {
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
            Err(error) => Err(unreachable_supervisor(error, &resolved.generation)),
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

fn unreachable_supervisor(error: EndpointFailure, generation: &str) -> Failure {
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
            "generation {generation}: {}; use the terminal's own `eon` from its PATH",
            error.detail
        ),
    )
}

/// The terminals a final close stops, and the workspace it removes while
/// others remain.
type FinalClose = (Vec<String>, Option<String>);

/// The terminals a tab or pane close would stop when it closes the final tab,
/// labelled by pane identity and popup entry, with the workspace's name or wN
/// while other workspaces remain, and `None` for any other action. A close
/// that cannot inspect the workspace first fails rather than skip the
/// question.
fn final_close_terminals(
    socket: &Path,
    action: &Action,
    generation: &str,
    caller: Option<&str>,
) -> Result<Option<FinalClose>, Failure> {
    if !matches!(
        action,
        Action::Workspace(WorkspaceAction::CloseTab { .. }) | Action::ClosePane { .. }
    ) {
        return Ok(None);
    }
    let snapshot = inspect(socket, caller, generation)?;
    let [tab] = snapshot.tabs.as_slice() else {
        return Ok(None);
    };
    let last = match action {
        Action::Workspace(WorkspaceAction::CloseTab { tab: id }) => *id == tab.id,
        // Runtime refuses closing the last pane under a popup, so it is not asked about.
        Action::ClosePane { pane } => {
            let mut panes = tab.panes();
            tab.selected_popup.is_none()
                && panes.next().is_some_and(|only| only.id == *pane)
                && panes.next().is_none()
        }
        _ => false,
    };
    // The close removes the workspace the reply projects while others remain (EON-C33).
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.id == snapshot.workspace)
        .filter(|_| snapshot.workspaces.len() >= 2)
        .map(|workspace| {
            workspace
                .name
                .clone()
                .unwrap_or_else(|| workspace.id.clone())
        });
    Ok(last.then(|| {
        (
            tab.panes()
                .map(|pane| pane.label.clone())
                .chain(tab.popups.iter().map(|popup| popup.entry.clone()))
                .collect(),
            workspace,
        )
    }))
}

/// The final-close question and its cancel note: the window's, or EON-C33's
/// wording for the workspace the close removes while others remain.
fn final_close_prompt(terminals: &[String], workspace: Option<&str>) -> (String, String) {
    let ends = workspace.map_or_else(
        || "quits the window".into(),
        |name| format!("closes workspace {}", name.escape_debug()),
    );
    let question = format!(
        "Close the last tab? This {ends} and stops {} terminal{} [{}]. [y/N] ",
        terminals.len(),
        if terminals.len() == 1 { "" } else { "s" },
        terminals.join(", ")
    );
    let cancelled = workspace.map_or_else(
        || "cancelled; the window stays open\n".into(),
        |name| format!("cancelled; workspace {} stays open\n", name.escape_debug()),
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
        .and_then(|resolved| inspect(&resolved.socket, caller.as_deref(), &resolved.generation))
    {
        Ok(snapshot) => snapshot,
        Err(error) => return report_failure(&error, json),
    };
    let Some(tab) = snapshot
        .tabs
        .iter()
        .find(|tab| tab.popups.iter().any(|item| item.id == popup))
    else {
        let detail = format!(
            "popup {} is not live in this workspace",
            popup.escape_debug()
        );
        return report_failure(&failure("unknown-popup", detail), json);
    };
    let action = Action::DismissPopup(PopupTarget {
        tab: tab.id.clone(),
        instance: popup,
    });
    control(inputs, action, json, false, target)
}

/// The workspace `eon workspace` reports: the caller's, else the shown one.
fn inspect(socket: &Path, caller: Option<&str>, generation: &str) -> Result<Snapshot, Failure> {
    match send_action_from(
        Ok(socket),
        caller.map(str::to_owned),
        Action::Workspace(WorkspaceAction::Inspect),
    ) {
        Ok(ControlResponse::Workspace(Response::Snapshot(snapshot))) => Ok(snapshot),
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => Err(failure),
        Ok(_) => Err(failure(
            "unrepresentable-state",
            "Eon returned the wrong result for workspace inspection",
        )),
        Err(error) => Err(unreachable_supervisor(error, generation)),
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
    let mut target = WorkspaceTarget::default();
    let mut values = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            values.push(argument.clone());
            values.extend(arguments.cloned());
            break;
        }
        // Named operands stay literal, even `--window`.
        if argument == "--name" || argument == "--directory" {
            values.push(argument.clone());
            values.extend(arguments.next().cloned());
            continue;
        }
        let slot = if argument == "--generation" {
            &mut target.generation
        } else if argument == "--window" {
            &mut target.window
        } else {
            values.push(argument.clone());
            continue;
        };
        if slot.is_some() {
            return Err(Error::usage(EON_USAGE));
        }
        let value = arguments
            .next()
            .and_then(|argument| argument.to_str())
            .ok_or_else(|| Error::usage(EON_USAGE))?;
        if (argument == "--generation" && !valid_generation(value))
            || (argument == "--window" && !windows::valid_window_id(value))
        {
            return Err(Error::usage(format!(
                "invalid {argument:?} identity {value:?}"
            )));
        }
        *slot = Some(value.to_owned());
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
            .ok_or_else(|| Error::usage("Eon workspace actions require UTF-8 arguments"))?;
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
        ["workspace"] | ["workspaces"] => Action::Workspace(WorkspaceAction::Inspect),
        ["tab", "create"] => Action::Workspace(WorkspaceAction::CreateTab),
        ["tab", "close", tab] => {
            Action::Workspace(WorkspaceAction::CloseTab { tab: (*tab).into() })
        }
        ["tab", "move", "left"] => Action::Workspace(WorkspaceAction::Move(Direction::Left)),
        ["tab", "move", "right"] => Action::Workspace(WorkspaceAction::Move(Direction::Right)),
        ["pane", "close", pane] => Action::ClosePane {
            pane: (*pane).into(),
        },
        ["pane", "read", target] => Action::ReadTerminal {
            target: (*target).into(),
        },
        ["pane", "history", target] => Action::ReadTerminalHistory {
            target: (*target).into(),
        },
        ["pane", "move", "up"] => Action::Workspace(WorkspaceAction::Move(Direction::Up)),
        ["pane", "move", "down"] => Action::Workspace(WorkspaceAction::Move(Direction::Down)),
        ["pane", "move", "left"] => Action::MovePaneSideways(Side::Left),
        ["pane", "move", "right"] => Action::MovePaneSideways(Side::Right),
        ["focus", "left"] => Action::Workspace(WorkspaceAction::Focus(Direction::Left)),
        ["focus", "right"] => Action::Workspace(WorkspaceAction::Focus(Direction::Right)),
        ["focus", "up"] => Action::Workspace(WorkspaceAction::Focus(Direction::Up)),
        ["focus", "down"] => Action::Workspace(WorkspaceAction::Focus(Direction::Down)),
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
        eon_workspace_protocol::v33::validate_pane_name(name)
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

/// `workspace new|switch|rename|close`. WS is a `wN` or a name, which the
/// runtime resolves; after `--`, operands may start with `-`. Names are
/// trimmed, as pane names are, and an empty rename clears the name.
fn parse_workspace_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, Error> {
    let [command, operation, rest @ ..] = arguments else {
        return Ok(None);
    };
    let operation = match operation.to_str() {
        Some(operation @ ("new" | "switch" | "rename" | "close")) if command == "workspace" => {
            operation
        }
        _ => return Ok(None),
    };
    let (mut json, mut focus, mut name, mut literal) = (false, true, None, false);
    let mut values = Vec::new();
    let mut rest = rest.iter();
    while let Some(argument) = rest.next() {
        let value = argument
            .to_str()
            .ok_or_else(|| Error::usage("Eon workspace names must be UTF-8"))?;
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
            value if !literal && value.starts_with('-') => return Err(Error::usage(EON_USAGE)),
            _ => values.push(value),
        }
    }
    let action = match (operation, values.as_slice()) {
        ("new", []) => WorkspaceAction::NewWorkspace {
            name: name.map(str::to_owned),
            focus,
        },
        ("switch", [workspace]) => WorkspaceAction::SwitchWorkspace {
            workspace: (*workspace).into(),
        },
        ("rename", [name]) => WorkspaceAction::RenameWorkspace {
            workspace: None,
            name: name.trim().into(),
        },
        ("rename", [workspace, name]) => WorkspaceAction::RenameWorkspace {
            workspace: Some((*workspace).into()),
            name: name.trim().into(),
        },
        ("close", [workspace]) => WorkspaceAction::CloseWorkspace {
            workspace: (*workspace).into(),
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
            Some("--directory") if directory.is_none() => {
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
    fn help_flags_that_are_operands_or_anima_options_reach_their_commands() {
        for arguments in [
            &["run", "--", "--help"][..],
            &["edit", "--", "-h"],
            &["anima", "--help"],
            &["anima", "aquarium", "-h"],
            &["pane", "send", "p1", "--", "--help"],
            &["workspace", "rename", "--", "-h"],
            &["workspace", "switch", "--", "--help"],
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
            final_close_prompt(&terminals[..1], None),
            (
                "Close the last tab? This quits the window and stops 1 terminal [p1]. [y/N] "
                    .to_owned(),
                "cancelled; the window stays open\n".to_owned()
            )
        );
        assert_eq!(
            final_close_prompt(&terminals, Some("my \"notes\"")),
            (
                "Close the last tab? This closes workspace my \\\"notes\\\" and stops 2 \
                 terminals [p1, agent]. [y/N] "
                    .to_owned(),
                "cancelled; workspace my \\\"notes\\\" stays open\n".to_owned()
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
            "pane",
            "--window",
            "0000abcd",
            "create",
            "--generation",
            generation,
            "--",
            "echo",
            "--window",
            "literal",
        ])
        .unwrap();
        assert_eq!(target.window.as_deref(), Some("0000abcd"));
        assert_eq!(target.generation.as_deref(), Some(generation));
        assert_eq!(
            args,
            ["pane", "create", "--", "echo", "--window", "literal"].map(OsString::from)
        );
        assert!(
            matches!(parse_control_arguments(&args).unwrap().0, Action::CreateCommandPane { argv, .. } if argv == [b"echo".to_vec(), b"--window".to_vec(), b"literal".to_vec()])
        );
        for args in [
            vec!["workspace", "--generation"],
            vec!["workspace", "--window", "bad/id"],
            vec!["workspace", "--generation", "g1-ABC"],
            vec![
                "workspace",
                "--generation",
                generation,
                "--generation",
                generation,
            ],
            vec!["workspace", "--window", "0000abcd", "--window", "0000abcd"],
        ] {
            assert_eq!(parse(&args).err().unwrap().status, 2, "{args:?}");
        }
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
                .any(|line| line.ends_with("Quit this window        eon quit"))
        );
        for (label, arguments, _) in RUNNER_COMMANDS {
            let arguments = runner_arguments(arguments, "t2");
            assert!(!arguments.contains(&"TAB"), "{label}");
            // The rest stand for what the popup inspects, picks or reads, as the runner fills them.
            let arguments: Vec<OsString> = arguments
                .into_iter()
                .flat_map(|argument| match argument {
                    "PANE" => vec!["p1"],
                    "WORKSPACE" | "WS" => vec!["w2"],
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
            } else if ["workspace", "tab", "pane"].contains(&arguments[0].to_str().unwrap()) {
                assert!(parse_control_arguments(&arguments).is_ok(), "{label}");
            }
        }
    }

    #[test]
    fn workspace_picker_lists_by_recency_with_the_shown_last_and_maps_its_result() {
        let workspace =
            |id: &str, name: Option<&str>, tab_count, recency, attention| WorkspaceInfo {
                id: id.into(),
                name: name.map(Into::into),
                tab_count,
                attention,
                recency,
            };
        // Shown w1, then w3, then w2: the previous one leads, needs input only marks.
        assert_eq!(
            workspace_rows(
                &[
                    workspace("w1", Some("work"), 2, 2, Some(AttentionState::NeedsInput)),
                    workspace("w2", None, 1, 0, Some(AttentionState::NeedsInput)),
                    workspace("w3", Some("notes"), 3, 1, Some(AttentionState::Finished)),
                ],
                "w2",
            ),
            "w3\tnotes\t3 tabs\nw1\twork\t2 tabs · needs input\n\
             w2\tw2\t1 tab · needs input · shown\n"
        );
        // fzf prints the query, the --expect key (empty for Enter), then the row.
        assert_eq!(
            workspace_pick_arguments("wo\n\nw1\twork\t2 tabs\n"),
            Some(vec!["workspace", "switch", "w1"])
        );
        assert_eq!(
            workspace_pick_arguments("\n\nw2\tw2\t1 tab · shown\n"),
            Some(vec!["workspace", "switch", "w2"])
        );
        assert_eq!(
            workspace_pick_arguments("wo\nctrl-x\nw1\twork\t2 tabs\n"),
            Some(vec!["workspace", "close", "w1"])
        );
        // fzf exits 1 on Ctrl+X without a row, so it never prints this.
        assert_eq!(workspace_pick_arguments("zz\nctrl-x\n"), None);
        // Closing another workspace always lists again, and the next listing reports any
        // failure; closing the picker's own w1 lists again only while w1 is still live (it was
        // declined); a failure or any other command ends the picker.
        let live = |ids: &[&str]| {
            Some(
                ids.iter()
                    .map(|id| workspace(id, None, 1, 0, None))
                    .collect::<Vec<_>>(),
            )
        };
        let close = |id| ["workspace", "close", id];
        assert_eq!(workspace_pick_end(&close("w1"), 0, "w1", || None), Some(0));
        assert_eq!(
            workspace_pick_end(&close("w1"), 0, "w1", || live(&["w2"])),
            Some(0)
        );
        assert_eq!(
            workspace_pick_end(&close("w1"), 0, "w1", || live(&["w1"])),
            None
        );
        assert_eq!(
            workspace_pick_end(&close("w2"), 0, "w1", || live(&["w1"])),
            None
        );
        assert_eq!(workspace_pick_end(&close("w2"), 0, "w1", || None), None);
        assert_eq!(
            workspace_pick_end(&close("w2"), 1, "w1", || live(&["w1"])),
            Some(1)
        );
        assert_eq!(
            workspace_pick_end(&["workspace", "switch", "w2"], 0, "w1", || live(&["w1"])),
            Some(0)
        );
        assert_eq!(
            workspace_pick_arguments(" --fresh \n"),
            Some(vec!["workspace", "new", "--name", " --fresh "])
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
            (vec!["--directory", "project"], true, false),
            (
                vec!["--json", "--directory", "project", "--no-focus"],
                false,
                true,
            ),
            (
                vec!["--no-focus", "--json", "--directory", "project"],
                false,
                true,
            ),
            (
                vec!["--directory", "project", "--no-focus", "--json"],
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
                "--directory".into(),
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
        let generation = "g1-0123456789abcdef0123456789abcdef";
        let (args, target) = parse_workspace_target_arguments(
            &[
                "tab",
                "create",
                "--window",
                "0000abcd",
                "--directory",
                "/",
                "--generation",
                generation,
                "--",
                "hx",
                "--window",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(target.window.as_deref(), Some("0000abcd"));
        assert_eq!(target.generation.as_deref(), Some(generation));
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
            &["tab", "create", "--directory", "--window"].map(OsString::from),
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
            vec!["--directory"],
            vec!["--directory", ""],
            vec!["--directory", "--", "hx"],
            vec!["--directory", "/", "--directory", "/"],
            vec!["--json", "--json", "--directory", "/"],
            vec!["--no-focus", "--no-focus", "--directory", "/"],
            vec!["--unknown"],
            vec!["--workspace", "w1", "--directory", "/"],
        ] {
            let mut command = vec!["tab", "create"];
            command.extend(args);
            assert_eq!(parse(&command).unwrap_err().status, 2, "{command:?}");
        }
        for args in [
            vec!["tab", "create", "--"],
            vec!["tab", "create", "--", ""],
            vec!["tab", "create", "--directory", "/", "--"],
            vec!["tab", "create", "--directory", "/", "--", ""],
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
            &["workspace", "--no-focus"],
        ] {
            assert!(parse(arguments).is_err(), "{arguments:?}");
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
    fn close_parsers_require_one_stable_identity() {
        assert_eq!(
            parse_control_arguments(&["tab", "close", "t2", "--json"].map(Into::into)).unwrap(),
            (
                Action::Workspace(WorkspaceAction::CloseTab { tab: "t2".into() }),
                true,
            )
        );
        assert!(parse_control_arguments(&["tab", "close"].map(Into::into)).is_err());
        assert_eq!(
            parse_control_arguments(&["pane", "close", "p3"].map(Into::into)).unwrap(),
            (Action::ClosePane { pane: "p3".into() }, false)
        );
        assert!(parse_control_arguments(&["pane", "close"].map(Into::into)).is_err());
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
    fn pane_read_and_history_take_one_target_and_an_optional_json_flag() {
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
            parse(&["workspaces", "--json"]).unwrap(),
            (Action::Workspace(WorkspaceAction::Inspect), true)
        );
        assert_eq!(
            parse(&["workspace", "new", "--no-focus", "--name", "-dash"]).map_err(|_| ()),
            workspace(WorkspaceAction::NewWorkspace {
                name: Some("-dash".into()),
                focus: false,
            })
        );
        assert_eq!(
            parse(&["workspace", "new", "--name", "--window"]).map_err(|_| ()),
            workspace(WorkspaceAction::NewWorkspace {
                name: Some("--window".into()),
                focus: true,
            })
        );
        assert_eq!(
            parse(&["workspace", "rename", "--", "--json"]).map_err(|_| ()),
            workspace(WorkspaceAction::RenameWorkspace {
                workspace: None,
                name: "--json".into(),
            })
        );
        assert_eq!(
            parse(&["workspace", "rename", "w2", "  "]).map_err(|_| ()),
            workspace(WorkspaceAction::RenameWorkspace {
                workspace: Some("w2".into()),
                name: String::new(),
            })
        );
        for args in [
            &["workspace", "new", "extra"][..],
            &["workspace", "new", "--name"],
            &["workspace", "switch", "w2", "--no-focus"],
            &["workspace", "switch"],
            &["workspace", "close", "w2", "w3"],
            &["workspace", "close", "--force", "w2"],
            &["workspace", "rename"],
            &["workspaces", "--no-focus"],
        ] {
            assert_eq!(parse(args).unwrap_err().status, 2, "{args:?}");
        }
    }
}
