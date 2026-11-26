//! Owned Helix and Neovim listeners for Editor popups (EON-C10).

use super::{
    control::{self, StreamSocketProbe},
    diagnostics::DiagnosticLog,
    managed_environment::{self, ManagedPrograms, Tool},
    platform,
    sessions::{self, RunningSession},
    supervisor::{effective_uid, prepare_runtime, validate_private_directory},
};
use eon_workspace_protocol::v51::Failure;
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) enum EditCommand {
    Pane(managed_environment::PreparedCommand),
    Popup(managed_environment::PreparedCommand),
    Reuse(ListenerKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ListenerKind {
    Helix,
    Neovim,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u8,
    kind: ListenerKind,
}

const FRAME_LIMIT: usize = 65_536;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);
// Canonical /private/tmp adds eight bytes; keep Editor within the management socket budget.
const SOCKET_DIRECTORY: &str = "ed";

fn failure(code: &str, detail: impl Into<String>) -> Failure {
    Failure {
        code: code.into(),
        detail: detail.into(),
    }
}

/// The marker preserves transport identity across recovery and Ready-before-bind.
pub(super) fn listener_kind(endpoint: &Path) -> Result<Option<ListenerKind>, Failure> {
    let path = socket_path(endpoint).with_extension("owner");
    let unsafe_marker = || {
        failure(
            "editor-unavailable",
            "Editor listener marker is not an owned private file",
        )
    };
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unsafe_marker()),
    };
    let metadata = file.metadata().map_err(|_| unsafe_marker())?;
    if !metadata.is_file() || metadata.uid() != effective_uid() || metadata.mode() & 0o7777 != 0o600
    {
        return Err(unsafe_marker());
    }
    validate_private_directory(path.parent().unwrap())
        .map_err(|detail| failure("editor-unavailable", detail))?;
    // Fixed bound for this tiny version/kind record, never an untrusted allocation.
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(257)
        .read_to_end(&mut bytes)
        .map_err(|_| unsafe_marker())?;
    if !fs::symlink_metadata(&path).is_ok_and(|current| same_file(&metadata, &current)) {
        return Err(unsafe_marker());
    }
    let marker: Marker = serde_json::from_slice(&bytes).map_err(|_| unsupported_marker())?;
    if bytes.len() > 256 || marker.version != 1 {
        return Err(unsupported_marker());
    }
    Ok(Some(marker.kind))
}

fn unsupported_marker() -> Failure {
    failure(
        "editor-unavailable",
        "Editor listener metadata is unsupported; save your work, then restart the Editor popup",
    )
}

pub(super) fn reuse(
    endpoint: &Path,
    file: &Path,
    id: &str,
    kind: ListenerKind,
    key: &eon_workspace_protocol::v51::PopupKey,
) -> Result<(), Failure> {
    match kind {
        ListenerKind::Helix => open(endpoint, file, id),
        ListenerKind::Neovim => exchange(endpoint, neovim_request(file)?, kind, id, Some(key)),
    }
}

fn open(endpoint: &Path, file: &Path, id: &str) -> Result<(), Failure> {
    let path = file.to_str().ok_or_else(|| {
        failure(
            "editor-rejected",
            "the running Editor accepts only UTF-8 file names",
        )
    })?;
    let mut request = serde_json::to_vec(&serde_json::json!({
        "version": 1, "id": id, "action": "open", "paths": [path], "timeout_ms": 750
    }))
    .expect("JSON scalar values serialize");
    request.push(b'\n');
    exchange(endpoint, request, ListenerKind::Helix, id, None)
}

fn should_retry_connect(
    endpoint: &Path,
    socket: &Path,
    kind: ListenerKind,
    error: &io::Error,
) -> Result<bool, Failure> {
    match error.kind() {
        io::ErrorKind::NotFound => Ok(listener_kind(endpoint)? == Some(kind)),
        // Between bind(2) and listen(2) (and on macOS with a full backlog),
        // stream connect returns ECONNREFUSED while a datagram probe still
        // sees the live stream socket (EPROTOTYPE) or a concurrent unlink.
        io::ErrorKind::ConnectionRefused => Ok(listener_kind(endpoint)? == Some(kind)
            && matches!(
                control::probe_stream_socket(socket),
                Ok(StreamSocketProbe::LiveStream | StreamSocketProbe::Missing)
            )),
        _ => Ok(false),
    }
}

fn connect_listener(
    endpoint: &Path,
    socket: &Path,
    kind: ListenerKind,
    mut on_retry: impl FnMut(&io::Error),
) -> Result<std::os::unix::net::UnixStream, Failure> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut waiting_for_bind = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failure(
                if waiting_for_bind {
                    "editor-unavailable"
                } else {
                    "editor-unresponsive"
                },
                "Editor connect exceeded 100 ms; no request sent",
            ));
        }
        match sessions::unix_connect_with_timeout(socket, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) if should_retry_connect(endpoint, socket, kind, &error)? => {
                waiting_for_bind = true;
                on_retry(&error);
                let delay = deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(5));
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            }
            Err(error) => {
                let detail = format!("Editor connection failed: {error}; no request sent");
                let mut failure = io_failure(error);
                failure.detail = detail;
                return Err(failure);
            }
        }
    }
}

fn exchange(
    endpoint: &Path,
    request: Vec<u8>,
    kind: ListenerKind,
    id: &str,
    key: Option<&eon_workspace_protocol::v51::PopupKey>,
) -> Result<(), Failure> {
    if request.len() > FRAME_LIMIT {
        return Err(failure("editor-rejected", "Editor request exceeds 64 KiB"));
    }
    let socket = socket_path(endpoint);
    let mut stream = connect_listener(endpoint, &socket, kind, |_| {})?;
    answer(&mut stream, &request, kind, id).map_err(|mut error| {
        if error.code == "editor-unresponsive"
            && let Some(key) = key
        {
            let shortcut = managed_environment::popup_shortcut(key);
            let advice = if shortcut.is_empty() {
                "no Editor shortcut is active; configure `[popups.editor] keybinding`".to_owned()
            } else {
                format!("press {shortcut} to reveal Editor")
            };
            error
                .detail
                .push_str(&format!("; Editor may be waiting for input; {advice}"));
        }
        error
    })
}

fn answer(
    stream: &mut std::os::unix::net::UnixStream,
    request: &[u8],
    kind: ListenerKind,
    id: &str,
) -> Result<(), Failure> {
    let deadline = Instant::now() + ANSWER_TIMEOUT;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                failure(
                    "editor-unresponsive",
                    "Editor answer exceeded 1 second; open outcome is unknown",
                )
            })
    };
    let mut bytes = request;
    while !bytes.is_empty() {
        sessions::bound_io(stream, stream.set_write_timeout(Some(remaining()?)))
            .map_err(io_failure)?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(failure(
                    "editor-unavailable",
                    "Editor closed before receiving the request; open outcome is unknown",
                ));
            }
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io_failure(error)),
        }
    }
    let mut reply = Vec::new();
    let mut consumed = 0;
    let mut notifications = 0;
    loop {
        remaining()?;
        if kind == ListenerKind::Neovim {
            loop {
                match neovim_reply(&reply[consumed..]) {
                    Ok((count, NeovimReply::Notification)) if notifications < 16 => {
                        consumed += count;
                        notifications += 1;
                    }
                    Ok((_, NeovimReply::Answer(result))) => return result,
                    Err(Decode::Incomplete) => break,
                    _ => {
                        return Err(failure(
                            "editor-unavailable",
                            "invalid or excessive Editor RPC reply; open outcome is unknown",
                        ));
                    }
                }
            }
        }
        sessions::bound_io(stream, stream.set_read_timeout(Some(remaining()?)))
            .map_err(io_failure)?;
        let mut chunk = [0; 4096];
        let count = match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(failure(
                    "editor-unavailable",
                    "Editor closed before a complete reply; open outcome is unknown",
                ));
            }
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_failure(error)),
        };
        let end = (kind == ListenerKind::Helix)
            .then(|| chunk[..count].iter().position(|byte| *byte == b'\n'))
            .flatten();
        reply.extend_from_slice(&chunk[..end.map_or(count, |end| end + 1)]);
        if reply.len() > FRAME_LIMIT
            || (kind == ListenerKind::Helix && reply.len() == FRAME_LIMIT && end.is_none())
        {
            return Err(failure(
                "editor-unavailable",
                "Editor reply exceeds 64 KiB; open outcome is unknown",
            ));
        }
        if end.is_some() {
            break;
        }
    }
    parse_reply(&reply, id)
}

fn io_failure(error: io::Error) -> Failure {
    let code = if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        "editor-unresponsive"
    } else {
        "editor-unavailable"
    };
    failure(
        code,
        format!("Editor exchange failed: {error}; a sent request's open outcome is unknown"),
    )
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    version: u8,
    id: String,
    status: String,
    error: Option<EditorError>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EditorError {
    class: String,
    message: String,
    path_index: Option<usize>,
}

