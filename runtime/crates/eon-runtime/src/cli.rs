use super::{
    control::{
        ControlResponse, EndpointFailure, EndpointFailureKind, Error, failure, report_failure,
        send_action, send_action_from, validate_action, write_stdout,
    },
    generation::{
        WorkspaceTarget, attach_generation, confirm, current_generation, generations_command,
        resolve_workspace, stop_generation, stop_generations, valid_generation,
    },
    managed_environment,
    sessions::{TAB_VARIABLE, TERMINAL_VARIABLE, terminal_session},
    supervisor::{LaunchMode, configuration_directory, launch_current, prepare_configuration},
    windows,
    workspace::{
        caller_output, human as human_output, json as json_output, json_escape, list_human,
        list_json, resolve_selector, terminal_text_json,
    },
};
use eon_workspace_protocol::v26::{
    Action, AttentionState, Direction, Failure, PopupTarget, Response, Side, Snapshot, VERSION,
    WorkspaceAction, WorkspaceInfo, valid_caller,
};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{IsTerminal, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const EON_USAGE: &str = "invalid command or arguments\nusage: eon [COMMAND]\nRun `eon --help` for commands and examples.";
const EONBASE_USAGE: &str = "usage: eonbase [--no-decorations] [--application-id ID] -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json]";

pub struct Inputs {
    pub version: &'static str,
    pub component_report: Result<String, String>,
    pub assembly: &'static [u8],
    pub defaults: managed_environment::Defaults,
    pub orbit: PathBuf,
    pub venus: PathBuf,
    pub anima: PathBuf,
    pub codexbar: PathBuf,
    pub managed: managed_environment::ManagedPrograms,
}

impl Inputs {
    pub(crate) fn component_report(&self) -> Result<&str, String> {
        self.component_report.as_deref().map_err(Clone::clone)
    }
}

pub fn run(inputs: Inputs) -> (&'static str, Result<i32, Error>) {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let mut arguments: Vec<OsString> = arguments.collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == OsStr::new("__directory-picker"))
    {
        arguments.remove(0);
        return (
            "eon-directory-picker",
            directory_picker(&inputs, arguments).map_err(Error::from),
        );
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == OsStr::new("__startup-anima"))
    {
        arguments.remove(0);
        return (
            "eon-directory-picker",
            startup_anima(&inputs, arguments).map_err(Error::from),
        );
    }
    match Path::new(&invocation).file_name().and_then(OsStr::to_str) {
        Some("eonbase") => ("eonbase", execute_eonbase(&inputs, arguments)),
        Some("eon-commands") => ("eon-commands", Ok(command_runner(&inputs))),
        Some("eon-workspaces") => ("eon-workspaces", Ok(workspace_picker(&inputs))),
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
/// popup's own tab, `GENERATION` for its window's generation, `PANE` for its tab's selected pane
/// and `WORKSPACE` for its own workspace. `NAME` is typed in a name box, where an empty name
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
        ("Quit this window", &["stop", "GENERATION"], Close),
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

fn runner_arguments<'a>(arguments: &[&'a str], tab: &'a str, generation: &'a str) -> Vec<&'a str> {
    arguments
        .iter()
        .map(|argument| match *argument {
            "TAB" => tab,
            "GENERATION" => generation,
            _ => argument,
        })
        .collect()
}

