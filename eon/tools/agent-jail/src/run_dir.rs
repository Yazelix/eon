//! Host-side handling of directories a jail wrote into. Every host reader
//! opens paths through [`open_inside`]: one component at a time with
//! `O_NOFOLLOW`, refusing links, `..` and absolute paths. [`open_up`] undoes
//! the mode tricks a jailed command can play before a scan or a removal.

use std::fs::File;
use std::path::{Component, Path};

use rustix::fs::{Mode, OFlags, openat};

use crate::Result;

/// Opens `relative` under `root` for reading without following any link.
pub fn open_inside(root: &Path, relative: &Path) -> Result<File> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = rustix::fs::open(root, flags | OFlags::DIRECTORY, Mode::empty())
        .map_err(|err| format!("{}: {err}", root.display()))?;
    let components: Vec<Component> = relative.components().collect();
    let Some((last, parents)) = components.split_last() else {
        return Err(format!("{}: empty path", relative.display()));
    };
    for component in parents {
        let Component::Normal(name) = component else {
            return Err(format!(
                "{}: only plain components are allowed",
                relative.display()
            ));
        };
        directory = openat(&directory, *name, flags | OFlags::DIRECTORY, Mode::empty())
            .map_err(|err| format!("{}: {err}", relative.display()))?;
    }
    let Component::Normal(name) = last else {
        return Err(format!(
            "{}: only plain components are allowed",
            relative.display()
        ));
    };
    let file = openat(&directory, *name, flags, Mode::empty())
        .map_err(|err| format!("{}: {err}", relative.display()))?;
    Ok(File::from(file))
}

/// Reads `relative` under `root` as text, capped at `limit` bytes.
pub fn read_inside(root: &Path, relative: &Path, limit: u64) -> Result<String> {
    use std::io::Read;
    let mut text = String::new();
    open_inside(root, relative)?
        .take(limit)
        .read_to_string(&mut text)
        .map_err(|err| format!("{}: {err}", relative.display()))?;
    Ok(text)
}

/// Adds the owner bits back under `root`, following no link: `u+rwx` on
/// directories (overlayfs leaves `work/work` at mode 000) and `u+r` on
/// files, so a scan can read and a removal can delete what a jailed command
/// locked. Every file there belongs to the invoking user (the jail maps only
/// that uid and has no capabilities). Returns each error, and each special
/// file but an overlay whiteout ([`crate::scan::special_file`]), never
/// stopping at one; an absent `root` has nothing to open.
pub fn open_up(root: &Path) -> Vec<String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mut errors = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if path == root && err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => {
                errors.push(format!("{}: {err}", path.display()));
                continue;
            }
        };
        if let Some(kind) = crate::scan::special_file(metadata.mode(), metadata.rdev()) {
            errors.push(format!("{}: a {kind}", path.display()));
            continue;
        }
        let mode = metadata.permissions().mode();
        let wanted = if metadata.is_dir() {
            mode | 0o700
        } else if metadata.is_file() {
            mode | 0o400
        } else {
            continue;
        };
        if wanted != mode
            && let Err(err) =
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(wanted & 0o7777))
        {
            errors.push(format!("{}: {err}", path.display()));
        }
        if metadata.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => stack.push(entry.path()),
                            Err(err) => errors.push(format!("{}: {err}", path.display())),
                        }
                    }
                }
                Err(err) => errors.push(format!("{}: {err}", path.display())),
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_escapes_are_refused() {
        let root = crate::test_dir("run-dir");
        let outside = crate::test_dir("run-dir-outside");
        std::fs::create_dir(root.join("meta")).unwrap();
        std::fs::write(root.join("meta/ok.txt"), "ok").unwrap();
        std::fs::write(outside.join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), root.join("meta/link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dirlink")).unwrap();
        assert_eq!(
            read_inside(&root, Path::new("meta/ok.txt"), 100).unwrap(),
            "ok"
        );
        for bad in [
            "meta/link",
            "dirlink/secret",
            "../x",
            "/etc/passwd",
            "meta/../meta/ok.txt",
        ] {
            assert!(open_inside(&root, Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn open_up_restores_owner_access_without_following_links() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_dir("open-up");
        let outside = crate::test_dir("open-up-outside");
        std::fs::write(outside.join("keep"), "x").unwrap();
        std::fs::set_permissions(outside.join("keep"), std::fs::Permissions::from_mode(0o200))
            .unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/token"), "x").unwrap();
        std::os::unix::fs::symlink(outside.join("keep"), root.join("a/link")).unwrap();
        let mode = |path: &Path, bits| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap()
        };
        mode(&root.join("a/b/token"), 0o000);
        mode(&root.join("a/b"), 0o000);
        mode(&root.join("a"), 0o500);
        assert_eq!(open_up(&root), Vec::<String>::new());
        assert_eq!(
            std::fs::read_to_string(root.join("a/b/token")).unwrap(),
            "x"
        );
        let bits = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(bits(&root.join("a")), 0o700);
        // The link's target outside is untouched.
        assert_eq!(bits(&outside.join("keep")), 0o200);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(open_up(&root).is_empty());
    }
}
