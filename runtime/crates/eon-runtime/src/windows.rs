use super::{
    cli::{Inputs, calling_terminal},
    control::{Error, failure, report_failure, write_stdout},
    generation::{
        Ask, GenerationRecord, Select, WorkspaceTarget, attach_generation, current_generation,
        damaged_generations, generation_directory, private_directory_exists, resolve_workspace,
        running_generations, stop_generation, stop_generations, valid_generation, window_cells,
        window_generation, window_json,
    },
    platform::socket_path_fits,
    sessions::TERMINAL_VARIABLE,
    supervisor::{
        LaunchMode, SESSION_START_TIMEOUT, base_runtime_directory, prepare_runtime,
        probe_supervisor, request_id, runtime_directory, validate_private_directory,
    },
    workspace::{failure_json, json_escape, terminal_text},
};
use eon_workspace_protocol::v34::Failure;
use std::{
    collections::hash_map::RandomState,
    env,
    ffi::OsStr,
    fs::{self, DirBuilder, OpenOptions},
    hash::BuildHasher,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) const WINDOW_RUNTIME: &str = "EON_WINDOW_RUNTIME_DIR";

/// The ID of this build's window in the default namespace (EON-C23).
pub(super) const DEFAULT_WINDOW: &str = "default";

pub(super) fn valid_window_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn windows_root() -> PathBuf {
    base_runtime_directory("eon").join("w")
}

