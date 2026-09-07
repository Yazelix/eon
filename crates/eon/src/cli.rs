use super::{
    control::{
        ControlResponse, EndpointFailureKind, failure, report_failure, send_action, write_stdout,
    },
    generation::{
        attach_generation, current_generation, generations_command, stop_generation,
        valid_generation,
    },
    managed_environment,
    supervisor::{
        LaunchMode, MANIFEST, configuration_directory, launch_current, prepare_configuration,
        prepare_generation_runtime, runtime_directory,
    },
    workspace::{human as human_output, json as json_output},
};
use eon_workspace_protocol::v4::{Action, Direction, Response, VERSION};
use std::{
    env,
    ffi::{OsStr, OsString},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
};

const EON_USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach [GENERATION] | generations [--json] | stop GENERATION [--json] | workspace [--json] | tab create [--json] | tab close TAB [--json] | tab directory TAB [--json] -- DIRECTORY | tab move <left|right> [--json] | pane create [--json] | pane move <up|down> [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
const EONTERM_USAGE: &str = "usage: eonterm [--no-decorations] [--application-id ID] -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json]";

pub(super) fn run() -> (&'static str, Result<i32, String>) {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let mut arguments: Vec<OsString> = arguments.collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == OsStr::new("__directory-picker"))
    {
        arguments.remove(0);
        return ("eon-directory-picker", directory_picker(arguments));
    }
    let eonterm = Path::new(&invocation).file_name() == Some(OsStr::new("eonterm"));
    let product = if eonterm { "eonterm" } else { "eon" };
    let result = if eonterm {
        execute_eonterm(arguments)
    } else {
        match managed_environment::tool(&invocation) {
            Some(tool) => launch_managed(tool, arguments),
            None => execute(arguments),
        }
    };
    (product, result)
}

fn directory_picker(arguments: Vec<OsString>) -> Result<i32, String> {
    let [socket, tab] = arguments.as_slice() else {
        return Err("usage: eon-directory-picker EON_SOCKET TAB".into());
    };
    let tab = tab
        .to_str()
        .ok_or_else(|| "directory picker tab identity must be UTF-8".to_string())?;
    let mut history = Command::new("zoxide")
        .args(["query", "--list"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot launch packaged directory history: {error}"))?;
    let output = Command::new("fzf")
        .args([
            "--exact", "--no-sort", "--bind=ctrl-z:ignore,btab:up,tab:down",
            "--cycle", "--keep-right", "--info=inline", "--layout=reverse",
            "--tabstop=1", "--border=none", "--expect=esc", "--print0",
            "--prompt=Quick search > ",
            "--header=Enter Open · Esc Browse folders · Ctrl+C Cancel\nBrowser: arrows Navigate · Enter Use current folder · Shift+Z Search · F1 Help",
        ])
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .stdin(history.stdout.take().expect("directory history stdout is piped"))
        .stderr(Stdio::inherit())
        .output();
    if output.is_err() {
        let _ = history.kill();
    }
    let history_status = history
        .wait()
        .map_err(|error| format!("cannot reap directory history: {error}"))?;
    let output =
        output.map_err(|error| format!("cannot launch packaged directory picker: {error}"))?;
    if !history_status.success() {
        return Err(format!(
            "packaged directory history exited with status {history_status}"
        ));
    }
    if output.status.code() == Some(130) {
        return Ok(0);
    }
    let fields: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    let browse = matches!(fields.as_slice(), [b"esc", b""] | [b"esc", _, b""]);
    // fzf reports status 1 for an expected key when the result list is empty.
    if !output.status.success() && !(browse && output.status.code() == Some(1)) {
        return Err(format!(
            "packaged directory picker exited with status {}",
            output.status
        ));
    }
    let directory = match fields.as_slice() {
        [b"esc", b""] | [b"esc", _, b""] => match browse_directory()? {
            Some(directory) => directory,
            None => return Ok(0),
        },
        [b"", directory, b""] if !directory.is_empty() => directory.to_vec(),
        _ => return Err("directory picker returned an invalid selection".into()),
    };
    match send_action(
        Path::new(socket),
        Action::SetTabDirectory {
            tab: tab.into(),
            directory,
        },
    ) {
        Ok(ControlResponse::Workspace(Response::Snapshot(_))) => Ok(0),
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => Err(format!(
            "cannot retarget tab: {}: {}",
            failure.code, failure.detail
        )),
        Ok(ControlResponse::Lifecycle(_)) => {
            Err("Eon returned a lifecycle result for the directory picker".into())
        }
        Err(error) => Err(format!("cannot retarget tab: {}", error.detail)),
    }
}

fn browse_directory() -> Result<Option<Vec<u8>>, String> {
    let config = env::var_os("EON_DIRECTORY_PICKER_CONFIG")
        .ok_or("the installed Eon package has no folder browser configuration")?;
    let output = Command::new(managed_environment::configured_program("EON_YAZI", "yazi"))
        // Yazi draws through its terminal handle; stdout carries only the raw CWD.
        .args(["--cwd-file", "/dev/stdout"])
        // Yazi prefers PWD even when it disagrees with the Session's actual CWD.
        .env_remove("PWD")
        .env("YAZI_CONFIG_HOME", config)
        .env(
            "YAZI_ZOXIDE_OPTS",
            "--no-preview --border=none --header='Enter Jump · Esc Back to folders'",
        )
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot launch packaged folder browser: {error}"))?;
    if output.status.code() == Some(130) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(format!(
            "packaged folder browser exited with status {}",
            output.status
        ));
    }
    if output.stdout.is_empty() {
        return Err("folder browser returned no directory".into());
    }
    Ok(Some(output.stdout))
}

fn launch_managed(
    tool: managed_environment::Tool,
    arguments: Vec<OsString>,
) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut command = managed_environment::command(tool, &config, &arguments)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eon")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [] => launch_current(LaunchMode::Workspace, &[], true, false, "eon"),
        [command] if command == "run" => {
            launch_current(LaunchMode::Workspace, &[], false, false, "eon")
        }
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Workspace, child, false, false, "eon")
        }
        [command, ..]
            if command == "workspace"
                || command == "tab"
                || command == "pane"
                || command == "focus" =>
        {
            control(&arguments)
        }
        [command] if command == "versions" => {
            write_stdout(format!(
                "eon {} {}\neonw {}\n{}\n",
                env!("CARGO_PKG_VERSION"),
                current_generation()?,
                VERSION,
                eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?
            ))?;
            Ok(0)
        }
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            write_stdout(format!("{}\n", path.display()))?;
            Ok(0)
        }
        _ => Err(EON_USAGE.into()),
    }
}

