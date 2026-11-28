//! Persistent session identity and optional names; no process or layout authority.
// Secure reads and atomic writes adapted from Yu Sun's PR #2, commit
// 610b053c184f66144b75bcc9b73dc44f44a2561e, supervisor/named.rs:221-254,331-346.
//
// Set-aside (EON-C38) takes an invalid registry out of use by renaming the
// whole directory, unchanged, under its own locks, the way SQLite's `.recover`
// and journald's `~` files keep damaged bytes and start fresh. One atomic
// no-replace rename (rustix `renameat_with`: `RENAME_NOREPLACE` on Linux,
// `RENAME_EXCL` on Apple) beats moving files one by one: no partial state,
// directory-level damage included, and nothing re-moded or created.

use crate::supervisor::{effective_uid, request_id};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 8192;
const LOCK: &str = "registry.lock";
/// Appended to on-disk validation failures that set-aside accepts.
const SET_ASIDE_HINT: &str =
    "; 'eon session registry set-aside' moves this registry aside and keeps its files";

/// Only the format number, read before the strict parse so a later format
/// with other fields still says it is a later format.
#[derive(Deserialize)]
struct Header {
    format: u32,
}

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
    damage: bool,
    /// Set by `Registry::hint` on damage that set-aside would move.
    hint: bool,
}

impl Failure {
    pub(super) fn into_control(self) -> eon_workspace_protocol::v51::Failure {
        self.bounded(eon_workspace_protocol::v51::MAX_DETAIL_BYTES)
    }