/// The picker's lines: index, label and the exact `eon` command each runs.
fn runner_list(tab: &str, generation: &str) -> String {
    RUNNER_COMMANDS
        .iter()
        .enumerate()
        .map(|(index, (label, arguments, _))| {
            let command = runner_arguments(arguments, tab, generation).join(" ");
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
    let generation = current_generation(inputs)?;
    let Some(output) = run_picker(
        packaged_fzf("Command > ", "Enter Run · Esc/Ctrl+C Cancel")
            .args(["--delimiter=\t", "--with-nth=2.."]),
        &runner_list(&tab, &generation),
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
    let mut arguments: Vec<OsString> = runner_arguments(arguments, &tab, &generation)
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
/// that name, and Escape or Ctrl+C does nothing.
fn pick_and_run_workspace(inputs: &Inputs) -> Result<i32, String> {
    let snapshot = caller_snapshot(inputs)?;
    let Some(output) = run_picker(
        packaged_fzf(
            "Workspace > ",
            "Enter Switch · No match: Enter Create · Esc/Ctrl+C Cancel",
        )
        .args([
            "--exact",
            "--delimiter=\t",
            "--with-nth=2..",
            "--nth=1",
            "--print-query",
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
    // Runtime would read a departed row's wN as another workspace's name, so a row switches only
    // while its wN is still live; the check and the switch are not atomic.
    if arguments[1] == "switch"
        && !caller_snapshot(inputs)?
            .workspaces
            .iter()
            .any(|workspace| workspace.id == arguments[2])
    {
        let detail = format!("workspace {} is not live", arguments[2]);
        return report_failure(&failure("unknown-workspace", detail), false);
    }
    Ok(run_as_caller(
        inputs,
        arguments.into_iter().map(OsString::from).collect(),
    ))
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

/// The `eon` arguments for the workspace picker's output: its query line, then the chosen row
/// when one matched. A query that matched no row names a new workspace; a blank one runs nothing.
fn workspace_pick_arguments(output: &str) -> Option<Vec<&str>> {
    let mut lines = output.lines();
    let query = lines.next().unwrap_or_default();
    match lines.next() {
        Some(row) => Some(vec!["workspace", "switch", row.split('\t').next()?]),
        None if !query.trim().is_empty() => Some(vec!["workspace", "new", "--name", query]),
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

fn startup_anima(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, String> {
    let [style, seconds, socket, tab, instance] = arguments.as_slice() else {
        return Err(
            "usage: eon-directory-picker __startup-anima STYLE SECONDS EON_SOCKET TAB POPUP".into(),
        );
    };
    let duration: u64 = seconds
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|value| (1..=30).contains(value))
        .ok_or("invalid internal Anima duration")?;
    if let Err(error) = play_startup_anima(inputs, style, duration) {
        eprintln!("eon: startup Anima: {error}; continuing to the directory picker");
    }
    directory_picker(inputs, vec![socket.clone(), tab.clone(), instance.clone()])
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

/// The packaged fzf with the look the Project picker and Commands share, ignoring the
/// user's own fzf options.
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

fn directory_picker(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, String> {
    let [socket, tab, instance] = arguments.as_slice() else {
        return Err("usage: eon-directory-picker EON_SOCKET TAB POPUP".into());
    };
    let tab = tab
        .to_str()
        .ok_or_else(|| "directory picker tab identity must be UTF-8".to_string())?;
    let instance = instance
        .to_str()
        .ok_or_else(|| "directory picker popup identity must be UTF-8".to_string())?;
    let chooser_file = Path::new(socket)
        .parent()
        .ok_or("directory picker socket has no runtime directory")?
        .join(format!(
            ".directory-picker-selection-{}",
            std::process::id()
        ));
    let mut browse_from = PathBuf::from(".");
    let directory = loop {
        let mut history = Command::new("zoxide")
            .args(["query", "--list"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot launch packaged directory history: {error}"))?;
        let output = packaged_fzf(
            "Quick search > ",
            "Enter Use directory · Tab Browse folders · Esc/Ctrl+C Cancel",
        )
        .args([
            "--exact",
            "--no-sort",
            "--keep-right",
            "--tabstop=1",
            "--expect=tab",
            "--print0",
        ])
        .stdin(
            history
                .stdout
                .take()
                .expect("directory history stdout is piped"),
        )
        .output();
        // No quick-search exit needs more input, including early Enter and Tab.
        let history_stopped = history.kill().is_ok();
        let history_status = history
            .wait()
            .map_err(|error| format!("cannot reap directory history: {error}"))?;
        let output =
            output.map_err(|error| format!("cannot launch packaged directory picker: {error}"))?;
        if output.status.code() == Some(130) {
            return Ok(0);
        }
        if !(history_status.success()
            || history_stopped && history_status.signal() == Some(libc::SIGKILL))
        {
            return Err(format!(
                "packaged directory history exited with status {history_status}"
            ));
        }
        let fields: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
        match (output.status.code(), fields.as_slice()) {
            // fzf reports status 1 for an expected key when the result list is empty.
            (Some(0 | 1), [b"tab", b""] | [b"tab", _, b""]) => {
                let output = browse_directory(inputs, &browse_from, &chooser_file)?;
                if output.status.code() == Some(130) {
                    return Ok(0);
                }
                if output.status.success() {
                    break output.stdout;
                }
                browse_from = PathBuf::from(OsStr::from_bytes(&output.stdout));
            }
            (Some(0), [b"", directory, b""]) if !directory.is_empty() => break directory.to_vec(),
            (Some(0), _) => return Err("directory picker returned an invalid selection".into()),
            _ => {
                return Err(format!(
                    "packaged directory picker exited with status {}",
                    output.status
                ));
            }
        }
    };
    match send_action(
        Path::new(socket),
        Action::CommitDirectory {
            target: PopupTarget {
                tab: tab.into(),
                instance: instance.into(),
            },
            directory,
        },
    ) {
        Ok(ControlResponse::Workspace(Response::Snapshot(_))) => Ok(0),
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => Err(format!(
            "cannot retarget tab: {}: {}",
            failure.code, failure.detail
        )),
        Ok(
            ControlResponse::Workspace(Response::Ended(_) | Response::Text(_) | Response::Sent(_))
            | ControlResponse::Lifecycle(_),
        ) => Err("Eon returned the wrong result for the directory picker".into()),
        Err(error) => Err(format!("cannot retarget tab: {}", error.detail)),
    }
}

fn browse_directory(
    inputs: &Inputs,
    directory: &Path,
    chooser_file: &Path,
) -> Result<Output, String> {
    let config = env::var_os("EON_DIRECTORY_PICKER_CONFIG")
        .ok_or("the installed Eon package has no folder browser configuration")?;
    match fs::remove_file(chooser_file) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!(
                "cannot reset folder browser selection {}: {error}",
                chooser_file.display()
            ));
        }
        _ => {}
    }
    let mut output = Command::new(&inputs.managed.yazi)
        // Yazi draws through its terminal handle; stdout carries its raw CWD.
        .args(["--cwd-file", "/dev/stdout", "--chooser-file"])
        .arg(chooser_file)
        .arg("--")
        .arg(directory)
        // Yazi prefers PWD even when it disagrees with the terminal's actual CWD.
        .env_remove("PWD")
        .env("YAZI_CONFIG_HOME", config)
        .env(
            "YAZI_ZOXIDE_OPTS",
            "--no-preview --border=none --footer='Enter Jump · Esc Back to folders' --footer-border=none --color=footer:-1",
        )
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot launch packaged folder browser: {error}"))?;
    if output.status.code() == Some(130) {
        return Ok(output);
    }
    // The packaged Yazi Tab binding returns its CWD without accepting it.
    if !matches!(output.status.code(), Some(0 | 10)) {
        return Err(format!(
            "packaged folder browser exited with status {}",
            output.status
        ));
    }
    if output.status.success() {
        // In chooser mode, native Yazi `open` writes the hovered path separately.
        output.stdout = fs::read(chooser_file).map_err(|error| {
            format!(
                "cannot read folder browser selection {}: {error}",
                chooser_file.display()
            )
        })?;
        fs::remove_file(chooser_file).map_err(|error| {
            format!(
                "cannot clear folder browser selection {}: {error}",
                chooser_file.display()
            )
        })?;
    }
    if output.stdout.is_empty() {
        return Err("folder browser returned no directory".into());
    }
    Ok(output)
}

fn launch_managed(
    inputs: &Inputs,
    tool: managed_environment::Tool,
    arguments: Vec<OsString>,
) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut command =
        managed_environment::command(tool, &inputs.managed, &config, &arguments, &inputs.defaults)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn execute(inputs: &Inputs, arguments: Vec<OsString>) -> Result<i32, Error> {
    if let Some(code) = lifecycle_command(inputs, &arguments, "eon")? {
        return Ok(code);
    }
    let result = match arguments.as_slice() {
        [flag] if flag == "-h" || flag == "--help" => print_help(inputs),
        [] => launch_current(inputs, LaunchMode::Workspace, &[], true, false, "eon"),
        [command, action] if command == "window" && action == "new" => windows::new_window(inputs),
        [command, action, id] if command == "window" && action == "attach" => {
            Ok(windows::attach_window(inputs, id)?)
        }
        [command, action, all] if command == "window" && action == "stop" && all == "all" => {
            Ok(windows::stop_all_windows(false)?)
        }
        [command, action, all, flag]
            if command == "window" && action == "stop" && all == "all" && flag == "--json" =>
        {
            Ok(windows::stop_all_windows(true)?)
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
            let (action, json) = parse_control_arguments(&arguments)?;
            let list = arguments.first().is_some_and(|value| value == "workspaces");
            control(inputs, action, json, list, target)
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
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            write_stdout(format!("{}\n", path.display()))?;
            Ok(0)
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

/// The first words `execute` hands to `parse_control_arguments`.
fn control_command(command: &OsStr) -> bool {
    [
        "workspace",
        "workspaces",
        "tab",
        "pane",
        "edit",
        "focus",
        "--generation",
        "--window",
    ]
    .iter()
    .any(|word| command == *word)
}

fn print_config_defaults(inputs: &Inputs, json: bool) -> Result<i32, String> {
    let output = managed_environment::print_defaults(&inputs.defaults, json)?;
    std::io::stdout()
        .lock()
        .write_all(output.as_bytes())
        .map_err(|error| format!("cannot write stdout: {error}"))?;
    Ok(0)
}

fn print_help(inputs: &Inputs) -> Result<i32, String> {
    let color = std::io::stdout().is_terminal()
        && env::var_os("TERM").as_deref() != Some(OsStr::new("dumb"))
        && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    write_stdout(help_text(inputs.version, color))?;
    Ok(0)
}

fn help_text(version: &str, color: bool) -> String {
    let (heading, command, reset) = if color {
        ("\x1b[1;35m", "\x1b[36m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    format!(
        "{heading}Eon{reset} {version}  ·  Terminal workspaces. Persistent terminals.

{heading}Usage{reset}  {command}eon [COMMAND]{reset}

{heading}Start{reset}
  {command}eon{reset}                                         Open or present the workspace
  {command}run [-- COMMAND...]{reset}                         Start a fresh workspace

{heading}Windows{reset}
  {command}window new{reset}                                  Open an independent window
  {command}window attach ID{reset}                            Reopen a window
  {command}window stop ID [--json]{reset}                     End that window's terminals
  {command}window stop all [--json]{reset}                    End every window's terminals
  {command}windows [--json]{reset}                            List independent windows

{heading}Attach & stop{reset}
  {command}attach [GENERATION]{reset}                         Reattach to current or named work
  {command}generations [--json]{reset}                        List current and older work
  {command}stop GENERATION|previous|all [--json]{reset}       End selected terminals

{heading}Workspaces{reset}
  {command}workspaces [--json]{reset}                         List this window's workspaces
  {command}workspace [--json]{reset}                          Inspect the workspace
  {command}workspace new [--json] [--no-focus]{reset}         Add a workspace and show it
  {command}workspace switch WS [--json]{reset}                Show workspace WS
  {command}workspace rename [WS] NAME [--json]{reset}         Name a workspace; empty clears
  {command}workspace close WS [--json]{reset}                 Stop its terminals and remove it

{heading}Tabs & panes{reset}
  {command}tab create [--json]{reset}                         Choose a directory for a new tab
  {command}tab create --directory DIRECTORY{reset}            Start a tab without the picker
    {command}[--json] [--no-focus] [-- COMMAND...]{reset}     COMMAND needs an Eon terminal
  {command}tab close TAB [--json]{reset}                      Close the active tab
  {command}tab directory TAB [--json] -- DIRECTORY{reset}     Set the tab's launch directory
  {command}tab move left|right [--json]{reset}                Move the active tab
  {command}pane create [--json] [--no-focus]{reset}           Add a pane to the active tab
  {command}pane create [--json] -- COMMAND...{reset}          Run COMMAND in a new pane
  {command}edit [--json] -- FILE{reset}                       Open FILE in Helix in a new pane
  {command}pane close PANE [--json]{reset}                    Close a pane of the active tab
  {command}pane rename PANE NAME [--json]{reset}              Pin a pane name; empty clears
  {command}pane rename PANE --clear [--json]{reset}           Restore automatic pane naming
  {command}pane move up|down|left|right [--json]{reset}       Move the selected pane
  {command}pane read PANE|POPUP [--json]{reset}               Print a pane's or popup's text
  {command}focus ID|left|right|up|down [--json]{reset}        Focus by ID or navigate

{heading}Tools & information{reset}
  {command}anima [STYLE] [CHILD OPTIONS...]{reset}            Play an animation here
  {command}versions{reset}                                    Show exact component versions
  {command}config-path{reset}                                 Create and print the config root
  {command}config defaults [--json]{reset}                    Print built-in configuration
  {command}-h, --help{reset}                                  Show this help

{heading}Examples{reset}
  {command}eon run -- eon-nu -c 'print hello'{reset}          Start with a command
  {command}eon windows --json{reset}                          Inspect windows as JSON
  {command}eon anima --help{reset}                            Browse animation styles/options

Closing a window detaches. Stop ends its terminals.
Inside Eon, workspace commands use the calling terminal's supervisor/workspace.
Tab move and pane commands retain their calling-tab rules.
Workspace flags: --generation GENERATION / --window ID, before any --.
Explicit targets still validate EON_TERMINAL; window-only uses attach's rule.
Results name generation, window (null for default), and workspace.
WS is a workspace ID (w1, w2, ...) or its exact name.
workspace new --name NAME names the new workspace; created gives its ID.
With --no-focus, workspace new keeps the shown workspace, and either
pane create keeps the selected pane and any popup.
pane read reaches hidden workspaces; [agents] terminal_access governs it.
Where shown, {command}--json{reset} prints JSON and skips confirmation.
Without a terminal, a confirmation needs yes or exits 1 (not-confirmed).
Exit status: 0 success, 1 failed, 2 invalid command, 3 target not found,
4 Eon unreachable, 130 interrupted.
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
    Ok(match argument.to_str() {
        Some("previous") if product == "eon" => stop_generations(inputs, false, json, product)?,
        Some("all") if product == "eon" => stop_generations(inputs, true, json, product)?,
        _ => stop_generation(inputs, generation_argument(argument)?, json, product)?,
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
    // and the close are not atomic. v26 lists only
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
    let window = resolved
        .window
        .as_deref()
        .map_or_else(|| "null".into(), |id| format!("\"{}\"", json_escape(id)));
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
        // Only `eon pane send` asks for this.
        ControlResponse::Workspace(Response::Sent(_)) => {
            Err("Eon supervisor returned a terminal result for a workspace action".into())
        }
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
fn calling_terminal() -> Result<Option<String>, Failure> {
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
        eon_workspace_protocol::v26::validate_pane_name(name)
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

/// Directory-based tab creation leaves bare `tab create` on the picker path.
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
    fn command_runner_workspace_entries_parse_as_eon_commands() {
        let generation = "g1-0123456789abcdef0123456789abcdef";
        assert!(
            runner_list("t2", generation)
                .lines()
                .any(|line| line
                    .ends_with(&format!("Quit this window        eon stop {generation}")))
        );
        for (label, arguments, _) in RUNNER_COMMANDS {
            let arguments = runner_arguments(arguments, "t2", generation);
            assert!(
                !arguments.contains(&"TAB") && !arguments.contains(&"GENERATION"),
                "{label}"
            );
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
            if ["workspace", "tab", "pane"].contains(&arguments[0].to_str().unwrap()) {
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
        assert_eq!(
            workspace_pick_arguments("wo\nw1\twork\t2 tabs\n"),
            Some(vec!["workspace", "switch", "w1"])
        );
        assert_eq!(
            workspace_pick_arguments("\nw2\tw2\t1 tab · shown\n"),
            Some(vec!["workspace", "switch", "w2"])
        );
        assert_eq!(
            workspace_pick_arguments(" --fresh \n"),
            Some(vec!["workspace", "new", "--name", " --fresh "])
        );
        assert_eq!(workspace_pick_arguments("  \n"), None);
        assert_eq!(workspace_pick_arguments(""), None);
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
    fn pane_read_takes_one_target_and_an_optional_json_flag() {
        let read = |target: &str| Action::ReadTerminal {
            target: target.into(),
        };
        assert_eq!(
            parse_control_arguments(&["pane", "read", "u2", "--json"].map(Into::into)).unwrap(),
            (read("u2"), true)
        );
        assert_eq!(
            parse_control_arguments(&["pane", "read", "p1"].map(Into::into)).unwrap(),
            (read("p1"), false)
        );
        for args in [
            &["pane", "read"][..],
            &["pane", "read", "p1", "p2"],
            &["pane", "read", "--", "p1"],
            &["pane", "read", "p1", "--no-focus"],
            &["pane", "read", "p1", "--json", "--json"],
        ] {
            assert!(
                parse_control_arguments(&args.iter().map(OsString::from).collect::<Vec<_>>())
                    .is_err(),
                "{args:?}"
            );
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