fn existing_windows() -> Result<Vec<(String, PathBuf)>, String> {
    let base = base_runtime_directory("eon");
    if !private_directory_exists(&base, "runtime directory")? {
        return Ok(Vec::new());
    }
    let parent = windows_root();
    if !private_directory_exists(&parent, "window directory")? {
        return Ok(Vec::new());
    }
    let mut windows = Vec::new();
    for entry in fs::read_dir(&parent)
        .map_err(|error| format!("cannot list {}: {error}", parent.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot inspect Eon window: {error}"))?;
        let id = entry.file_name();
        let id = id.to_str().ok_or("Eon window ID is not UTF-8")?;
        if !valid_window_id(id) {
            return Err(format!(
                "invalid Eon window ID {id:?} in {}",
                parent.display()
            ));
        }
        validate_private_directory(&entry.path())?;
        windows.push((id.to_owned(), entry.path()));
    }
    windows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(windows)
}

pub(super) fn selected_window(id: &OsStr) -> Result<PathBuf, Error> {
    let id = id
        .to_str()
        .ok_or_else(|| Error::usage("Eon window ID must be UTF-8"))?;
    if !valid_window_id(id) {
        return Err(Error::usage(format!("invalid Eon window ID {id:?}")));
    }
    window_root(id)?.ok_or_else(|| Error::missing(format!("Eon window {id} was not found")))
}

/// The private directory of independent window `id`, if it exists.
fn window_root(id: &str) -> Result<Option<PathBuf>, String> {
    let parent = windows_root();
    let root = parent.join(id);
    Ok(
        (private_directory_exists(&base_runtime_directory("eon"), "Eon runtime directory")?
            && private_directory_exists(&parent, "Eon windows directory")?
            && private_directory_exists(&root, "Eon window")?)
        .then_some(root),
    )
}

/// A window that an EON-C23 window ID selects.
pub(super) enum Window {
    /// This build's window in the default namespace.
    Default,
    /// Another build's window in the default namespace, by its generation.
    Other(String),
    Independent {
        id: String,
        root: PathBuf,
    },
}

/// A running window as `eon window ls` lists it.
pub(super) struct Listed {
    pub(super) id: String,
    pub(super) short_id: String,
    pub(super) root: PathBuf,
    pub(super) record: GenerationRecord,
}

/// Every running window: the default namespace's, this build's first, then
/// each independent window's by ID (EON-C23).
pub(super) fn running_windows(inputs: &Inputs) -> Result<Vec<Listed>, String> {
    let base = base_runtime_directory("eon");
    let current = current_generation(inputs)?;
    let mut windows = Vec::new();
    for record in running_generations(inputs, &base)? {
        let id = if record.id == current {
            DEFAULT_WINDOW.into()
        } else if valid_generation(&record.id) {
            record.id[3..].to_owned()
        } else {
            record.id.clone()
        };
        windows.push((id, base.clone(), record));
    }
    for (id, root) in existing_windows()? {
        for record in running_generations(inputs, &root)
            .map_err(|error| format!("cannot inspect Eon window {id}: {error}"))?
        {
            windows.push((id.clone(), root.clone(), record));
        }
    }
    let short = short_ids(
        &windows
            .iter()
            .map(|(id, ..)| id.as_str())
            .collect::<Vec<_>>(),
    );
    Ok(windows
        .into_iter()
        .zip(short)
        .map(|((id, root, record), short_id)| Listed {
            // A damaged entry shows its whole raw name.
            short_id: if root == base && !valid_generation(&record.id) {
                id.clone()
            } else {
                short_id
            },
            id,
            root,
            record,
        })
        .collect())
}

/// Each ID's shortest prefix of at least 4 characters that no other distinct
/// ID starts with, else the whole ID; `default` stays whole.
fn short_ids(ids: &[&str]) -> Vec<String> {
    ids.iter()
        .map(|id| {
            if *id == DEFAULT_WINDOW {
                return (*id).to_owned();
            }
            (4..=id.len())
                .filter_map(|length| id.get(..length))
                .find(|prefix| {
                    ids.iter()
                        .all(|other| other == id || !other.starts_with(prefix))
                })
                .unwrap_or(id)
                .to_owned()
        })
        .collect()
}

/// A window ID or prefix as typed: 4 to 32 lowercase hex digits, or a
/// prefix of `default` with at least 4 characters (EON-C23).
pub(super) fn window_id_argument(value: &OsStr) -> Result<&str, Error> {
    let invalid = || Error::usage(format!("invalid Eon window ID {value:?}"));
    let value = value.to_str().ok_or_else(invalid)?;
    if value.is_empty()
        || value.len() > 32
        || !(value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            || DEFAULT_WINDOW.starts_with(value))
    {
        return Err(invalid());
    }
    if value.len() < 4 {
        return Err(Error::usage(format!(
            "Eon window ID {value:?} is too short; type at least 4 characters"
        )));
    }
    Ok(value)
}

#[derive(Debug, PartialEq)]
enum Match {
    One(usize),
    Several(Vec<usize>),
    None,
}

/// The listed IDs `argument` selects: an exact ID, else the one distinct ID
/// it starts.
fn match_id(argument: &str, ids: &[&str]) -> Match {
    if let Some(index) = ids.iter().position(|id| *id == argument) {
        return Match::One(index);
    }
    let matches: Vec<usize> = (0..ids.len())
        .filter(|index| ids[*index].starts_with(argument))
        .collect();
    match matches.as_slice() {
        [] => Match::None,
        [first, rest @ ..] if rest.iter().all(|index| ids[*index] == ids[*first]) => {
            Match::One(*first)
        }
        _ => Match::Several(matches),
    }
}

/// The window a syntactically valid ID or prefix selects (EON-C23).
pub(super) fn select_window(inputs: &Inputs, argument: &str) -> Result<Window, Failure> {
    let base = base_runtime_directory("eon");
    // A default-namespace directory that names no generation keeps its raw
    // name as its ID; an argument that names or reaches one selects nothing,
    // and never another window.
    let unavailable = |error| failure("supervisor-unavailable", error);
    if damaged_generations(&base)
        .map_err(unavailable)?
        .iter()
        .any(|name| name == argument)
    {
        return Err(damaged_window(argument, argument));
    }
    if argument == DEFAULT_WINDOW {
        return Ok(Window::Default);
    }
    if valid_window_id(argument)
        && let Some(root) = window_root(argument).map_err(unavailable)?
    {
        return Ok(Window::Independent {
            id: argument.to_owned(),
            root,
        });
    }
    let listed = running_windows(inputs)
        .map_err(|error| unavailable(format!("cannot list Eon windows: {error}")))?;
    let damaged = |window: &Listed| window.root == base && !valid_generation(&window.record.id);
    let ids: Vec<&str> = listed.iter().map(|window| window.id.as_str()).collect();
    match match_id(argument, &ids) {
        Match::One(index) => {
            let window = &listed[index];
            if listed
                .iter()
                .any(|other| other.id == window.id && damaged(other))
            {
                return Err(damaged_window(argument, &window.id));
            }
            Ok(if window.root != base {
                Window::Independent {
                    id: window.id.clone(),
                    root: window.root.clone(),
                }
            } else if window.id == DEFAULT_WINDOW {
                Window::Default
            } else {
                Window::Other(window.record.id.clone())
            })
        }
        Match::Several(matches) => {
            let mut named: Vec<&str> = Vec::new();
            let mut windows = Vec::new();
            for index in matches {
                let window = &listed[index];
                if !named.contains(&window.id.as_str()) {
                    named.push(&window.id);
                    windows.push(format!("{} ({})", window.short_id, window.record.label()));
                }
            }
            Err(ambiguous_window(argument, &windows))
        }
        Match::None => Err(failure(
            "unknown-window",
            format!(
                "no running Eon window has an ID that starts with {argument}; run `eon window ls` to list them"
            ),
        )),
    }
}

/// The failure for an argument that selects the damaged entry `name`.
fn damaged_window(argument: &str, name: &str) -> Failure {
    failure(
        "unknown-window",
        format!(
            "{argument} selects {name}, a damaged Eon generation directory that is not a window; run `eon window ls` to list them"
        ),
    )
}

/// The `ambiguous-window-id` failure naming every candidate. This CLI failure
/// is not an EONW message, so it keeps each candidate past EONW's detail limit.
fn ambiguous_window(argument: &str, windows: &[String]) -> Failure {
    Failure {
        code: "ambiguous-window-id".into(),
        detail: format!(
            "{argument} starts the IDs of {} Eon windows: {}; type more of the ID",
            windows.len(),
            windows.join(", ")
        ),
    }
}

pub(super) fn runtime_window(root: &Path) -> Option<String> {
    let relative = root.strip_prefix(windows_root()).ok()?;
    let id = relative.to_str()?;
    valid_window_id(id).then(|| id.to_owned())
}

fn window_command(root: &Path) -> Result<Command, String> {
    let binary = env::current_exe()
        .map_err(|error| format!("cannot find the running Eon executable: {error}"))?;
    let mut command = Command::new(binary);
    command.env(WINDOW_RUNTIME, root);
    Ok(command)
}

fn namespace_command(id: Option<&str>, root: &Path) -> Result<Command, String> {
    let mut command = window_command(root)?;
    if id.is_none() {
        command.env_remove(WINDOW_RUNTIME);
    }
    Ok(command)
}

fn namespaces(include_default: bool) -> Result<Vec<(Option<String>, PathBuf)>, String> {
    let mut namespaces = Vec::new();
    if include_default {
        namespaces.push((None, base_runtime_directory("eon")));
    }
    namespaces.extend(
        existing_windows()?
            .into_iter()
            .map(|(id, root)| (Some(id), root)),
    );
    Ok(namespaces)
}

fn candidates(inputs: &Inputs) -> Result<Vec<(Option<String>, PathBuf, GenerationRecord)>, String> {
    let mut candidates = Vec::new();
    for (id, root) in namespaces(true)? {
        for generation in running_generations(inputs, &root)? {
            candidates.push((id.clone(), root.clone(), generation));
        }
    }
    Ok(candidates)
}

pub(super) fn quit(inputs: &Inputs, json: bool) -> Result<i32, Error> {
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => return Ok(report_failure(&error, json)?),
    };
    if let Some(caller) = caller {
        let resolved = match resolve_workspace(inputs, WorkspaceTarget::default(), Some(&caller)) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(report_failure(&error, json)?),
        };
        return Ok(stop_generation(
            inputs,
            &runtime_directory("eon"),
            &resolved.generation,
            json,
            &Ask::Quit {
                caller: Some(caller),
                this_window: true,
            },
        )?);
    }
    let candidates = candidates(inputs)?;
    match candidates.as_slice() {
        [] => Ok(report_failure(
            &failure(
                "missing-supervisor",
                "no Eon window is running; run `eon` first",
            ),
            json,
        )?),
        [(_, root, generation)] => Ok(stop_generation(
            inputs,
            root,
            &generation.id,
            json,
            &Ask::Quit {
                caller: None,
                this_window: false,
            },
        )?),
        candidates => {
            let mut detail = String::from("several Eon windows are running; select one to stop:\n");
            for (id, _, generation) in candidates {
                let generation_id = generation.id.escape_debug().to_string();
                let command = match id {
                    _ if !generation.stop.available => {
                        format!("cannot stop: {}", terminal_text(&generation.stop.reason))
                    }
                    Some(id) => format!("eon window stop {id}"),
                    None => format!("env -u EON_WINDOW_RUNTIME_DIR eon stop {generation_id}"),
                };
                let terminals = if generation.state == "live" {
                    generation.sessions.len().to_string()
                } else {
                    "unknown".into()
                };
                detail.push_str(&format!(
                    "window {} generation {} state={} terminals={terminals}; {command}\n",
                    id.as_deref().unwrap_or("default"),
                    generation_id,
                    generation.state,
                ));
            }
            detail.push_str("or run `eon quit --all` to stop every window");
            if json {
                // This CLI inventory is not an EONW message; retain every candidate.
                Ok(report_failure(
                    &Failure {
                        code: "ambiguous-window".into(),
                        detail,
                    },
                    true,
                )?)
            } else {
                // Damaged generation IDs are escaped above; preserve the inventory's rows.
                eprintln!("error ambiguous-window: {detail}");
                Ok(1)
            }
        }
    }
}

