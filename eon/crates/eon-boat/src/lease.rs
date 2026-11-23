use super::{NOUN, Record};
use crate::config::MAX_PARALLEL_LEASES;
use fs2::FileExt;
use rota::launch::atomic_write;
use rota::slots::{Listing, private};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const SLOTS: std::ops::RangeInclusive<u8> = 1..=MAX_PARALLEL_LEASES;

fn lock_path(root: &Path, slot: u8) -> PathBuf {
    if slot == 1 {
        root.join("boat/lease.lock")
    } else {
        root.join("boat").join(format!("lease-{slot}.lock"))
    }
}

fn journal_path(root: &Path, slot: u8) -> PathBuf {
    if slot == 1 {
        root.join("boat/current.json")
    } else {
        root.join("boat").join(format!("current-{slot}.json"))
    }
}

fn private_lock(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.uid() == rustix::process::geteuid().as_raw() && meta.mode() & 0o077 == 0
}

fn open_lock(path: &Path, create: bool) -> Result<Option<File>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if !create && e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("boat-lease-open".into()),
    };
    if !private_lock(&file.metadata().map_err(|_| "boat-lease-metadata")?) {
        return Err("unsafe-boat-lease".into());
    }
    Ok(Some(file))
}

fn lock(file: File) -> Result<Option<File>, String> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(file)),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(_) => Err("boat-lease-lock".into()),
    }
}

fn create_lock(path: &Path) -> Result<Option<File>, String> {
    lock(open_lock(path, true)?.ok_or("boat-lease-open")?)
}

pub(super) struct Lease {
    _lock: File,
    pub root: PathBuf,
    pub slot: u8,
}

impl Lease {
    fn new(file: File, root: &Path, slot: u8) -> Self {
        Self {
            _lock: file,
            root: root.to_owned(),
            slot,
        }
    }

    pub fn claim(root: &Path, slot: u8) -> Result<Option<Self>, String> {
        Ok(create_lock(&lock_path(root, slot))?.map(|file| Self::new(file, root, slot)))
    }

    pub fn save(&self, record: &Record) -> Result<(), String> {
        let bytes = serde_json::to_vec(record).map_err(|_| "boat-record-format")?;
        atomic_write(&journal_path(&self.root, self.slot), &bytes)
            .map_err(|_| "boat-record-write")?;
        atomic_write(
            &self
                .root
                .join("runs")
                .join(&record.run_id)
                .join("receipt.json"),
            &bytes,
        )
        .map_err(|_| "boat-receipt-write".into())
    }
}

pub(super) struct Admission {
    pub lease: Lease,
    pub gate: File,
}

impl Admission {
    /// Called under the FIFO queue lock. Provider work begins after that lock is released.
    pub fn claim(root: &Path, max: u8) -> Result<Option<Self>, String> {
        private::directory(&root.join("boat"), NOUN)?;
        let Some(gate) = create_lock(&root.join("boat/admission.lock"))? else {
            return Ok(None);
        };
        let mut occupied = 0;
        let mut vacant = None;
        for slot in SLOTS {
            match open_lock(&lock_path(root, slot), false)? {
                Some(file) => match lock(file)? {
                    Some(file) if slot <= max && vacant.is_none() => {
                        vacant = Some((slot, Some(file)));
                    }
                    Some(_) => (),
                    None => {
                        occupied += 1;
                        // A held lease without known live work may still be starting or retiring.
                        if record(root, slot)?.is_none_or(|record| !record.has_live_vm()) {
                            return Ok(None);
                        }
                    }
                },
                None if slot <= max && vacant.is_none() => vacant = Some((slot, None)),
                None => (),
            }
        }
        let Some((slot, file)) = vacant.filter(|_| occupied < max) else {
            return Ok(None);
        };
        let lease = match file {
            Some(file) => Some(Lease::new(file, root, slot)),
            None => Lease::claim(root, slot)?,
        };
        Ok(lease.map(|lease| Self { lease, gate }))
    }
}

