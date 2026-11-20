//! A content hash of a directory tree (`agent-bench tree-hash DIR`).
//!
//! SHA-256 over the sorted lines `<kind> <x-bit> <size> <content> <path>\n`, one per entry
//! below `DIR` (the root itself has no line):
//!
//! - kind `d` (directory), `f` (regular file) or `l` (symlink);
//! - x-bit `x` when a file's owner execute bit is set, else `-` (always `-` for `d` and `l`);
//! - size: the file's length, the link target's length in bytes, 0 for a directory;
//! - content: the file's SHA-256, the link target's bytes in hex, `-` for a directory;
//! - path: relative to `DIR`, `/`-separated.
//!
//! Links are never followed, `.git` is included like any other name, and times, owners and
//! other mode bits are not hashed. Sockets, FIFOs and devices are refused, as is a name with
//! a newline (it would forge a line).

use crate::workspace::walk;
use crate::{Error, Result, fail, hex};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The tree hash of `dir`, in lowercase hex.
pub fn hash(dir: &Path) -> Result<String> {
    let meta = fs::symlink_metadata(dir).map_err(|error| context(dir, error))?;
    if !meta.is_dir() {
        fail!("{} is not a directory", dir.display());
    }
    let mut lines: Vec<Vec<u8>> = Vec::new();
    walk(dir, true, &mut |path, meta| {
        let relative = path.strip_prefix(dir).expect("walk stays under its root");
        if relative.as_os_str().is_empty() {
            return Ok(());
        }
        let name = relative.as_os_str().as_bytes();
        if name.contains(&b'\n') {
            fail!("{}: a name holds a newline", path.display());
        }
        let file_type = meta.file_type();
        let (kind, x_bit, size, content) = if file_type.is_dir() {
            ("d", "-", 0, "-".to_owned())
        } else if file_type.is_file() {
            let (size, digest) = file_digest(path)?;
            let x_bit = if meta.permissions().mode() & 0o100 != 0 {
                "x"
            } else {
                "-"
            };
            ("f", x_bit, size, digest)
        } else if file_type.is_symlink() {
            let target = fs::read_link(path).map_err(|error| context(path, error))?;
            let target = target.as_os_str().as_bytes();
            ("l", "-", target.len() as u64, hex(target))
        } else {
            fail!(
                "{}: not a directory, regular file or symlink",
                path.display()
            );
        };
        let mut line = format!("{kind} {x_bit} {size} {content} ").into_bytes();
        line.extend_from_slice(name);
        line.push(b'\n');
        lines.push(line);
        Ok(())
    })?;
    lines.sort();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line);
    }
    Ok(hex(&hasher.finalize()))
}

/// Streams a file through SHA-256, returning the bytes read and the digest.
fn file_digest(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path).map_err(|error| context(path, error))?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file, &mut hasher).map_err(|error| context(path, error))?;
    Ok((size, hex(&hasher.finalize())))
}

fn context(path: &Path, error: std::io::Error) -> Error {
    Error(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::{AtFlags, CWD, FileType, Mode, Timespec, Timestamps, mknodat, utimensat};
    use std::os::unix::fs::symlink;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("a/b/file"), "content\n").unwrap();
        fs::write(root.join(".git/HEAD"), "ref\n").unwrap();
        fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
        symlink("a/b/file", root.join("link")).unwrap();
        dir
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn known_lines_give_the_documented_hash() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        fs::write(dir.path().join("d/f"), "x").unwrap();
        set_mode(&dir.path().join("d/f"), 0o755);
        symlink("t", dir.path().join("l")).unwrap();
        let x = crate::sha256_hex(b"x");
        let expected =
            crate::sha256_hex(format!("d - 0 - d\nf x 1 {x} d/f\nl - 1 74 l\n").as_bytes());
        assert_eq!(hash(dir.path()).unwrap(), expected);
    }

    #[test]
    fn mtimes_do_not_change_the_hash() {
        let dir = tree();
        let before = hash(dir.path()).unwrap();
        let at = Timespec {
            tv_sec: 1_000_000,
            tv_nsec: 5,
        };
        let times = Timestamps {
            last_access: at,
            last_modification: at,
        };
        for name in ["a", "a/b/file", ".git/HEAD", "link"] {
            utimensat(
                CWD,
                dir.path().join(name),
                &times,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .unwrap();
        }
        // Group and other bits are not hashed either.
        set_mode(&dir.path().join("a/b/file"), 0o600);
        assert_eq!(hash(dir.path()).unwrap(), before);
    }

    #[test]
    fn content_mode_link_and_name_changes_change_the_hash() {
        type Edit = fn(&Path);
        let edits: Vec<(&str, Edit)> = vec![
            ("content", |root| {
                fs::write(root.join("a/b/file"), "other\n").unwrap()
            }),
            ("content under .git", |root| {
                fs::write(root.join(".git/HEAD"), "other\n").unwrap()
            }),
            ("exec bit", |root| set_mode(&root.join("run.sh"), 0o755)),
            ("link target", |root| {
                fs::remove_file(root.join("link")).unwrap();
                symlink("run.sh", root.join("link")).unwrap();
            }),
            ("rename", |root| {
                fs::rename(root.join("a/b/file"), root.join("a/b/renamed")).unwrap()
            }),
            ("empty directory", |root| {
                fs::create_dir(root.join("a/empty")).unwrap()
            }),
        ];
        let base = hash(tree().path()).unwrap();
        for (name, edit) in edits {
            let dir = tree();
            edit(dir.path());
            assert_ne!(hash(dir.path()).unwrap(), base, "{name}");
        }
    }

    #[test]
    fn a_fifo_is_refused() {
        let dir = tree();
        let fifo = dir.path().join("a/pipe");
        mknodat(CWD, &fifo, FileType::Fifo, Mode::RUSR | Mode::WUSR, 0).unwrap();
        let error = hash(dir.path()).unwrap_err().0;
        assert!(error.contains("a/pipe"), "{error}");
    }

    #[test]
    fn the_root_must_be_a_directory() {
        let dir = tree();
        assert!(hash(&dir.path().join("link")).is_err());
        assert!(hash(&dir.path().join("missing")).is_err());
    }
}