fn execute_eonterm(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eonterm")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [separator, child @ ..] if separator == "--" && !child.is_empty() => {
            launch_current(LaunchMode::Terminal, child, true, true, "eonterm")
        }
        [flag, separator, child @ ..]
            if flag == "--no-decorations" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Terminal, child, true, false, "eonterm")
        }
        [flag, application_id, separator, child @ ..]
            if flag == "--application-id" && separator == "--" && !child.is_empty() =>
        {
            launch_current(
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
                LaunchMode::Terminal,
                child,
                true,
                false,
                application_id_argument(application_id)?,
            )
        }
        _ => Err(EONTERM_USAGE.into()),
    }
}

fn application_id_argument(argument: &OsStr) -> Result<&str, String> {
    let value = argument
        .to_str()
        .ok_or_else(|| "Eon application identities must be UTF-8".to_string())?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(format!("invalid Eon application identity {value:?}"));
    }
    Ok(value)
}

fn lifecycle_command(arguments: &[OsString], product: &str) -> Result<Option<i32>, String> {
    match arguments {
        [command] if command == "attach" => attach_generation(None, product),
        [command, generation] if command == "attach" => {
            attach_generation(Some(generation_argument(generation)?), product)
        }
        [command] if command == "generations" => generations_command(false, product),
        [command, flag] if command == "generations" && flag == "--json" => {
            generations_command(true, product)
        }
        [command, generation] if command == "stop" => {
            stop_generation(generation_argument(generation)?, false, product)
        }
        [command, generation, flag] | [command, flag, generation]
            if command == "stop" && flag == "--json" =>
        {
            stop_generation(generation_argument(generation)?, true, product)
        }
        _ => return Ok(None),
    }
    .map(Some)
}

fn generation_argument(argument: &OsStr) -> Result<&str, String> {
    let generation = argument
        .to_str()
        .ok_or_else(|| "Eon generation identities must be UTF-8".to_string())?;
    if generation != "legacy" && !valid_generation(generation) {
        return Err(format!("invalid Eon generation identity {generation:?}"));
    }
    Ok(generation)
}

