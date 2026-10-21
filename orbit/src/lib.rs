use std::{
    io,
    os::fd::RawFd,
    time::{Duration, Instant},
};

mod boundary;
mod cli;
mod frame;
pub mod host;
mod input;
mod management;
mod platform;
mod pointer;
mod pty;
mod socket;

/// Whether a nonblocking step finished or must wait on a descriptor.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Done,
    Blocked(RawFd),
}

fn errno(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

/// A one-shot deadline the host's wait watches. The wait polls until the
/// earliest armed deadline, so no timer descriptor is needed and the same
/// loop runs on Linux and macOS.
#[derive(Debug, Default)]
pub(crate) struct Timer {
    at: Option<Instant>,
}

impl Timer {
    /// Arms the timer `milliseconds` from now, or disarms it with None.
    pub(crate) fn set(&mut self, milliseconds: Option<u32>) {
        self.at = milliseconds
            .map(|milliseconds| Instant::now() + Duration::from_millis(milliseconds.into()));
    }

    /// Whether the deadline has passed; reporting it disarms the timer.
    pub(crate) fn expired(&mut self) -> bool {
        let expired = self.at.is_some_and(|at| Instant::now() >= at);
        if expired {
            self.at = None;
        }
        expired
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.at
    }
}

#[cfg(test)]
mod test_support {
    use std::{
        fs, io,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    pub(crate) static NEXT: AtomicU64 = AtomicU64::new(0);

    /// The errno a failed mechanism step reports.
    pub(crate) fn code<T>(result: io::Result<T>) -> Option<i32> {
        result.err().and_then(|error| error.raw_os_error())
    }

    pub(crate) static FORK_AND_SOCKET_TEST: Mutex<()> = Mutex::new(());

    pub(crate) struct TestDir(pub(crate) PathBuf);

    impl TestDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yazelix-orbit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
