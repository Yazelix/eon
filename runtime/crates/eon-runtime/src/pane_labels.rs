//! Runtime's semantic pane labels and read-only Orbit metadata observations.

use eon_workspace_protocol::v22::{MAX_PANE_NAME_CHARS, pane_label_character};
use orbit_protocol::session::{self, ClientMessage, ServerMessage};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{ffi::OsStringExt, net::UnixStream},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

// Match existing bounded negotiation/status budgets; retries stay off the request path.
const NEGOTIATION: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct Labels {
    observations: HashMap<(String, PathBuf), Observation>,
}

impl Labels {
    pub(super) fn reconcile(&mut self, panes: &[super::workspace::Session]) {
        let targets = panes
            .iter()
            .map(|pane| (pane.id.clone(), pane.endpoint.clone()))
            .collect::<HashSet<_>>();
        self.observations.retain(|key, _| targets.contains(key));
        for (id, endpoint) in targets {
            self.observations
                .entry((id, endpoint.clone()))
                .or_insert_with(|| Observation::start(endpoint));
        }
    }

    pub(super) fn present(
        &self,
        snapshot: &mut eon_workspace_protocol::v22::Snapshot,
        home: Option<&Path>,
    ) {
        for tab in &mut snapshot.tabs {
            let directory = PathBuf::from(OsString::from_vec(tab.directory.clone()));
            for pane in tab.columns.iter_mut().flat_map(|column| &mut column.panes) {
                let key = (
                    pane.session.clone(),
                    PathBuf::from(OsString::from_vec(pane.endpoint.clone())),
                );
                let metadata =
                    self.observations
                        .get(&key)
                        .map_or(Metadata::Connecting, |observer| {
                            observer
                                .metadata
                                .lock()
                                .expect("pane metadata lock poisoned")
                                .clone()
                        });
                pane.label = compose(
                    &pane.id,
                    pane.live,
                    pane.pinned_name.as_deref(),
                    &metadata,
                    &directory,
                    home,
                );
            }
        }
    }
}

struct Observation {
    metadata: Arc<Mutex<Metadata>>,
    control: Arc<Control>,
}

#[derive(Default)]
struct Control {
    cancelled: AtomicBool,
    stream: Mutex<Option<UnixStream>>,
}

