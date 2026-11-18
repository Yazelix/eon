//! The benchmark's only unsafe code, each piece behind a safe function whose
//! callers cannot break its preconditions: the fork-to-exec hooks, the
//! signal record, this process's resource usage and, outside Linux, the load
//! average. No safe API covers these; `lib.rs` allows unsafe code here alone.

use std::process::Command;

/// Has `command`'s process get SIGKILL when this process dies, however it
/// dies, and refuse to start if this process died before it could ask
/// (Linux; elsewhere a no-op). A spawn from a thread that ends first kills
/// it too, so callers spawn from the main thread.
pub(crate) fn die_with_parent(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{self, Pid, Signal};
        use std::os::unix::process::CommandExt;
        let parent = std::process::id();
        // SAFETY: the hook runs in the forked child before exec and makes only
        // the prctl and getppid system calls; errno converts to io::Error
        // without allocating.
        unsafe {
            command.pre_exec(move || {
                process::set_parent_process_death_signal(Some(Signal::KILL))?;
                if u32::try_from(Pid::as_raw(process::getppid())) != Ok(parent) {
                    return Err(rustix::io::Errno::SRCH.into());
                }
                Ok(())
            });
        }
    }
    command
}

/// Starts `command` in a new session whose controlling terminal is its
/// stdin, which the caller has set to a PTY slave.
pub(crate) fn own_session_on_stdin(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // Taken before the fork, so the child never initializes it.
    let stdin = std::io::stdin();
    // SAFETY: the hook runs in the forked child before exec and makes only
    // the setsid and TIOCSCTTY system calls, on fd 0, which Command has
    // already pointed at the configured stdin; errno converts to io::Error
    // without allocating.
    unsafe {
        command.pre_exec(move || {
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(&stdin)?;
            Ok(())
        });
    }
    command
}

/// Signals caught into one per-process record that suites poll.
pub(crate) mod signal {
    use std::{
        io,
        sync::atomic::{AtomicI32, Ordering},
    };

    pub(crate) use rustix::process::Signal;

    /// The last signal a `catch` handler took in this process. It is never
    /// cleared, so a signal that arrives before a later install is kept.
    /// The suite interrupt, the response run and the pressure worker each
    /// read it as "stop"; no process installs two of them for different
    /// purposes (the response run and the worker are whole processes).
    static RECEIVED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn record(signal: libc::c_int) {
        RECEIVED.store(signal, Ordering::SeqCst);
    }

    /// Whether a second delivery of a caught signal is caught again.
    #[derive(Clone, Copy)]
    pub(crate) enum Reset {
        /// Every delivery is recorded.
        Never,
        /// The first delivery is recorded and restores the default action,
        /// so a second one takes it.
        OnFirst,
    }

    /// Caught signals; dropping it puts back the actions it replaced.
    #[must_use]
    pub(crate) struct Caught(Vec<(Signal, libc::sigaction)>);

    impl Caught {
        /// Keeps the handlers for the rest of the process.
        pub(crate) fn forever(mut self) {
            self.0.clear();
        }
    }

    impl Drop for Caught {
        fn drop(&mut self) {
            for (signal, previous) in self.0.drain(..).rev() {
                // SAFETY: previous is the complete action sigaction reported
                // for this signal, so reinstalling it is valid.
                unsafe {
                    libc::sigaction(signal.as_raw(), &raw const previous, std::ptr::null_mut())
                };
            }
        }
    }

    /// Records each of `signals` in place of its action, with interrupted
    /// system calls restarted. On failure the ones already caught are put
    /// back.
    pub(crate) fn catch(signals: &[Signal], reset: Reset) -> io::Result<Caught> {
        let flags = match reset {
            Reset::Never => libc::SA_RESTART,
            Reset::OnFirst => libc::SA_RESTART | libc::SA_RESETHAND,
        };
        let mut caught = Caught(Vec::with_capacity(signals.len()));
        for &signal in signals {
            // SAFETY: record only stores to a lock-free atomic, which is
            // async-signal-safe. Both actions are zeroed plain data, and the
            // new one gets its handler, flags and an emptied mask before
            // sigaction reads it.
            let previous = unsafe {
                let mut action = std::mem::zeroed::<libc::sigaction>();
                action.sa_sigaction = record as extern "C" fn(libc::c_int) as libc::sighandler_t;
                action.sa_flags = flags;
                libc::sigemptyset(&raw mut action.sa_mask);
                let mut previous = std::mem::zeroed::<libc::sigaction>();
                (libc::sigaction(signal.as_raw(), &raw const action, &raw mut previous) == 0)
                    .then_some(previous)
            };
            match previous {
                Some(previous) => caught.0.push((signal, previous)),
                None => return Err(io::Error::last_os_error()),
            }
        }
        Ok(caught)
    }