    /// The control failure with its detail cut to `bound` bytes, keeping `tail()`.
    pub(super) fn bounded(self, bound: usize) -> eon_workspace_protocol::v51::Failure {
        let mut detail = self.to_string();
        if detail.len() > bound {
            let tail = format!("...{}", self.tail());
            detail.truncate(detail.floor_char_boundary(bound - tail.len()));
            detail.push_str(&tail);
        }
        eon_workspace_protocol::v51::Failure {
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
            damage: false,
            hint: false,
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

    /// Marks confirmed on-disk damage, which set-aside moves; an error that
    /// only prevented inspection is not.
    fn damaged(mut self) -> Self {
        self.damage = true;
        self
    }

    fn uncertain(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            publication: Publication::Uncertain,
            ..Self::at(path, error)
        }
    }

    /// What the bound in `bounded` keeps.
    fn tail(&self) -> String {
        let mut tail = self
            .reserved_id
            .as_ref()
            .map_or_else(String::new, |id| format!("; reserved persistent_id={id}"));
        if self.hint {
            tail.push_str(SET_ASIDE_HINT);
        }
        tail
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.publication == Publication::Uncertain {
            write!(f, "session registry result is uncertain: ")?;
        }
        write!(f, "{}{}", self.detail, self.tail())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Registry {
    root: PathBuf,
    #[cfg(test)]
    fault: Option<Fault>,
}

/// What `Registry::set_aside` did.
#[derive(Debug)]
pub(super) enum SetAside {
    Absent,
    Valid,
    Moved {
        destination: PathBuf,
        /// Why the registry was invalid; never carries the set-aside hint.
        validation_error: Failure,
    },
}

/// An flock on a registry lock or directory, released on drop.
struct RegistryLock(File);

/// `registry.lock`'s flock, when it is present, and the registry
/// directory's, released in that order.
struct Locked {
    file: Option<RegistryLock>,
    directory: RegistryLock,
}

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
    /// Whether no operation awaits its completion.
    pub(super) fn is_idle(&self) -> bool {
        !self.in_flight
    }

    pub(super) fn start(registry: Registry) -> Result<Self, Failure> {
        #[cfg(test)]
        WORKER_STARTS.with(|starts| starts.set(starts.get() + 1));
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
                #[cfg(test)]
                if registry.fault == Some(Fault::SignalWorkerEnd) {
                    let _ = fs::write(registry.root.join(WORKER_ENDED), b"ended");
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

/// How long a busy registry is retried before the busy failure stands.
const BUSY_RETRY_LIMIT: Duration = Duration::from_secs(2);
/// The first pause after a busy attempt; each later pause doubles, up to
/// `MAX_BUSY_BACKOFF`.
const FIRST_BUSY_BACKOFF: Duration = Duration::from_millis(20);
const MAX_BUSY_BACKOFF: Duration = Duration::from_millis(100);

/// Runs `operation` until it is not busy or `limit` has passed on `clock`,
/// pausing through `pause` between attempts; the last pause ends at the limit.
fn retry_with<T>(
    mut operation: impl FnMut() -> Result<T, Failure>,
    limit: Duration,
    mut clock: impl FnMut() -> Instant,
    mut pause: impl FnMut(Duration) -> Result<(), Failure>,
) -> Result<T, Failure> {
    let deadline = clock() + limit;
    let mut backoff = FIRST_BUSY_BACKOFF;
    loop {
        match operation() {
            Err(error) if error.busy && clock() < deadline => {
                pause(backoff.min(deadline.saturating_duration_since(clock())))?;
                backoff = (backoff * 2).min(MAX_BUSY_BACKOFF);
            }
            result => return result,
        }
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
    /// A busy retry's pause writes `.tmp-test-retrying` and waits for
    /// `.tmp-test-release`; the attempt after it always runs. Its retry
    /// limit is an hour, so the pause is reached however late the first
    /// busy attempt is.
    HandOffBusyRetry,
    /// Busy failures are not retried.
    BusyRetryLimitZero,
    /// The worker's thread writes `WORKER_ENDED` once its loop ends.
    SignalWorkerEnd,
    /// Writes `paused-<point>` in the state directory and waits for
    /// `release-<point>` there.
    Pause(Pause),
    /// Reading a registration fails with `EIO`.
    ReadError,
    /// The rename fails with `EINVAL`, as on a filesystem without it.
    SetAsideRename,
    SetAsideInspectDestination,
    SetAsideParentSync,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pause {
    SetAsideAfterLock,
    SetAsideBeforeValidate,
    SetAsideBeforeRename,
    SetAsideAfterRename,
    PreflightAfterRead,
    AfterLockOpen,
    AfterPrepareDirectory,
    AfterLockDirectoryOpen,
    BeforeLockCreate,
}

/// The file `Fault::SignalWorkerEnd` writes in the registry root.
#[cfg(test)]
pub(crate) const WORKER_ENDED: &str = ".tmp-test-worker-ended";

#[cfg(test)]
thread_local! {
    /// `Worker::start` calls on this thread, counted on entry.
    static WORKER_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How long the next busy-retried operation on this thread takes before
    /// its first attempt.
    pub(crate) static FIRST_ATTEMPT_DELAY: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
    /// The next set-aside destination name on this thread, instead of a
    /// random one.
    pub(crate) static SET_ASIDE_NAME: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// How often this thread entered `Worker::start`.
#[cfg(test)]
pub(crate) fn worker_starts() -> usize {
    WORKER_STARTS.with(std::cell::Cell::get)
}

impl Registry {
    pub(super) fn perform(&self, operation: Operation) -> Completion {
        match operation {
            Operation::Register => self.retry_busy(|| self.create(None)).map(Some),
            Operation::Create { name } => self.create(name.as_deref()).map(Some),
            Operation::Rename { id, name } => self.rename(&id, name.as_deref()),
        }
    }
    /// `operation`, retried while another holder makes the registry busy, for
    /// up to `BUSY_RETRY_LIMIT`; after that the busy failure stands.
    pub(super) fn retry_busy<T>(
        &self,
        operation: impl FnMut() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        #[cfg(test)]
        let operation = {
            let mut operation = operation;
            let mut delay = FIRST_ATTEMPT_DELAY.with(std::cell::Cell::take);
            move || {
                if let Some(delay) = delay.take() {
                    thread::sleep(delay);
                }
                operation()
            }
        };
        // A hand-off test waits for its own release, never for this clock.
        #[cfg(test)]
        let limit = match self.fault {
            Some(Fault::BusyRetryLimitZero) => Duration::ZERO,
            Some(Fault::HandOffBusyRetry) => Duration::from_secs(3600),
            _ => BUSY_RETRY_LIMIT,
        };
        #[cfg(not(test))]
        let limit = BUSY_RETRY_LIMIT;
        retry_with(operation, limit, Instant::now, |pause| {
            #[cfg(test)]
            if matches!(
                self.fault,
                Some(Fault::ObserveBusyRetry | Fault::HandOffBusyRetry)
            ) {
                fs::write(self.root.join(".tmp-test-retrying"), b"retry")
                    .map_err(|error| Failure::at(&self.root, error))?;
            }
            #[cfg(test)]
            if self.fault == Some(Fault::HandOffBusyRetry) {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !self.root.join(".tmp-test-release").exists() {
                    if Instant::now() >= deadline {
                        return Err(Failure::new("test busy retry was not released"));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            thread::sleep(pause);
            Ok(())
        })
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

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// Writes `paused-<point>` in the state directory and waits for
    /// `release-<point>` there, when that pause is injected.
    #[cfg(test)]
    fn pause(&self, point: Pause) {
        if self.fault != Some(Fault::Pause(point)) {
            return;
        }
        let state = self.root.parent().expect("a registry root has a parent");
        fs::write(state.join(format!("paused-{point:?}")), b"paused").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !state.join(format!("release-{point:?}")).exists() {
            assert!(Instant::now() < deadline, "{point:?} was not released");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Adds the set-aside hint to confirmed damage that set-aside's own
    /// checks would accept.
    fn hint<T>(&self, operation: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
        operation().map_err(|mut error| {
            error.hint = error.damage
                && !error.busy
                && error.publication == Publication::Definitive
                && matches!(self.inspect(), Ok(Some(_)));
            error
        })
    }

    /// Set-aside's checks before locking (EON-C38): the trusted state
    /// directory, then the registry directory and its lock as set-aside
    /// opens them; `None` when the registry is absent.
    fn inspect(&self) -> Result<Option<(File, File, Option<File>)>, Failure> {
        use rustix::fs::{FileType, Mode, OFlags};
        let (Some(state), Some(name)) = (self.root.parent(), self.root.file_name()) else {
            return Err(Failure::new("cannot resolve session state directory"));
        };
        let Some(parent) = open_state_directory(state)? else {
            return Ok(None);
        };
        let refuse =
            |path: &Path, why: &dyn std::fmt::Display| refused("session registry", path, why);
        let openable = |stat: &rustix::fs::Stat, kind, mode| {
            FileType::from_raw_mode(stat.st_mode) == kind
                && stat.st_uid == effective_uid()
                && Mode::from_raw_mode(stat.st_mode).contains(mode)
        };
        let Some(found) = stat_at(&parent, name).map_err(|error| io_at(&self.root, error))? else {
            return Ok(None);
        };
        if !openable(&found, FileType::Directory, Mode::RUSR | Mode::XUSR) {
            return Err(refuse(&self.root, &"is not an owned directory it can open"));
        }
        let directory = open_at(&parent, name, OFlags::DIRECTORY)
            .map_err(|error| refuse(&self.root, &std::io::Error::from(error)))?;
        if identity(&stat_of(&directory, &self.root)?) != identity(&found) {
            return Err(changed());
        }
        let path = self.root.join(LOCK);
        let lock = match open_at(&directory, LOCK, OFlags::empty()) {
            Ok(file) if openable(&stat_of(&file, &path)?, FileType::RegularFile, Mode::RUSR) => {
                Some(file)
            }
            Ok(_) => return Err(refuse(&path, &"is not an owned regular file it can open")),
            Err(rustix::io::Errno::NOENT) => None,
            Err(error) => return Err(refuse(&path, &std::io::Error::from(error))),
        };
        Ok(Some((parent, directory, lock)))
    }

    /// EON-C38 set-aside: moves an invalid registry, unchanged, to a unique
    /// sibling under both registry locks; leaves a valid or absent one alone.
    pub(super) fn set_aside(&self) -> Result<SetAside, Failure> {
        use rustix::io::Errno;
        let Some((parent, directory, lock)) = self.inspect()? else {
            return Ok(SetAside::Absent);
        };
        let state = self
            .root
            .parent()
            .expect("inspect resolved the state directory");
        let name = self
            .root
            .file_name()
            .expect("inspect resolved the registry name");
        let lock_path = self.root.join(LOCK);
        let directory = locked(directory, &self.root, false)?;
        let lock = lock
            .map(|file| locked(file, &lock_path, false))
            .transpose()?;
        #[cfg(test)]
        self.pause(Pause::SetAsideAfterLock);
        // What is no longer what set-aside locked, with the registry at `at`.
        let mismatch = |at: &OsStr| -> rustix::io::Result<Option<&'static str>> {
            let named = match rustix::fs::stat(state) {
                Ok(stat) => Some(identity(&stat)),
                Err(Errno::NOENT) => None,
                Err(error) => return Err(error),
            };
            if named != Some(identity(&rustix::fs::fstat(&parent)?)) {
                return Ok(Some("its state directory path now names another directory"));
            }
            let held = rustix::fs::fstat(&*directory)?;
            if stat_at(&parent, at)?.map(|stat| identity(&stat)) != Some(identity(&held)) {
                return Ok(Some(
                    "it is not the directory that was checked: something replaced the registry as it moved",
                ));
            }
            let held = lock
                .as_ref()
                .map(|lock| rustix::fs::fstat(&**lock))
                .transpose()?;
            let current = stat_at(&directory, LOCK)?;
            Ok(
                (current.as_ref().map(identity) != held.as_ref().map(identity)).then_some(
                    "its registry.lock changed as it moved: an older Eon build may still write to the registry path",
                ),
            )
        };
        let unchanged = || match mismatch(name) {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(changed()),
            Err(error) => Err(io_at(&self.root, error)),
        };
        unchanged()?;
        #[cfg(test)]
        self.pause(Pause::SetAsideBeforeValidate);
        // Through the held descriptors, as every read validates.
        let validation = stat_of(&directory, &self.root)
            .and_then(|stat| check_private_directory(&self.root, &stat))
            .and_then(|()| {
                if let Some(lock) = &lock {
                    validate_file(lock, &directory, LOCK, &lock_path)?;
                }
                self.validated(&directory, lock.is_some())
            });
        let validation_error = match validation {
            Ok(_) => return Ok(SetAside::Valid),
            Err(error) if error.damage => error,
            // What prevented inspection is not damage: nothing moves.
            Err(error) => return Err(error),
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos());
        let random = random_hex::<8>().map_err(|error| Failure::at(&self.root, error))?;
        let target = format!("{}.set-aside-{nanos}-{random}", name.to_string_lossy());
        #[cfg(test)]
        let target = SET_ASIDE_NAME
            .with(|name| name.borrow_mut().take())
            .unwrap_or(target);
        let destination = state.join(&target);
        unchanged()?;
        #[cfg(test)]
        self.pause(Pause::SetAsideBeforeRename);
        let rename = || {
            let flags = rustix::fs::RenameFlags::NOREPLACE;
            rustix::fs::renameat_with(&parent, name, &parent, target.as_str(), flags)
        };
        #[cfg(test)]
        let rename = || match self.fault {
            Some(Fault::SetAsideRename) => Err(Errno::INVAL),
            _ => rename(),
        };
        if let Err(errno) = rename() {
            let atomic = [Errno::INVAL, Errno::NOTSUP, Errno::OPNOTSUPP, Errno::NOSYS];
            return Err(Failure::at(
                &self.root,
                format!(
                    "cannot move it to {}: {}; nothing was moved{}",
                    destination.display(),
                    std::io::Error::from(errno),
                    if atomic.contains(&errno) {
                        ", as this filesystem cannot move it atomically: move it aside by hand"
                    } else {
                        ""
                    }
                ),
            ));
        }
        // Moved: every later failure is uncertain, and nothing is renamed back.
        let uncertain = |why: &dyn std::fmt::Display| {
            Failure::uncertain(
                &self.root,
                format!("moved to {destination:?}, but {why}; nothing was deleted"),
            )
        };
        #[cfg(test)]
        self.pause(Pause::SetAsideAfterRename);
        let inspected = mismatch(OsStr::new(&target));
        #[cfg(test)]
        let inspected = match self.fault {
            Some(Fault::SetAsideInspectDestination) => Err(Errno::IO),
            _ => inspected,
        };
        if let Some(why) = inspected.map_err(|error| uncertain(&std::io::Error::from(error)))? {
            return Err(uncertain(&why));
        }
        #[cfg(test)]
        if self.fault == Some(Fault::SetAsideParentSync) {
            return Err(uncertain(&"injected parent sync failure"));
        }
        parent
            .sync_all()
            .map_err(|error| uncertain(&format_args!("{}: {error}", state.display())))?;
        Ok(SetAside::Moved {
            destination,
            validation_error,
        })
    }

    pub(super) fn list(&self) -> Result<Vec<Registration>, Failure> {
        self.hint(|| {
            if !self.present()? {
                return Ok(Vec::new());
            }
            let Some(held) = self.lock(false, true)? else {
                return Ok(Vec::new());
            };
            Ok(self.validated(&held.directory, held.file.is_some())?.1)
        })
    }

    /// The whole registry in `directory` as every read validates it, with a
    /// lock present or not.
    fn validated(
        &self,
        directory: &File,
        locked: bool,
    ) -> Result<(Option<Allocator>, Vec<Registration>), Failure> {
        let (allocator, records) = self.load(directory)?;
        if !locked && allocator.is_some() {
            return Err(self.missing_lock());
        }
        Ok((allocator, records))
    }

    fn missing_lock(&self) -> Failure {
        Failure::at(&self.root.join(LOCK), "is missing").damaged()
    }

    /// Preflight validates a snapshot without opening or taking the registry lock.
    pub(super) fn list_for_preflight(&self) -> Result<Vec<Registration>, Failure> {
        self.hint(|| {
            if !self.present()? {
                return Ok(Vec::new());
            }
            let directory = open_directory(&self.root).map_err(|error| io_at(&self.root, error))?;
            let lock = self.root.join(LOCK);
            let locked = stat_at(&directory, LOCK)
                .map_err(|error| io_at(&lock, error))?
                .map(|stat| check_file(&lock, &stat))
                .transpose()?
                .is_some();
            let (_, records) = self.validated(&directory, locked)?;
            #[cfg(test)]
            self.pause(Pause::PreflightAfterRead);
            // The registry path still names the directory that was read.
            let held = stat_of(&directory, &self.root)?;
            if !rustix::fs::lstat(&self.root).is_ok_and(|now| identity(&now) == identity(&held)) {
                return Err(Failure::at(&self.root, "changed during preflight"));
            }
            check_private_directory(&self.root, &held)?;
            Ok(records)
        })
    }

    pub(super) fn lookup(&self, id: &str) -> Result<Option<Registration>, Failure> {
        validate_id(id)?;
        Ok(self.list()?.into_iter().find(|record| record.id == id))
    }

    pub(super) fn create(&self, name: Option<&str>) -> Result<Registration, Failure> {
        self.hint(|| {
            validate_name(name)?;
            self.prepare_directory()?;
            #[cfg(test)]
            self.pause(Pause::AfterPrepareDirectory);
            let Some(Locked {
                file: Some(lock),
                directory,
            }) = self.lock(true, false)?
            else {
                return Err(Failure::busy());
            };
            let (allocator, records) = self.validated(&directory, true)?;
            if allocator.is_none() {
                // A racing initializer may have opened the lock before its creator synced it.
                lock.sync_all()
                    .and_then(|()| directory.sync_all())
                    .map_err(|error| Failure::at(&self.root, error))?;
            }
            let mut allocator = match allocator {
                Some(allocator) => allocator,
                None => Allocator {
                    format: 1,
                    namespace: random_hex::<16>()
                        .map_err(|error| Failure::at(&self.root, error))?,
                    sequence: 0,
                },
            };
            let allocator_path = self.root.join("allocator.json");
            allocator.sequence = allocator.sequence.checked_add(1).ok_or_else(|| {
                Failure::at(&allocator_path, "session registry sequence exhausted")
            })?;
            let id = format!("{}{:016x}", allocator.namespace, allocator.sequence);
            if records.iter().any(|record| record.id == id) {
                return Err(Failure::at(
                    &self.record_path(&id),
                    "session registry identity collision",
                ));
            }
            let record = Registration {
                format: 1,
                id,
                name: name.map(str::to_owned),
            };
            let bytes =
                serde_json::to_vec(&record).map_err(|error| Failure::at(&self.root, error))?;
            self.publish(
                &directory,
                "allocator.json",
                &serde_json::to_vec(&allocator)
                    .map_err(|error| Failure::at(&allocator_path, error))?,
            )?;
            #[cfg(test)]
            if self.fault == Some(Fault::ExitAfterReservation) {
                std::process::exit(73);
            }
            // Only a confirmed allocator sync makes this ID safe to disclose, even on failure.
            self.publish(&directory, &format!("{}.json", record.id), &bytes)
                .map_err(|mut error| {
                    error.reserved_id = Some(record.id.clone());
                    error
                })?;
            Ok(record)
        })
    }

    pub(super) fn rename(
        &self,
        id: &str,
        name: Option<&str>,
    ) -> Result<Option<Registration>, Failure> {
        self.hint(|| {
            validate_id(id)?;
            validate_name(name)?;
            if !self.present()? {
                return Ok(None);
            }
            let Some(held) = self.lock(false, false)? else {
                return Ok(None);
            };
            let (_, records) = self.validated(&held.directory, held.file.is_some())?;
            let Some(mut record) = records.into_iter().find(|record| record.id == id) else {
                return Ok(None);
            };
            record.name = name.map(str::to_owned);
            let bytes = serde_json::to_vec(&record)
                .map_err(|error| Failure::at(&self.record_path(id), error))?;
            self.publish(&held.directory, &format!("{id}.json"), &bytes)?;
            Ok(Some(record))
        })
    }

    fn prepare_directory(&self) -> Result<(), Failure> {
        // The lock is created only after the directory sync barrier has completed.
        if self.present()? && fs::symlink_metadata(self.root.join(LOCK)).is_ok() {
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
        self.present().map(drop)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn present(&self) -> Result<bool, Failure> {
        match rustix::fs::lstat(&self.root) {
            Ok(stat) => check_private_directory(&self.root, &stat).map(|()| true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(error) => Err(io_at(&self.root, error)),
        }
    }

    /// Flocks the registry directory, exclusively only around first-time
    /// lock creation and shared otherwise, so set-aside never moves a
    /// registry an operation of this build is using, even one whose lock
    /// was removed; then `registry.lock`. `None` when the registry is absent;
    /// the lock is absent only without `create`.
    fn lock(&self, create: bool, shared: bool) -> Result<Option<Locked>, Failure> {
        use rustix::{fs::OFlags, io::Errno};
        let path = self.root.join(LOCK);
        let directory = match open_directory(&self.root) {
            Ok(directory) => directory,
            Err(Errno::NOENT) if !create => return Ok(None),
            Err(Errno::NOENT) => return Err(Failure::busy()),
            Err(error) => return Err(io_at(&self.root, error)),
        };
        let first = create
            && stat_at(&directory, LOCK)
                .map_err(|error| io_at(&path, error))?
                .is_none();
        #[cfg(test)]
        self.pause(Pause::AfterLockDirectoryOpen);
        let directory = locked(directory, &self.root, !first)?;
        // After the flock, so a directory set-aside moved, or one re-moded
        // since `prepare_directory`, is used no further.
        let held = stat_of(&directory, &self.root)?;
        match rustix::fs::lstat(&self.root) {
            Ok(now) if identity(&now) == identity(&held) => {}
            Ok(_) | Err(Errno::NOENT) => return Err(Failure::busy()),
            Err(error) => return Err(io_at(&self.root, error)),
        }
        check_private_directory(&self.root, &held)?;
        let mut file = open_private(&directory, LOCK, &path)?;
        if file.is_none() && first {
            let state = self.load(&directory);
            // An older build may have created the lock while this read ran.
            file = open_private(&directory, LOCK, &path)?;
            if file.is_none() {
                if state?.0.is_some() {
                    return Err(self.missing_lock());
                }
                #[cfg(test)]
                self.pause(Pause::BeforeLockCreate);
                let flags = OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC;
                let mode = rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR;
                file = Some(
                    rustix::fs::openat(&*directory, LOCK, flags, mode)
                        .map(File::from)
                        .map_err(|error| io_at(&path, error))?,
                );
            }
        } else if file.is_none() && create {
            // Removed since the directory was opened.
            return Err(Failure::busy());
        }
        let Some(file) = file else {
            return Ok(Some(Locked {
                file: None,
                directory,
            }));
        };
        #[cfg(test)]
        self.pause(Pause::AfterLockOpen);
        let file = locked(file, &path, shared)?;
        validate_file(&file, &directory, LOCK, &path)?;
        Ok(Some(Locked {
            file: Some(file),
            directory,
        }))
    }

    fn load(&self, directory: &File) -> Result<(Option<Allocator>, Vec<Registration>), Failure> {
        let allocator_path = self.root.join("allocator.json");
        let allocator = read_record(directory, "allocator.json", &allocator_path)?
            .map(|bytes| {
                later_format(&allocator_path, &bytes)?;
                let value: Allocator = serde_json::from_slice(&bytes)
                    .map_err(|error| Failure::at(&allocator_path, error).damaged())?;
                if value.format != 1 || !lower_hex(&value.namespace, 32) {
                    return Err(Failure::at(
                        &allocator_path,
                        "invalid allocator format or namespace",
                    )
                    .damaged());
                }
                Ok(value)
            })
            .transpose()?;
        let mut records = Vec::new();
        let entries =
            rustix::fs::Dir::read_from(directory).map_err(|error| io_at(&self.root, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_at(&self.root, error))?;
            let bytes = entry.file_name().to_bytes();
            let path = self.root.join(OsStr::from_bytes(bytes));
            let invalid = || Failure::at(&path, "invalid registry filename").damaged();
            let name = std::str::from_utf8(bytes).map_err(|_| invalid())?;
            if matches!(name, "." | ".." | "allocator.json" | LOCK) || name.starts_with(".tmp-") {
                continue;
            }
            let id = name.strip_suffix(".json").ok_or_else(invalid)?;
            validate_id(id).map_err(|error| Failure::at(&path, error.detail).damaged())?;
            #[cfg(test)]
            if self.fault == Some(Fault::ReadError) {
                return Err(io_at(&path, rustix::io::Errno::IO));
            }
            let bytes = read_record(directory, name, &path)?
                .ok_or_else(|| Failure::at(&path, "registration disappeared"))?;
            later_format(&path, &bytes)?;
            let record: Registration = serde_json::from_slice(&bytes)
                .map_err(|error| Failure::at(&path, error).damaged())?;
            if record.format != 1 || record.id != id {
                return Err(
                    Failure::at(&path, "invalid registration format or identity").damaged(),
                );
            }
            validate_name(record.name.as_deref())
                .map_err(|error| Failure::at(&path, error.detail).damaged())?;
            records.push(record);
        }
        if !records.is_empty() && allocator.is_none() {
            return Err(
                Failure::at(&allocator_path, "allocator is missing beside registrations").damaged(),
            );
        }
        if let Some(allocator) = &allocator {
            let latest_id = format!("{}{:016x}", allocator.namespace, allocator.sequence);
            for record in &records {
                if !record.id.starts_with(&allocator.namespace) {
                    return Err(Failure::at(
                        &self.record_path(&record.id),
                        "session registration has a different registry namespace",
                    )
                    .damaged());
                }
                if record.id > latest_id {
                    return Err(Failure::at(
                        &allocator_path,
                        "session registry allocator sequence is stale",
                    )
                    .damaged());
                }
            }
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok((allocator, records))
    }

    /// Publishes `name` in the locked `directory` through a synced temporary file.
    fn publish(&self, directory: &File, name: &str, bytes: &[u8]) -> Result<(), Failure> {
        use rustix::fs::{Mode, OFlags};
        let path = self.root.join(name);
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Failure::at(&path, "record exceeds 8192 bytes"));
        }
        #[cfg(test)]
        let allocator = name == "allocator.json";
        let temporary = format!(".tmp-{}", request_id());
        let temporary_path = self.root.join(&temporary);
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW;
        let mut file = rustix::fs::openat(
            directory,
            temporary.as_str(),
            flags | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map(File::from)
        .map_err(|error| io_at(&temporary_path, error))?;
        let mut published = false;
        let result = (|| {
            validate_file(&file, directory, &temporary, &temporary_path)?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| Failure::at(&temporary_path, error))?;
            #[cfg(test)]
            if self.fault == Some(Fault::WaitBeforeRecordPublication) && !allocator {
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
                (allocator, self.fault),
                (true, Some(Fault::AllocatorBeforePublish))
                    | (false, Some(Fault::RecordBeforePublish))
            ) {
                return Err(Failure::at(&path, "injected failure before publication"));
            }
            rustix::fs::renameat(directory, temporary.as_str(), directory, name)
                .map_err(|error| io_at(&path, error))?;
            published = true;
            #[cfg(test)]
            if matches!(
                (allocator, self.fault),
                (true, Some(Fault::AllocatorDirectorySync))
                    | (false, Some(Fault::RecordDirectorySync))
            ) {
                return Err(Failure::at(&path, "injected directory sync failure"));
            }
            directory
                .sync_all()
                .map_err(|error| Failure::at(&self.root, error))
        })();
        if !published && validate_file(&file, directory, &temporary, &temporary_path).is_ok() {
            let _ =
                rustix::fs::unlinkat(directory, temporary.as_str(), rustix::fs::AtFlags::empty());
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
        eon_workspace_protocol::v51::validate_workspace_name(name)
            .map_err(|error| Failure::new(error.to_string()))?;
    }
    Ok(())
}

/// `file` is still `name` in `directory`, an owned mode-0600 regular file.
fn validate_file(file: &File, directory: &File, name: &str, path: &Path) -> Result<(), Failure> {
    let held = stat_of(file, path)?;
    match stat_at(directory, name).map_err(|error| io_at(path, error))? {
        Some(now) if identity(&now) == identity(&held) => check_file(path, &held),
        _ => Err(Failure::at(path, "changed while it was opened")),
    }
}

/// Every registry file is an owned mode-0600 regular file.
fn check_file(path: &Path, stat: &rustix::fs::Stat) -> Result<(), Failure> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != effective_uid()
    {
        return Err(Failure::at(path, "must be an owned mode-0600 regular file").damaged());
    }
    check_mode(
        path,
        stat,
        "an owned mode-0600 regular file",
        Mode::RUSR | Mode::WUSR,
    )
}

fn check_private_directory(path: &Path, stat: &rustix::fs::Stat) -> Result<(), Failure> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != effective_uid()
    {
        return Err(Failure::at(path, "must be an owned private directory").damaged());
    }
    check_mode(path, stat, "an owned private directory", Mode::RWXU)
}

fn check_mode(
    path: &Path,
    stat: &rustix::fs::Stat,
    expected: &str,
    mode: rustix::fs::Mode,
) -> Result<(), Failure> {
    let actual = rustix::fs::Mode::from_raw_mode(stat.st_mode);
    if actual == mode {
        return Ok(());
    }
    Err(Failure::at(
        path,
        format!("must be {expected}, not mode {:04o}", actual.bits()),
    )
    .damaged())
}

/// Reads a file's `format` before its strict parse.
fn later_format(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    match serde_json::from_slice::<Header>(bytes) {
        Ok(Header { format }) if format > 1 => {
            Err(Failure::at(path, format!("uses format {format} from a later Eon build")).damaged())
        }
        _ => Ok(()),
    }
}

fn random_hex<const N: usize>() -> std::io::Result<String> {
    let mut random = [0; N];
    File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut random))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn locked(file: File, path: &Path, shared: bool) -> Result<RegistryLock, Failure> {
    match if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    } {
        Ok(()) => Ok(RegistryLock(file)),
        Err(fs::TryLockError::WouldBlock) => Err(Failure::busy()),
        Err(error) => Err(Failure::at(path, error)),
    }
}

fn changed() -> Failure {
    Failure {
        busy: true,
        ..Failure::new("session registry changed while it was being set aside; nothing was moved")
    }
}

fn refused(what: &str, path: &Path, why: &dyn std::fmt::Display) -> Failure {
    Failure::new(format!(
        "{what} {}: {why}; set-aside refuses it and moved nothing",
        path.display()
    ))
}

fn io_at(path: &Path, error: rustix::io::Errno) -> Failure {
    Failure::at(path, std::io::Error::from(error))
}

fn stat_of(file: &File, path: &Path) -> Result<rustix::fs::Stat, Failure> {
    rustix::fs::fstat(file).map_err(|error| io_at(path, error))
}

/// Opens `name` in `directory` read-only, never following a final link.
fn open_at(
    directory: impl AsFd,
    name: impl rustix::path::Arg,
    flags: rustix::fs::OFlags,
) -> rustix::io::Result<File> {
    use rustix::fs::OFlags;
    rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | flags,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
}

fn open_directory(path: &Path) -> rustix::io::Result<File> {
    open_at(rustix::fs::CWD, path, rustix::fs::OFlags::DIRECTORY)
}

/// `name` in `directory`, not following a link; `None` when absent.
fn stat_at(
    directory: &File,
    name: impl rustix::path::Arg,
) -> rustix::io::Result<Option<rustix::fs::Stat>> {
    match rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(stat)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error),
    }
}

/// A stat's file identity.
fn identity(stat: &rustix::fs::Stat) -> impl PartialEq + use<> {
    (stat.st_dev, stat.st_ino)
}

/// The trusted state directory (EON-C38 L0), or `None` when it and so the
/// registry are absent; nothing is created.
fn open_state_directory(path: &Path) -> Result<Option<File>, Failure> {
    let refuse =
        |at: &Path, why: &dyn std::fmt::Display| refused("session state directory", at, why);
    let directory = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            for ancestor in path.ancestors() {
                let metadata = match fs::symlink_metadata(ancestor) {
                    Ok(metadata) if metadata.is_symlink() => fs::metadata(ancestor),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    found => found,
                }
                .map_err(|error| refuse(ancestor, &error))?;
                if !metadata.is_dir() {
                    return Err(refuse(ancestor, &"is not a directory"));
                }
                if ancestor == path {
                    return Err(Failure::busy());
                }
                return Ok(None);
            }
            return Err(refuse(path, &"has no existing ancestor"));
        }
        Err(error) => return Err(refuse(path, &error)),
    };
    let metadata = directory.metadata().map_err(|error| refuse(path, &error))?;
    let mode = metadata.mode();
    if !trusted_state(metadata.uid(), metadata.gid(), mode) {
        return Err(refuse(
            path,
            &format_args!("other users can write it (mode {:04o})", mode & 0o7777),
        ));
    }
    Ok(Some(directory))
}

/// EON-C38 q5: the user owns the state directory, others cannot write it,
/// and its group can only when it is the user's own primary group.
fn trusted_state(uid: u32, gid: u32, mode: u32) -> bool {
    uid == effective_uid()
        && mode & 0o002 == 0
        && (mode & 0o020 == 0 || gid == rustix::process::getegid().as_raw())
}

/// The owned mode-0600 regular file `name` in `directory`; `None` when absent.
fn open_private(directory: &File, name: &str, path: &Path) -> Result<Option<File>, Failure> {
    let Some(stat) = stat_at(directory, name).map_err(|error| io_at(path, error))? else {
        return Ok(None);
    };
    check_file(path, &stat)?;
    let file = open_at(directory, name, rustix::fs::OFlags::empty())
        .map_err(|error| io_at(path, error))?;
    validate_file(&file, directory, name, path)?;
    Ok(Some(file))
}

fn read_record(directory: &File, name: &str, path: &Path) -> Result<Option<Vec<u8>>, Failure> {
    let Some(file) = open_private(directory, name, path)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Failure::at(path, error))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Failure::at(path, "record exceeds 8192 bytes").damaged());
    }
    validate_file(&file, directory, name, path)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