impl Observation {
    fn start(endpoint: PathBuf) -> Self {
        let metadata = Arc::new(Mutex::new(Metadata::Connecting));
        let control = Arc::new(Control::default());
        let result = thread::Builder::new()
            .name("eon-pane-metadata".into())
            .spawn({
                let metadata = Arc::clone(&metadata);
                let control = Arc::clone(&control);
                move || {
                    while !control.cancelled.load(Ordering::Acquire) {
                        let _ = observe(&endpoint, &control, &metadata);
                        control
                            .stream
                            .lock()
                            .expect("pane metadata control poisoned")
                            .take();
                        if control.cancelled.load(Ordering::Acquire) {
                            break;
                        }
                        *metadata.lock().expect("pane metadata lock poisoned") =
                            Metadata::Unavailable;
                        for _ in 0..RETRY.as_millis() / 25 {
                            if control.cancelled.load(Ordering::Acquire) {
                                return;
                            }
                            thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            });
        if result.is_err() {
            *metadata.lock().expect("pane metadata lock poisoned") = Metadata::Unavailable;
        }
        Self { metadata, control }
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.control.cancelled.store(true, Ordering::Release);
        if let Some(stream) = self
            .control
            .stream
            .lock()
            .expect("pane metadata control poisoned")
            .take()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

fn observe(endpoint: &Path, control: &Control, metadata: &Mutex<Metadata>) -> io::Result<()> {
    let mut stream = super::sessions::unix_connect_with_timeout(endpoint, NEGOTIATION)?;
    stream.set_read_timeout(Some(NEGOTIATION))?;
    stream.set_write_timeout(Some(NEGOTIATION))?;
    {
        let mut active = control
            .stream
            .lock()
            .expect("pane metadata control poisoned");
        if control.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        *active = Some(stream.try_clone()?);
    }
    let request = session::encode_client_message(&ClientMessage::ObserveMetadata)
        .map_err(io::Error::other)?;
    stream.write_all(&request)?;
    if !matches!(
        read_metadata_message(&mut stream)?,
        ServerMessage::ObservingMetadata
    ) {
        return Err(io::Error::other("metadata observation refused"));
    }
    let mut revision = None;
    loop {
        let ServerMessage::Metadata(next) = read_metadata_message(&mut stream)? else {
            return Err(io::Error::other("unexpected metadata reply"));
        };
        if revision.is_some_and(|previous| next.revision <= previous) {
            return Err(io::Error::other("metadata revision did not advance"));
        }
        revision = Some(next.revision);
        *metadata.lock().expect("pane metadata lock poisoned") = Metadata::Available {
            title: next.title,
            directory: next.working_directory,
        };
        stream.set_read_timeout(None)?;
    }
}

fn read_metadata_message(stream: &mut UnixStream) -> io::Result<ServerMessage> {
    let mut header = [0; session::HEADER_BYTES];
    stream.read_exact(&mut header)?;
    let length = session::server_message_len(&header)
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("incomplete metadata header"))?;
    // Only observation messages are admitted; the canonical decoder bounds their payload.
    if length > session::MAX_METADATA_BYTES + session::HEADER_BYTES {
        return Err(io::Error::other("oversized metadata message"));
    }
    let mut bytes = Vec::from(header);
    bytes.resize(length, 0);
    stream.read_exact(&mut bytes[session::HEADER_BYTES..])?;
    session::decode_server_message(&bytes).map_err(io::Error::other)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Metadata {
    Connecting,
    Available {
        title: String,
        directory: String,
    },
    #[default]
    Unavailable,
}

pub(super) fn compose(
    id: &str,
    live: bool,
    pin: Option<&str>,
    metadata: &Metadata,
    tab_directory: &Path,
    home: Option<&Path>,
) -> String {
    let state = match (live, metadata) {
        (false, _) => Some("offline"),
        (_, Metadata::Connecting) => Some("connecting"),
        (_, Metadata::Unavailable) => Some("unavailable"),
        _ => None,
    };
    if let Some(state) = state {
        return pin.map_or_else(
            || format!("{id}  {state}"),
            |pin| {
                let room = MAX_PANE_NAME_CHARS - state.len() - 3;
                format!("{id}  {} · {state}", bounded(pin, room))
            },
        );
    }
    let Metadata::Available { title, directory } = metadata else {
        unreachable!("availability handled above");
    };
    let title = clean(title);
    let directory = local_directory(directory).map(lexical);
    let compact = directory.as_deref().map(|path| compact(path, home));
    let name = pin
        .filter(|pin| !pin.is_empty())
        .or_else(|| (!title.is_empty()).then_some(title.as_str()));
    let name_directory = name.map(
        |name| match home.filter(|home| !home.as_os_str().is_empty()) {
            Some(home) if name == "~" => home.to_path_buf(),
            Some(home) if name.starts_with("~/") => home.join(name[2..].trim_start_matches('/')),
            _ => tab_directory.join(name),
        },
    );
    let name = name.or(compact.as_deref()).unwrap_or("");
    let mut body = name.to_owned();
    // Sanitizing may make distinct paths display identically.
    if let (Some(directory), Some(compact)) = (&directory, &compact)
        && *directory != lexical(tab_directory)
        && name != compact
        && name_directory.is_some_and(|name| lexical(&name) != *directory)
    {
        let relative = relative(directory, &lexical(tab_directory));
        let relative = relative.to_string_lossy();
        let context = if relative.chars().count() < compact.chars().count() {
            clean(&relative)
        } else {
            compact.clone()
        };
        if !context.is_empty() && context != name {
            body.push_str(" · ");
            body.push_str(&context);
        }
    }
    if body.is_empty() {
        id.to_owned()
    } else {
        format!("{id}  {}", bounded(&body, MAX_PANE_NAME_CHARS))
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|&character| pane_label_character(character))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn bounded(text: &str, maximum: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= maximum {
        return text.to_owned();
    }
    let left = maximum / 2;
    chars[..left]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(&chars[chars.len() - (maximum - left - 1)..])
        .collect()
}

fn local_directory(value: &str) -> Option<&Path> {
    let value = value
        .strip_prefix("file://")
        .and_then(|path| path.find('/').map(|slash| &path[slash..]))
        .unwrap_or(value);
    (!value.trim().is_empty() && Path::new(value).is_absolute()).then(|| Path::new(value))
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

fn compact(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) {
        if path == home {
            return "~".into();
        }
        if let Ok(tail) = path.strip_prefix(home) {
            return clean(&format!("~/{}", tail.display()));
        }
    }
    clean(&path.to_string_lossy())
}

fn relative(path: &Path, base: &Path) -> PathBuf {
    let path = path.components().collect::<Vec<_>>();
    let base = base.components().collect::<Vec<_>>();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &path[common..] {
        relative.push(component.as_os_str());
    }
    relative
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_retries_admission_keeps_latest_and_releases_its_slot() {
        use std::os::unix::net::UnixListener;
        use std::time::Instant;
        let root = crate::supervisor::temporary_directory();
        std::fs::create_dir_all(&root).unwrap();
        let endpoint = root.join("pane.sock");
        let listener = UnixListener::bind(&endpoint).unwrap();
        let server = thread::spawn(move || {
            let (mut busy, _) = listener.accept().unwrap();
            busy.write_all(&session::encode_server_message(&ServerMessage::Busy).unwrap())
                .unwrap();
            drop(busy);
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; session::HEADER_BYTES];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(
                session::decode_client_message(&request).unwrap(),
                ClientMessage::ObserveMetadata
            );
            for message in [
                ServerMessage::ObservingMetadata,
                ServerMessage::Metadata(session::Metadata {
                    revision: 7,
                    title: "working".into(),
                    working_directory: "/tmp/one".into(),
                }),
                ServerMessage::Metadata(session::Metadata {
                    revision: 8,
                    title: "ready".into(),
                    working_directory: "/tmp/two".into(),
                }),
            ] {
                stream
                    .write_all(&session::encode_server_message(&message).unwrap())
                    .unwrap();
            }
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        let observer = Observation::start(endpoint);
        let expected = Metadata::Available {
            title: "ready".into(),
            directory: "/tmp/two".into(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while *observer.metadata.lock().unwrap() != expected {
            assert!(
                Instant::now() < deadline,
                "metadata did not converge after Busy"
            );
            thread::sleep(Duration::from_millis(10));
        }
        drop(observer);
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    fn label(pin: Option<&str>, title: &str, directory: &str) -> String {
        compose(
            "p1",
            true,
            pin,
            &Metadata::Available {
                title: title.into(),
                directory: directory.into(),
            },
            Path::new("/home/u/projects/eon"),
            Some(Path::new("/home/u")),
        )
    }

    #[test]
    fn precedence_and_lexical_directory_context() {
        assert_eq!(
            label(Some("Review"), "cargo test", "/home/u/projects/eon"),
            "p1  Review"
        );
        assert_eq!(
            label(None, "cargo test", "file://host/home/u/projects/other"),
            "p1  cargo test · ../other"
        );
        assert_eq!(
            label(Some("Review"), "cargo test", "/home/u/projects/eon/src"),
            "p1  Review · src"
        );
        assert_eq!(
            label(None, "", "/home/u/projects/other"),
            "p1  ~/projects/other"
        );
        assert_eq!(label(None, "shell", "/home/u"), "p1  shell · ~");
        assert_eq!(label(None, "test", "/opt/x"), "p1  test · /opt/x");
        assert_eq!(
            label(None, "test", "/home/u/projects/eon/src/.."),
            "p1  test"
        );
        assert_eq!(label(None, "", ""), "p1");
        assert_eq!(
            label(Some("~/projects/other"), "", "/home/u/projects/other"),
            "p1  ~/projects/other"
        );
    }

    #[test]
    fn directory_names_do_not_repeat_equivalent_context() {
        for (name, directory) in [
            ("/home/u/projects/other", "/home/u/projects/other"),
            ("/home/u/projects/eon/../other/./", "/home/u/projects/other"),
            ("~/projects/other", "/home/u/projects/other"),
            ("~/projects/other", "/home/u/projects/other "),
            ("~/projects/other", "/home/u/projects/oth\u{202e}er"),
            ("~//projects/other", "/home/u/projects/other"),
            ("~/projects/eon/../other/", "/home/u/projects/other/./"),
            ("../other", "/home/u/projects/other"),
            ("../eon/../other/", "/home/u/projects/other"),
            ("./src/../src", "/home/u/projects/eon/src"),
            ("/home/u", "/home/u"),
            ("~", "/home/u"),
            ("../..", "/home/u"),
            ("/opt/x/../x", "/opt/x"),
        ] {
            assert_eq!(label(None, name, directory), format!("p1  {name}"));
            assert_eq!(
                label(Some(name), "cargo test", directory),
                format!("p1  {name}")
            );
        }
        assert_eq!(
            label(None, "../other2", "/home/u/projects/other"),
            "p1  ../other2 · ../other"
        );
        assert_eq!(
            label(Some("~//opt/x"), "", "/opt/x"),
            "p1  ~//opt/x · /opt/x"
        );
    }

    #[test]
    fn untrusted_text_is_disarmed_and_visibly_bounded() {
        assert_eq!(label(None, "\u{202e}cargo\u{200b}\n", ""), "p1  cargo");
        assert_eq!(
            label(None, "\u{202e}\n", "/home/u/projects/eon"),
            "p1  ~/projects/eon"
        );
        let full = label(Some(&"界".repeat(80)), "", "/home/u/projects/other");
        assert_eq!(full.strip_prefix("p1  ").unwrap().chars().count(), 80);
        assert!(full.contains('…'));
        assert!(full.len() <= 450);
        assert!(label(None, &"界".repeat(1000), "").contains('…'));
    }
}
