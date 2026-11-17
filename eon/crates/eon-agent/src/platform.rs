//! Operations without a behavior-equivalent safe std/rustix 1.1.4 API.
use rustix::process::{Pid, Signal};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, IntoRawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub(crate) static RUN_STOP: AtomicBool = AtomicBool::new(false);
pub(crate) static BOAT_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn request_stop(_: libc::c_int) {
    RUN_STOP.store(true, Ordering::SeqCst);
}

extern "C" fn cancel(signal: libc::c_int) {
    BOAT_SIGNAL.store(signal, Ordering::Relaxed);
}

pub(crate) fn install_run_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: no safe handler-registration API exists; this handler only stores an atomic.
        unsafe { libc::signal(signal, request_stop as *const () as libc::sighandler_t) };
    }
}

pub(crate) struct BoatSignals(Vec<(i32, libc::sighandler_t)>);

impl BoatSignals {
    pub(crate) fn install() -> Self {
        BOAT_SIGNAL.store(0, Ordering::Relaxed);
        Self(
            [libc::SIGINT, libc::SIGTERM, libc::SIGHUP]
                .into_iter()
                .map(|signal| {
                    // SAFETY: no safe handler-registration API exists; cancel only stores an atomic.
                    (signal, unsafe {
                        libc::signal(signal, cancel as *const () as libc::sighandler_t)
                    })
                })
                .collect(),
        )
    }
}

impl Drop for BoatSignals {
    fn drop(&mut self) {
        for (signal, previous) in &self.0 {
            // SAFETY: no safe disposition API exists; restore the value returned by signal.
            unsafe { libc::signal(*signal, *previous) };
        }
    }
}

pub(crate) fn foreground(tty: impl AsFd, group: Pid) -> io::Result<()> {
    // SAFETY: no safe disposition API exists; SIG_IGN is a valid static disposition.
    let previous = unsafe { libc::signal(libc::SIGTTOU, libc::SIG_IGN) };
    if previous == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    let result = rustix::termios::tcsetpgrp(tty, group).map_err(io::Error::from);
    // SAFETY: no safe disposition API exists; restore the value returned by signal.
    unsafe { libc::signal(libc::SIGTTOU, previous) };
    result
}

pub(crate) fn foreground_on_exec(command: &mut Command, tty: &File) -> io::Result<()> {
    let tty = tty.try_clone()?;
    // SAFETY: pre_exec has no safe equivalent; the owned tty stays live and only
    // async-signal-safe signal/tcsetpgrp/getpid calls run in the forked child.
    unsafe { command.pre_exec(move || foreground(&tty, rustix::process::getpid())) };
    Ok(())
}

pub(crate) fn nice_on_exec(command: &mut Command, increment: i32) {
    // SAFETY: pre_exec has no safe equivalent; this captures only an integer
    // and calls the async-signal-safe nice wrapper without allocating.
    unsafe {
        command.pre_exec(move || {
            rustix::process::nice(increment)
                .map(|_| ())
                .map_err(Into::into)
        })
    };
}

pub(crate) fn close(file: File) -> io::Result<()> {
    // SAFETY: safe owners discard close errors; consume this sole owner, close
    // exactly once and never retry, even if Linux reports an error.
    if unsafe { libc::close(file.into_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn check_write(file: &File) -> io::Result<()> {
    use rustix::fs::{Access, AtFlags};
    match rustix::fs::accessat(
        file,
        c".",
        Access::WRITE_OK | Access::EXEC_OK,
        AtFlags::EACCESS,
    ) {
        Err(rustix::io::Errno::NOSYS) => libc_check_write(file),
        result => result.map_err(Into::into),
    }
}

fn libc_check_write(file: &File) -> io::Result<()> {
    // SAFETY: rustix lacks libc's effective-ID emulation on pre-5.8 Linux when
    // IDs differ; the borrowed fd and static name stay live and retain no pointers.
    if unsafe {
        libc::faccessat(
            file.as_raw_fd(),
            c".".as_ptr(),
            libc::W_OK | libc::X_OK,
            libc::AT_EACCESS,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn pidfd_signal(fd: impl AsFd, signal: i32) -> bool {
    if let Some(signal) = Signal::from_named_raw(signal) {
        return rustix::process::pidfd_send_signal(fd, signal).is_ok();
    }
    // SAFETY: rustix cannot represent signal zero or construct realtime signals
    // safely; this syscall borrows a live fd, passes no pointers and retains nothing.
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_fd().as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        ) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_access_fallback_keeps_success_and_directory_errors() {
        let root = tempfile::tempdir().unwrap();
        let directory = File::open(root.path()).unwrap();
        libc_check_write(&directory).unwrap();
        check_write(&directory).unwrap();
        let file = tempfile::tempfile().unwrap();
        assert_eq!(
            libc_check_write(&file).unwrap_err().raw_os_error(),
            Some(libc::ENOTDIR)
        );
        assert_eq!(
            check_write(&file).unwrap_err().raw_os_error(),
            Some(libc::ENOTDIR)
        );
    }
}
