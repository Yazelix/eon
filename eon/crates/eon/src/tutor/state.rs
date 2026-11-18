use super::{LESSONS, Operation};
use rustix::fs::{AtFlags, Mode, OFlags};
use serde_json::{Value, json};
use std::{
    env,
    ffi::CStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 16 * 1024;

#[derive(Default)]
pub(super) struct Progress {
    pub current: usize,
    pub read: [bool; 10],
    pub verified: [bool; 10],
}

impl Progress {
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let invalid = || "Malformed or unsupported tutor progress; file left unchanged".to_owned();
        let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let object = value.as_object().ok_or_else(invalid)?;
        if object.len() != 5 || value["version"] != 1 || value["catalog_version"] != 1 {
            return Err(invalid());
        }
        let current = value["current_lesson"]
            .as_u64()
            .filter(|n| (1..=10).contains(n))
            .ok_or_else(invalid)? as usize
            - 1;
        let marks = |key: &str| -> Result<[bool; 10], String> {
            let mut result = [false; 10];
            for id in value[key].as_array().ok_or_else(invalid)? {
                let index = LESSONS
                    .iter()
                    .position(|lesson| Some(lesson.id) == id.as_str())
                    .ok_or_else(invalid)?;
                if result[index] {
                    return Err(invalid());
                }
                result[index] = true;
            }
            Ok(result)
        };
        let progress = Self {
            current,
            read: marks("read")?,
            verified: marks("verified")?,
        };
        if progress.verified[9] || (0..10).any(|i| progress.verified[i] && !progress.read[i]) {
            return Err(invalid());
        }
        Ok(progress)
    }

    fn encode(&self) -> Vec<u8> {
        let ids = |marks: &[bool; 10]| {
            LESSONS
                .iter()
                .enumerate()
                .filter_map(|(i, lesson)| marks[i].then_some(lesson.id))
                .collect::<Vec<_>>()
        };
        serde_json::to_vec(&json!({
            "version": 1, "catalog_version": 1, "current_lesson": self.current + 1,
            "read": ids(&self.read), "verified": ids(&self.verified),
        }))
        .expect("progress consists only of JSON scalars")
    }

    fn apply(&mut self, operation: Operation) {
        match operation {
            Operation::Reset => *self = Self::default(),
            Operation::Lesson(index) => self.current = index,
            Operation::Next => {
                self.read[self.current] = true;
                self.current = (self.current + 1).min(LESSONS.len() - 1);
            }
            Operation::Current | Operation::List => {}
        }
        self.read[self.current] = true;
    }
}

fn private(file: &File, directory: Option<&Path>) -> Result<(), String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    let valid = if directory.is_some() {
        metadata.is_dir() && metadata.mode() & 0o022 == 0
    } else {
        metadata.is_file() && metadata.mode() & 0o7777 == 0o600 && metadata.nlink() == 1
    };
    if metadata.uid() == rustix::process::geteuid().as_raw() && valid {
        return Ok(());
    }
    Err(match directory {
        Some(path) => format!(
            "Tutor directory {path:?} must be owned by you and not writable by group or others; check ownership and chmod go-w on this directory"
        ),
        None => "Tutor state requires owned mode-0600 regular files with one link".into(),
    })
}

fn open_at(root: &File, name: &CStr, flags: OFlags) -> std::io::Result<File> {
    Ok(File::from(rustix::fs::openat(
        root,
        name,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::RUSR | Mode::WUSR,
    )?))
}

fn identity(root: &File) -> Result<Option<(u64, u64)>, String> {
    match open_at(root, c"tutor-v1.json", OFlags::RDONLY) {
        Ok(file) => {
            private(&file, None)?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            Ok(Some((metadata.dev(), metadata.ino())))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot inspect tutor progress: {error}")),
    }
}

pub(super) fn progress(operation: Operation) -> Result<Progress, String> {
    let absolute = |key| {
        env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let path = absolute("XDG_STATE_HOME")
        .or_else(|| absolute("HOME").map(|p| p.join(".local/state")))
        .ok_or("Tutor needs an absolute XDG_STATE_HOME or HOME")?
        .join("eon");
    let mutate = operation != Operation::List;
    if mutate {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .map_err(|e| format!("Cannot create tutor state directory: {e}"))?;
    }
    let root = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
    {
        Ok(root) => root,
        Err(error) if !mutate && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Progress::default());
        }
        Err(error) => return Err(format!("Cannot open tutor state directory: {error}")),
    };
    private(&root, Some(&path))?;
    let _lock = if mutate {
        let lock = open_at(&root, c"tutor-v1.lock", OFlags::RDWR | OFlags::CREATE)
            .map_err(|e| format!("Cannot lock tutor progress: {e}"))?;
        private(&lock, None)?;
        // Refuse contention rather than hang a shell.
        if lock.try_lock().is_err() {
            return Err("Tutor progress is busy; try again".into());
        }
        Some(lock)
    } else {
        None
    };
    let (mut progress, previous) = match open_at(&root, c"tutor-v1.json", OFlags::RDONLY) {
        Ok(file) => {
            private(&file, None)?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            let identity = (metadata.dev(), metadata.ino());
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err("Tutor progress exceeds its size bound".into());
            }
            (Progress::decode(&bytes)?, Some(identity))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Progress::default(), None),
        Err(error) => return Err(format!("Cannot read tutor progress: {error}")),
    };
    if !mutate {
        return Ok(progress);
    }
    progress.apply(operation);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let name =
        std::ffi::CString::new(format!(".tutor-{}-{nonce}.tmp", std::process::id())).unwrap();
    let mut temporary = open_at(&root, &name, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL)
        .map_err(|e| format!("Cannot prepare tutor progress: {e}"))?;
    let result = (|| {
        temporary
            .write_all(&progress.encode())
            .map_err(|e| e.to_string())?;
        temporary.sync_all().map_err(|e| e.to_string())?;
        private(&root, Some(&path))?;
        let expected = root.metadata().map_err(|e| e.to_string())?;
        let actual = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino())
            || previous != identity(&root)?
        {
            return Err("Tutor state changed before publication".into());
        }
        rustix::fs::renameat(&root, &name, &root, c"tutor-v1.json")
            .map_err(|error| std::io::Error::from(error).to_string())?;
        root.sync_all()
            .map_err(|e| format!("Tutor progress published but directory sync failed: {e}"))
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(&root, &name, AtFlags::empty());
    }
    result?;
    Ok(progress)
}