fn parse_reply(bytes: &[u8], id: &str) -> Result<(), Failure> {
    let invalid = || {
        failure(
            "editor-unavailable",
            "invalid Editor reply; open outcome is unknown",
        )
    };
    let answer: Answer = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if answer.version != 1 || answer.id != id {
        return Err(invalid());
    }
    match (answer.status.as_str(), answer.error) {
        ("ok", None) => Ok(()),
        ("error", Some(error)) => {
            if error.path_index.is_some_and(|index| index != 0) {
                return Err(invalid());
            }
            let code = match error.class.as_str() {
                "timeout" => "editor-unresponsive",
                "rejected" | "invalid" | "unsupported" => "editor-rejected",
                "unavailable" => "editor-unavailable",
                _ => return Err(invalid()),
            };
            let detail: String = error
                .message
                .chars()
                .filter(|character| !character.is_control())
                .take(512)
                .collect();
            if detail.is_empty() {
                return Err(invalid());
            }
            Err(failure(
                code,
                if error.class == "timeout" {
                    format!("{detail}; queued open was abandoned")
                } else {
                    detail
                },
            ))
        }
        _ => Err(invalid()),
    }
}

// Paths are RPC arguments; only this constant is executable code.
const NEOVIM_OPEN: &str = r#"
local path, line, column = ...
if line == vim.NIL then line = nil end
if column == vim.NIL then column = nil end
local target = vim.fn.bufadd(path)
if target <= 0 then error('Editor could not add the file buffer', 0) end
local current = vim.api.nvim_get_current_buf()
local hidden = vim.bo[current].bufhidden
if current ~= target and vim.bo[current].modified
    and (hidden == 'unload' or hidden == 'delete' or hidden == 'wipe')
    and #vim.fn.win_findbuf(current) == 1 then
  error('Editor cannot switch: the modified current buffer would be unloaded, deleted or wiped', 0)
end
if not vim.api.nvim_buf_is_loaded(target) then vim.fn.bufload(target) end
if not vim.api.nvim_buf_is_loaded(target) then error('Editor could not load the file buffer', 0) end
vim.bo[target].buflisted = true
vim.api.nvim_cmd({cmd = 'buffer', args = {tostring(target)}, mods = {hide = true}}, {})
if line then vim.fn.cursor(line, column or 1) end
return true
"#;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Location<'a> {
    pub(super) path: &'a str,
    line: Option<i32>,
    column: Option<i32>,
}

pub(super) fn neovim_location(file: &Path) -> Result<Location<'_>, Failure> {
    let path = file.to_str().ok_or_else(|| {
        failure(
            "editor-rejected",
            "Neovim Editor accepts only UTF-8 file names",
        )
    })?;
    let literal = Location {
        path,
        line: None,
        column: None,
    };
    if fs::symlink_metadata(file).is_ok() {
        return Ok(literal);
    }
    let number = |text: &str| {
        (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse::<i32>().ok().map(|value| value.max(1)))
            .flatten()
    };
    let Some((prefix, last)) = path.rsplit_once(':') else {
        return Ok(literal);
    };
    let Some(last) = number(last) else {
        return Ok(literal);
    };
    if let Some((name, previous)) = prefix.rsplit_once(':')
        && !previous.is_empty()
        && previous.bytes().all(|byte| byte.is_ascii_digit())
    {
        let Some(line) = number(previous) else {
            return Ok(literal);
        };
        return Ok(Location {
            path: name,
            line: Some(line),
            column: Some(last),
        });
    }
    Ok(Location {
        path: prefix,
        line: Some(last),
        column: None,
    })
}

/// Append a popup file only after selecting the fresh listener's interpretation.
pub(super) fn initial_file(
    command: &mut managed_environment::PreparedCommand,
    file: &Path,
) -> Result<bool, Failure> {
    match command.listener {
        Some((ListenerKind::Neovim, _)) => {
            let location = neovim_location(file)?;
            if Path::new(location.path).is_dir() {
                return Ok(false);
            }
            if let Some(line) = location.line {
                command
                    .argv
                    .push(format!("+call cursor({line},{})", location.column.unwrap_or(1)).into());
            }
            command
                .argv
                .extend([OsString::from("--"), location.path.into()]);
        }
        Some((ListenerKind::Helix, _)) => command.argv.push(file.as_os_str().to_owned()),
        None => return Ok(false),
    }
    Ok(true)
}

fn neovim_request(file: &Path) -> Result<Vec<u8>, Failure> {
    use rmp::encode::{write_array_len, write_nil, write_sint, write_str, write_uint};
    let location = neovim_location(file)?;
    // Check before encoding: the path alone cannot exceed the complete request budget.
    if location.path.len() > FRAME_LIMIT {
        return Err(failure("editor-rejected", "Editor request exceeds 64 KiB"));
    }
    let mut request = Vec::new();
    write_array_len(&mut request, 4).unwrap();
    write_uint(&mut request, 0).unwrap();
    write_uint(&mut request, 1).unwrap();
    write_str(&mut request, "nvim_exec_lua").unwrap();
    write_array_len(&mut request, 2).unwrap();
    write_str(&mut request, NEOVIM_OPEN).unwrap();
    write_array_len(&mut request, 3).unwrap();
    write_str(&mut request, location.path).unwrap();
    for number in [location.line, location.column] {
        if let Some(number) = number {
            write_sint(&mut request, i64::from(number)).unwrap();
        } else {
            write_nil(&mut request).unwrap();
        }
    }
    Ok(request)
}

#[derive(Debug, PartialEq, Eq)]
enum Decode {
    Incomplete,
    Invalid,
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8], Decode> {
    if count > FRAME_LIMIT {
        return Err(Decode::Invalid);
    }
    let (value, rest) = bytes.split_at_checked(count).ok_or(Decode::Incomplete)?;
    *bytes = rest;
    Ok(value)
}

// Neovim strings carry arbitrary bytes, including in notification/error values.
fn string_bytes<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], Decode> {
    if !matches!(
        rmp::Marker::from_u8(*bytes.first().ok_or(Decode::Incomplete)?),
        rmp::Marker::FixStr(_) | rmp::Marker::Str8 | rmp::Marker::Str16 | rmp::Marker::Str32
    ) {
        return Err(Decode::Invalid);
    }
    let count = rmp::decode::read_str_len(bytes).map_err(|_| Decode::Incomplete)?;
    take(bytes, count as usize)
}

/// Walk bounded borrowed bytes, never allocate from peer-declared lengths.
fn skip_value(bytes: &mut &[u8], depth: usize) -> Result<(), Decode> {
    use rmp::{Marker::*, decode};
    let marker = rmp::Marker::from_u8(*bytes.first().ok_or(Decode::Incomplete)?);
    match marker {
        Reserved => return Err(Decode::Invalid),
        FixArray(_) | Array16 | Array32 | FixMap(_) | Map16 | Map32 => {
            if depth == 32 {
                return Err(Decode::Invalid);
            }
            let count = if matches!(marker, FixMap(_) | Map16 | Map32) {
                decode::read_map_len(bytes)
                    .map_err(|_| Decode::Incomplete)?
                    .checked_mul(2)
                    .ok_or(Decode::Invalid)?
            } else {
                decode::read_array_len(bytes).map_err(|_| Decode::Incomplete)?
            };
            if count as usize > FRAME_LIMIT {
                return Err(Decode::Invalid);
            }
            for _ in 0..count {
                skip_value(bytes, depth + 1)?;
            }
        }
        FixStr(_) | Str8 | Str16 | Str32 => {
            string_bytes(bytes)?;
        }
        Bin8 | Bin16 | Bin32 => {
            let count = decode::read_bin_len(bytes).map_err(|_| Decode::Incomplete)?;
            take(bytes, count as usize)?;
        }
        FixExt1 | FixExt2 | FixExt4 | FixExt8 | FixExt16 | Ext8 | Ext16 | Ext32 => {
            let meta = decode::read_ext_meta(bytes).map_err(|_| Decode::Incomplete)?;
            take(bytes, meta.size as usize)?;
        }
        _ => {
            let count = match marker {
                U8 | I8 => 1,
                U16 | I16 => 2,
                U32 | I32 | F32 => 4,
                U64 | I64 | F64 => 8,
                FixPos(_) | FixNeg(_) | Null | True | False => 0,
                _ => unreachable!("containers and strings handled above"),
            };
            take(bytes, count + 1)?;
        }
    }
    Ok(())
}

enum NeovimReply {
    Notification,
    Answer(Result<(), Failure>),
}

fn neovim_reply(bytes: &[u8]) -> Result<(usize, NeovimReply), Decode> {
    use rmp::decode;
    let mut rest = bytes;
    skip_value(&mut rest, 0)?;
    let consumed = bytes.len() - rest.len();
    let mut value = &bytes[..consumed];
    let fields = decode::read_array_len(&mut value).map_err(|_| Decode::Invalid)?;
    let kind: u64 = decode::read_int(&mut value).map_err(|_| Decode::Invalid)?;
    if fields == 3 && kind == 2 {
        string_bytes(&mut value)?;
        decode::read_array_len(&mut value).map_err(|_| Decode::Invalid)?;
        return Ok((consumed, NeovimReply::Notification));
    }
    if fields != 4
        || kind != 1
        || decode::read_int::<u64, _>(&mut value).map_err(|_| Decode::Invalid)? != 1
    {
        return Err(Decode::Invalid);
    }
    let answer = if value.first() == Some(&0xc0) {
        decode::read_nil(&mut value).map_err(|_| Decode::Invalid)?;
        if !decode::read_bool(&mut value).map_err(|_| Decode::Invalid)? {
            return Err(Decode::Invalid);
        }
        Ok(())
    } else {
        if decode::read_array_len(&mut value).map_err(|_| Decode::Invalid)? != 2 {
            return Err(Decode::Invalid);
        }
        decode::read_int::<i64, _>(&mut value).map_err(|_| Decode::Invalid)?;
        let detail = string_bytes(&mut value)?;
        decode::read_nil(&mut value).map_err(|_| Decode::Invalid)?;
        let detail: String = std::str::from_utf8(detail)
            .unwrap_or("")
            .chars()
            .take_while(|character| !character.is_control())
            .take(512)
            .collect();
        let detail = if detail.is_empty() {
            "Neovim refused the file".to_owned()
        } else {
            detail
        };
        Err(failure("editor-rejected", detail))
    };
    if !value.is_empty() {
        return Err(Decode::Invalid);
    }
    Ok((consumed, NeovimReply::Answer(answer)))
}