pub(super) fn new_window(inputs: &Inputs) -> Result<i32, String> {
    let generation = current_generation(inputs)?;
    let base = base_runtime_directory("eon");
    prepare_runtime(&base)?;
    let parent = windows_root();
    prepare_runtime(&parent)?;
    let (id, root) = loop {
        let id = format!("{:08x}", RandomState::new().hash_one(request_id()) as u32);
        let root = parent.join(&id);
        if !socket_path_fits(
            &generation_directory(&root, &generation).join("session-9999.sock.management"),
        ) {
            return Err(format!(
                "Eon runtime root {} is too long for independent terminal sockets",
                base.display()
            ));
        }
        match DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => break (id, root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot reserve Eon window {id}: {error}")),
        }
    };
    let log = root.join("launch.log");
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&log)
        .map_err(|error| format!("cannot create {}: {error}", log.display()))?;
    let mut command = window_command(&root)?;
    command
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    // SAFETY: setsid is async-signal-safe and has no parent-process side effects.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start Eon window {id}: {error}"))?;
    let socket = generation_directory(&root, &generation).join("eon.sock");
    let deadline = Instant::now() + SESSION_START_TIMEOUT.saturating_mul(4);
    loop {
        if let Ok((LaunchMode::Workspace, _)) = probe_supervisor(inputs, &socket, &generation) {
            write_stdout(format!("{id}\n"))?;
            return Ok(0);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon window {id}: {error}"))?
        {
            let detail = fs::read_to_string(&log).unwrap_or_default();
            return Err(format!(
                "Eon window {id} exited {status}: {}",
                detail.trim().chars().take(1000).collect::<String>()
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Eon window {id} did not become ready; its process and log remain at {} for inspection",
                root.display()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// `eon window attach [ID]`: presents the selected window, `default` when
/// no ID is given (EON-C23).
pub(super) fn attach_window(inputs: &Inputs, argument: Option<&OsStr>) -> Result<i32, Error> {
    let window = match argument {
        None => Window::Default,
        Some(argument) => match select_window(inputs, window_id_argument(argument)?) {
            Ok(window) => window,
            Err(failure) => return Ok(report_failure(&failure, false)?),
        },
    };
    let base = base_runtime_directory("eon");
    match window {
        Window::Default => {
            let current = current_generation(inputs)?;
            if !running_generations(inputs, &base)?
                .iter()
                .any(|record| record.id == current)
            {
                return Err(Error::unreachable(
                    "the default Eon window is not running; run `eon` to open it",
                ));
            }
            attach_generation(inputs, &base, Some(&current), "eon")
        }
        Window::Other(generation) => attach_generation(inputs, &base, Some(&generation), "eon"),
        Window::Independent { id, root } => {
            let generation = window_generation(inputs, &id, &root)?;
            attach_generation(inputs, &root, Some(&generation), "eon")
        }
    }
}

/// `eon window stop ID`: Stop of the selected window's running generations,
/// asking first in human mode (EON-C23).
pub(super) fn stop_window(inputs: &Inputs, argument: &OsStr, json: bool) -> Result<i32, Error> {
    let window = match select_window(inputs, window_id_argument(argument)?) {
        Ok(window) => window,
        Err(failure) => return Ok(report_failure(&failure, json)?),
    };
    let base = base_runtime_directory("eon");
    let (id, root, select) = match &window {
        Window::Default => (DEFAULT_WINDOW, base, Select::Current),
        Window::Other(generation) => (&generation[3..], base, Select::Generation(generation)),
        Window::Independent { id, root } => (id.as_str(), root.clone(), Select::All),
    };
    if !json {
        return Ok(stop_generations(inputs, &root, select, false, &Ask::Stop)?);
    }
    write_stdout(format!(
        "[{{\"window\":\"{}\",\"results\":",
        json_escape(id)
    ))?;
    // A namespace that cannot be listed still completes the result array.
    let status = match stop_generations(inputs, &root, select, true, &Ask::Stop) {
        Ok(status) => status,
        Err(error) => {
            let report = failure_json(&failure("stop-failed", error));
            write_stdout(format!("[{}]", report.trim_end()))?;
            1
        }
    };
    write_stdout("}]\n")?;
    Ok(status)
}

/// The namespaces, the caller's own last.
fn ordered_namespaces(include_default: bool) -> Result<Vec<(Option<String>, PathBuf)>, String> {
    let current = if include_default && env::var_os(TERMINAL_VARIABLE).is_none() {
        base_runtime_directory("eon")
    } else {
        runtime_directory("eon")
    };
    let mut windows = namespaces(include_default)?;
    windows.sort_by_key(|(_, root)| root == &current);
    Ok(windows)
}

/// `eon quit --all`: in human mode, asks for each window as `eon quit` does,
/// passing the calling terminal to every supervisor; only its owner leaves
/// it out (EON-C11).
pub(super) fn quit_all(inputs: &Inputs, json: bool) -> Result<i32, Error> {
    if json {
        return stop_namespaces(true, true);
    }
    // As with `--json`, a caller that names no terminal excludes nothing.
    let caller = calling_terminal().ok().flatten();
    let ask = Ask::Quit {
        caller,
        this_window: false,
    };
    let mut status = 0;
    for (id, root) in ordered_namespaces(true)? {
        match &id {
            Some(id) => eprintln!("Quit Eon window {id}:"),
            None => eprintln!("Quit the default Eon window:"),
        }
        let code = match stop_generations(inputs, &root, Select::All, false, &ask) {
            Ok(code) => code,
            Err(error) => report_failure(&failure("stop-failed", error), false)?,
        };
        status = status.max(code);
    }
    Ok(status)
}

pub(super) fn stop_namespaces(include_default: bool, json: bool) -> Result<i32, Error> {
    let windows = ordered_namespaces(include_default)?;
    if json {
        write_stdout("[")?;
    }
    let mut status = 0;
    for (index, (id, root)) in windows.into_iter().enumerate() {
        if json {
            let (code, results) = match namespace_command(id.as_deref(), &root) {
                Ok(command) => stop_window_json(command),
                Err(error) => (
                    1,
                    format!(
                        "[{}]",
                        failure_json(&failure("stop-failed", error)).trim_end()
                    ),
                ),
            };
            write_stdout(format!(
                "{}{{\"window\":{},\"results\":{results}}}",
                if index == 0 { "" } else { "," },
                id.as_ref()
                    .map_or_else(|| "null".into(), |id| format!("\"{}\"", json_escape(id)))
            ))?;
            status = status.max(code);
        } else {
            match &id {
                Some(id) => eprintln!("Stop Eon window {id}:"),
                None => eprintln!("Stop the default Eon window:"),
            }
            let code = namespace_command(id.as_deref(), &root)?
                .args(["stop", "all"])
                .status()
                .map_err(|error| format!("cannot run Eon window action: {error}"))?
                .code()
                .unwrap_or(1);
            status = status.max(code);
        }
    }
    if json {
        write_stdout("]\n")?;
    }
    Ok(status)
}

/// Runs the window's `eon stop all --json` and returns its status and result
/// array, or a `stop-failed` array when it yields none.
fn stop_window_json(mut command: Command) -> (i32, String) {
    let output = command
        .args(["stop", "all", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot run Eon window action: {error}"));
    let (code, detail) = match output {
        Ok(output) => {
            let code = output.status.code().unwrap_or(1);
            let results = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if serde_json::from_str::<serde_json::Value>(&results)
                .is_ok_and(|value| value.is_array())
            {
                return (code, results);
            }
            (
                code.max(1),
                format!(
                    "window Stop ended with {} and no result array",
                    output.status
                ),
            )
        }
        Err(error) => (1, error),
    };
    let report = failure_json(&failure("stop-failed", detail));
    (code, format!("[{}]", report.trim_end()))
}

/// `eon window ls [--json]` (EON-C23).
pub(super) fn list_windows(inputs: &Inputs, json: bool) -> Result<i32, String> {
    let windows = running_windows(inputs)?;
    if json {
        let windows: Vec<String> = windows
            .iter()
            .map(|window| window_json(&window.record, &window.id, &window.short_id))
            .collect();
        write_stdout(format!("{{\"windows\":[{}]}}\n", windows.join(",")))?;
        return Ok(0);
    }
    if windows.is_empty() {
        write_stdout("no running Eon windows\n")?;
        return Ok(0);
    }
    let components = inputs.component_report()?;
    let mut rows = vec![
        [
            "ID",
            "NAME",
            "VERSION",
            "STARTED",
            "TERMINALS",
            "COMPATIBLE",
        ]
        .map(String::from),
    ];
    for window in &windows {
        let [name, version, started, terminals, compatible] =
            window_cells(&window.record, components);
        rows.push([
            // A damaged generation directory keeps its raw name as its ID.
            window.short_id.escape_debug().to_string(),
            name,
            version,
            started,
            terminals,
            compatible,
        ]);
    }
    let mut widths = [0; 6];
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut output = String::new();
    for row in &rows {
        let last = row.len() - 1;
        for (column, cell) in row.iter().enumerate() {
            output.push_str(cell);
            if column != last {
                let padding = widths[column] - cell.chars().count() + 2;
                output.extend(std::iter::repeat_n(' ', padding));
            }
        }
        output.push('\n');
    }
    for window in windows
        .iter()
        .filter(|window| !window.record.stop.available)
    {
        output.push_str(&format!(
            "{} cannot stop: {}\n",
            window.short_id.escape_debug(),
            terminal_text(&window.record.stop.reason)
        ));
    }
    write_stdout(output)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{Match, ambiguous_window, match_id, short_ids, window_id_argument};
    use std::ffi::OsStr;

    #[test]
    fn short_ids_are_the_shortest_unique_prefixes_of_at_least_four() {
        let long = format!("abcd1234{}", "f".repeat(24));
        for (ids, short) in [
            (
                vec!["default", "0a1b2c3d", "0a1b9999"],
                vec!["default", "0a1b2", "0a1b9"],
            ),
            (vec!["abcd1234", &long], vec!["abcd1234", "abcd1234f"]),
            (vec!["defa1234", "default"], vec!["defa1", "default"]),
            (vec!["0a1b2c3d"], vec!["0a1b"]),
            (vec!["0a1b2c3d", "0a1b2c3d"], vec!["0a1b", "0a1b"]),
        ] {
            assert_eq!(short_ids(&ids), short, "{ids:?}");
        }
    }

    #[test]
    fn window_id_arguments_take_ids_and_prefixes_of_at_least_four() {
        let full = "0123456789abcdef0123456789abcdef";
        for valid in ["default", "defa", "0a1b", "0a1b2c3d", full] {
            assert_eq!(
                window_id_argument(OsStr::new(valid)).map_err(|error| error.message),
                Ok(valid)
            );
        }
        let long = format!("{full}0");
        for invalid in [
            "",
            "abc",
            "def",
            "ABCD",
            "bad/id",
            "defaults",
            "g1-0123456789abcdef0123456789abcdef",
            &long,
        ] {
            assert_eq!(
                window_id_argument(OsStr::new(invalid)).unwrap_err().status,
                2,
                "{invalid:?}"
            );
        }
        assert!(
            window_id_argument(OsStr::new("abc"))
                .unwrap_err()
                .message
                .contains("too short")
        );
        assert!(
            window_id_argument(OsStr::new("bad/id"))
                .unwrap_err()
                .message
                .contains("invalid Eon window ID")
        );
    }

    /// Catches EONW's 1,024-byte detail limit dropping candidates or the advice.
    #[test]
    fn ambiguity_names_every_candidate_past_the_detail_limit() {
        let name = "n".repeat(64);
        let windows: Vec<String> = (0..20)
            .map(|index| format!("abcd{index:02} ({name}, Eon 0.1.18)"))
            .collect();
        let failure = ambiguous_window("abcd", &windows);
        assert!(failure.detail.len() > 1024, "{}", failure.detail.len());
        assert_eq!(failure.code, "ambiguous-window-id");
        for window in &windows {
            assert!(failure.detail.contains(window.as_str()), "{window}");
        }
        assert!(failure.detail.ends_with("; type more of the ID"));
    }

    #[test]
    fn exact_ids_win_and_prefixes_must_be_unique() {
        let long = format!("abcd1234{}", "0".repeat(24));
        let ids = ["default", "abcd1234", &long, "abce0000", "abce0000"];
        assert_eq!(match_id("abcd1234", &ids), Match::One(1));
        assert_eq!(match_id("abcd12340", &ids), Match::One(2));
        assert_eq!(match_id("abcd", &ids), Match::Several(vec![1, 2]));
        assert_eq!(match_id("abce", &ids), Match::One(3));
        assert_eq!(match_id("defa", &ids), Match::One(0));
        assert_eq!(match_id("ffff", &ids), Match::None);
    }
}
