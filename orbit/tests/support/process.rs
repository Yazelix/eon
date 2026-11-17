//! Darwin records shared by the two integration-test targets.
use std::io;

pub fn bsd_info(pid: i32, include_zombies: bool) -> io::Result<libc::proc_bsdinfo> {
    // SAFETY: proc_bsdinfo contains only integers and byte arrays, valid at zero.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
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
