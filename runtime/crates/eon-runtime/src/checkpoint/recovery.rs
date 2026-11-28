//! Recovery's per-generation EONR file. The caller holds the lifecycle lock through fencing.

use super::{Checkpoint, Core, Header, MAX_BYTES};
use crate::{
    generation::generation_directory,
    private_file::{self, PublishError, Stage},
    supervisor::validate_private_directory,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread,
};

const MAX_PENDING: usize = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Codec(super::Error),
    Io(String),
    Publish(PublishError),
    StaleRevision,
    TerminalHighWaterDecreased,
    Busy,
    Closed,
    WorkerEnded,
}

pub(crate) type Completion = (u64, Result<(), Error>);

fn path(header: &Header) -> PathBuf {
    generation_directory(Path::new(&header.namespace), &header.generation)
        .join("recovery.checkpoint")
}

fn read(path: &Path, header: &Header) -> Result<Option<Checkpoint>, Error> {
    // Inspect the namespace entry itself even when the spelling ends in `/` or `/.`.
    let root = Path::new(&header.namespace).components().as_path();
    validate_private_directory(root).map_err(Error::Io)?;
    validate_private_directory(&root.join("generations")).map_err(Error::Io)?;
    private_file::read(path, MAX_BYTES)
        .map_err(Error::Io)?
        .map(|bytes| Checkpoint::decode(&bytes, header, &[]).map_err(Error::Codec))
        .transpose()
}

fn commit(
    path: &Path,
    header: &Header,
    core: Core,
    before: impl FnMut(Stage) -> Result<(), PublishError>,
) -> Result<(), Error> {
    let sections = match read(path, header)? {
        Some(previous) => {
            if core.revision <= previous.core.revision {
                return Err(Error::StaleRevision);
            }
            if core.terminal_id_high_water < previous.core.terminal_id_high_water {
                return Err(Error::TerminalHighWaterDecreased);
            }
            previous.sections
        }
        None => Vec::new(),
    };
    let checkpoint = Checkpoint {
        header: header.clone(),
        core,
        sections,
    };
    let bytes = checkpoint.encode().map_err(Error::Codec)?;
    private_file::publish_with(path, &bytes, before).map_err(Error::Publish)
}

/// One serial publisher with bounded requests and completions.
pub(crate) struct Worker {
    sender: Option<SyncSender<Core>>,
    receiver: Receiver<Completion>,
    cancelled: Arc<AtomicBool>,
    pending: usize,
    admitted: Option<u64>,
    retry_latest: bool,
}

impl Worker {
    pub(crate) fn start(header: Header) -> Result<Self, Error> {
        Self::start_with(header, |_| Ok(()))
    }

    fn start_with(
        header: Header,
        mut before: impl FnMut(Stage) -> Result<(), PublishError> + Send + 'static,
    ) -> Result<Self, Error> {
        header.validate().map_err(Error::Codec)?;
        let path = path(&header);
        let (sender, operations) = mpsc::sync_channel::<Core>(MAX_PENDING);
        let (results, receiver) = mpsc::sync_channel(MAX_PENDING);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::Builder::new()
            .name("eon-recovery-checkpoint".into())
            .spawn(move || {
                while let Ok(core) = operations.recv() {
                    let revision = core.revision;
                    let result = if worker_cancelled.load(Ordering::Acquire) {
                        Err(Error::Publish(PublishError::Cancelled))
                    } else {
                        commit(&path, &header, core, |stage| {
                            before(stage)?;
                            if stage == Stage::Rename && worker_cancelled.load(Ordering::Acquire) {
                                return Err(PublishError::Cancelled);
                            }
                            Ok(())
                        })
                    };
                    if results.send((revision, result)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| Error::Io(format!("cannot start checkpoint worker: {error}")))?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            cancelled,
            pending: 0,
            admitted: None,
            retry_latest: false,
        })
    }

    pub(crate) fn submit(&mut self, core: Core) -> Result<(), Error> {
        let sender = self.sender.as_ref().ok_or(Error::Closed)?;
        let revision = core.revision;
        if self.admitted.is_some_and(|admitted| {
            revision < admitted || (revision == admitted && !self.retry_latest)
        }) {
            return Err(Error::StaleRevision);
        }
        if self.pending == MAX_PENDING {
            return Err(Error::Busy);
        }
        sender.try_send(core).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => Error::Busy,
            mpsc::TrySendError::Disconnected(_) => Error::WorkerEnded,
        })?;
        self.pending += 1;
        self.admitted = Some(revision);
        self.retry_latest = false;
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Result<Option<Completion>, Error> {
        if self.pending == 0 {
            return Ok(None);
        }
        match self.receiver.try_recv() {
            Ok((revision, result)) => {
                self.pending -= 1;
                // Keep the admission watermark even when its write failed.
                self.retry_latest = self.admitted == Some(revision)
                    && matches!(
                        &result,
                        Err(Error::Io(_)
                            | Error::Codec(_)
                            | Error::TerminalHighWaterDecreased
                            | Error::Publish(PublishError::Failed(_)))
                    );
                Ok(Some((revision, result)))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.cancel();
                self.pending = 0;
                Err(Error::WorkerEnded)
            }
        }
    }

    /// Close admission immediately. An in-progress rename still reports its real result.
    pub(crate) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.sender = None;
    }

    /// Draining completions acknowledges that no further rename can occur.
    pub(crate) fn is_fenced(&self) -> bool {
        self.sender.is_none() && self.pending == 0
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests;