pub(super) fn listen_position(
    argv: &[OsString],
    programs: &ManagedPrograms,
    directory: &Path,
) -> Result<Option<usize>, String> {
    let Some(helix) = &programs.helix else {
        return Ok(None);
    };
    let program = Path::new(&argv[0]);
    let Ok(metadata) = fs::metadata(program) else {
        return Ok(None);
    };
    let matches = |path: &Path| fs::metadata(path).is_ok_and(|other| same_file(&metadata, &other));
    let managed = managed_environment::tool(program.as_os_str()) == Some(Tool::Helix)
        && programs
            .shell_bin
            .as_ref()
            .is_some_and(|bin| matches(&directory.join(bin).join("hx")));
    if !matches(&directory.join(helix)) && !managed {
        return Ok(None);
    }

    // The pinned Helix parser consumes the next token for these options, even
    // when it spells "--" or "--listen".
    let mut arguments = argv.iter().enumerate().skip(1);
    while let Some((index, argument)) = arguments.next() {
        match argument.to_str() {
            Some("--") => return Ok(Some(index)),
            Some("--listen") => {
                return Err("Editor supplies the packaged Helix listener; remove --listen from [editor] command to open the popup".into());
            }
            Some(
                "-g" | "--grammar" | "-c" | "--config" | "--config-dir" | "--log" | "-w"
                | "--working-dir",
            ) => {
                arguments
                    .next()
                    .ok_or_else(|| format!("[editor] command's {argument:?} needs an argument"))?;
            }
            _ => {}
        }
    }
    Ok(Some(argv.len()))
}

