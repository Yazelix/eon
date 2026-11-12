//! Persistent session identity and optional names; no process or layout authority.
// Secure reads and atomic writes adapted from Yu Sun's PR #2, commit
// 610b053c184f66144b75bcc9b73dc44f44a2561e, supervisor/named.rs:221-254,331-346.

use crate::supervisor::{effective_uid, request_id, validate_private_directory};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Registration {
    format: u32,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocator {
    format: u32,
    namespace: String,
    sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Publication {
    Definitive,
    Uncertain,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub publication: Publication,
    pub reserved_id: Option<String>,
    pub detail: String,
    busy: bool,
}

impl Failure {
    pub(super) fn into_control(self) -> eon_workspace_protocol::v42::Failure {
        let mut detail = self.to_string();
        let bound = eon_workspace_protocol::v42::MAX_DETAIL_BYTES;
        if detail.len() > bound {
            let tail = self
                .reserved_id
                .as_ref()
                .map_or_else(String::new, |id| format!("; reserved persistent_id={id}"));
            let mut end = bound - tail.len() - 3;
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail.truncate(end);
            detail.push_str("...");
            detail.push_str(&tail);
        }
        eon_workspace_protocol::v42::Failure {
            code: if self.publication == Publication::Uncertain {
                "registry-uncertain"
            } else if self.busy {
                "registry-busy"
            } else {
                "session-registry"
            }
            .into(),
            detail,
        }
    }

    fn new(detail: impl Into<String>) -> Self {
        Self {
            publication: Publication::Definitive,
            reserved_id: None,
            detail: detail.into(),
            busy: false,
        }
    }

    pub(super) fn busy() -> Self {
        Self {
            busy: true,
            ..Self::new("session registry is busy; no change was published")
        }
    }

    fn at(path: &Path, error: impl std::fmt::Display) -> Self {
        Self::new(format!("session registry {}: {error}", path.display()))
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.publication == Publication::Uncertain {
            write!(f, "session registry result is uncertain: ")?;
        }
        write!(f, "{}", self.detail)?;
        if let Some(id) = &self.reserved_id {
            write!(f, "; reserved persistent_id={id}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Registry {
    root: PathBuf,
    #[cfg(test)]
    fault: Option<Fault>,
}

struct RegistryLock(File);

impl std::ops::Deref for RegistryLock {
    type Target = File;
    fn deref(&self) -> &File {
        &self.0
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        // A concurrent fork can retain the open description until exec.
        let _ = self.0.unlock();
    }
}

#[derive(Debug, Clone)]
pub(super) enum Operation {
    Register,
    Create { name: Option<String> },
    Rename { id: String, name: Option<String> },
}

pub(super) type Completion = Result<Option<Registration>, Failure>;

/// One operation at a time; the supervisor only submits and polls.
pub(super) struct Worker {
    sender: SyncSender<Operation>,
    receiver: Receiver<Completion>,
    in_flight: bool,
}

impl Worker {
    pub(super) fn start(registry: Registry) -> Result<Self, Failure> {
        #[cfg(test)]
        if registry.fault == Some(Fault::WorkerSpawn) {
            return Err(Failure::new(
                "injected session registry worker spawn failure",
            ));
        }
        let (sender, operations) = mpsc::sync_channel(1);
        let (results, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("eon-session-registry".into())
            .spawn(move || {
                while let Ok(operation) = operations.recv() {
                    if results.send(registry.perform(operation)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| {
                Failure::new(format!("cannot start session registry worker: {error}"))
            })?;
        Ok(Self {
            sender,
            receiver,
            in_flight: false,
        })
    }

    pub(super) fn submit(&mut self, operation: Operation) -> Result<(), Failure> {
        if self.in_flight {
            return Err(Failure::busy());
        }
        self.sender
            .try_send(operation)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Failure::busy(),
                mpsc::TrySendError::Disconnected(_) => {
                    Failure::new("session registry worker ended")
                }
            })?;
        self.in_flight = true;
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Option<Completion> {
        if !self.in_flight {
            return None;
        }
        let result = match self.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(Failure::new(
                "session registry worker ended with an operation in flight",
            )),
        };
        self.in_flight = false;
        Some(result)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    AllocatorBeforePublish,
    AllocatorDirectorySync,
    RecordBeforePublish,
    RecordDirectorySync,
    ExitAfterReservation,
    ExitAfterDirectoryCreate,
    InitializationParentSync,
    WaitBeforeDirectoryCreate,
    WaitBeforeRecordPublication,
    WorkerSpawn,
    ObserveBusyRetry,
}

impl Registry {
    pub(super) fn perform(&self, operation: Operation) -> Completion {
        match operation {
            Operation::Register => {
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut backoff = Duration::from_millis(20);
                loop {
                    match self.create(None) {
                        Err(error) if error.busy && Instant::now() < deadline => {
                            #[cfg(test)]
                            if self.fault == Some(Fault::ObserveBusyRetry) {
                                fs::write(self.root.join(".tmp-test-retrying"), b"retry")
                                    .map_err(|error| Failure::at(&self.root, error))?;
                            }
                            thread::sleep(
                                backoff.min(deadline.saturating_duration_since(Instant::now())),
                            );
                            backoff = (backoff * 2).min(Duration::from_millis(100));
                        }
                        result => return result.map(Some),
                    }
                }
            }
            Operation::Create { name } => self.create(name.as_deref()).map(Some),
            Operation::Rename { id, name } => self.rename(&id, name.as_deref()),
        }
    }
    #[cfg(test)]
    pub(crate) fn in_test_directory(root: PathBuf) -> Self {
        Self { root, fault: None }
    }

    #[cfg(test)]
    pub(crate) fn inject(&mut self, fault: Option<Fault>) {
        self.fault = fault;
    }

    pub(super) fn new() -> Result<Self, Failure> {
        crate::diagnostics::state_directory()
            .map(|root| Self {
                root: root.join("sessions"),
                #[cfg(test)]
                fault: None,
            })
            .ok_or_else(|| Failure::new("cannot resolve session state directory"))
    }

    pub(super) fn list(&self) -> Result<Vec<Registration>, Failure> {
        if !self.present()? {
            return Ok(Vec::new());
        }
        let lock = self.lock(false, true)?;
        let (allocator, records) = self.load()?;
        if lock.is_none() && allocator.is_some() {
            return Err(Failure::new("session registry lock is missing"));
        }
        Ok(records)
    }

    pub(super) fn lookup(&self, id: &str) -> Result<Option<Registration>, Failure> {
        validate_id(id)?;
        Ok(self.list()?.into_iter().find(|record| record.id == id))
    }

    pub(super) fn create(&self, name: Option<&str>) -> Result<Registration, Failure> {
        validate_name(name)?;
        self.prepare_directory()?;
        let _lock = self.lock(true, false)?;
        let (allocator, records) = self.load()?;
        if allocator.is_none() {
            // A racing initializer may have opened the lock before its creator synced it.
            _lock
                .as_ref()
                .expect("create acquires a lock")
                .sync_all()
                .map_err(|error| Failure::at(&self.root, error))?;
            sync_directory(&self.root)?;
        }
        let mut allocator = match allocator {
            Some(allocator) => allocator,
            None => {
                let mut random = [0; 16];
                File::open("/dev/urandom")
                    .and_then(|mut file| file.read_exact(&mut random))
                    .map_err(|error| Failure::at(&self.root, error))?;
                Allocator {
                    format: 1,
                    namespace: random.iter().map(|byte| format!("{byte:02x}")).collect(),
                    sequence: 0,
                }
            }
        };
        allocator.sequence = allocator
            .sequence
            .checked_add(1)
            .ok_or_else(|| Failure::new("session registry sequence exhausted"))?;
        let id = format!("{}{:016x}", allocator.namespace, allocator.sequence);
        if records.iter().any(|record| record.id == id) {
            return Err(Failure::new("session registry identity collision"));
        }
        let record = Registration {
            format: 1,
            id,
            name: name.map(str::to_owned),
        };
        let bytes = serde_json::to_vec(&record).map_err(|error| Failure::at(&self.root, error))?;
        let allocator_path = self.root.join("allocator.json");
        self.publish(
            &allocator_path,
            &serde_json::to_vec(&allocator).map_err(|error| Failure::at(&allocator_path, error))?,
        )?;
        #[cfg(test)]
        if self.fault == Some(Fault::ExitAfterReservation) {
            std::process::exit(73);
        }
        // Only a confirmed allocator sync makes this ID safe to disclose, even on failure.
        self.publish(&self.record_path(&record.id), &bytes)
            .map_err(|mut error| {
                error.reserved_id = Some(record.id.clone());
                error
            })?;
        Ok(record)
    }

    pub(super) fn rename(
        &self,
        id: &str,
        name: Option<&str>,
    ) -> Result<Option<Registration>, Failure> {
        validate_id(id)?;
        validate_name(name)?;
        if !self.present()? {
            return Ok(None);
        }
        let lock = self.lock(false, false)?;
        let (allocator, records) = self.load()?;
        if lock.is_none() && allocator.is_some() {
            return Err(Failure::new("session registry lock is missing"));
        }
        let Some(mut record) = records.into_iter().find(|record| record.id == id) else {
            return Ok(None);
        };
        record.name = name.map(str::to_owned);
        let path = self.record_path(id);
        self.publish(
            &path,
            &serde_json::to_vec(&record).map_err(|error| Failure::at(&path, error))?,
        )?;
        Ok(Some(record))
    }

    fn prepare_directory(&self) -> Result<(), Failure> {
        // The lock is created only after the directory sync barrier has completed.
        if self.present()? && open_private_file(&self.root.join("registry.lock"), false)?.is_some()
        {
            return Ok(());
        }
        let open_directory = |path: &Path| {
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(
                    libc::O_DIRECTORY
                        | libc::O_CLOEXEC
                        | if path == self.root {
                            libc::O_NOFOLLOW
                        } else {
                            0
                        },
                )
                .open(path)
        };
        let mut missing = Vec::new();
        let mut parent = None;
        for path in self.root.ancestors() {
            let directory = match open_directory(path) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(path);
                    continue;
                }
                Err(error) => return Err(Failure::at(path, error)),
            };
            #[cfg(test)]
            if self.fault == Some(Fault::InitializationParentSync) {
                return Err(Failure::at(
                    path,
                    "injected initialization parent sync failure",
                ));
            }
            directory
                .sync_all()
                .map_err(|error| Failure::at(path, error))?;
            parent = Some((path, directory));
            break;
        }
        let (mut parent_path, mut parent) =
            parent.ok_or_else(|| Failure::new("no accessible session registry parent"))?;
        for path in missing.into_iter().rev() {
            #[cfg(test)]
            if self.fault == Some(Fault::WaitBeforeDirectoryCreate) {
                tests::initialization_rendezvous();
            }
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) =>
                {
                    #[cfg(test)]
                    if self.fault == Some(Fault::ExitAfterDirectoryCreate) {
                        std::process::exit(74);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(Failure::at(path, error)),
            }
            let directory = open_directory(path).map_err(|error| Failure::at(path, error))?;
            directory
                .sync_all()
                .map_err(|error| Failure::at(path, error))?;
            parent
                .sync_all()
                .map_err(|error| Failure::at(parent_path, error))?;
            parent = directory;
            parent_path = path;
        }
        validate_private_directory(&self.root).map_err(Failure::new)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn present(&self) -> Result<bool, Failure> {
        match fs::symlink_metadata(&self.root) {
            Ok(_) => {
                validate_private_directory(&self.root).map_err(Failure::new)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Failure::at(&self.root, error)),
        }
    }

    fn lock(&self, create: bool, shared: bool) -> Result<Option<RegistryLock>, Failure> {
        let path = self.root.join("registry.lock");
        let mut file = open_private_file(&path, false)?;
        if file.is_none() && create {
            let state = self.load();
            // Another initializer may have published while this unlocked read ran.
            file = open_private_file(&path, false)?;
            if file.is_none() {
                if state?.0.is_some() {
                    return Err(Failure::new("session registry lock is missing"));
                }
                file = open_private_file(&path, true)?;
            }
        }
        let Some(file) = file else {
            return Ok(None);
        };
        match if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        } {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(Failure::busy()),
            Err(error) => return Err(Failure::at(&path, error)),
        }
        let lock = RegistryLock(file);
        validate_file(&lock, &path)?;
        Ok(Some(lock))
    }

    fn load(&self) -> Result<(Option<Allocator>, Vec<Registration>), Failure> {
        let allocator_path = self.root.join("allocator.json");
        let allocator = read_record(&allocator_path)?
            .map(|bytes| {
                let value: Allocator = serde_json::from_slice(&bytes)
                    .map_err(|error| Failure::at(&allocator_path, error))?;
                if value.format != 1 || !lower_hex(&value.namespace, 32) {
                    return Err(Failure::at(
                        &allocator_path,
                        "invalid allocator format or namespace",
                    ));
                }
                Ok(value)
            })
            .transpose()?;
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| Failure::at(&self.root, error))? {
            let entry = entry.map_err(|error| Failure::at(&self.root, error))?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| Failure::at(&entry.path(), "invalid registry filename"))?;
            if matches!(name, "allocator.json" | "registry.lock") || name.starts_with(".tmp-") {
                continue;
            }
            let id = name
                .strip_suffix(".json")
                .ok_or_else(|| Failure::at(&entry.path(), "invalid registry filename"))?;
            validate_id(id)?;
            let path = entry.path();
            let bytes = read_record(&path)?
                .ok_or_else(|| Failure::at(&path, "registration disappeared"))?;
            let record: Registration =
                serde_json::from_slice(&bytes).map_err(|error| Failure::at(&path, error))?;
            if record.format != 1 || record.id != id {
                return Err(Failure::at(
                    &path,
                    "invalid registration format or identity",
                ));
            }
            validate_name(record.name.as_deref())?;
            records.push(record);
        }
        if !records.is_empty() && allocator.is_none() {
            return Err(Failure::at(
                &allocator_path,
                "allocator is missing beside registrations",
            ));
        }
        if let Some(allocator) = &allocator {
            let latest_id = format!("{}{:016x}", allocator.namespace, allocator.sequence);
            for record in &records {
                if !record.id.starts_with(&allocator.namespace) {
                    return Err(Failure::new(
                        "session registration has a different registry namespace",
                    ));
                }
                if record.id > latest_id {
                    return Err(Failure::new("session registry allocator sequence is stale"));
                }
            }
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok((allocator, records))
    }

    fn publish(&self, path: &Path, bytes: &[u8]) -> Result<(), Failure> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Failure::at(path, "record exceeds 8192 bytes"));
        }
        let temporary = self.root.join(format!(".tmp-{}", request_id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .map_err(|error| Failure::at(&temporary, error))?;
        let mut published = false;
        let result = (|| {
            validate_file(&file, &temporary)?;
            file.write_all(bytes)
                .map_err(|error| Failure::at(&temporary, error))?;
            file.sync_all()
                .map_err(|error| Failure::at(&temporary, error))?;
            #[cfg(test)]
            if self.fault == Some(Fault::WaitBeforeRecordPublication)
                && path != self.root.join("allocator.json")
            {
                fs::write(self.root.join(".tmp-test-waiting"), b"ready")
                    .map_err(|error| Failure::at(&self.root, error))?;
                let deadline = Instant::now() + Duration::from_secs(10);
                while !self.root.join(".tmp-test-release").exists() {
                    if Instant::now() >= deadline {
                        return Err(Failure::new("test publication was not released"));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            #[cfg(test)]
            if matches!(
                (path == self.root.join("allocator.json"), self.fault),
                (true, Some(Fault::AllocatorBeforePublish))
                    | (false, Some(Fault::RecordBeforePublish))
            ) {
                return Err(Failure::at(path, "injected failure before publication"));
            }
            fs::rename(&temporary, path).map_err(|error| Failure::at(path, error))?;
            published = true;
            #[cfg(test)]
            if matches!(
                (path == self.root.join("allocator.json"), self.fault),
                (true, Some(Fault::AllocatorDirectorySync))
                    | (false, Some(Fault::RecordDirectorySync))
            ) {
                return Err(Failure::at(path, "injected directory sync failure"));
            }
            sync_directory(&self.root)
        })();
        if !published && validate_file(&file, &temporary).is_ok() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|mut error: Failure| {
            if published {
                error.publication = Publication::Uncertain;
            }
            error
        })
    }
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn validate_id(id: &str) -> Result<(), Failure> {
    if lower_hex(id, 48) {
        Ok(())
    } else {
        Err(Failure::new("invalid persistent session identity"))
    }
}

fn validate_name(name: Option<&str>) -> Result<(), Failure> {
    if let Some(name) = name {
        eon_workspace_protocol::v42::validate_workspace_name(name)
            .map_err(|error| Failure::new(error.to_string()))?;
    }
    Ok(())
}

fn validate_file(file: &File, path: &Path) -> Result<(), Failure> {
    let metadata = file.metadata().map_err(|error| Failure::at(path, error))?;
    let current = fs::symlink_metadata(path).map_err(|error| Failure::at(path, error))?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || !current.is_file()
        || (metadata.dev(), metadata.ino()) != (current.dev(), current.ino())
    {
        return Err(Failure::at(
            path,
            "must be an owned mode-0600 regular file, unchanged while opened",
        ));
    }
    Ok(())
}

fn open_private_file(path: &Path, create: bool) -> Result<Option<File>, Failure> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Failure::at(path, error)),
    };
    validate_file(&file, path)?;
    Ok(Some(file))
}

fn read_record(path: &Path) -> Result<Option<Vec<u8>>, Failure> {
    let Some(file) = open_private_file(path, false)? else {
        return Ok(None);
    };
    if file
        .metadata()
        .map_err(|error| Failure::at(path, error))?
        .len()
        > MAX_BYTES
    {
        return Err(Failure::at(path, "record exceeds 8192 bytes"));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Failure::at(path, error))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Failure::at(path, "record exceeds 8192 bytes"));
    }
    validate_file(&file, path)?;
    Ok(Some(bytes))
}

fn sync_directory(path: &Path) -> Result<(), Failure> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| Failure::at(path, error))
}

#[cfg(test)]
mod tests;
