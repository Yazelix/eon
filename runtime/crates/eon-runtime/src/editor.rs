//! Owned Helix and Neovim listeners for Editor popups (EON-C10).

use super::{
    diagnostics::DiagnosticLog,
    managed_environment::{self, ManagedPrograms, Tool},
    platform,
    sessions::{self, RunningSession},
    supervisor::{effective_uid, prepare_runtime, validate_private_directory},
};
use eon_workspace_protocol::v44::Failure;
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
    key: &eon_workspace_protocol::v44::PopupKey,
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

fn exchange(
    endpoint: &Path,
    request: Vec<u8>,
    kind: ListenerKind,
    id: &str,
    key: Option<&eon_workspace_protocol::v44::PopupKey>,
) -> Result<(), Failure> {
    if request.len() > FRAME_LIMIT {
        return Err(failure("editor-rejected", "Editor request exceeds 64 KiB"));
    }
    let socket = socket_path(endpoint);
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut waiting_for_bind = false;
    let mut stream = loop {
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
        match sessions::unix_connect_with_timeout(&socket, remaining) {
            Ok(stream) => break stream,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && listener_kind(endpoint)? == Some(kind) =>
            {
                waiting_for_bind = true;
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(error) => {
                let detail = format!("Editor connection failed: {error}; no request sent");
                let mut failure = io_failure(error);
                failure.detail = detail;
                return Err(failure);
            }
        }
    };
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
        let runtime = fs::canonicalize(endpoint.parent().expect("terminal endpoint has a parent"))
            .map_err(|error| format!("cannot resolve Neovim Editor runtime directory: {error}"))?;
        let path = socket_path(
            &runtime.join(
                endpoint
                    .file_name()
                    .expect("terminal endpoint has a filename"),
            ),
        );
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
        let directory = runtime.join("editor");
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
            let canonical = fs::canonicalize(directory)
                .map_err(|error| format!("cannot resolve Editor socket directory: {error}"))?
                .join(
                    endpoint
                        .file_name()
                        .expect("terminal endpoint has a filename"),
                );
            if canonical.to_str().is_none() || !platform::socket_path_fits(&canonical) {
                return Err(format!(
                    "Editor socket path {} must fit a Unix socket and be UTF-8",
                    canonical.display()
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

fn socket_path(endpoint: &Path) -> PathBuf {
    endpoint.with_file_name("editor").join(
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

fn remove_stale(path: &Path) -> Result<(), String> {
    let failure =
        |detail: &str| format!("cannot remove Editor socket {}: {detail}", path.display());
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(&error.to_string())),
    };
    validate_private_directory(path.parent().expect("Editor socket has a parent"))?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(failure("expected an owned socket"));
    }
    // A nonblocking attempt distinguishes a refused socket from one that may listen.
    match sessions::unix_connect_with_timeout(path, Duration::ZERO) {
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => return Err(failure("the socket may still be listening")),
    }
    match fs::symlink_metadata(path) {
        Ok(current) if same_file(&metadata, &current) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => return Err(failure("the socket changed before cleanup")),
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failure(&error.to_string())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::{fs::symlink, net::UnixListener};

    #[test]
    fn neovim_listener_refuses_colons_in_the_canonical_runtime_path() {
        let root = super::super::supervisor::temporary_directory();
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
            !runtime.join("editor").exists(),
            "refusal created listener state"
        );
        // Helix's Unix socket parser accepts colons, so its existing route stays usable.
        let socket = sockets
            .prepare(&endpoint, &[], ListenerKind::Helix)
            .unwrap();
        assert_eq!(listener_kind(&endpoint).unwrap(), Some(ListenerKind::Helix));
        assert_eq!(socket.parent().unwrap(), runtime.join("editor"));
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
            &eon_workspace_protocol::v44::PopupKey::Unbound,
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
            &eon_workspace_protocol::v44::PopupKey::Unbound,
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
                &eon_workspace_protocol::v44::PopupKey::Unbound,
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
        let server = std::thread::spawn(move || {
            // Exercise Ready-before-bind using the versioned Neovim marker.
            std::thread::sleep(Duration::from_millis(5));
            let listener = UnixListener::bind(socket).unwrap();
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
            &eon_workspace_protocol::v44::PopupKey::Unbound,
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
        let bound = socket.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(5));
            let listener = UnixListener::bind(&bound).unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            stream
                .write_all(b"{\"version\":1,\"id\":\"r1\",\"status\":\"ok\"}\n")
                .unwrap();
        });
        started.recv().unwrap();
        open(&endpoint, Path::new("/a"), "r1").unwrap();
        server.join().unwrap();
        let refused = open(&endpoint, Path::new("/a"), "r2").unwrap_err();
        assert_eq!(refused.code, "editor-unavailable");
        assert!(refused.detail.contains("no request sent"), "{refused:?}");
        fs::remove_file(&socket).unwrap();
        let listener =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        listener
            .bind(&socket2::SockAddr::unix(&socket).unwrap())
            .unwrap();
        listener.listen(1).unwrap();
        let mut queued = Vec::new();
        for _ in 0..10 {
            match sessions::unix_connect_with_timeout(&socket, Duration::from_millis(10)) {
                Ok(stream) => queued.push(stream),
                Err(_) => break,
            }
        }
        assert!(
            !queued.is_empty() && queued.len() < 10,
            "queue did not fill"
        );
        let full = open(&endpoint, Path::new("/a"), "r3").unwrap_err();
        assert_eq!(full.code, "editor-unavailable", "{full:?}");
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
            root.canonicalize().unwrap().join("editor/terminal-2.sock")
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
        let diagnostic = fs::read_dir(logs).unwrap().next().unwrap().unwrap().path();
        let text = fs::read_to_string(diagnostic).unwrap();
        assert_eq!(text.matches("the socket may still be listening").count(), 1);

        drop(listener);
        sockets.retain(&[], now + Duration::from_secs(1), &log);
        assert!(path.exists(), "cleanup waits until the retry is due");
        sockets.retain(&[], now + Duration::from_secs(2), &log);
        assert!(!path.parent().unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }
}
