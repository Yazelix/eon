//! Native process and FIFO operations shared by the integration-test targets.
use std::io;

#[cfg(target_os = "macos")]
pub fn bsd_info(pid: i32, include_zombies: bool) -> io::Result<libc::proc_bsdinfo> {
    let mut info = libc::proc_bsdinfo {
        pbi_flags: 0,
        pbi_status: 0,
        pbi_xstatus: 0,
        pbi_pid: 0,
        pbi_ppid: 0,
        pbi_uid: 0,
        pbi_gid: 0,
        pbi_ruid: 0,
        pbi_rgid: 0,
        pbi_svuid: 0,
        pbi_svgid: 0,
        rfu_1: 0,
        pbi_comm: [0; libc::MAXCOMLEN],
        pbi_name: [0; 32],
        pbi_nfiles: 0,
        pbi_pgid: 0,
        pbi_pjobc: 0,
        e_tdev: 0,
        e_tpgid: 0,
        pbi_nice: 0,
        pbi_start_tvsec: 0,
        pbi_start_tvusec: 0,
    };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // SAFETY: info provides size writable bytes for the whole call; consumers
    // receive it only if libproc returns the complete record.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            u64::from(include_zombies),
            (&raw mut info).cast(),
            size,
        )
    };
    if read == size {
        Ok(info)
    } else if read < 0 {
        Err(io::Error::last_os_error())
    } else {
        Err(io::Error::other("incomplete proc_pidinfo record"))
    }
}

// Used only by the lifecycle target.
#[allow(dead_code)]
/// CPU seconds of the children this process has reaped. Under nextest, which
/// runs one test per process, those are the test's own Orbit with its worker
/// and shell.
pub fn reaped_children_cpu() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: usage is a valid rusage for getrusage to fill, and stays zeroed
    // if it fails.
    let usage = unsafe {
        libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr());
        usage.assume_init()
    };
    let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}

// Used only by the real_pty target.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub fn seize(child: i32) -> io::Result<()> {
    // SAFETY: PTRACE_SEIZE targets only the test-owned child; address/data
    // are pointer-typed nulls, no Rust memory is accessed or pointer retained.
    let result = unsafe {
        libc::ptrace(
            libc::PTRACE_SEIZE,
            child,
            std::ptr::null_mut::<libc::c_void>(),
            std::ptr::null_mut::<libc::c_void>(),
        )
    };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// Used only by the real_pty target.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn make_fifo(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: path is live and NUL-terminated; mkfifo retains no pointer.
    // Pinned rustix 1.1.4 has no mknodat/mkfifoat on Darwin; revisit on upgrade.
    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
