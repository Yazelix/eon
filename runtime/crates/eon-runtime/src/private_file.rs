//! Private file I/O; callers serialize publication with their lifecycle lock.
use crate::{
    control::{SocketIdentity, socket_identity_from},
    supervisor::{effective_uid, validate_private_directory},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(super) fn object(path: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(socket_identity_from(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

pub(super) fn revalidate(path: &Path, expected: Option<SocketIdentity>) -> Result<(), String> {
    if object(path)? != expected {
        return Err(format!(
            "{} changed before publication or cleanup",
            path.display()
        ));
    }
    Ok(())
}

fn revalidate_directory(path: &Path, expected: Option<SocketIdentity>) -> Result<(), String> {
    validate_private_directory(path)?;
    if object(path)?.map(|id| (id.0, id.1)) != expected.map(|id| (id.0, id.1)) {
        return Err(format!("{} directory was replaced", path.display()));
    }
    Ok(())
}

fn validate(file: &File, path: &Path, uid: u32) -> Result<SocketIdentity, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o600 {
        return Err(format!(
            "{} must be an owned mode-0600 regular file",
            path.display()
        ));
    }
    let identity = socket_identity_from(&metadata);
    revalidate(path, Some(identity))?;
    Ok(identity)
}

fn open(path: &Path) -> Result<Option<File>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot open {}: {error}", path.display())),
    };
    validate(&file, path, effective_uid())?;
    Ok(Some(file))
}

pub(super) fn read(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>, String> {
    let directory = path.parent().ok_or("private file has no parent")?;
    validate_private_directory(directory)?;
    let parent = object(directory)?;
    let Some(file) = open(path)? else {
        return Ok(None);
    };
    let expected = validate(&file, path, effective_uid())?;
    let mut bytes = Vec::new();
    (&file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() > maximum {
        return Err(format!("{} exceeds {maximum} bytes", path.display()));
    }
    revalidate(path, Some(expected))?;
    validate(&file, path, effective_uid())?;
    revalidate_directory(directory, parent)?;
    Ok(Some(bytes))
}

pub(super) fn publish(path: &Path, bytes: &[u8]) -> Result<(), String> {
    publish_with(path, bytes, |_| Ok(())).map_err(PublishError::into_detail)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Write,
    FileSync,
    Rename,
    DirectorySync,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PublishError {
    Failed(String),
    Uncertain(String),
    Cancelled,
}

impl From<String> for PublishError {
    fn from(detail: String) -> Self {
        Self::Failed(detail)
    }
}

impl PublishError {
    fn into_detail(self) -> String {
        match self {
            Self::Failed(detail) | Self::Uncertain(detail) => detail,
            Self::Cancelled => "publication cancelled before rename".into(),
        }
    }
}

pub(super) fn publish_with(
    path: &Path,
    bytes: &[u8],
    mut before: impl FnMut(Stage) -> Result<(), PublishError>,
) -> Result<(), PublishError> {
    let directory = path
        .parent()
        .ok_or_else(|| "private file has no parent".to_owned())?;
    validate_private_directory(directory)?;
    let parent = object(directory)?;
    let directory_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory)
        .map_err(|error| format!("cannot open {}: {error}", directory.display()))?;
    let existing = open(path)?;
    let expected = existing
        .as_ref()
        .map(|file| validate(file, path, effective_uid()))
        .transpose()?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(
        path.file_name()
            .ok_or_else(|| "private file has no name".to_owned())?,
    );
    temporary_name.push(".tmp");
    let temporary = directory.join(temporary_name);
    // The lifecycle lock settles this publisher's fixed temp after a crash.
    if let Some(stale) = open(&temporary)? {
        let identity = validate(&stale, &temporary, effective_uid())?;
        revalidate(&temporary, Some(identity))?;
        fs::remove_file(&temporary)
            .map_err(|error| format!("cannot settle {}: {error}", temporary.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
    validate(&file, &temporary, effective_uid())?;
    let mut published = false;
    let result = (|| {
        before(Stage::Write)?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        before(Stage::FileSync)?;
        file.sync_all()
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        revalidate_directory(directory, parent)?;
        revalidate(path, expected)?;
        validate(&file, &temporary, effective_uid())?;
        before(Stage::Rename)?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot publish {}: {error}", path.display()))?;
        published = true;
        before(Stage::DirectorySync)
            .map_err(PublishError::into_detail)
            .and_then(|()| directory_file.sync_all().map_err(|error| error.to_string()))
            .map_err(|detail| {
                PublishError::Uncertain(format!(
                    "{} publication is uncertain after rename: {detail}",
                    path.display()
                ))
            })
    })();
    if !published && validate(&file, &temporary, effective_uid()).is_ok() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{prepare_runtime, temporary_directory};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn publication_settles_its_crashed_temp_and_replaces_whole_record() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        prepare_runtime(&root).unwrap();
        let path = root.join("identity");
        publish(&path, b"old").unwrap();
        let temp = root.join(".identity.tmp");
        fs::create_dir(&temp).unwrap();
        assert!(publish(&path, b"must not publish").is_err());
        assert_eq!(read(&path, 100).unwrap().unwrap(), b"old");
        assert!(temp.is_dir());
        fs::remove_dir(&temp).unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .unwrap();
        file.write_all(b"interrupted").unwrap();
        file.sync_all().unwrap();
        publish(&path, b"new whole record").unwrap();
        assert_eq!(read(&path, 100).unwrap().unwrap(), b"new whole record");
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let error = publish_with(&path, b"committed but uncertain", |stage| {
            if stage == Stage::DirectorySync {
                return Err("injected sync failure".to_owned().into());
            }
            Ok(())
        })
        .unwrap_err()
        .into_detail();
        assert!(error.contains("uncertain after rename"), "{error}");
        assert_eq!(
            read(&path, 100).unwrap().unwrap(),
            b"committed but uncertain"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ownership_and_replacement_validation_prevents_unlinking_other_objects() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        prepare_runtime(&root).unwrap();
        let path = root.join("identity");
        publish(&path, b"old").unwrap();
        let opened = open(&path).unwrap().unwrap();
        let expected = object(&path).unwrap();
        assert!(validate(&opened, &path, effective_uid().wrapping_add(1)).is_err());
        fs::rename(&path, root.join("old")).unwrap();
        publish(&path, b"replacement").unwrap();
        assert!(validate(&opened, &path, effective_uid()).is_err());
        assert!(revalidate(&path, expected).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        fs::remove_dir_all(root).unwrap();
    }
}
