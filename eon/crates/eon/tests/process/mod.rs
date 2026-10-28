//! What the kernel reports about the test's processes, in Linux's `/proc`
//! formats on every platform so assertions read the same bytes.

#[cfg(target_os = "linux")]
use std::{fs, path::Path};

/// Whether `pid` still names a process, an unreaped zombie included.
#[cfg(target_os = "linux")]
pub fn exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// This process's start identity, in the unit Eon Runtime compares with
/// Orbit's `process_start`.
#[cfg(target_os = "linux")]
pub fn own_start_identity() -> u64 {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap()
}

/// The environment `pid` started with: each entry ends in a NUL.
#[cfg(target_os = "linux")]
pub fn environ(pid: u32) -> Vec<u8> {
    fs::read(format!("/proc/{pid}/environ")).unwrap()
}

/// The arguments `pid` started with: each ends in a NUL.
#[cfg(target_os = "linux")]
pub fn cmdline(pid: u32) -> Vec<u8> {
    fs::read(format!("/proc/{pid}/cmdline")).unwrap()
}

#[cfg(target_os = "macos")]
fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // A nonzero argument makes libproc report zombies too.
    // SAFETY: proc_pidinfo fills at most `size` bytes of the zeroed record.
    let read = unsafe {
        libc::proc_pidinfo(
            libc::pid_t::try_from(pid).unwrap(),
            libc::PROC_PIDTBSDINFO,
            1,
            (&raw mut info).cast(),
            size,
        )
    };
    if read <= 0 {
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        return None;
    }
    assert_eq!(read, size);
    Some(info)
}

/// Whether `pid` still names a process, an unreaped zombie included.
#[cfg(target_os = "macos")]
pub fn exists(pid: u32) -> bool {
    bsd_info(pid).is_some()
}

/// This process's start identity, in the unit Eon Runtime compares with
/// Orbit's `process_start`: microseconds, as libproc reports it.
#[cfg(target_os = "macos")]
pub fn own_start_identity() -> u64 {
    let info = bsd_info(std::process::id()).unwrap();
    info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec
}

/// The arguments and then the environment `pid` started with, each entry
/// ending in a NUL, from `KERN_PROCARGS2`: the argument count, the
/// executable path, NUL padding, the arguments, then the environment up to an
/// empty string.
#[cfg(target_os = "macos")]
fn procargs(pid: u32) -> (Vec<u8>, Vec<u8>) {
    let mut size = libc::c_int::default();
    let mut length = std::mem::size_of_val(&size);
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl writes one c_int into `size`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut size).cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    let mut buffer = vec![0_u8; usize::try_from(size).unwrap()];
    let mut length = buffer.len();
    let mut mib = [
        libc::CTL_KERN,
        libc::KERN_PROCARGS2,
        libc::c_int::try_from(pid).unwrap(),
    ];
    // SAFETY: sysctl writes at most `length` bytes into `buffer`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    let (count, strings) = buffer[..length].split_at(std::mem::size_of::<libc::c_int>());
    let count = usize::try_from(libc::c_int::from_ne_bytes(count.try_into().unwrap())).unwrap();
    let executable = strings.iter().position(|byte| *byte == 0).unwrap();
    let mut strings = strings[executable..]
        .split(|byte| *byte == 0)
        .skip_while(|entry| entry.is_empty());
    let arguments = terminated(strings.by_ref().take(count));
    let environment = terminated(strings.take_while(|entry| !entry.is_empty()));
    (arguments, environment)
}

#[cfg(target_os = "macos")]
fn terminated<'a>(entries: impl Iterator<Item = &'a [u8]>) -> Vec<u8> {
    entries
        .flat_map(|entry| entry.iter().copied().chain([0]))
        .collect()
}

/// The environment `pid` started with: each entry ends in a NUL.
#[cfg(target_os = "macos")]
pub fn environ(pid: u32) -> Vec<u8> {
    procargs(pid).1
}

/// The arguments `pid` started with: each ends in a NUL.
#[cfg(target_os = "macos")]
pub fn cmdline(pid: u32) -> Vec<u8> {
    procargs(pid).0
}
