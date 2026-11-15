//! Capture through a temporary index, then export verified raw blobs without checkout filters.

use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub(super) struct Source {
    _temporary: tempfile::TempDir,
    pub directory: PathBuf,
    pub tree: String,
    pub base: String,
    pub cwd: String,
}

fn git_command(root: &Path, index: Option<&Path>, args: &[&OsStr]) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"GIT_") {
            command.env_remove(key);
        }
    }
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .stdin(Stdio::null());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command
}

fn git(root: &Path, index: Option<&Path>, args: &[&OsStr]) -> Result<Vec<u8>, String> {
    let output = git_command(root, index, args)
        .output()
        .map_err(|_| "git-unavailable")?;
    if !output.status.success() {
        return Err("tree-not-exportable".into());
    }
    Ok(output.stdout)
}

fn verify_blob(root: &Path, data: &[u8], expected: &str) -> Result<(), String> {
    let mut child = git_command(
        root,
        None,
        &[
            "hash-object".as_ref(),
            "--no-filters".as_ref(),
            "--stdin".as_ref(),
        ],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|_| "git-unavailable")?;
    let written = child.stdin.take().unwrap().write_all(data);
    let output = child.wait_with_output().map_err(|_| "source-blob-verify")?;
    if written.is_err() || !output.status.success() || text(output.stdout)? != expected {
        return Err("source-blob-verify".into());
    }
    Ok(())
}

fn text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes)
        .map(|s| s.trim_end_matches('\n').to_owned())
        .map_err(|_| "tree-not-exportable".into())
}