    /// The last caught signal, if any arrived.
    pub(crate) fn received() -> Option<Signal> {
        Signal::from_named_raw(RECEIVED.load(Ordering::SeqCst))
    }

    /// Ends the process by `signal`'s default action, so its parent sees
    /// that signal.
    pub(crate) fn reraise(signal: Signal) {
        // SAFETY: restoring the default action and raising a signal take no
        // pointers.
        unsafe {
            libc::signal(signal.as_raw(), libc::SIG_DFL);
            libc::raise(signal.as_raw());
        }
    }
}

/// CPU time and major faults this process has used so far.
pub(crate) struct Usage {
    pub(crate) cpu_ms: u64,
    pub(crate) major_faults: u64,
}

pub(crate) fn self_usage() -> Usage {
    // SAFETY: rusage is plain data, and getrusage fills it for RUSAGE_SELF,
    // which always exists.
    let usage = unsafe {
        let mut usage = std::mem::zeroed::<libc::rusage>();
        libc::getrusage(libc::RUSAGE_SELF, &raw mut usage);
        usage
    };
    let ms = |time: libc::timeval| time.tv_sec as u64 * 1000 + time.tv_usec as u64 / 1000;
    Usage {
        cpu_ms: ms(usage.ru_utime) + ms(usage.ru_stime),
        major_faults: usage.ru_majflt as u64,
    }
}

/// The 1-minute load average (Linux reads `/proc/loadavg` without this).
#[cfg(not(target_os = "linux"))]
pub(crate) fn load_average() -> Option<f64> {
    let mut loads = [0.0; 1];
    // SAFETY: getloadavg writes at most the one value the array holds.
    (unsafe { libc::getloadavg(loads.as_mut_ptr(), 1) } == 1).then_some(loads[0])
}

#[cfg(test)]
mod tests {
    use super::signal::{self, Reset, Signal};
    use std::sync::Mutex;

    /// Plain `cargo test` shares one process between tests; nextest does not.
    static SIGNALS: Mutex<()> = Mutex::new(());

    fn handler(signal: Signal) -> libc::sighandler_t {
        // SAFETY: with no new action, sigaction only writes the current one
        // into this zeroed plain-data sigaction.
        unsafe {
            let mut current = std::mem::zeroed::<libc::sigaction>();
            assert_eq!(
                libc::sigaction(signal.as_raw(), std::ptr::null(), &raw mut current),
                0
            );
            current.sa_sigaction
        }
    }

    /// Delivers `signal` to this thread; it is handled before this returns.
    fn raise(signal: Signal) {
        // SAFETY: raise takes no pointers; the signal is caught by record.
        assert_eq!(unsafe { libc::raise(signal.as_raw()) }, 0);
    }

    #[test]
    fn caught_signal_is_recorded_and_the_old_action_restored() {
        let _lock = SIGNALS.lock().unwrap_or_else(|poison| poison.into_inner());
        let before = handler(Signal::USR1);
        let caught = signal::catch(&[Signal::USR2, Signal::USR1], Reset::Never).unwrap();
        let ours = handler(Signal::USR1);
        assert_ne!(ours, before);
        raise(Signal::USR1);
        assert_eq!(signal::received(), Some(Signal::USR1));
        assert_eq!(handler(Signal::USR1), ours, "Never keeps the handler");
        drop(caught);
        assert_eq!(handler(Signal::USR1), before);
    }

    #[test]
    fn reset_on_first_leaves_the_default_action() {
        let _lock = SIGNALS.lock().unwrap_or_else(|poison| poison.into_inner());
        let caught = signal::catch(&[Signal::USR2], Reset::OnFirst).unwrap();
        assert_ne!(handler(Signal::USR2), libc::SIG_DFL);
        raise(Signal::USR2);
        assert_eq!(signal::received(), Some(Signal::USR2));
        assert_eq!(handler(Signal::USR2), libc::SIG_DFL);
        drop(caught);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn own_session_on_stdin_gives_the_child_a_session_and_terminal() {
        use std::process::{Command, Stdio};
        let (_master, slave) = crate::pty::open(24, 80).unwrap();
        let mut command = Command::new("cat");
        command
            .arg("/proc/self/stat")
            .stdin(slave)
            .stdout(Stdio::piped());
        let child = super::own_session_on_stdin(&mut command).spawn().unwrap();
        let pid = child.id().to_string();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let stat = String::from_utf8(output.stdout).unwrap();
        // After the command name: state, ppid, pgrp, session, tty_nr.
        let fields: Vec<_> = stat.rsplit_once(") ").unwrap().1.split(' ').collect();
        assert_eq!(fields[3], pid, "the child leads its own session");
        assert_ne!(fields[4], "0", "the child has a controlling terminal");
    }
}
