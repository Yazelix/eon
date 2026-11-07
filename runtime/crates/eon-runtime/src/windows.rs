use super::{
    cli::{Inputs, calling_terminal},
    control::{Error, failure, report_failure, write_stdout},
    generation::{
        Ask, DELEGATION_MARKER, GenerationRecord, WorkspaceTarget, attach_generation,
        caller_generation, current_generation, damaged_generations, delegated_elsewhere,
        delegated_generation, generation_directory, generation_running, private_directory_exists,
        resolve_workspace, running_generations, stop_generation, stop_generations,
        valid_generation, window_cells, window_generation, window_json,
    },
    platform::socket_path_fits,
    supervisor::{
        LaunchMode, SESSION_START_TIMEOUT, base_runtime_directory, prepare_runtime,
        probe_supervisor, request_id, runtime_directory, validate_private_directory,
    },
    workspace::{failure_json, json_escape, terminal_text},
};
use eon_workspace_protocol::v37::Failure;
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
/// Why a command that needs the default window cannot reach it.
pub(super) const DEFAULT_NOT_RUNNING: &str =
    "the default Eon window is not running; run `eon` to open it";

pub(super) fn valid_window_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn windows_root() -> PathBuf {
    base_runtime_directory("eon").join("w")
}

/// An entry under `w/` by name, with its private root or why it is not a safe
/// Eon window.
type WindowDirectory = (String, Result<PathBuf, String>);

/// Each entry under `w/`, sorted by name.
fn window_entries() -> Result<Vec<WindowDirectory>, String> {
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
        let name = entry.file_name();
        let root = match name.to_str() {
            Some(id) if valid_window_id(id) => {
                validate_private_directory(&entry.path()).map(|_| entry.path())
            }
            Some(id) => Err(format!(
                "invalid Eon window ID {id:?} in {}",
                parent.display()
            )),
            None => Err("Eon window ID is not UTF-8".into()),
        };
        windows.push((name.to_string_lossy().into_owned(), root));
    }
    windows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(windows)
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

/// A namespace whose generations cannot be inspected: the default one, as
/// `default`, or an independent window by its entry's name.
struct Uninspectable {
    id: String,
    root: PathBuf,
    error: String,
}

/// The full ID of default-namespace `generation` (EON-C23): `default` for
/// this build's, else its 32 hex digits; a damaged entry keeps its raw name.
pub(super) fn default_namespace_id(generation: &str, current: &str) -> String {
    if generation == current {
        DEFAULT_WINDOW.into()
    } else if valid_generation(generation) {
        generation[3..].to_owned()
    } else {
        generation.to_owned()
    }
}

