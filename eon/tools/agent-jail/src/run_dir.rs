//! Host-side reads of a run directory. The jail wrote into parts of it, so
//! every host reader opens paths through [`open_inside`]: one component at
//! a time with `O_NOFOLLOW`, refusing links, `..` and absolute paths.

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
}
