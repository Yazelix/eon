//! Linux directory-handle operations. Names passed here are single components.
use rustix::fs::{self, AtFlags, Mode, OFlags, ResolveFlags};
use std::collections::BTreeSet;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
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

pub(super) fn ancestors_match(path: &Path, protected: &BTreeSet<(u64, u64)>) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    // Inspection follows /proc/PID/cwd and crosses mount roots to recognize bind aliases.
    let mut dir = File::from(fs::open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    loop {
        let stat = dir.metadata()?;
        let id = (stat.dev(), stat.ino());
        if protected.contains(&id) {
            return Ok(true);
        }
        let up = File::from(fs::openat(
            &dir,
            c"..",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
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
        let mut dir = Self(File::from(fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?));
        for part in path.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name) => {
                    dir = Self(dir.open(name, OFlags::RDONLY | OFlags::DIRECTORY)?);
                }
                _ => return Err(invalid("non-normal directory path")),
            }
        }
        Ok(dir)
    }

    fn open(&self, name: &OsStr, flags: OFlags) -> io::Result<File> {
        let name = component(name)?;
        Ok(File::from(fs::openat(
            &self.0,
            &name,
            flags | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?))
    }

    pub fn child(&self, name: &OsStr) -> io::Result<Self> {
        let name = component(name)?;
        // O_NOFOLLOW alone permits bind mounts into unrelated evidence trees.
        Ok(Self(File::from(fs::openat2(
            &self.0,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )?)))
    }

    pub fn stat(&self, name: &OsStr) -> io::Result<Stat> {
        let name = component(name)?;
        let stat = fs::statat(&self.0, &name, AtFlags::SYMLINK_NOFOLLOW)?;
        Ok(Stat {
            dev: stat.st_dev,
            ino: stat.st_ino,
            mode: stat.st_mode,
            blocks: stat.st_blocks.max(0) as u64,
            mtime: stat.st_mtime,
            nanos: stat.st_mtime_nsec as i64,
        })
    }

    pub fn names(&self) -> io::Result<Vec<OsString>> {
        let fd = self.open(OsStr::new("."), OFlags::RDONLY | OFlags::DIRECTORY)?;
        let mut buffer = [std::mem::MaybeUninit::uninit(); 8192];
        let mut names = Vec::new();
        let result = {
            let mut entries = fs::RawDir::new(&fd, &mut buffer);
            loop {
                match entries.next() {
                    Some(Ok(entry)) => {
                        let name = entry.file_name().to_bytes();
                        if name != b"." && name != b".." {
                            names.push(OsString::from_vec(name.to_vec()));
                        }
                    }
                    // libc readdir treats an unlinked directory as end-of-directory.
                    Some(Err(rustix::io::Errno::NOENT)) | None => break Ok(()),
                    Some(Err(error)) => break Err(io::Error::from(error)),
                }
            }
        };
        let closed = crate::platform::close(fd);
        result?;
        closed?;
        names.sort();
        Ok(names)
    }

    pub fn read(&self, name: &OsStr, expected: &Stat) -> io::Result<Vec<u8>> {
        if !expected.is(libc::S_IFREG) {
            return Err(invalid("not a regular file"));
        }
        let fd = self.open(name, OFlags::RDONLY)?;
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
        let n = fs::readlinkat_raw(&self.0, &name, &mut bytes)?;
        if n == bytes.len() {
            return Err(invalid("symlink text too long"));
        }
        bytes.truncate(n);
        Ok(OsString::from_vec(bytes))
    }

    pub fn unlink(&self, name: &OsStr, directory: bool) -> io::Result<()> {
        event(if directory { "rmdir" } else { "unlink" })?;
        let name = component(name)?;
        fs::unlinkat(
            &self.0,
            &name,
            if directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            },
        )?;
        Ok(())
    }

    pub fn check_write(&self) -> io::Result<()> {
        crate::platform::check_write(&self.0)
    }

    pub fn writable(&self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let mode = self.0.metadata()?.mode();
        if mode & 0o700 != 0o700 {
            fs::fchmod(&self.0, Mode::from_bits_retain(mode | 0o700))?;
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

#[cfg(test)]
mod handle_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn removed_directory_is_an_empty_listing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gone-work");
        std::fs::create_dir(&path).unwrap();
        let dir = Dir::absolute(&path).unwrap();
        std::fs::remove_dir(&path).unwrap();
        assert!(dir.names().unwrap().is_empty());
    }

    #[test]
    fn directory_handles_preserve_flags_names_and_errors() {
        let root = tempfile::tempdir().unwrap();
        let name = OsStr::from_bytes(b"file-\xff");
        std::fs::write(root.path().join(name), b"owned").unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();
        symlink(name, root.path().join("link")).unwrap();
        let dir = Dir::absolute(root.path()).unwrap();
        let flags = fs::fcntl_getfl(&dir.0).unwrap();
        assert!(flags.contains(OFlags::DIRECTORY | OFlags::NONBLOCK));
        assert!(
            rustix::io::fcntl_getfd(&dir.0)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let child = dir.child(OsStr::new("child")).unwrap();
        assert!(
            !fs::fcntl_getfl(&child.0)
                .unwrap()
                .contains(OFlags::NONBLOCK)
        );
        assert!(
            rustix::io::fcntl_getfd(&child.0)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let names = vec![
            OsString::from("child"),
            name.to_owned(),
            OsString::from("link"),
        ];
        assert_eq!(dir.names().unwrap(), names);
        assert_eq!(dir.names().unwrap(), names);
        assert_eq!(dir.link(OsStr::new("link")).unwrap(), name);
        assert_eq!(dir.read(name, &dir.stat(name).unwrap()).unwrap(), b"owned");
        assert_eq!(
            dir.stat(OsStr::new("absent")).unwrap_err().raw_os_error(),
            Some(libc::ENOENT)
        );
        assert_eq!(
            dir.child(OsStr::new("link")).err().unwrap().raw_os_error(),
            Some(libc::ENOTDIR)
        );
        assert!(dir.open(OsStr::new("../file"), OFlags::RDONLY).is_err());
    }
}
