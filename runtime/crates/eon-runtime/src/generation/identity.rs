//! Format-1 lifetime/build identity and its non-authoritative launcher projection.
use super::valid_generation;
use crate::{
    private_file,
    supervisor::{effective_uid, validate_private_directory},
};
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, symlink},
    },
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 4210;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub generation: String,
    pub created_build: String,
    pub active_build: String,
    pub active_launcher: Option<PathBuf>,
}

impl Identity {
    pub(crate) fn initial(build: &str, active_launcher: Option<PathBuf>) -> Self {
        Self {
            generation: build.into(),
            created_build: build.into(),
            active_build: build.into(),
            active_launcher,
        }
    }

    pub(crate) fn require_build(&self, build: &str) -> Result<(), String> {
        if self.generation != build || self.active_build != build {
            return Err(format!(
                "identity conflicts with invoked build {build}: generation {}, active build {}",
                self.generation, self.active_build
            ));
        }
        Ok(())
    }

    fn validate(&self, runtime: &Path) -> Result<(), String> {
        if [&self.generation, &self.created_build, &self.active_build]
            .into_iter()
            .any(|id| !valid_generation(id))
        {
            return Err("invalid identity in private record".into());
        }
        if runtime.file_name().and_then(|name| name.to_str()) != Some(self.generation.as_str()) {
            return Err("private identity does not match generation directory".into());
        }
        if let Some(path) = &self.active_launcher {
            let bytes = path.as_os_str().as_bytes();
            if bytes.is_empty() || bytes.len() > 4096 || !path.is_absolute() || bytes.contains(&0) {
                return Err(
                    "private identity launcher must be an absolute 1..4096-byte path without NUL"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn decode(runtime: &Path, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 112 || bytes.len() > MAX_BYTES || &bytes[..6] != b"EONI\x01\x00" {
            return Err("invalid private identity magic, version or length".into());
        }
        let id = |offset| {
            std::str::from_utf8(&bytes[offset..offset + 35])
                .map(str::to_owned)
                .map_err(|_| "invalid private identity bytes".to_owned())
        };
        let active_launcher = match bytes[111] {
            0 if bytes.len() == 112 => None,
            1 if bytes.len() >= 114 => {
                let length = usize::from(u16::from_le_bytes([bytes[112], bytes[113]]));
                if length == 0 || length > 4096 || bytes.len() != 114 + length {
                    return Err("invalid private identity launcher length".into());
                }
                Some(PathBuf::from(OsString::from_vec(bytes[114..].to_vec())))
            }
            _ => return Err("invalid private identity presence or trailing bytes".into()),
        };
        let record = Self {
            generation: id(6)?,
            created_build: id(41)?,
            active_build: id(76)?,
            active_launcher,
        };
        record.validate(runtime)?;
        Ok(record)
    }

    fn encode(&self, runtime: &Path) -> Result<Vec<u8>, String> {
        self.validate(runtime)?;
        let mut bytes = b"EONI\x01\x00".to_vec();
        for id in [&self.generation, &self.created_build, &self.active_build] {
            bytes.extend_from_slice(id.as_bytes());
        }
        bytes.push(u8::from(self.active_launcher.is_some()));
        if let Some(path) = &self.active_launcher {
            let path = path.as_os_str().as_bytes();
            bytes.extend_from_slice(&(path.len() as u16).to_le_bytes());
            bytes.extend_from_slice(path);
        }
        Ok(bytes)
    }

    /// None is missing; malformed or unsafe existing files are errors.
    pub(crate) fn read(runtime: &Path) -> Result<Option<Self>, String> {
        private_file::read(&runtime.join("identity"), MAX_BYTES)?
            .map(|bytes| Self::decode(runtime, &bytes))
            .transpose()
    }

    /// The caller holds the lifecycle lock and decides whether replacement is allowed.
    pub(crate) fn publish(&self, runtime: &Path) -> Result<(), String> {
        let bytes = self.encode(runtime)?;
        Self::read(runtime)?;
        private_file::publish(&runtime.join("identity"), &bytes)
    }

    /// Only the committed record supplies the projection's path.
    pub(crate) fn project_launcher(&self, runtime: &Path) -> Result<(), String> {
        validate_private_directory(runtime)?;
        let link = runtime.join("eon");
        match fs::symlink_metadata(&link) {
            Ok(metadata) => {
                if !metadata.file_type().is_symlink() || metadata.uid() != effective_uid() {
                    return Err(format!(
                        "{} must be an owned launcher symlink",
                        link.display()
                    ));
                }
                if self
                    .active_launcher
                    .as_ref()
                    .is_some_and(|target| fs::read_link(&link).is_ok_and(|path| path == *target))
                {
                    return Ok(());
                }
                private_file::revalidate(
                    &link,
                    Some(crate::control::socket_identity_from(&metadata)),
                )?;
                fs::remove_file(&link).map_err(|error| {
                    format!("cannot remove projection {}: {error}", link.display())
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect projection {}: {error}",
                    link.display()
                ));
            }
        }
        if let Some(path) = &self.active_launcher {
            symlink(path, &link)
                .map_err(|error| format!("cannot project {}: {error}", link.display()))?;
        }
        Ok(())
    }
}
