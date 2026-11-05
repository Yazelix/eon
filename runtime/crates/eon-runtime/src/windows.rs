use super::{
    cli::{Inputs, calling_terminal},
    control::{Error, failure, report_failure, write_stdout},
    generation::{
        Ask, GenerationRecord, WorkspaceTarget, current_generation, generation_directory,
        private_directory_exists, resolve_workspace, running_generations, stop_generation,
        stop_generations, window_generation, window_inventory,
    },
    platform::socket_path_fits,
    sessions::TERMINAL_VARIABLE,
    supervisor::{
        LaunchMode, SESSION_START_TIMEOUT, base_runtime_directory, prepare_runtime,
        probe_supervisor, request_id, runtime_directory, validate_private_directory,
    },
    workspace::{failure_json, json_escape, terminal_text},
};
use eon_workspace_protocol::v33::Failure;
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
    let parent = windows_root();
    let root = parent.join(id);
    if !private_directory_exists(&base_runtime_directory("eon"), "Eon runtime directory")?
        || !private_directory_exists(&parent, "Eon windows directory")?
        || !private_directory_exists(&root, "Eon window")?
    {
        return Err(Error::missing(format!("Eon window {id} was not found")));
    }
    Ok(root)
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

pub(super) fn attach_window(inputs: &Inputs, id: &OsStr) -> Result<i32, Error> {
    let root = selected_window(id)?;
    let generation = window_generation(inputs, &id.to_string_lossy(), &root)?;
    run_in_window(&root, &["attach", &generation])
}

pub(super) fn stop_window(id: &OsStr, json: bool) -> Result<i32, Error> {
    let root = selected_window(id)?;
    run_in_window(
        &root,
        if json {
            &["stop", "all", "--json"]
        } else {
            &["stop", "all"]
        },
    )
}

fn run_in_window(root: &Path, arguments: &[&str]) -> Result<i32, Error> {
    let status = window_command(root)?
        .args(arguments)
        .status()
        .map_err(|error| format!("cannot run Eon window action: {error}"))?;
    Ok(status.code().unwrap_or(1))
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
        let code = match stop_generations(inputs, &root, true, false, &ask) {
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
            let code = if let Some(id) = id {
                stop_window(OsStr::new(&id), false)?
            } else {
                namespace_command(None, &root)?
                    .args(["stop", "all"])
                    .status()
                    .map_err(|error| format!("cannot run Eon window action: {error}"))?
                    .code()
                    .unwrap_or(1)
            };
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

pub(super) fn list_windows(inputs: &Inputs, json: bool) -> Result<i32, String> {
    let mut output = if json {
        String::from("{\"windows\":[")
    } else {
        String::new()
    };
    for (index, (id, root)) in existing_windows()?.iter().enumerate() {
        let inventory = window_inventory(inputs, root, json)
            .map_err(|error| format!("cannot inspect Eon window {id}: {error}"))?;
        if json {
            if index > 0 {
                output.push(',');
            }
            output.push_str(&format!(
                "{{\"id\":\"{}\",\"ended\":{},\"inventory\":{}}}",
                json_escape(id),
                inventory.is_none(),
                inventory
                    .as_deref()
                    .map_or("{\"generations\":[]}", str::trim)
            ));
        } else if let Some(generations) = inventory {
            output.push_str(&format!("window {id}\n"));
            for line in generations.lines() {
                output.push_str(&format!("  {line}\n"));
            }
        } else {
            output.push_str(&format!("window {id} ended\n"));
        }
    }
    if json {
        output.push_str("]}\n");
    } else if output.is_empty() {
        output.push_str("no Eon windows\n");
    }
    write_stdout(output)?;
    Ok(0)
}
