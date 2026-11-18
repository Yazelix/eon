//! Operations without a behavior-equivalent safe std/rustix 1.1.4 API.
use std::{
    os::unix::process::CommandExt,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

pub(crate) struct Signals(Vec<(libc::c_int, libc::sigaction)>);

impl Signals {
    pub(crate) fn new() -> std::io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Relaxed);
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            // SAFETY: std/rustix has no safe native sigaction constructor; zero is
            // valid for its fields, and the mask and handler are set before use.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut previous = action;
            action.sa_sigaction = interrupt as *const () as usize;
            // SAFETY: no safe native sigset initializer exists; action's mask is
            // writable and remains live through registration.
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            // SAFETY: no safe sigaction API exists; the handler only stores an atomic,
            // both records are live, and the guard restores every successful install.
            if unsafe { libc::sigaction(signal, &action, &mut previous) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            guard.0.push((signal, previous));
        }
        Ok(guard)
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, previous) in &self.0 {
            // SAFETY: no safe disposition-restoration API exists; previous is the
            // exact live record returned by this guard's successful sigaction call.
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}

pub(crate) fn owned_child(command: &mut Command) -> &mut Command {
    command.process_group(0);
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{self, Signal};
        let parent = process::getpid();
        // SAFETY: pre_exec has no safe equivalent; the forked child calls only
        // async-signal-safe prctl/getppid/getpid/kill, without allocation or locks.
        unsafe {
            command.pre_exec(move || {
                process::set_parent_process_death_signal(Some(Signal::KILL))?;
                // Cover parent death between fork and installing the death signal.
                if process::getppid() != Some(parent) {
                    let _ = process::kill_process(process::getpid(), Signal::KILL);
                }
                Ok(())
            });
        }
    }
    command
}