fn control(arguments: &[OsString]) -> Result<i32, String> {
    let (action, json) = parse_control_arguments(arguments)?;
    let generation = current_generation()?;
    let root = runtime_directory("eon");
    let runtime = prepare_generation_runtime(&root, &generation)?;
    let socket = runtime.join("eon.sock");
    let response = match send_action(&socket, action) {
        Ok(response) => response,
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            return report_failure(&failure("malformed-action", error.detail), json);
        }
        Err(error) => {
            let code = if error.kind == EndpointFailureKind::Dead {
                "missing-supervisor"
            } else {
                "supervisor-unavailable"
            };
            return report_failure(
                &failure(code, format!("{}; run `eon` first", error.detail)),
                json,
            );
        }
    };
    match response {
        ControlResponse::Workspace(Response::Snapshot(snapshot)) => {
            write_stdout(if json {
                json_output(&snapshot)
            } else {
                human_output(&snapshot)
            })?;
            Ok(0)
        }
        ControlResponse::Workspace(Response::Failure(failure)) => report_failure(&failure, json),
        ControlResponse::Lifecycle(_) => {
            Err("Eon supervisor returned a lifecycle result for a workspace action".into())
        }
    }
}

fn parse_control_arguments(arguments: &[OsString]) -> Result<(Action, bool), String> {
    if let Some(action) = parse_tab_directory_arguments(arguments)? {
        return Ok(action);
    }
    let mut json = false;
    let mut values = Vec::new();
    for argument in arguments {
        let argument = argument
            .to_str()
            .ok_or_else(|| "Eon workspace actions require UTF-8 arguments".to_string())?;
        if argument == "--json" {
            if json {
                return Err(EON_USAGE.into());
            }
            json = true;
        } else {
            values.push(argument);
        }
    }
    let action = match values.as_slice() {
        ["workspace"] => Action::Inspect,
        ["tab", "create"] => Action::CreateTab,
        ["tab", "close", tab] => Action::CloseTab { tab: (*tab).into() },
        ["tab", "move", "left"] => Action::Move(Direction::Left),
        ["tab", "move", "right"] => Action::Move(Direction::Right),
        ["pane", "create"] => Action::CreatePane,
        ["pane", "move", "up"] => Action::Move(Direction::Up),
        ["pane", "move", "down"] => Action::Move(Direction::Down),
        ["focus", "left"] => Action::Focus(Direction::Left),
        ["focus", "right"] => Action::Focus(Direction::Right),
        ["focus", "up"] => Action::Focus(Direction::Up),
        ["focus", "down"] => Action::Focus(Direction::Down),
        ["focus", id] => Action::FocusId((*id).into()),
        _ => return Err(EON_USAGE.into()),
    };
    Ok((action, json))
}

fn parse_tab_directory_arguments(arguments: &[OsString]) -> Result<Option<(Action, bool)>, String> {
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
            return Err(EON_USAGE.into());
        }
        _ => return Ok(None),
    };
    let tab = tab
        .to_str()
        .ok_or_else(|| "Eon tab identities must be UTF-8".to_string())?;
    Ok(Some((
        Action::SetTabDirectory {
            tab: tab.into(),
            directory: directory.as_os_str().as_bytes().to_vec(),
        },
        json,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

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
                Action::SetTabDirectory {
                    tab: "t2".into(),
                    directory: b"/tmp/eon-\xff".to_vec(),
                },
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
    fn reorder_parser_accepts_only_each_workspace_axis() {
        for (scope, name, direction) in [
            ("tab", "left", Direction::Left),
            ("tab", "right", Direction::Right),
            ("pane", "up", Direction::Up),
            ("pane", "down", Direction::Down),
        ] {
            assert_eq!(
                parse_control_arguments(&[scope, "move", name, "--json"].map(Into::into)).unwrap(),
                (Action::Move(direction), true)
            );
        }
        for (scope, name) in [
            ("tab", "up"),
            ("tab", "down"),
            ("pane", "left"),
            ("pane", "right"),
        ] {
            assert!(parse_control_arguments(&[scope, "move", name].map(Into::into)).is_err());
        }
    }

    #[test]
    fn tab_close_parser_requires_one_stable_identity() {
        assert_eq!(
            parse_control_arguments(&["tab", "close", "t2", "--json"].map(Into::into)).unwrap(),
            (Action::CloseTab { tab: "t2".into() }, true)
        );
        assert!(parse_control_arguments(&["tab", "close"].map(Into::into)).is_err());
    }
}