/// Check Neovim's startup prefix without interpreting option values as flags.
pub(super) fn neovim_position(argv: &[OsString]) -> Result<usize, String> {
    use std::os::unix::ffi::OsStrExt;
    let incompatible = || {
        "[editor] command conflicts with Eon's Neovim listener; remove initial files, stdin (-), tag (-t), quickfix (-q), --listen, --server, --remote*, --, --embed, --headless, -l or Ex mode flags, and leave room in Neovim's ten startup commands".to_owned()
    };
    let mut arguments = argv.iter().skip(1).peekable();
    let mut commands = 0;
    let mut pre_commands = 0;
    while let Some(argument) = arguments.next() {
        let bytes = argument.as_bytes();
        if bytes.starts_with(b"+") {
            commands += 1;
        } else if bytes.starts_with(b"--") {
            let option = bytes.to_ascii_lowercase();
            if option == b"--"
                || option == b"--embed"
                || option == b"--headless"
                || option.starts_with(b"--listen")
                || option.starts_with(b"--server")
                || option.starts_with(b"--remote")
            {
                return Err(incompatible());
            }
            if option == b"--cmd" || option == b"--startuptime" {
                if option == b"--cmd" {
                    pre_commands += 1;
                }
                arguments.next().ok_or_else(incompatible)?;
            }
        } else if bytes.starts_with(b"-") {
            if bytes == b"-" {
                return Err(incompatible());
            }
            let mut index = 1;
            while index < bytes.len() {
                let option = bytes[index];
                index += 1;
                match option {
                    b'e' | b'E' | b'l' | b't' | b'q' => return Err(incompatible()),
                    b'c' => {
                        commands += 1;
                        if index == bytes.len() {
                            arguments.next().ok_or_else(incompatible)?;
                        }
                        break;
                    }
                    b'S' => {
                        commands += 1;
                        if index != bytes.len() {
                            return Err(incompatible());
                        }
                        if arguments
                            .peek()
                            .is_some_and(|next| !next.as_bytes().starts_with(b"-"))
                        {
                            arguments.next();
                        }
                        break;
                    }
                    b'i' | b'u' | b'U' | b'W' | b's' => {
                        if index != bytes.len() {
                            return Err(incompatible());
                        }
                        arguments.next().ok_or_else(incompatible)?;
                        break;
                    }
                    b'V' => break, // The remaining bytes are a level or filename.
                    b'w' if !bytes.get(index).is_some_and(u8::is_ascii_digit) => {
                        if index != bytes.len() {
                            return Err(incompatible());
                        }
                        arguments.next().ok_or_else(incompatible)?;
                        break;
                    }
                    b'w' | b'o' | b'O' | b'p' => {
                        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                            index += 1;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            return Err(incompatible());
        }
        // Reserve one post-config command for Eon's initial cursor position.
        if commands >= 10 || pre_commands > 10 {
            return Err(incompatible());
        }
    }
    Ok(argv.len())
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    (left.dev(), left.ino()) == (right.dev(), right.ino())
}

pub(super) fn validate_listener_path(endpoint: &Path, kind: ListenerKind) -> Result<(), String> {
    if kind == ListenerKind::Neovim {
        let path = canonical_socket_path(endpoint)
            .map_err(|error| format!("cannot resolve Neovim Editor runtime directory: {error}"))?;
        // Neovim parses the last colon as host:port even in an absolute path.
        if path.as_os_str().as_encoded_bytes().contains(&b':') {
            return Err(format!(
                "Neovim Editor listener path {} contains ':' and would be interpreted as a TCP address; choose EON_RUNTIME_DIR with a resolved path without ':'",
                path.display()
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Sockets {
    listeners: Vec<Listener>,
}

struct Listener {
    endpoint: PathBuf,
    retry_at: Option<Instant>,
}

impl Sockets {
    pub(super) fn recover(
        runtime: &Path,
        sessions: &[RunningSession],
        log: &DiagnosticLog,
    ) -> Self {
        let directory = runtime.join(SOCKET_DIRECTORY);
        let mut sockets = Self::default();
        let recovered = (|| {
            match fs::symlink_metadata(&directory) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
            validate_private_directory(&directory)?;
            let entries = fs::read_dir(&directory).map_err(|error| error.to_string())?;
            for entry in entries {
                let name = entry.map_err(|error| error.to_string())?.file_name();
                let path = Path::new(&name);
                if matches!(
                    path.extension().and_then(|suffix| suffix.to_str()),
                    Some("sock" | "owner")
                ) && path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(sessions::session_number)
                    .is_some()
                {
                    sockets.track(runtime.join(path).with_extension("sock"));
                }
            }
            sockets.retain(sessions, Instant::now(), log);
            sockets.remove_empty_root(&directory);
            Ok(())
        })();
        if let Err(error) = recovered {
            log.record(&format!("cannot sweep Editor sockets: {error}"));
        }
        sockets
    }

    pub(super) fn prepare(
        &mut self,
        endpoint: &Path,
        sessions: &[RunningSession],
        kind: ListenerKind,
    ) -> Result<PathBuf, String> {
        validate_listener_path(endpoint, kind)?;
        let path = socket_path(endpoint);
        let directory = path.parent().expect("Editor socket has a parent");
        prepare_runtime(directory)?;
        let prepared = (|| {
            if terminal_may_run(endpoint, sessions) {
                return Err(format!(
                    "Editor socket {} belongs to a terminal that may still run",
                    path.display()
                ));
            }
            let canonical = canonical_socket_path(endpoint)
                .map_err(|error| format!("cannot resolve Editor socket directory: {error}"))?;
            if canonical.to_str().is_none() {
                return Err(format!(
                    "Editor socket path {} must be UTF-8; set EON_RUNTIME_DIR to a runtime root with a UTF-8 resolved path",
                    canonical.display()
                ));
            }
            if !platform::socket_path_fits(&canonical) {
                return Err(format!(
                    "Editor socket path {} is {} bytes, exceeding the {}-byte Unix socket limit; set EON_RUNTIME_DIR to a shorter runtime root whose resolved path fits; command was not started",
                    canonical.display(),
                    canonical.as_os_str().len(),
                    platform::SOCKET_PATH_BYTES,
                ));
            }
            remove_stale(&canonical)?;
            let marker = canonical.with_extension("owner");
            remove_marker(&marker)?;
            // Orbit can report Ready, and the supervisor can restart, before the editor binds.
            let mut record = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&marker)
                .map_err(|error| {
                    format!("cannot mark Editor listener {}: {error}", marker.display())
                })?;
            let bytes = serde_json::to_vec(&Marker { version: 1, kind })
                .expect("listener marker serializes");
            if let Err(error) = record.write_all(&bytes) {
                let _ = fs::remove_file(&marker);
                return Err(format!(
                    "cannot write Editor listener {}: {error}",
                    marker.display()
                ));
            }
            Ok(canonical)
        })();
        if prepared.is_ok() {
            self.track(endpoint.to_owned());
        } else {
            self.remove_empty_root(directory);
        }
        prepared
    }

    fn track(&mut self, endpoint: PathBuf) {
        if let Some(listener) = self
            .listeners
            .iter_mut()
            .find(|listener| listener.endpoint == endpoint)
        {
            listener.retry_at = None;
        } else {
            self.listeners.push(Listener {
                endpoint,
                retry_at: None,
            });
        }
    }

    pub(super) fn retain(
        &mut self,
        sessions: &[RunningSession],
        now: Instant,
        log: &DiagnosticLog,
    ) {
        let mut removed = None;
        self.listeners.retain_mut(|listener| {
            if listener.retry_at.is_some_and(|retry| now < retry)
                || terminal_may_run(&listener.endpoint, sessions)
            {
                return true;
            }
            let path = socket_path(&listener.endpoint);
            if let Err(error) =
                remove_stale(&path).and_then(|()| remove_marker(&path.with_extension("owner")))
            {
                if listener.retry_at.is_none() {
                    log.record(&error);
                }
                listener.retry_at = Some(now + Duration::from_secs(1));
                return true;
            }
            removed = Some(path);
            false
        });
        if let Some(path) = removed {
            self.remove_empty_root(path.parent().expect("Editor socket has a parent"));
        }
    }

    /// Tracks the listener of the terminal at `endpoint`.
    #[cfg(test)]
    pub(super) fn tracking(endpoint: PathBuf) -> Self {
        let mut sockets = Self::default();
        sockets.track(endpoint);
        sockets
    }

    /// Whether no removal waits for a retry and every listener's terminal
    /// is a live row; a listener kept only by a failed start's launch record
    /// is not settled.
    pub(super) fn is_settled(&self, sessions: &[RunningSession]) -> bool {
        self.listeners.iter().all(|listener| {
            listener.retry_at.is_none()
                && sessions
                    .iter()
                    .any(|session| session.endpoint == listener.endpoint && !session.is_finished())
        })
    }

    fn remove_empty_root(&self, directory: &Path) {
        if self.listeners.is_empty() {
            let _ = fs::remove_dir(directory);
        }
    }
}

fn terminal_may_run(endpoint: &Path, running: &[RunningSession]) -> bool {
    running
        .iter()
        .any(|session| session.endpoint == endpoint && !session.is_finished())
        || sessions::launch_record_remains(endpoint)
}

/// The listener of the terminal at `endpoint` as Eon binds it: inside the
/// resolved generation directory, so on macOS `/tmp` reads `/private/tmp`.
/// It names the same file as [`socket_path`] because the listener directory
/// must be an owned real directory (EON-C10).
fn canonical_socket_path(endpoint: &Path) -> io::Result<PathBuf> {
    let runtime = fs::canonicalize(endpoint.parent().expect("terminal endpoint has a parent"))?;
    Ok(socket_path(
        &runtime.join(
            endpoint
                .file_name()
                .expect("terminal endpoint has a filename"),
        ),
    ))
}

fn socket_path(endpoint: &Path) -> PathBuf {
    endpoint.with_file_name(SOCKET_DIRECTORY).join(
        endpoint
            .file_name()
            .expect("terminal endpoint has a filename"),
    )
}

fn remove_marker(marker: &Path) -> Result<(), String> {
    let failure = |error| {
        format!(
            "cannot remove Editor listener marker {}: {error}",
            marker.display()
        )
    };
    match fs::symlink_metadata(marker) {
        Ok(metadata) if metadata.is_file() && metadata.uid() == effective_uid() => {}
        Ok(_) => {
            return Err(format!(
                "Editor listener marker {} is not an owned file",
                marker.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(error)),
    }
    validate_private_directory(marker.parent().expect("Editor marker has a parent"))?;
    match fs::remove_file(marker) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failure(error)),
        _ => Ok(()),
    }
}

fn remove_stale_failure(path: &Path, detail: &str) -> String {
    format!("cannot remove Editor socket {}: {detail}", path.display())
}

fn remove_stale(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(remove_stale_failure(path, &error.to_string())),
    };
    validate_private_directory(path.parent().expect("Editor socket has a parent"))?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(remove_stale_failure(path, "expected an owned socket"));
    }
    probe_and_remove_stale(path, control::socket_identity_from(&metadata))
}

fn probe_and_remove_stale(path: &Path, identity: control::SocketIdentity) -> Result<(), String> {
    match control::probe_stream_socket(path) {
        Ok(StreamSocketProbe::Dead) => {}
        Ok(StreamSocketProbe::Missing) => return Ok(()),
        Ok(StreamSocketProbe::LiveStream) => {
            return Err(remove_stale_failure(
                path,
                "the socket may still be listening",
            ));
        }
        Ok(StreamSocketProbe::LiveDatagram) => {
            return Err(remove_stale_failure(
                path,
                "the path is a Unix datagram socket",
            ));
        }
        Err(error) => return Err(remove_stale_failure(path, &error.to_string())),
    }
    remove_socket_if_identity(path, identity)
}

fn remove_socket_if_identity(path: &Path, identity: control::SocketIdentity) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(current) if control::socket_identity_from(&current) == identity => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => {
            return Err(remove_stale_failure(
                path,
                "the socket changed before cleanup",
            ));
        }
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(remove_stale_failure(path, &error.to_string()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::{fs::symlink, net::UnixListener};

    fn runtime_at_management_socket_limit(expansion: usize) -> (PathBuf, PathBuf, PathBuf) {
        let root = super::super::supervisor::temporary_directory()
            .canonicalize()
            .unwrap();
        let management = "terminal-9999.sock.management";
        let alias = (1..=255)
            .map(|length| root.join("a".repeat(length)))
            .find(|alias| {
                platform::socket_path_fits(&alias.join(management))
                    && !platform::socket_path_fits(&alias.join(format!("{management}x")))
            })
            .unwrap_or_else(|| {
                panic!(
                    "test parent {} is too long to calibrate a private runtime at the {}-byte Unix socket limit",
                    root.display(),
                    platform::SOCKET_PATH_BYTES,
                )
            });
        let physical = root.join("r".repeat(alias.file_name().unwrap().len() + expansion));
        prepare_runtime(&physical).unwrap();
        symlink(&physical, &alias).unwrap();
        (root, alias, physical)
    }

    #[test]
    fn editor_listener_fits_the_management_socket_budget_after_canonical_expansion() {
        for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
            let (root, alias, physical) = runtime_at_management_socket_limit(8);
            let endpoint = alias.join("terminal-9999.sock");
            let management = alias.join("terminal-9999.sock.management");
            assert!(platform::socket_path_fits(&management));
            assert!(!platform::socket_path_fits(
                &alias.join("terminal-9999.sock.managementx")
            ));
            let old_editor = physical.join("editor/terminal-9999.sock");
            assert_eq!(
                old_editor.as_os_str().len(),
                management.as_os_str().len() + 4
            );
            assert!(!platform::socket_path_fits(&old_editor));
            prepare_runtime(old_editor.parent().unwrap()).unwrap();
            assert!(UnixListener::bind(&old_editor).is_err());
            fs::remove_dir(old_editor.parent().unwrap()).unwrap();
            for address in [endpoint.clone(), management.clone(), alias.join("eon.sock")] {
                let listener = UnixListener::bind(&address).unwrap();
                drop(listener);
                fs::remove_file(address).unwrap();
            }
            let mut sockets = Sockets::default();
            let selected = sockets.prepare(&endpoint, &[], kind).unwrap();
            assert_eq!(selected.as_os_str().len(), management.as_os_str().len());
            let listener = UnixListener::bind(&selected).unwrap();
            let marker = selected.with_extension("owner");
            assert_eq!(listener_kind(&endpoint).unwrap(), Some(kind));
            assert_eq!(
                fs::metadata(selected.parent().unwrap()).unwrap().mode() & 0o7777,
                0o700
            );
            assert_eq!(fs::metadata(&marker).unwrap().mode() & 0o7777, 0o600);
            drop(listener);
            let log = DiagnosticLog::open_in(Ok(root.join("logs")), "g1-test", "eon");
            sockets.retain(&[], Instant::now(), &log);
            assert!(!selected.exists());
            assert!(!marker.exists());
            assert!(!selected.parent().unwrap().exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn editor_listener_refuses_one_byte_over_the_canonical_limit_without_state() {
        for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
            let (root, alias, physical) = runtime_at_management_socket_limit(9);
            let endpoint = alias.join("terminal-9999.sock");
            let selected = socket_path(&physical.join("terminal-9999.sock"));
            assert!(platform::socket_path_fits(
                &alias.join("terminal-9999.sock.management")
            ));
            assert_eq!(selected.as_os_str().len(), platform::SOCKET_PATH_BYTES + 1);
            let mut sockets = Sockets::default();
            let error = sockets.prepare(&endpoint, &[], kind).unwrap_err();
            assert!(
                error.contains(&format!("is {} bytes", selected.as_os_str().len())),
                "{error}"
            );
            assert!(
                error.contains(&format!(
                    "{}-byte Unix socket limit",
                    platform::SOCKET_PATH_BYTES
                )),
                "{error}"
            );
            assert!(
                error.contains("set EON_RUNTIME_DIR to a shorter runtime root"),
                "{error}"
            );
            assert!(
                !selected.parent().unwrap().exists(),
                "refusal left listener state"
            );
            assert!(sockets.listeners.is_empty());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn editor_listener_refuses_non_utf8_canonical_paths_without_state() {
        use std::os::unix::ffi::OsStrExt;
        for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
            let root = super::super::supervisor::temporary_directory();
            // APFS rejects non-UTF-8 objects; the uncreated endpoint still tests refusal.
            let endpoint = root.join(std::ffi::OsStr::from_bytes(b"terminal-\xff.sock"));
            let mut sockets = Sockets::default();
            let error = sockets.prepare(&endpoint, &[], kind).unwrap_err();
            assert!(
                error.contains("must be UTF-8") && error.contains("EON_RUNTIME_DIR"),
                "{error}"
            );
            assert!(!socket_path(&endpoint).parent().unwrap().exists());
            assert!(sockets.listeners.is_empty());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn neovim_listener_refuses_colons_in_the_canonical_runtime_path() {
        let root = super::super::supervisor::temporary_directory()
            .canonicalize()
            .unwrap();
        let runtime = root.join("with:colon");
        fs::create_dir(&runtime).unwrap();
        let alias = root.join("alias");
        symlink(&runtime, &alias).unwrap();
        let endpoint = alias.join("terminal-1.sock");
        let mut sockets = Sockets::default();
        let error = sockets
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap_err();
        assert!(
            error.contains("TCP") && error.contains("EON_RUNTIME_DIR"),
            "{error}"
        );
        assert!(
            !runtime.join(SOCKET_DIRECTORY).exists(),
            "refusal created listener state"
        );
        // Helix's Unix socket parser accepts colons, so its existing route stays usable.
        let socket = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));
        assert_eq!(socket.parent().unwrap(), runtime.join(SOCKET_DIRECTORY));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_reuse_rejects_non_utf8_before_connecting() {
        use std::os::unix::ffi::OsStrExt;
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        Sockets::default()
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap();
        let error = reuse(
            &endpoint,
            Path::new(std::ffi::OsStr::from_bytes(b"/bad\xff")),
            "edit-1",
            ListenerKind::Neovim,
            &eon_workspace_protocol::v51::PopupKey::Unbound,
        )
        .unwrap_err();
        assert_eq!(error.code, "editor-rejected");
        let mut command = managed_environment::PreparedCommand {
            argv: vec!["nvim".into()],
            listener: Some((ListenerKind::Neovim, 1)),
        };
        assert_eq!(
            initial_file(
                &mut command,
                Path::new(std::ffi::OsStr::from_bytes(b"/bad\xff"))
            )
            .unwrap_err()
            .code,
            "editor-rejected"
        );
        assert_eq!(command.argv, [OsString::from("nvim")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_exchange_encodes_data_and_handles_fragmented_coalesced_notifications() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = Sockets::default()
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap();
        let listener = UnixListener::bind(socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            // Independent wire bytes, not a round trip through the request encoder.
            let mut expected = b"\x94\x00\x01\xadnvim_exec_lua\x92\xda".to_vec();
            expected.extend_from_slice(&(NEOVIM_OPEN.len() as u16).to_be_bytes());
            expected.extend_from_slice(NEOVIM_OPEN.as_bytes());
            expected.extend_from_slice(b"\x93\xa8/a\n\"b.rs\x02\x03");
            let mut request = vec![0; expected.len()];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request, expected);
            // A nested map, binary and variable extension notification precedes success.
            let reply =
                b"\x93\x02\xa5event\x92\x81\xc4\x01x\xc7\x02\x01ab\xa1\xff\x94\x01\x01\xc0\xc3";
            for part in reply.chunks(3) {
                stream.write_all(part).unwrap();
                std::thread::sleep(Duration::from_millis(2));
            }
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err(), "a request was retried");
        });
        reuse(
            &endpoint,
            Path::new("/a\n\"b.rs:2:3"),
            "edit-1",
            ListenerKind::Neovim,
            &eon_workspace_protocol::v51::PopupKey::Unbound,
        )
        .unwrap();
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_exchange_bounds_notifications_bytes_and_uncertain_delivery() {
        let notification = b"\x93\x02\xa1x\x90";
        let success = b"\x94\x01\x01\xc0\xc3";
        let mut sixteen = notification.repeat(16);
        sixteen.extend_from_slice(success);
        let mut seventeen = notification.repeat(17);
        seventeen.extend_from_slice(success);
        let mut oversized = b"\x93\x02\xa1x\x91\xc6\0\x01\0\0".to_vec();
        oversized.extend(std::iter::repeat_n(0, FRAME_LIMIT));
        for (reply, passes) in [
            (sixteen, true),
            (seventeen, false),
            (oversized, false),
            (vec![], false),
            (vec![0x94, 1], false),
            (vec![0xc1], false),
        ] {
            let root = super::super::supervisor::temporary_directory();
            let endpoint = root.join("terminal-1.sock");
            let socket = Sockets::default()
                .prepare(&endpoint, &[], ListenerKind::Neovim)
                .unwrap();
            let listener = UnixListener::bind(socket).unwrap();
            let count = neovim_request(Path::new("/a")).unwrap().len();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream.read_exact(&mut vec![0; count]).unwrap();
                let _ = stream.write_all(&reply);
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "uncertain request retried");
            });
            let result = reuse(
                &endpoint,
                Path::new("/a"),
                "r1",
                ListenerKind::Neovim,
                &eon_workspace_protocol::v51::PopupKey::Unbound,
            );
            if passes {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, "editor-unavailable");
                assert!(error.detail.contains("outcome is unknown"), "{error:?}");
            }
            server.join().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn neovim_timeout_leaves_late_execution_unknown_without_retry() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = Sockets::default()
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap();
        let count = neovim_request(Path::new("/a")).unwrap().len();
        let (accepted, waiting) = std::sync::mpsc::channel();
        let listener = UnixListener::bind(socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.read_exact(&mut vec![0; count]).unwrap();
            std::thread::sleep(Duration::from_millis(1200));
            accepted.send("executed late").unwrap();
            let _ = stream.write_all(b"\x94\x01\x01\xc0\xc3");
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        });
        let error = reuse(
            &endpoint,
            Path::new("/a"),
            "r1",
            ListenerKind::Neovim,
            &eon_workspace_protocol::v51::PopupKey::Unbound,
        )
        .unwrap_err();
        assert_eq!(error.code, "editor-unresponsive");
        assert!(
            error.detail.contains("outcome is unknown")
                && error.detail.contains("may be waiting for input")
                && error.detail.contains("[popups.editor] keybinding"),
            "{error:?}"
        );
        assert_eq!(
            waiting.recv_timeout(Duration::from_secs(3)).unwrap(),
            "executed late"
        );
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_locations_keep_literals_and_bound_native_numbers() {
        let root = super::super::supervisor::temporary_directory();
        let literal = root.join("literal:2:3");
        fs::write(&literal, "").unwrap();
        assert_eq!(
            neovim_location(&literal).unwrap().path,
            literal.to_str().unwrap()
        );
        assert_eq!(neovim_location(&literal).unwrap().line, None);
        let link = root.join("broken:4:5");
        symlink(root.join("missing"), &link).unwrap();
        assert_eq!(neovim_location(&link).unwrap().line, None);
        for (input, path, line, column) in [
            ("/a:0:0", "/a", Some(1), Some(1)),
            ("/a:23", "/a", Some(23), None),
            (
                "/a:2147483647:2147483647",
                "/a",
                Some(i32::MAX),
                Some(i32::MAX),
            ),
            ("/a:2147483648:2", "/a:2147483648:2", None, None),
            ("/a:2:2147483648", "/a:2:2147483648", None, None),
            ("/a:2:no", "/a:2:no", None, None),
            ("/a:+2", "/a:+2", None, None),
            ("/a:1:", "/a:1:", None, None),
            ("/-é \"\n|:2:3", "/-é \"\n|", Some(2), Some(3)),
        ] {
            assert_eq!(
                neovim_location(Path::new(input)).unwrap(),
                Location { path, line, column },
                "{input}"
            );
        }
        let mut command = managed_environment::PreparedCommand {
            argv: vec!["nvim".into(), "+set nu".into()],
            listener: Some((ListenerKind::Neovim, 2)),
        };
        assert!(initial_file(&mut command, Path::new("/a:0:3")).unwrap());
        assert_eq!(
            command.argv,
            ["nvim", "+set nu", "+call cursor(1,3)", "--", "/a"].map(OsString::from)
        );
        let mut directory = managed_environment::PreparedCommand {
            argv: vec!["nvim".into()],
            listener: Some((ListenerKind::Neovim, 1)),
        };
        assert!(
            !initial_file(
                &mut directory,
                &PathBuf::from(format!("{}:2:3", root.display()))
            )
            .unwrap()
        );
        assert_eq!(directory.argv, [OsString::from("nvim")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_values_bound_the_complete_grammar_without_peer_sized_allocations() {
        for bytes in [
            &b"\xc0\xc2\xc3\x00\xff\xa1\xff"[..],
            &b"\xcc\x80\xcd\x00\x01\xce\x00\x00\x00\x01\xcf\0\0\0\0\0\0\0\x01"[..],
            &b"\xd0\x80\xd1\0\0\xd2\0\0\0\0\xd3\0\0\0\0\0\0\0\0"[..],
            &b"\xca\0\0\0\0\xcb\0\0\0\0\0\0\0\0"[..],
            &b"\xa1a\xd9\x01a\xda\0\x01a\xdb\0\0\0\x01a"[..],
            &b"\xc4\x01x\xc5\0\x01x\xc6\0\0\0\x01x"[..],
            &b"\xd4\0x\xd5\0xx\xd6\0xxxx\xd7\0xxxxxxxx\xd8\0xxxxxxxxxxxxxxxx"[..],
            &b"\xc7\x01\0x\xc8\0\x01\0x\xc9\0\0\0\x01\0x"[..],
            &b"\x90\xdc\0\0\xdd\0\0\0\0\x80\xde\0\0\xdf\0\0\0\0"[..],
        ] {
            let mut rest = bytes;
            while !rest.is_empty() {
                skip_value(&mut rest, 0).unwrap();
            }
        }
        for bytes in [
            vec![0xc1],
            vec![0xc6, 255, 255, 255, 255],
            vec![0xdf, 255, 255, 255, 255],
            vec![0x91; 33],
        ] {
            assert_eq!(
                skip_value(&mut bytes.as_slice(), 0),
                Err(Decode::Invalid),
                "{bytes:?}"
            );
        }
        let mut deepest = vec![0x91; 32];
        deepest.push(0xc0);
        assert!(skip_value(&mut deepest.as_slice(), 0).is_ok());
        for bytes in [
            b"\xc7\x01\0".as_slice(),
            b"\x91".as_slice(),
            b"\xda\0".as_slice(),
        ] {
            assert_eq!(skip_value(&mut &*bytes, 0), Err(Decode::Incomplete));
        }
    }

    #[test]
    fn neovim_reply_refusals_and_bad_envelopes_are_distinct() {
        let reply = b"\x94\x01\x01\x92\0\xa5bad\n!\xc0";
        let (_, NeovimReply::Answer(Err(error))) = neovim_reply(reply).unwrap() else {
            panic!("refusal");
        };
        assert_eq!(error.code, "editor-rejected");
        assert_eq!(error.detail, "bad");
        let (_, NeovimReply::Answer(Err(error))) =
            neovim_reply(b"\x94\x01\x01\x92\x00\xa1\xff\xc0").unwrap()
        else {
            panic!("binary error is a refusal");
        };
        assert_eq!(error.code, "editor-rejected");
        assert_eq!(error.detail, "Neovim refused the file");
        for bytes in [
            b"\x94\x00\x01\xc0\xc3".as_slice(),
            b"\x94\x01\x02\xc0\xc3".as_slice(),
            b"\x94\x01\x01\xc0\xc2".as_slice(),
            b"\x93\x02\xa1x\xc0".as_slice(),
            b"\x94\x01\x01\x91\xc0\xc0".as_slice(),
            b"\xc1".as_slice(),
        ] {
            assert!(
                matches!(neovim_reply(bytes), Err(Decode::Invalid)),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn listener_markers_are_versioned_private_and_fail_closed() {
        use std::os::unix::fs::PermissionsExt;
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        assert_eq!(listener_kind(&endpoint).unwrap(), None);
        let mut sockets = Sockets::default();
        for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
            let path = sockets.prepare(&endpoint, &[], kind).unwrap();
            assert_eq!(listener_kind(&endpoint).unwrap(), Some(kind));
            assert_eq!(
                fs::metadata(path.with_extension("owner")).unwrap().mode() & 0o7777,
                0o600
            );
        }
        let marker = socket_path(&endpoint).with_extension("owner");
        for bytes in [
            "",
            "{}",
            "not json",
            r#"{"version":2,"kind":"helix"}"#,
            r#"{"version":1,"kind":"other"}"#,
            r#"{"version":1,"kind":"helix","extra":true}"#,
        ] {
            fs::write(&marker, bytes).unwrap();
            let error = listener_kind(&endpoint).unwrap_err();
            assert_eq!(error.code, "editor-unavailable");
            assert!(
                error
                    .detail
                    .contains("save your work, then restart the Editor popup")
            );
        }
        fs::write(&marker, vec![b' '; 257]).unwrap();
        assert!(
            listener_kind(&endpoint)
                .unwrap_err()
                .detail
                .contains("save your work, then restart the Editor popup")
        );
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            !listener_kind(&endpoint)
                .unwrap_err()
                .detail
                .contains("restart")
        );
        fs::remove_file(&marker).unwrap();
        symlink(root.join("absent"), &marker).unwrap();
        assert!(listener_kind(&endpoint).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn neovim_prefix_checks_flags_option_values_and_the_command_quota() {
        let check =
            |args: &[&str]| neovim_position(&args.iter().map(OsString::from).collect::<Vec<_>>());
        for flag in [
            "--listen",
            "--listen=x",
            "--server",
            "--remote",
            "--remote-tab-silent",
            "--",
            "--embed",
            "--headless",
            "--HEADLESS",
            "-l",
            "-e",
            "-E",
            "-es",
            "-Es",
            "-ne",
            "README.md",
            "-",
            "-t",
            "-tSYMBOL",
            "-ntSYMBOL",
            "-q",
            "-qerrors.txt",
        ] {
            assert!(check(&["nvim", flag]).is_err(), "{flag}");
        }
        for args in [
            vec!["nvim", "-u", "--listen"],
            vec!["nvim", "-u", "README.md", "--startuptime", "trace.log"],
            vec!["nvim", "-c", "--embed"],
            vec!["nvim", "--cmd", "--server"],
            vec!["nvim", "-n", "-u", "NONE", "+set nu"],
            vec!["nvim", "-Vfile", "-p2"],
            vec!["nvim", "-S"],
        ] {
            assert_eq!(check(&args).unwrap(), args.len(), "{args:?}");
        }
        assert!(check(&["nvim", "-u"]).is_err());
        assert!(check(&["nvim", "-c"]).is_err());
        let mut args = vec!["nvim".into()];
        args.extend(std::iter::repeat_n(OsString::from("+set nu"), 9));
        assert!(neovim_position(&args).is_ok());
        args.push("+set nu".into());
        assert!(neovim_position(&args).is_err());
    }

    #[test]
    fn editor_replies_distinguish_rejection_timeout_and_uncertain_delivery() {
        for (class, code) in [
            ("timeout", "editor-unresponsive"),
            ("rejected", "editor-rejected"),
            ("invalid", "editor-rejected"),
            ("unsupported", "editor-rejected"),
            ("unavailable", "editor-unavailable"),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "version":1,"id":"r1","status":"error",
                "error":{"class":class,"message":"bad\u{1b}[2J\npath","path_index":0}
            }))
            .unwrap();
            let error = parse_reply(&bytes, "r1").unwrap_err();
            assert_eq!(error.code, code);
            assert!(!error.detail.chars().any(char::is_control));
        }
        for reply in [
            r#"{"version":2,"id":"r1","status":"ok"}"#,
            r#"{"version":1,"id":"other","status":"ok"}"#,
            r#"{"version":1,"id":"r1","status":"unknown"}"#,
            r#"{"version":1,"id":"r1","status":"error"}"#,
            r#"{"version":1,"id":"r1","status":"ok","extra":true}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"unknown","message":"bad"}}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"rejected","message":""}}"#,
            r#"{"version":1,"id":"r1","status":"error","error":{"class":"rejected","message":"\u001b\n"}}"#,
            "invalid JSON",
        ] {
            assert_eq!(
                parse_reply(reply.as_bytes(), "r1").unwrap_err().code,
                "editor-unavailable",
                "{reply}"
            );
        }
    }

    #[test]
    fn editor_exchange_bounds_frames_and_lost_replies_without_retry() {
        for (reply, code) in [
            (Vec::new(), "editor-unavailable"),
            (b"{\"version\":1".to_vec(), "editor-unavailable"),
            (vec![b'x'; FRAME_LIMIT + 1], "editor-unavailable"),
        ] {
            let root = super::super::supervisor::temporary_directory();
            let endpoint = root.join("terminal-1.sock");
            let socket = socket_path(&endpoint);
            fs::create_dir(socket.parent().unwrap()).unwrap();
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let _ = stream.write_all(&reply);
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "a failed exchange was retried");
            });
            assert_eq!(
                open(&endpoint, Path::new("/a"), "r1").unwrap_err().code,
                code
            );
            server.join().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
        use std::os::unix::ffi::OsStrExt;
        let invalid = Path::new(std::ffi::OsStr::from_bytes(b"/bad\xff"));
        assert_eq!(
            open(Path::new("/no/socket"), invalid, "r1")
                .unwrap_err()
                .code,
            "editor-rejected"
        );
        let oversized = "/".to_owned() + &"a".repeat(FRAME_LIMIT);
        assert_eq!(
            open(Path::new("/no/socket"), Path::new(&oversized), "r1")
                .unwrap_err()
                .code,
            "editor-rejected"
        );
    }

    #[test]
    fn editor_answer_deadline_does_not_start_another_exchange() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        fs::create_dir(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let (release, held) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            held.recv_timeout(Duration::from_secs(5)).unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        });
        let started = Instant::now();
        assert_eq!(
            open(&endpoint, Path::new("/a"), "r1").unwrap_err().code,
            "editor-unresponsive"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        release.send(()).unwrap();
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn wait_for_dead_socket(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match control::probe_stream_socket(path) {
                Ok(StreamSocketProbe::Dead) => return,
                other if Instant::now() >= deadline => {
                    panic!(
                        "expected {} to become Dead after dropping listener, got {other:?}",
                        path.display()
                    );
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    fn read_diagnostic_log(directory: &Path) -> String {
        let entry = fs::read_dir(directory).unwrap().next().unwrap().unwrap();
        fs::read_to_string(entry.path()).unwrap()
    }

    fn spawn_helix_ok_server(
        listener: UnixListener,
        ids: &'static [&'static str],
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for &id in ids {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["id"], id);
                let mut reply = serde_json::to_vec(&serde_json::json!({
                    "version": 1,
                    "id": id,
                    "status": "ok",
                }))
                .unwrap();
                reply.push(b'\n');
                stream.write_all(&reply).unwrap();
            }
        })
    }

    fn answer_helix_open(mut stream: std::os::unix::net::UnixStream, id: &str) {
        let mut request = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "id": id,
            "action": "open",
            "paths": ["/a"],
            "timeout_ms": 750,
        }))
        .unwrap();
        request.push(b'\n');
        answer(&mut stream, &request, ListenerKind::Helix, id).unwrap();
    }

    #[test]
    fn editor_connect_handles_initial_binding_refusal_and_a_full_queue() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        prepare_runtime(socket.parent().unwrap()).unwrap();
        Sockets::default()
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));

        // Phase 1: retry NotFound until the listener binds synchronously in the
        // retry hook, then accept and reply outside the 100 ms connect window
        // for both the retried stream and a full `open` call.
        let mut listener = None;
        let mut saw_not_found = false;
        let stream = connect_listener(&endpoint, &socket, ListenerKind::Helix, |error| {
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
            saw_not_found = true;
            if listener.is_none() {
                listener = Some(UnixListener::bind(&socket).unwrap());
            }
        })
        .unwrap();
        assert!(
            saw_not_found,
            "unbound socket did not produce an initial NotFound"
        );
        let server = spawn_helix_ok_server(listener.unwrap(), &["r1-retry", "r1"]);
        answer_helix_open(stream, "r1-retry");
        open(&endpoint, Path::new("/a"), "r1").unwrap();
        server.join().unwrap();

        // Phase 2: a closed listener fails immediately without spinning the 100 ms deadline.
        wait_for_dead_socket(&socket);
        let refused = open(&endpoint, Path::new("/a"), "r2").unwrap_err();
        assert_eq!(refused.code, "editor-unavailable");
        assert!(
            refused.detail.contains("Editor connection failed:")
                && refused.detail.contains("no request sent"),
            "{refused:?}"
        );

        // Phase 3: a socket bound before listen(2) refuses stream connect with ECONNREFUSED
        // while the datagram probe reports LiveStream; retry only for the matching marker kind.
        fs::remove_file(&socket).unwrap();
        let bound_listener =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        bound_listener
            .bind(&socket2::SockAddr::unix(&socket).unwrap())
            .unwrap();
        let mismatched = connect_listener(&endpoint, &socket, ListenerKind::Neovim, |_| {
            panic!("must not retry ConnectionRefused for a mismatched listener kind");
        })
        .unwrap_err();
        assert_eq!(mismatched.code, "editor-unavailable");
        assert!(
            mismatched.detail.contains("Editor connection failed:")
                && mismatched.detail.contains("no request sent"),
            "{mismatched:?}"
        );
        let mut bound_listener = Some(bound_listener);
        let mut listener = None;
        let mut saw_refused = false;
        let stream = connect_listener(&endpoint, &socket, ListenerKind::Helix, |error| {
            assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
            saw_refused = true;
            if let Some(bound) = bound_listener.take() {
                bound.listen(1).unwrap();
                listener = Some(UnixListener::from(std::os::fd::OwnedFd::from(bound)));
            }
        })
        .unwrap();
        assert!(
            saw_refused,
            "bound socket did not produce an initial ConnectionRefused"
        );
        let server = spawn_helix_ok_server(listener.unwrap(), &["r3"]);
        answer_helix_open(stream, "r3");
        server.join().unwrap();

        // Phase 4: a saturated listener fails cleanly without sending a request.
        fs::remove_file(&socket).unwrap();
        let (listener, queued) = crate::fixtures::busy_listener(&socket);
        let full = open(&endpoint, Path::new("/a"), "r4").unwrap_err();
        assert_eq!(full.code, "editor-unavailable", "{full:?}");
        assert!(full.detail.contains("no request sent"), "{full:?}");
        drop(queued);
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_a_file_checks_the_reply_and_escapes_the_path() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-1.sock");
        let socket = socket_path(&endpoint);
        fs::create_dir(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["paths"], serde_json::json!(["/a\n\"b.rs:2:3"]));
            assert_eq!(request["timeout_ms"], 750);
            stream
                .write_all(b"{\"version\":1,\"id\":\"edit-1\",\"status\":\"ok\"}\n")
                .unwrap();
        });
        open(&endpoint, Path::new("/a\n\"b.rs:2:3"), "edit-1").unwrap();
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_the_packaged_helix_gets_an_eon_listener_option() {
        let root = super::super::supervisor::temporary_directory();
        let mut programs = crate::fixtures::inputs().managed;
        let packaged = root.join("packaged-hx");
        let user_hx = root.join("hx");
        let session_bin = root.join("bin");
        fs::create_dir(&session_bin).unwrap();
        let launcher = session_bin.join("hx");
        let alias = root.join("eon-hx");
        fs::write(&packaged, "").unwrap();
        fs::write(&user_hx, "").unwrap();
        fs::write(&launcher, "").unwrap();
        symlink(&launcher, &alias).unwrap();
        programs.helix = Some(packaged.clone());
        programs.shell_bin = Some(session_bin);
        let argv = |path: &Path, arguments: &[&str]| {
            std::iter::once(path.as_os_str().to_owned())
                .chain(arguments.iter().map(OsString::from))
                .collect::<Vec<_>>()
        };
        for path in [&packaged, &launcher, &alias] {
            let command = argv(path, &["--log", "--", "--", "--listen"]);
            assert_eq!(
                listen_position(&command, &programs, &root).unwrap(),
                Some(3)
            );
            let command = argv(path, &["--log", "--listen"]);
            assert_eq!(
                listen_position(&command, &programs, &root).unwrap(),
                Some(3)
            );
            let command = argv(path, &["--listen", "/other.sock"]);
            assert!(
                listen_position(&command, &programs, &root)
                    .unwrap_err()
                    .contains("[editor] command")
            );
        }
        let command = argv(&user_hx, &["--listen", "/other.sock"]);
        assert_eq!(listen_position(&command, &programs, &root).unwrap(), None);
        programs.helix = None;
        assert_eq!(
            listen_position(&argv(&launcher, &[]), &programs, &root).unwrap(),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_preflight_leaves_a_live_socket_and_reuses_it_after_exit() {
        let root = super::super::supervisor::temporary_directory();
        let endpoint = root.join("terminal-2.sock");
        let path = socket_path(&endpoint);
        prepare_runtime(path.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        let mut sockets = Sockets::default();
        assert!(
            sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap_err()
                .contains("may still be listening")
        );
        assert!(path.exists());
        drop(listener);
        let selected = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        assert_eq!(
            selected,
            root.canonicalize()
                .unwrap()
                .join(SOCKET_DIRECTORY)
                .join("terminal-2.sock")
        );
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_cleanup_retries_quietly_after_a_listener_outlives_its_terminal() {
        let root = super::super::supervisor::temporary_directory();
        let logs = root.join("logs");
        let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
        let endpoint = root.join("terminal-2.sock");
        let mut sockets = Sockets::default();
        let path = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        let marker = path.with_extension("owner");
        let now = Instant::now();
        sockets.retain(&[], now, &log);
        sockets.retain(&[], now + Duration::from_secs(1), &log);
        assert!(marker.exists());
        let text = read_diagnostic_log(&logs);
        assert_eq!(text.matches("the socket may still be listening").count(), 1);

        drop(listener);
        sockets.retain(&[], now + Duration::from_secs(1), &log);
        assert!(path.exists(), "cleanup waits until the retry is due");
        sockets.retain(&[], now + Duration::from_secs(2), &log);
        assert!(!path.parent().unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_cleanup_preserves_busy_and_bound_listeners_and_reclaims_closed_ones() {
        let root = super::super::supervisor::temporary_directory();
        let logs = root.join("logs");
        let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
        let endpoint = root.join("terminal-2.sock");
        let mut sockets = Sockets::default();
        let path = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        let marker = path.with_extension("owner");

        // Saturated listener: full accept queue must neither block nor unlink.
        let (listener, queued) = crate::fixtures::busy_listener(&path);
        let now = Instant::now();
        let probe_endpoint = endpoint.clone();
        let probe_log = std::sync::Arc::clone(&log);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sockets.retain(&[], now, &probe_log);
            let prepare_result =
                Sockets::default().prepare(&probe_endpoint, &[], ListenerKind::Helix);
            sender.send((sockets, prepare_result)).unwrap();
        });
        let (mut sockets, prepare_result) = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("Editor socket cleanup blocked on a busy listener");
        worker.join().unwrap();

        let prepare_error = prepare_result.unwrap_err();
        assert!(
            prepare_error.contains("the socket may still be listening"),
            "{prepare_error}"
        );
        assert!(path.exists(), "busy Editor socket was unlinked");
        assert!(marker.exists(), "busy Editor owner marker was unlinked");
        assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));
        assert!(!sockets.is_settled(&[]));
        let text = read_diagnostic_log(&logs);
        assert_eq!(
            text.matches("the socket may still be listening").count(),
            1,
            "{text}"
        );

        drop(queued);
        drop(listener);
        wait_for_dead_socket(&path);
        sockets.retain(&[], now + Duration::from_secs(2), &log);
        assert!(sockets.is_settled(&[]));
        assert!(!path.exists());
        assert!(!marker.exists());
        assert!(!path.parent().unwrap().exists());

        // Bound-before-listen stream socket: stream connect gets ECONNREFUSED on both
        // Linux and macOS, while the datagram probe still identifies it as live.
        let mut sockets = Sockets::default();
        let path = sockets
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap();
        let marker = path.with_extension("owner");
        let bound =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        bound
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        assert_eq!(
            sessions::unix_connect_with_timeout(&path, Duration::ZERO)
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionRefused
        );

        let bound_logs = root.join("bound-logs");
        let bound_log = DiagnosticLog::open_in(Ok(bound_logs.clone()), "g1-test", "eon");
        sockets.retain(&[], Instant::now(), &bound_log);
        let prepare_error = Sockets::default()
            .prepare(&endpoint, &[], ListenerKind::Neovim)
            .unwrap_err();
        assert!(
            prepare_error.contains("the socket may still be listening"),
            "{prepare_error}"
        );
        assert!(path.exists(), "bound Editor socket was unlinked");
        assert!(marker.exists(), "bound Editor owner marker was unlinked");
        assert_eq!(
            listener_kind(&endpoint).unwrap(),
            Some(ListenerKind::Neovim)
        );
        assert!(!sockets.is_settled(&[]));
        let bound_text = read_diagnostic_log(&bound_logs);
        assert_eq!(
            bound_text
                .matches("the socket may still be listening")
                .count(),
            1,
            "{bound_text}"
        );

        drop(bound);
        wait_for_dead_socket(&path);
        let reclaimed = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        assert_eq!(reclaimed, path);
        assert!(!path.exists());
        assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_socket_cleanup_enforces_directory_type_and_identity_guards() {
        use std::os::unix::fs::PermissionsExt;

        // Live UnixDatagram socket: preserved by both prepare and retain.
        {
            let root = super::super::supervisor::temporary_directory();
            let logs = root.join("logs");
            let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
            let endpoint = root.join("terminal-2.sock");
            let mut sockets = Sockets::default();
            let path = sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap();
            let marker = path.with_extension("owner");
            let datagram = std::os::unix::net::UnixDatagram::bind(&path).unwrap();
            let observed = control::socket_identity(&path).unwrap().unwrap();

            let error = Sockets::default()
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap_err();
            assert!(error.contains("Unix datagram socket"), "{error}");

            sockets.retain(&[], Instant::now(), &log);
            let text = read_diagnostic_log(&logs);
            assert!(text.contains("Unix datagram socket"), "{text}");
            assert!(!sockets.is_settled(&[]));
            assert_eq!(control::socket_identity(&path).unwrap(), Some(observed));
            assert!(marker.exists());
            assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));

            drop(datagram);
            fs::remove_dir_all(root).unwrap();
        }

        // Regular file at socket path: refused by both prepare and retain.
        {
            let root = super::super::supervisor::temporary_directory();
            let logs = root.join("logs");
            let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
            let endpoint = root.join("terminal-2.sock");
            let mut sockets = Sockets::default();
            let path = sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap();
            let marker = path.with_extension("owner");
            fs::write(&path, b"not a socket").unwrap();

            let error = Sockets::default()
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap_err();
            assert!(error.contains("expected an owned socket"), "{error}");

            sockets.retain(&[], Instant::now(), &log);
            let text = read_diagnostic_log(&logs);
            assert!(text.contains("expected an owned socket"), "{text}");
            assert!(!sockets.is_settled(&[]));
            assert_eq!(fs::read(&path).unwrap(), b"not a socket");
            assert!(marker.exists());

            fs::remove_dir_all(root).unwrap();
        }

        // Unsafe ed/ permissions (0755): retain refuses even for a dead socket.
        {
            let root = super::super::supervisor::temporary_directory();
            let logs = root.join("logs");
            let log = DiagnosticLog::open_in(Ok(logs.clone()), "g1-test", "eon");
            let endpoint = root.join("terminal-2.sock");
            let mut sockets = Sockets::default();
            let path = sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap();
            let marker = path.with_extension("owner");
            let stale = UnixListener::bind(&path).unwrap();
            drop(stale);
            wait_for_dead_socket(&path);
            fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();

            sockets.retain(&[], Instant::now(), &log);
            let text = read_diagnostic_log(&logs);
            assert!(text.contains("owned private directory"), "{text}");
            assert!(!sockets.is_settled(&[]));
            assert!(path.exists());
            assert!(marker.exists());

            fs::remove_dir_all(root).unwrap();
        }

        // Post-probe TOCTOU live replacement: remove_socket_if_identity refuses without
        // unlinking the live replacement (mirrors control.rs).
        {
            let root = super::super::supervisor::temporary_directory();
            let endpoint = root.join("terminal-2.sock");
            let mut sockets = Sockets::default();
            let path = sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap();
            let stale = UnixListener::bind(&path).unwrap();
            let observed = control::socket_identity(&path).unwrap().unwrap();
            drop(stale);
            fs::rename(&path, root.join("stale.sock")).unwrap();
            let replacement = UnixListener::bind(&path).unwrap();

            let error = remove_socket_if_identity(&path, observed).unwrap_err();
            assert!(
                error.contains("the socket changed before cleanup"),
                "{error}"
            );
            assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());

            drop(replacement);
            fs::remove_dir_all(root).unwrap();
        }

        // Vanished paths: vanished before probe (Missing => Ok(())), vanished after
        // probe (NotFound => Ok(())), and missing before metadata inspection.
        {
            let root = super::super::supervisor::temporary_directory();
            let endpoint = root.join("terminal-2.sock");
            let mut sockets = Sockets::default();
            let path = sockets
                .prepare(&endpoint, &[], ListenerKind::Helix)
                .unwrap();
            let ephemeral = UnixListener::bind(&path).unwrap();
            let observed = control::socket_identity(&path).unwrap().unwrap();
            drop(ephemeral);
            fs::remove_file(&path).unwrap();

            probe_and_remove_stale(&path, observed).unwrap();
            remove_socket_if_identity(&path, observed).unwrap();
            remove_stale(&path).unwrap();

            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn owner_sockets_derive_from_terminal_endpoints_inside_the_generation_directory() {
        use eon_workspace_protocol::v51::MAX_SESSIONS;
        let base = super::super::supervisor::temporary_directory()
            .canonicalize()
            .unwrap();
        // An independent window's root, reached through an alias as macOS reaches
        // /tmp and /var/folders through /private.
        let physical = base.join("p");
        let window = physical.join("w/0000000a");
        prepare_runtime(&window).unwrap();
        let alias = base.join("a");
        symlink(&physical, &alias).unwrap();
        let generation =
            super::super::generation::generation_directory(&alias.join("w/0000000a"), "g1");
        prepare_runtime(&generation).unwrap();
        let canonical = window.join("generations/g1");
        assert_eq!(generation.canonicalize().unwrap(), canonical);
        assert_eq!(
            generation.join("eon.sock").parent(),
            Some(generation.as_path())
        );
        let log = DiagnosticLog::open_in(Ok(base.join("logs")), "g1-test", "eon");
        let mut bound = 0;
        for number in [1, 2, MAX_SESSIONS] {
            let endpoint = crate::workspace::terminal_endpoint(&generation, number);
            let name = endpoint.file_name().unwrap();
            assert_eq!(endpoint.parent(), Some(generation.as_path()));
            for suffix in [".management", ".record"] {
                let mut artifact = endpoint.clone().into_os_string();
                artifact.push(suffix);
                assert_eq!(Path::new(&artifact).parent(), Some(generation.as_path()));
            }
            assert_eq!(
                socket_path(&endpoint),
                generation.join(SOCKET_DIRECTORY).join(name)
            );
            // Editor terminals are popups, which never take terminal-1.
            if number == 1 {
                continue;
            }
            let expected = canonical.join(SOCKET_DIRECTORY).join(name);
            for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
                let mut sockets = Sockets::default();
                let prepared = sockets.prepare(&endpoint, &[], kind);
                if !platform::socket_path_fits(&expected) {
                    assert!(prepared.unwrap_err().contains("Unix socket limit"));
                    continue;
                }
                let selected = prepared.unwrap();
                assert_eq!(selected, expected);
                assert!(selected.starts_with(&window));
                assert_eq!(canonical_socket_path(&endpoint).unwrap(), selected);
                sockets.retain(&[], Instant::now(), &log);
                assert!(!selected.with_extension("owner").exists());
                bound += 1;
            }
        }
        assert!(bound >= 2, "the test parent {} is too long", base.display());
        // A listener directory that links out of the root is refused, and nothing
        // is created where it points.
        let outside = base.join("outside");
        prepare_runtime(&outside).unwrap();
        symlink(&outside, generation.join(SOCKET_DIRECTORY)).unwrap();
        let endpoint = crate::workspace::terminal_endpoint(&generation, 2);
        for kind in [ListenerKind::Helix, ListenerKind::Neovim] {
            let error = Sockets::default()
                .prepare(&endpoint, &[], kind)
                .unwrap_err();
            assert!(error.contains("owned private directory"), "{error}");
        }
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        fs::remove_dir_all(base).unwrap();
    }
}