impl Source {
    pub fn lock_key(&self) -> Result<String, String> {
        let mut hash = Sha256::new();
        for name in ["Cargo.lock", "flake.lock"] {
            hash.update(name.as_bytes());
            hash.update([0]);
            hash.update(fs::read(self.directory.join(name)).map_err(|_| "lockfiles-required")?);
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn capture(cwd: &Path) -> Result<Self, String> {
        let root = PathBuf::from(text(git(
            cwd,
            None,
            &["rev-parse".as_ref(), "--show-toplevel".as_ref()],
        )?)?);
        if !git(
            &root,
            None,
            &[
                "ls-files".as_ref(),
                "--others".as_ref(),
                "--exclude-standard".as_ref(),
                "-z".as_ref(),
            ],
        )?
        .is_empty()
        {
            return Err("untracked-files-not-proven".into());
        }
        let temporary = tempfile::Builder::new()
            .prefix("eon-boat-")
            .tempdir()
            .map_err(|_| "source-tempdir")?;
        let index = temporary.path().join("index");
        let actual_index = text(git(
            &root,
            None,
            &[
                "rev-parse".as_ref(),
                "--git-path".as_ref(),
                "index".as_ref(),
            ],
        )?)?;
        fs::copy(root.join(actual_index), &index).map_err(|_| "source-index")?;
        if !git(
            &root,
            Some(&index),
            &["ls-files".as_ref(), "--unmerged".as_ref(), "-z".as_ref()],
        )?
        .is_empty()
        {
            return Err("unmerged-source-index".into());
        }
        git(
            &root,
            Some(&index),
            &["add".as_ref(), "-u".as_ref(), "--".as_ref(), ".".as_ref()],
        )?;
        let tree = text(git(&root, Some(&index), &["write-tree".as_ref()])?)?;
        let entries = git(
            &root,
            Some(&index),
            &["ls-files".as_ref(), "--stage".as_ref(), "-z".as_ref()],
        )?;
        let directory = temporary.path().join("eon");
        fs::create_dir(&directory).map_err(|_| "source-directory")?;
        let mut links = Vec::new();
        for entry in entries
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            if entry.starts_with(b"160000 ") {
                return Err("gitlinks-not-exportable".into());
            }
            let tab = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or("source-index")?;
            let fields = std::str::from_utf8(&entry[..tab])
                .map_err(|_| "source-index")?
                .split(' ')
                .collect::<Vec<_>>();
            let [mode, oid, "0"] = fields.as_slice() else {
                return Err("source-index".into());
            };
            if !matches!(*mode, "100644" | "100755" | "120000")
                || !matches!(oid.len(), 40 | 64)
                || !oid.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("source-index".into());
            }
            let relative = Path::new(OsStr::from_bytes(&entry[tab + 1..]));
            if !relative
                .components()
                .all(|part| matches!(part, Component::Normal(name) if name != ".git"))
            {
                return Err("source-index-path".into());
            }
            let data = git(
                &root,
                None,
                &["cat-file".as_ref(), "blob".as_ref(), oid.as_ref()],
            )?;
            verify_blob(&root, &data, oid)?;
            let path = directory.join(relative);
            fs::create_dir_all(path.parent().ok_or("source-index-path")?)
                .map_err(|_| "source-directory")?;
            if *mode == "120000" {
                symlink(OsStr::from_bytes(&data), &path).map_err(|_| "source-symlink")?;
                if fs::read_link(&path)
                    .map_err(|_| "source-symlink")?
                    .as_os_str()
                    .as_bytes()
                    != data
                {
                    return Err("source-blob-verify".into());
                }
                links.push(path);
            } else {
                let permissions = if *mode == "100755" { 0o755 } else { 0o644 };
                fs::write(&path, &data).map_err(|_| "source-blob-write")?;
                fs::set_permissions(&path, fs::Permissions::from_mode(permissions))
                    .map_err(|_| "source-blob-write")?;
                if fs::read(&path).map_err(|_| "source-blob-verify")? != data
                    || fs::metadata(&path)
                        .map_err(|_| "source-blob-verify")?
                        .permissions()
                        .mode()
                        & 0o777
                        != permissions
                {
                    return Err("source-blob-verify".into());
                }
            }
        }
        for path in links {
            let target = fs::canonicalize(path).map_err(|_| "source-symlink")?;
            if !target.starts_with(&directory) {
                return Err("escaping-source-symlink".into());
            }
        }
        let relative = cwd.strip_prefix(&root).map_err(|_| "source-cwd")?;
        let cwd = relative.to_str().ok_or("source-cwd-not-utf8")?.to_owned();
        let base = text(git(&root, None, &["rev-parse".as_ref(), "HEAD".as_ref()])?)?;
        Ok(Self {
            _temporary: temporary,
            directory,
            tree,
            base,
            cwd,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_export_never_runs_a_local_smudge_filter_or_uploads_its_plaintext() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let secret = tempfile::NamedTempFile::new().unwrap();
        fs::write(secret.path(), "PRIVATE-ABSENT-FROM-TREE").unwrap();
        let run = |args: &[&str]| {
            git(
                root.path(),
                None,
                &args.iter().map(OsStr::new).collect::<Vec<_>>(),
            )
            .unwrap()
        };
        run(&["init", "-q"]);
        fs::write(
            root.path().join(".gitattributes"),
            "encrypted filter=decrypt\n",
        )
        .unwrap();
        fs::write(root.path().join("encrypted"), "ciphertext").unwrap();
        fs::write(root.path().join("executable"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            root.path().join("executable"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("encrypted", root.path().join("link")).unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=Build",
            "-c",
            "user.email=build@invalid",
            "commit",
            "-qm",
            "source",
        ]);
        let filter = format!(
            "cat '{}'",
            secret.path().display().to_string().replace('\'', "'\\''")
        );
        run(&["config", "filter.decrypt.smudge", &filter]);
        let source = Source::capture(root.path()).unwrap();
        assert_eq!(
            fs::read(source.directory.join("encrypted")).unwrap(),
            b"ciphertext"
        );
        assert_eq!(
            fs::read_link(source.directory.join("link")).unwrap(),
            Path::new("encrypted")
        );
        assert_eq!(
            fs::metadata(source.directory.join("executable"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert!(!source.directory.join(".git").exists());
    }

    #[test]
    fn export_proves_dirty_tracked_files_without_gitdir_or_untracked_secrets() {
        let root = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            git(
                root.path(),
                None,
                &args.iter().map(OsStr::new).collect::<Vec<_>>(),
            )
            .unwrap()
        };
        run(&["init", "-q"]);
        fs::write(root.path().join(".gitignore"), "secret\n").unwrap();
        fs::write(root.path().join("tracked"), "old").unwrap();
        fs::write(root.path().join("deleted"), "remove").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=Build",
            "-c",
            "user.email=build@invalid",
            "commit",
            "-qm",
            "source",
        ]);
        fs::write(root.path().join("tracked"), "dirty").unwrap();
        fs::remove_file(root.path().join("deleted")).unwrap();
        fs::write(root.path().join("added"), "new").unwrap();
        run(&["add", "added"]);
        fs::write(root.path().join("secret"), "DO-NOT-TRANSFER").unwrap();
        let before = fs::read(root.path().join(".git/index")).unwrap();
        let source = Source::capture(root.path()).unwrap();
        assert_eq!(
            fs::read(source.directory.join("tracked")).unwrap(),
            b"dirty"
        );
        assert_eq!(fs::read(source.directory.join("added")).unwrap(), b"new");
        assert!(!source.directory.join("deleted").exists());
        assert!(!source.directory.join(".git").exists());
        assert!(!source.directory.join("secret").exists());
        assert_eq!(fs::read(root.path().join(".git/index")).unwrap(), before);
        fs::write(root.path().join("unstaged"), "unproven").unwrap();
        assert!(Source::capture(root.path()).is_err());
    }
}