pub(super) fn record(root: &Path, slot: u8) -> Result<Option<Record>, String> {
    let record: Option<Record> = private::read(&journal_path(root, slot), NOUN)?;
    if record.as_ref().is_some_and(|record| {
        record.schema != 1
            || record.location != "boat"
            || record.run_id.len() != 32
            || !record
                .run_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        return Err("incompatible-boat-record".into());
    }
    Ok(record)
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Ok(meta) if private_lock(&meta) => Ok(Some(meta)),
        _ => Err("unsafe-or-unreadable-boat-lease".into()),
    }
}

pub(super) fn listing(
    root: &Path,
    lock_owner: &dyn Fn(&fs::Metadata) -> Option<Option<u32>>,
) -> Result<Listing, String> {
    let directory = root.join("boat");
    let directories = [root.parent().unwrap_or(root), root, &directory];
    let mut out = Listing::default();
    let mut lock_error = match fs::symlink_metadata(&directory) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            out.fields.insert("boat_lease_held".into(), false.into());
            return Ok(out);
        }
        Err(_) => Some("boat-lease-directory-unreadable".into()),
        Ok(_) => directories
            .into_iter()
            .find_map(|path| private::check(path, NOUN).err()),
    };
    let mut journal_error = None;
    let mut latest = None::<(u8, Record)>;
    let mut held = Vec::new();
    let mut leases = Vec::new();
    let format = |_| "boat-record-format".to_owned();
    let inspect_locks = lock_error.is_none();
    // Relaxed read permissions do not let another user replace a private journal.
    let inspect_journals = inspect_locks
        || directories.into_iter().all(|path| {
            fs::symlink_metadata(path).is_ok_and(|meta| {
                meta.is_dir()
                    && meta.uid() == rustix::process::geteuid().as_raw()
                    && meta.mode() & 0o022 == 0
            })
        });
    if inspect_journals {
        if inspect_locks {
            lock_error = metadata(&directory.join("admission.lock")).err();
        }
        for slot in SLOTS {
            let (known, owner) = if inspect_locks {
                match metadata(&lock_path(root, slot)) {
                    Ok(meta) => (true, meta.as_ref().and_then(lock_owner)),
                    Err(reason) => {
                        lock_error.get_or_insert(reason);
                        (false, None)
                    }
                }
            } else {
                (false, None)
            };
            if let Some(pid) = owner {
                held.push(pid);
                out.busy.extend(pid);
            }
            let mut unfinished = false;
            match record(root, slot) {
                Ok(Some(record)) => {
                    unfinished = record.phase != "stopped";
                    if unfinished {
                        out.lines.push(format!(
                            "boat slot {slot} held pid={} vm={} phase={} run={}",
                            record.pid,
                            record.vm_id.as_deref().unwrap_or("unknown"),
                            record.phase,
                            record.run_id
                        ));
                        let mut holder = serde_json::to_value(&record).map_err(format)?;
                        holder["boat_slot"] = slot.into();
                        out.holders.push(holder);
                    }
                    if latest
                        .as_ref()
                        .is_none_or(|(n, latest)| (record.started, slot) > (latest.started, *n))
                    {
                        latest = Some((slot, record));
                    }
                }
                Ok(None) => (),
                Err(reason) => {
                    journal_error.get_or_insert(reason);
                }
            }
            if owner.is_some() || unfinished || (inspect_locks && !known) {
                leases.push(serde_json::json!({
                    "slot": slot,
                    "held": known.then_some(owner.is_some()),
                    "pid": owner.flatten()
                }));
                if owner.is_some() && !unfinished {
                    out.lines
                        .push(format!("boat slot {slot} lease held (including preflight)"));
                }
            }
        }
    }
    out.fields.insert(
        "boat_lease_held".into(),
        lock_error.is_none().then_some(!held.is_empty()).into(),
    );
    if lock_error.is_none()
        && let [Some(pid)] = held.as_slice()
    {
        out.fields.insert("boat_lease_pid".into(), (*pid).into());
    }
    if !leases.is_empty() {
        out.fields.insert("boat_leases".into(), leases.into());
    }
    if let Some((_, record)) = latest {
        out.fields
            .insert("boat".into(), serde_json::to_value(record).map_err(format)?);
    }
    if let Some(reason) = lock_error.or(journal_error) {
        out.lines.push(format!("boat record unavailable: {reason}"));
        out.fields.insert("boat_error".into(), reason.into());
    }
    Ok(out)
}
