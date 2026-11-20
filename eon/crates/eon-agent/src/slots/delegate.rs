//! Host-only admission handoff; the configured runner owns local capacity.
use super::{Local, Route, Slots, Ticket, private};
use crate::launch::{Failure, atomic_write};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize, Serialize)]
struct Payload {
    argv: Vec<String>,
    cwd: PathBuf,
    source: Option<Value>,
    nesting_token_hash: String,
}

/// The delegate's noun in private-state error tokens.
const NOUN: &str = "slot";

fn failure(error: impl std::fmt::Display) -> Failure {
    (125, error.to_string())
}

pub(super) fn run(
    slots: &Slots<'_>,
    route: &dyn Route,
    ticket: &Ticket,
    source: Option<&Value>,
) -> Result<u8, Failure> {
    private::directory(&slots.root.join("delegates"), NOUN).map_err(failure)?;
    let directory = tempfile::Builder::new()
        .prefix("local-")
        .tempdir_in(slots.root.join("delegates"))
        .map_err(failure)?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).map_err(failure)?;
    let token = crate::run::random()?;
    let payload = serde_json::json!({ "argv": ticket.argv,
        "cwd": std::env::current_dir().map_err(failure)?, "source": source,
        "nesting_token_hash": super::token_hash(&token) });
    let path = directory.path().join("command.json");
    atomic_write(&path, &serde_json::to_vec(&payload).map_err(failure)?).map_err(failure)?;
    let mut command = Command::new(&slots.config.local_runner[0]);
    command.args(&slots.config.local_runner[1..]);
    command.arg(std::env::current_exe().map_err(failure)?);
    command.args(slots.reinvoke);
    command.args([
        "_slot-local-exec",
        path.to_str().ok_or_else(|| failure("local-record-path"))?,
        "--",
    ]);
    command.args(&ticket.argv);
    command
        .envs(&slots.config.env)
        .env("ROTA_SLOT_QUEUED_NS", ticket.queued_ns.to_string())
        .env("ROTA_SLOT_CLASS", &ticket.class)
        .env("ROTA_SLOT_RUN_KEY", ticket.run_key())
        .env(super::NESTING_TOKEN, token)
        .env("ROTA_SLOT_DELEGATE", directory.path().join("consumed.json"))
        .env("HEAVY_ANY_SLOT", if ticket.front { "1" } else { "0" })
        .env(
            "HEAVY_QUIET_EXEMPT",
            if ticket.quiet_exempt { "1" } else { "0" },
        );
    if let Some(dir) = &slots.config.legacy_dir {
        command.env("HEAVY_DIR", dir);
    }
    let fallback = ticket.fallback();
    let status = route
        .run_local(Local {
            root: &slots.root,
            command: &mut command,
            fallback: fallback.as_ref(),
            source,
            program: slots.program,
        })
        .map_err(failure)?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or_default()) as u8)
}

/// Private delegate payload, invoked only after host queue admission.
pub(crate) fn execute(
    path: &Path,
    argv: &[String],
    route: Option<&dyn Route>,
) -> Result<u8, Failure> {
    if !path.is_absolute() || path.file_name().is_none_or(|n| n != "command.json") {
        return Err(failure("invalid local handoff path"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| failure("local handoff directory"))?;
    private::directory(directory, NOUN).map_err(failure)?;
    let payload: Payload = private::read(path, NOUN)
        .map_err(failure)?
        .ok_or_else(|| failure("missing/consumed local handoff"))?;
    if payload.argv.first().is_none_or(|s| s.is_empty())
        || !payload.cwd.is_absolute()
        || payload.argv != argv
    {
        return Err(failure("invalid local handoff"));
    }
    fs::rename(path, directory.join("consumed.json")).map_err(failure)?;
    std::env::set_current_dir(&payload.cwd).map_err(failure)?;
    if let Some(source) = &payload.source {
        super::verify(route, source).map_err(failure)?;
    }
    let mut command = Command::new(&payload.argv[0]);
    command.args(&payload.argv[1..]);
    for key in [
        "ROTA_SLOT_QUEUED_NS",
        "ROTA_SLOT_CLASS",
        "ROTA_SLOT_RUN_KEY",
    ] {
        command.env_remove(key);
    }
    let ownership = fs::File::open(directory.join("consumed.json")).map_err(failure)?;
    ownership.lock_exclusive().map_err(failure)?;
    let status = command.status().map_err(failure)?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or_default()) as u8)
}

pub(super) fn nested() -> bool {
    let Some(path) = std::env::var_os("ROTA_SLOT_DELEGATE").map(PathBuf::from) else {
        return false;
    };
    if !path.is_absolute() || path.file_name().is_none_or(|n| n != "consumed.json") {
        return false;
    }
    let Ok(Some(record)) = private::read::<Payload>(&path, NOUN) else {
        return false;
    };
    let Ok(file) = fs::File::open(&path) else {
        return false;
    };
    std::env::var(super::NESTING_TOKEN).is_ok_and(|token| {
        record.nesting_token_hash == super::token_hash(&token)
            && FileExt::try_lock_shared(&file)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
    })
}