/// Every running window, as `running_windows` orders it, with each
/// namespace that cannot be inspected in its place.
fn window_listing(inputs: &Inputs) -> Result<Vec<Result<Listed, Uninspectable>>, String> {
    let base = base_runtime_directory("eon");
    let current = current_generation(inputs)?;
    let mut windows = Vec::new();
    match running_generations(inputs, &base) {
        Ok(records) => windows.extend(records.into_iter().map(|record| {
            (
                default_namespace_id(&record.id, &current),
                base.clone(),
                Ok(record),
            )
        })),
        Err(error) => windows.push((
            DEFAULT_WINDOW.into(),
            base.clone(),
            Err(format!(
                "cannot inspect Eon window {DEFAULT_WINDOW}: {error}"
            )),
        )),
    }
    for (id, root) in window_entries()? {
        let records = root
            .and_then(|root| running_generations(inputs, &root).map(|records| (root, records)))
            .map_err(|error| format!("cannot inspect Eon window {id}: {error}"));
        match records {
            Ok((root, records)) => windows.extend(
                records
                    .into_iter()
                    .map(|record| (id.clone(), root.clone(), Ok(record))),
            ),
            Err(error) => windows.push((id.clone(), windows_root().join(&id), Err(error))),
        }
    }
    // Short IDs stay unique among the windows that cannot be inspected too,
    // and never start the hex digits `default` answers to.
    let mut short = short_ids(
        &windows
            .iter()
            .map(|(id, ..)| id.as_str())
            .collect::<Vec<_>>(),
        current.get(3..),
    )
    .into_iter();
    Ok(windows
        .into_iter()
        .map(|(id, root, record)| match (record, short.next()) {
            (Ok(record), short_id) => {
                let short_id = short_id.unwrap_or_default();
                Ok(Listed {
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
            }
            (Err(error), _) => Err(Uninspectable { id, root, error }),
        })
        .collect())
}

/// Every running window: the default namespace's, this build's first, then
/// each independent window's by ID (EON-C23).
pub(super) fn running_windows(inputs: &Inputs) -> Result<Vec<Listed>, String> {
    window_listing(inputs)?
        .into_iter()
        .map(|found| found.map_err(|window| window.error))
        .collect()
}

/// Each ID's shortest prefix of at least 4 characters that no other distinct
/// ID, and not `alias` (the hex digits `default` also answers to), starts
/// with, else the whole ID; `default` stays whole.
fn short_ids(ids: &[&str], alias: Option<&str>) -> Vec<String> {
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
                        && alias.is_none_or(|alias| !alias.starts_with(prefix))
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

/// The listed windows `argument` selects: the one distinct ID it starts,
/// whole or in part. A window also answers to its `alias`, if any. No exact
/// ID wins over another window that `argument` also starts: an independent
/// window's ID and a default-namespace generation's digits share one space.
fn match_id(argument: &str, ids: &[&str], aliases: &[Option<&str>]) -> Match {
    let matches: Vec<usize> = (0..ids.len())
        .filter(|index| {
            ids[*index].starts_with(argument)
                || aliases[*index].is_some_and(|alias| alias.starts_with(argument))
        })
        .collect();
    match matches.as_slice() {
        [] => Match::None,
        [first, rest @ ..] if rest.iter().all(|index| ids[*index] == ids[*first]) => {
            Match::One(*first)
        }
        _ => Match::Several(matches),
    }
}

/// The window a syntactically valid ID or prefix selects (EON-C23), with the
/// ID that people read for it: as `eon window ls` shows it (a window that is
/// not running, as typed).
/// `default` also answers to its generation's hex digits, the form that
/// durable text names it by.
pub(super) fn select_window(inputs: &Inputs, argument: &str) -> Result<(Window, String), Failure> {
    let base = base_runtime_directory("eon");
    // A default-namespace directory that names no generation keeps its raw
    // name as its ID; an argument that names or reaches one selects nothing,
    // and never another window. Only an entry named `default` could be
    // `default`, so it needs no listing.
    let unavailable = |error| failure("supervisor-unavailable", error);
    let damaged = if argument == DEFAULT_WINDOW {
        let entry = generation_directory(&base, DEFAULT_WINDOW);
        match fs::symlink_metadata(&entry) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(unavailable(format!(
                    "cannot inspect {}: {error}",
                    entry.display()
                )));
            }
        }
    } else {
        damaged_generations(&base)
            .map_err(unavailable)?
            .iter()
            .any(|name| name == argument)
    };
    if damaged {
        return Err(damaged_window(argument, argument));
    }
    if argument == DEFAULT_WINDOW {
        return Ok((Window::Default, DEFAULT_WINDOW.into()));
    }
    // Every other argument is matched against every running window, so one
    // that two windows answer to is refused, never picked.
    let listed = running_windows(inputs)
        .map_err(|error| unavailable(format!("cannot list Eon windows: {error}")))?;
    let damaged = |window: &Listed| window.root == base && !valid_generation(&window.record.id);
    let ids: Vec<&str> = listed.iter().map(|window| window.id.as_str()).collect();
    let aliases: Vec<Option<&str>> = listed
        .iter()
        .map(|window| {
            (window.root == base && window.id == DEFAULT_WINDOW).then(|| &window.record.id[3..])
        })
        .collect();
    match match_id(argument, &ids, &aliases) {
        Match::One(index) => {
            let window = &listed[index];
            if listed
                .iter()
                .any(|other| other.id == window.id && damaged(other))
            {
                return Err(damaged_window(argument, &window.id));
            }
            let selected = if window.root != base {
                Window::Independent {
                    id: window.id.clone(),
                    root: window.root.clone(),
                }
            } else if window.id == DEFAULT_WINDOW {
                Window::Default
            } else {
                Window::Other(window.record.id.clone())
            };
            Ok((selected, window.short_id.clone()))
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
        // An independent window that is not running is still selected by its
        // whole ID.
        Match::None if valid_window_id(argument) => {
            match window_root(argument).map_err(unavailable)? {
                Some(root) => Ok((
                    Window::Independent {
                        id: argument.to_owned(),
                        root,
                    },
                    argument.to_owned(),
                )),
                None => Err(unknown_window(argument)),
            }
        }
        Match::None => Err(unknown_window(argument)),
    }
}

fn unknown_window(argument: &str) -> Failure {
    failure(
        "unknown-window",
        format!(
            "no running Eon window has an ID that starts with {argument}; run `eon window ls` to list them"
        ),
    )
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

/// The `ambiguous-window-id` failure naming every candidate.
fn ambiguous_window(argument: &str, windows: &[String]) -> Failure {
    cli_failure(
        "ambiguous-window-id",
        format!(
            "{argument} starts the IDs of {} Eon windows: {}; type more of the ID",
            windows.len(),
            windows.join(", ")
        ),
    )
}

/// A failure this CLI reports itself. It is not an EONW message, so it keeps
/// its whole detail past EONW's detail limit.
fn cli_failure(code: &str, detail: String) -> Failure {
    Failure {
        code: code.into(),
        detail,
    }
}

pub(super) fn runtime_window(root: &Path) -> Option<String> {
    let relative = root.strip_prefix(windows_root()).ok()?;
    let id = relative.to_str()?;
    valid_window_id(id).then(|| id.to_owned())
}

/// Whether independent window `root` holds generation `generation`.
fn runs(root: &Path, generation: &str) -> bool {
    fs::symlink_metadata(generation_directory(root, generation)).is_ok_and(|entry| entry.is_dir())
}

fn window_command(root: &Path) -> Result<Command, String> {
    let binary = env::current_exe()
        .map_err(|error| format!("cannot find the running Eon executable: {error}"))?;
    let mut command = Command::new(binary);
    command.env(WINDOW_RUNTIME, root);
    command.env_remove(DELEGATION_MARKER);
    Ok(command)
}

/// The failure for damaged entry `record`, which no Stop reaches, else none.
/// It names the whole raw name, escaped.
fn damaged_entry(record: &GenerationRecord) -> Option<Failure> {
    (!valid_generation(&record.id)).then(|| {
        cli_failure(
            "stop-unavailable",
            format!(
                "{} is a damaged Eon generation directory, not a window",
                record.id.escape_debug()
            ),
        )
    })
}

/// The ID that durable text, such as a notice or a log, names window
/// `window` (an independent window's ID) or, in the default namespace,
/// `generation` by: the first 8 hex digits of its generation, which every
/// build's `eon window` commands select, where `default` would name a newer
/// build's window after an upgrade (EON-C8).
pub(super) fn durable_window_id(window: Option<&str>, generation: &str) -> String {
    window.map_or_else(
        || generation.get(3..11).unwrap_or(generation).to_owned(),
        str::to_owned,
    )
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
        let root = runtime_directory("eon");
        let id = runtime_window(&root).unwrap_or_else(|| {
            current_generation(inputs).map_or_else(
                |_| resolved.generation.clone(),
                |current| default_namespace_id(&resolved.generation, &current),
            )
        });
        // As `eon window ls` shows it, else in full.
        let window = window_listing(inputs)
            .ok()
            .and_then(|windows| {
                windows
                    .into_iter()
                    .flatten()
                    .find(|window| window.root == root && window.record.id == resolved.generation)
            })
            .map_or(id, |window| window.short_id);
        return Ok(stop_generation(
            inputs,
            &root,
            &resolved.generation,
            json,
            &Ask::Quit {
                caller: Some(caller),
                this_window: true,
                window: &window,
            },
        )?);
    }
    let windows = running_windows(inputs)?;
    match windows.as_slice() {
        [] => Ok(report_failure(
            &failure(
                "missing-supervisor",
                "no Eon window is running; run `eon` first",
            ),
            json,
        )?),
        [window] => Ok(match damaged_entry(&window.record) {
            Some(damaged) => report_failure(&damaged, json)?,
            None => stop_generation(
                inputs,
                &window.root,
                &window.record.id,
                json,
                &Ask::Quit {
                    caller: None,
                    this_window: false,
                    window: &window.short_id,
                },
            )?,
        }),
        windows => {
            let mut detail = String::from("several Eon windows are running; select one to stop:\n");
            for window in windows {
                let record = &window.record;
                // A damaged entry's ID is its raw name.
                let id = window.short_id.escape_debug().to_string();
                let command = if record.stop.available {
                    format!("eon window stop {id}")
                } else {
                    format!("cannot stop: {}", terminal_text(&record.stop.reason))
                };
                let terminals = if record.state == "live" {
                    record.sessions.len().to_string()
                } else {
                    "unknown".into()
                };
                detail.push_str(&format!(
                    "window {id} name={} state={} terminals={terminals}; {command}\n",
                    record.name.as_deref().map_or("-".into(), terminal_text),
                    record.state,
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
                // Damaged IDs are escaped above; preserve the inventory's rows.
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
            &generation_directory(&root, &generation).join("terminal-9999.sock.management"),
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
/// no ID is given (EON-C23). A delegated attach presents only its marked
/// generation, in the selected window's namespace (EON-C11).
pub(super) fn attach_window(inputs: &Inputs, argument: Option<&OsStr>) -> Result<i32, Error> {
    let argument = argument.map(window_id_argument).transpose()?;
    let delegated = delegated_generation(inputs)?;
    let (window, shown) = match argument {
        None => (Window::Default, DEFAULT_WINDOW.to_owned()),
        Some(argument) => match select_window(inputs, argument) {
            Ok(selected) => selected,
            Err(failure) => return Ok(report_failure(&failure, false)?),
        },
    };
    let base = base_runtime_directory("eon");
    match (window, delegated) {
        (Window::Default, Some(generation)) => {
            attach_generation(inputs, &base, Some(&generation), Some(&shown))
        }
        (Window::Independent { root, .. }, Some(generation)) if runs(&root, &generation) => {
            attach_generation(inputs, &root, Some(&generation), Some(&shown))
        }
        (_, Some(_)) => Err(delegated_elsewhere(&shown)),
        (Window::Default, None) => {
            let current = current_generation(inputs)?;
            if !generation_running(inputs, &base, &current)? {
                return Err(Error::unreachable(DEFAULT_NOT_RUNNING));
            }
            attach_generation(inputs, &base, Some(&current), Some(&shown))
        }
        (Window::Other(generation), None) => {
            attach_generation(inputs, &base, Some(&generation), Some(&shown))
        }
        (Window::Independent { id, root }, None) => {
            let generation = window_generation(inputs, &id, &root)?;
            attach_generation(inputs, &root, Some(&generation), Some(&shown))
        }
    }
}

/// `eon window stop ID`: Stop of the selected window's running generations,
/// asking first in human mode (EON-C23).
pub(super) fn stop_window(inputs: &Inputs, argument: &OsStr, json: bool) -> Result<i32, Error> {
    let argument = window_id_argument(argument)?;
    let (window, shown) = match select_window(inputs, argument) {
        Ok(selected) => selected,
        Err(failure) => return Ok(report_failure(&failure, json)?),
    };
    let base = base_runtime_directory("eon");
    // `default` and another build's window each name one generation, which
    // is stopped without listing its namespace.
    let (id, root, exact) = match window {
        Window::Default => (
            DEFAULT_WINDOW.to_owned(),
            base,
            Some(current_generation(inputs)?),
        ),
        Window::Other(generation) => (generation[3..].to_owned(), base, Some(generation)),
        Window::Independent { id, root } => (id, root, None),
    };
    // Scripts keep full IDs; people read the IDs `eon window ls` shows.
    let label: &str = if json { &id } else { &shown };
    let ask = Ask::StopWindow(label);
    let stop = || -> Result<i32, String> {
        let Some(generation) = &exact else {
            return stop_generations(inputs, &root, json, &ask);
        };
        // Inspected before the array opens, so a failure is reported in its
        // place. A window that has ended stops nothing.
        let running = generation_running(inputs, &root, generation)?;
        if json {
            write_stdout("[")?;
        }
        let status = if running {
            stop_generation(inputs, &root, generation, json, &ask).or_else(|error| {
                report_failure(
                    &failure("stop-failed", format!("window {label}: {error}")),
                    json,
                )
            })?
        } else {
            0
        };
        if json {
            write_stdout("]\n")?;
        }
        Ok(status)
    };
    if !json {
        return Ok(stop()?);
    }
    write_stdout(format!(
        "[{{\"window\":\"{}\",\"results\":",
        json_escape(&id)
    ))?;
    // A namespace that cannot be listed still completes the result array.
    let status = match stop() {
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

/// The windows a bulk Stop selects.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Bulk {
    /// `eon window stop previous`: those another Eon build started.
    Previous,
    /// `eon window stop all`: every window.
    All,
    /// `eon quit --all`: every window, asking as `eon quit` does.
    Quit,
}

/// The caller's own window: the namespace and generation whose supervisor
/// validates the calling terminal, whichever build started it. Outside Eon it
/// is `default`. A caller whose window cannot be identified is refused: a
/// guess could stop it before the windows after it.
fn own_window(inputs: &Inputs) -> Result<(PathBuf, String), Failure> {
    let refused = |detail: String| {
        cli_failure(
            "unknown-caller",
            format!(
                "cannot identify the calling terminal's Eon window, so no Stop was sent: {detail}"
            ),
        )
    };
    match calling_terminal() {
        Ok(None) => Ok((
            base_runtime_directory("eon"),
            current_generation(inputs).map_err(refused)?,
        )),
        Ok(Some(caller)) => {
            let root = runtime_directory("eon");
            let current = current_generation(inputs).map_err(refused)?;
            let generation = caller_generation(&root, &current, &caller).map_err(refused)?;
            Ok((root, generation))
        }
        Err(error) => Err(error),
    }
}

/// `eon window stop previous|all` and `eon quit --all`: the validated Stop of
/// each selected running window, each window's generations in turn and the
/// caller's own window last (EON-C23). A namespace that cannot be inspected
/// and a damaged entry are reported and never sent a Stop. Human mode asks as
/// `Ask::StopWindow`, or as `Ask::Quit` with the calling terminal; `--json`
/// prints one result object per window.
pub(super) fn stop_windows(inputs: &Inputs, bulk: Bulk, json: bool) -> Result<i32, Error> {
    let (own_root, own_generation) = match own_window(inputs) {
        Ok(own) => own,
        Err(failure) => return Ok(report_failure(&failure, json)?),
    };
    // As with `--json`, a caller that names no terminal excludes nothing.
    let caller = (bulk == Bulk::Quit)
        .then(|| calling_terminal().ok().flatten())
        .flatten();
    /// One window to stop: a run of listed entries with the same ID and
    /// namespace.
    struct Target {
        id: String,
        short_id: String,
        root: PathBuf,
        records: Result<Vec<GenerationRecord>, String>,
    }
    let mut windows: Vec<Target> = Vec::new();
    for found in window_listing(inputs)? {
        match found {
            Ok(window) => {
                if bulk == Bulk::Previous && window.record.current() {
                    continue;
                }
                if let Some(Target {
                    id,
                    root,
                    records: Ok(records),
                    ..
                }) = windows.last_mut()
                    && *id == window.id
                    && *root == window.root
                {
                    records.push(window.record);
                    continue;
                }
                windows.push(Target {
                    id: window.id,
                    short_id: window.short_id,
                    root: window.root,
                    records: Ok(vec![window.record]),
                });
            }
            Err(Uninspectable { id, root, error }) => windows.push(Target {
                short_id: id.clone(),
                id,
                root,
                records: Err(error),
            }),
        }
    }
    // The caller's own window last.
    windows.sort_by_key(|window| {
        window.root == own_root
            && window
                .records
                .as_ref()
                .is_ok_and(|records| records.iter().any(|record| record.id == own_generation))
    });
    if json {
        write_stdout("[")?;
    }
    let mut status = 0;
    for (
        index,
        Target {
            id,
            short_id,
            root,
            records,
        },
    ) in windows.iter().enumerate()
    {
        if json {
            write_stdout(format!(
                "{}{{\"window\":\"{}\",\"results\":[",
                if index == 0 { "" } else { "," },
                json_escape(id)
            ))?;
        }
        // Scripts keep full IDs; people read the IDs `eon window ls` shows.
        let shown = if json { id } else { short_id };
        let ask = if bulk == Bulk::Quit {
            Ask::Quit {
                caller: caller.clone(),
                this_window: false,
                window: shown,
            }
        } else {
            Ask::StopWindow(shown)
        };
        let codes = match records {
            Err(error) => vec![report_failure(
                &cli_failure("stop-failed", error.clone()),
                json,
            )?],
            Ok(records) => {
                let mut codes = Vec::new();
                for (position, record) in records.iter().enumerate() {
                    if json && position != 0 {
                        write_stdout(",")?;
                    }
                    codes.push(match damaged_entry(record) {
                        Some(damaged) => report_failure(&damaged, json)?,
                        None => match stop_generation(inputs, root, &record.id, json, &ask) {
                            Ok(code) => code,
                            Err(error) => report_failure(
                                &failure("stop-failed", format!("window {shown}: {error}")),
                                json,
                            )?,
                        },
                    });
                }
                codes
            }
        };
        status = codes.into_iter().fold(status, i32::max);
        if json {
            write_stdout("]}")?;
        }
    }
    if json {
        write_stdout("]\n")?;
    }
    Ok(status)
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
    use super::{
        Match, ambiguous_window, durable_window_id, match_id, short_ids, window_id_argument,
    };
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
            assert_eq!(short_ids(&ids, None), short, "{ids:?}");
        }
        // Nor the generation hex `default` answers to.
        let own = format!("0a1b2c{}", "0".repeat(26));
        assert_eq!(
            short_ids(&["default", "0a1b9999"], Some(&own)),
            ["default", "0a1b9"]
        );
        assert_eq!(short_ids(&["0a1b2c3d"], Some(&own)), ["0a1b2c3"]);
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
    fn ids_and_prefixes_must_select_exactly_one_window() {
        let long = format!("abcd1234{}", "0".repeat(24));
        let ids = ["default", "abcd1234", &long, "abce0000", "abce0000"];
        let none = [None; 5];
        // A whole ID that also starts another window's selects neither.
        assert_eq!(
            match_id("abcd1234", &ids, &none),
            Match::Several(vec![1, 2])
        );
        assert_eq!(match_id("abcd12340", &ids, &none), Match::One(2));
        assert_eq!(match_id("abcd", &ids, &none), Match::Several(vec![1, 2]));
        assert_eq!(match_id("abce", &ids, &none), Match::One(3));
        assert_eq!(match_id("defa", &ids, &none), Match::One(0));
        assert_eq!(match_id("ffff", &ids, &none), Match::None);
    }

    #[test]
    fn the_default_window_answers_to_its_generation_hex() {
        let own = format!("1a2b3c4d{}", "0".repeat(24));
        let ids = ["default", "1a2b9999", "abcd1234"];
        let aliases = [Some(own.as_str()), None, None];
        assert_eq!(match_id("1a2b3c4d", &ids, &aliases), Match::One(0));
        assert_eq!(match_id(&own, &ids, &aliases), Match::One(0));
        assert_eq!(match_id("1a2b3", &ids, &aliases), Match::One(0));
        // A prefix shared with an independent window's ID stays ambiguous.
        assert_eq!(match_id("1a2b", &ids, &aliases), Match::Several(vec![0, 1]));
        // An independent window whose ID the hex also is: neither wins.
        let ids = ["default", "1a2b3c4d"];
        assert_eq!(
            match_id("1a2b3c4d", &ids, &aliases[..2]),
            Match::Several(vec![0, 1])
        );
        assert_eq!(match_id("default", &ids, &aliases[..2]), Match::One(0));
    }

    #[test]
    fn durable_text_names_the_default_namespace_by_generation_hex() {
        let generation = format!("g1-1a2b3c4d{}", "0".repeat(24));
        assert_eq!(durable_window_id(None, &generation), "1a2b3c4d");
        assert_eq!(durable_window_id(Some("abcd1234"), &generation), "abcd1234");
    }
}
