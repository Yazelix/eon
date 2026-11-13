//! Linux directory-handle operations. Names passed here are single components.
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};

pub(super) struct Dir(File);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub blocks: u64,
    pub mtime: i64,
    pub nanos: i64,
}

impl Stat {
    pub fn is(&self, kind: u32) -> bool {
        self.mode & libc::S_IFMT == kind
    }
    pub fn identity(&self, other: &Self) -> bool {
        (self.dev, self.ino, self.mode & libc::S_IFMT)
            == (other.dev, other.ino, other.mode & libc::S_IFMT)
    }
}

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

fn component(name: &OsStr) -> io::Result<CString> {
    if name.is_empty() || name.as_bytes().contains(&b'/') || name == ".." {
        return Err(invalid("invalid directory component"));
    }
    CString::new(name.as_bytes()).map_err(|_| invalid("NUL in name"))
}

fn file(fd: i32) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // The successful syscall transfers this fresh descriptor to File.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

pub(super) fn ancestors_match(path: &Path, protected: &BTreeSet<(u64, u64)>) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    // Inspection follows /proc/PID/cwd and crosses mount roots to recognize bind aliases.
    let mut dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)?;
    loop {
        let stat = dir.metadata()?;
        let id = (stat.dev(), stat.ino());
        if protected.contains(&id) {
            return Ok(true);
        }
        let up = file(unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c"..".as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        let stat = up.metadata()?;
        if (stat.dev(), stat.ino()) == id {
            return Ok(false);
        }
        dir = up;
    }
}

impl Dir {
    pub fn absolute(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid("directory must be absolute"));
        }
        let mut dir = Self(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")?,
        );
        for part in path.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name) => {
                    dir = Self(dir.open(name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?);
                }
                _ => return Err(invalid("non-normal directory path")),
            }
        }
        Ok(dir)
    }

    fn open(&self, name: &OsStr, flags: i32, mode: u32) -> io::Result<File> {
        let name = component(name)?;
        file(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                mode,
            )
        })
    }

    pub fn child(&self, name: &OsStr) -> io::Result<Self> {
        let name = component(name)?;
        let mut how: libc::open_how = unsafe { std::mem::zeroed() };
        how.flags =
            (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64;
        // O_NOFOLLOW alone permits bind mounts into unrelated evidence trees.
        how.resolve = libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_XDEV;
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                self.0.as_raw_fd(),
                name.as_ptr(),
                &how,
                std::mem::size_of::<libc::open_how>(),
            )
        };
        file(fd as i32).map(Self)
    }

    pub fn stat(&self, name: &OsStr) -> io::Result<Stat> {
        let name = component(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        Ok(Stat {
            dev: stat.st_dev,
            ino: stat.st_ino,
            mode: stat.st_mode,
            blocks: stat.st_blocks.max(0) as u64,
            mtime: stat.st_mtime,
            nanos: stat.st_mtime_nsec,
        })
    }

    pub fn names(&self) -> io::Result<Vec<OsString>> {
        let fd = self.open(OsStr::new("."), libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        use std::os::fd::IntoRawFd;
        let fd = fd.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(error);
        }
        let mut names = Vec::new();
        let result = loop {
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let errno = unsafe { *libc::__errno_location() };
                break if errno == 0 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(errno))
                };
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                names.push(OsString::from_vec(name.to_vec()));
            }
        };
        let closed = unsafe { libc::closedir(stream) };
        result?;
        if closed != 0 {
            return Err(io::Error::last_os_error());
        }
        names.sort();
        Ok(names)
    }

    pub fn read(&self, name: &OsStr, expected: &Stat) -> io::Result<Vec<u8>> {
        if !expected.is(libc::S_IFREG) {
            return Err(invalid("not a regular file"));
        }
        let fd = self.open(name, libc::O_RDONLY, 0)?;
        let meta = fd.metadata()?;
        use std::os::unix::fs::MetadataExt;
        if (meta.dev(), meta.ino(), meta.mode()) != (expected.dev, expected.ino, expected.mode) {
            return Err(invalid("file changed while opening"));
        }
        let mut bytes = Vec::new();
        fd.take(65_537).read_to_end(&mut bytes)?;
        if bytes.len() > 65_536 || self.stat(name)? != *expected {
            return Err(invalid("file changed or exceeds 64 KiB"));
        }
        Ok(bytes)
    }

    pub fn link(&self, name: &OsStr) -> io::Result<OsString> {
        let name = component(name)?;
        let mut bytes = vec![0; 65_536];
        let n = unsafe {
            libc::readlinkat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n as usize == bytes.len() {
            return Err(invalid("symlink text too long"));
        }
        bytes.truncate(n as usize);
        Ok(OsString::from_vec(bytes))
    }

    pub fn unlink(&self, name: &OsStr, directory: bool) -> io::Result<()> {
        event(if directory { "rmdir" } else { "unlink" })?;
        let name = component(name)?;
        if unsafe {
            libc::unlinkat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn check_write(&self) -> io::Result<()> {
        if unsafe {
            libc::faccessat(
                self.0.as_raw_fd(),
                c".".as_ptr(),
                libc::W_OK | libc::X_OK,
                libc::AT_EACCESS,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn writable(&self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let mode = self.0.metadata()?.mode();
        if mode & 0o700 != 0o700 && unsafe { libc::fchmod(self.0.as_raw_fd(), mode | 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(test))]
pub(super) fn event(_: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
type Hook = Box<dyn FnMut(&str) -> io::Result<()>>;

#[cfg(test)]
thread_local! {
    pub(super) static HOOK: std::cell::RefCell<Option<Hook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn event(name: &str) -> io::Result<()> {
    HOOK.with_borrow_mut(|hook| match hook {
        Some(hook) => hook(name),
        None => Ok(()),
    })
}
