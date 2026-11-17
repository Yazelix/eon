//! Host-only admission handoff; the configured runner owns local capacity.
use super::{Slots, Ticket, boat};
use crate::launch::{Failure, atomic_write};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize, Serialize)]
struct Payload {
    argv: Vec<String>,
    cwd: PathBuf,
    source: Option<boat::tree::Identity>,
    nesting_token_hash: String,
}

fn failure(error: impl std::fmt::Display) -> Failure {
    (125, error.to_string())
}

pub(super) fn run(
    slots: &Slots<'_>,
    ticket: &Ticket,
    source: Option<&boat::tree::Identity>,
) -> Result<u8, Failure> {
    boat::private_directory(&slots.root.join("delegates")).map_err(failure)?;
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
        .env("EON_SLOT_QUEUED_NS", ticket.queued_ns.to_string())
        .env("EON_SLOT_CLASS", &ticket.class)
        .env(super::NESTING_TOKEN, token)
        .env("EON_SLOT_DELEGATE", directory.path().join("consumed.json"))
        .env("HEAVY_ANY_SLOT", if ticket.front { "1" } else { "0" })
        .env(
            "HEAVY_QUIET_EXEMPT",
            if ticket.quiet_exempt { "1" } else { "0" },
        );
    if let Some(dir) = &slots.config.legacy_dir {
        command.env("HEAVY_DIR", dir);
    }
    let reason = ticket.fallback_reason.as_deref().unwrap_or("policy-local");
    let status = if reason == "policy-local" {
        boat::local::status(&mut command)
    } else {
        boat::local::run(
            &slots.root,
            &mut command,
            reason,
            source.map(boat::tree::Identity::tree),
            slots.program,
        )
    }
    .map_err(failure)?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or_default()) as u8)
}

/// Private delegate payload, invoked only after host queue admission.
pub(crate) fn execute(path: &Path, argv: &[String]) -> Result<u8, Failure> {
    if !path.is_absolute() || path.file_name().is_none_or(|n| n != "command.json") {
        return Err(failure("invalid local handoff path"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| failure("local handoff directory"))?;
    boat::private_directory(directory).map_err(failure)?;
    let payload: Payload = boat::read_private(path)
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
    if let Some(source) = payload.source {
        source.verify().map_err(failure)?;
    }
    let mut command = Command::new(&payload.argv[0]);
    command.args(&payload.argv[1..]);
    for key in ["EON_SLOT_QUEUED_NS", "EON_SLOT_CLASS"] {
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
    let Some(path) = std::env::var_os("EON_SLOT_DELEGATE").map(PathBuf::from) else {
        return false;
    };
    if !path.is_absolute() || path.file_name().is_none_or(|n| n != "consumed.json") {
        return false;
    }
    let Ok(Some(record)) = boat::read_private::<Payload>(&path) else {
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
