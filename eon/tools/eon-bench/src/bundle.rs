//! The Nix-free bundle candidate for installed journeys: its archive, the host
//! files it may use, and the jail every candidate process runs in.
//!
//! The host files are docs/MANUAL-BUNDLE.md's host boundary (glibc, `libgcc_s`,
//! `libwayland-client`), the host's Mesa lavapipe Vulkan driver with the
//! libraries it lists, and `/bin/sh`. The jail is one persistent PID namespace
//! with a fresh procfs: no `/nix`, not even through `/proc/*/root`, and a PATH
//! whose directories hold only `sh` and `dash`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::fs::PermissionsExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    json::{Value, object},
    launcher,
    sha256::Sha256,
};

type Result<T = ()> = std::result::Result<T, String>;

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Where the outer journey jail mounts each host file, so the bench's own Nix
/// programs never meet them at their usual paths.
pub(crate) const HOST_ROOT: &str = "/.host";
/// The bundle's ELF interpreter (the psABI path, docs/MANUAL-BUNDLE.md).
pub(crate) const LOADER: &str = "/lib64/ld-linux-x86-64.so.2";
/// The host's Mesa lavapipe ICD: the private Sway has no GPU.
pub(crate) const ICD: &str = "/usr/share/vulkan/icd.d/lvp_icd.json";
const MERGED: [&str; 3] = ["/bin", "/lib", "/lib64"];
const GLIBC: [&str; 15] = [
    "libc.so.6",
    "libm.so.6",
    "libmvec.so.1",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
    "libresolv.so.2",
    "libutil.so.1",
    "libanl.so.1",
    "libnss_files.so.2",
    "libnss_dns.so.2",
    "libnss_compat.so.2",
    "libBrokenLocale.so.1",
    "libc_malloc_debug.so.0",
    "libthread_db.so.1",
];
const BOUNDARY: [&str; 2] = ["libgcc_s.so.1", "libwayland-client.so.0"];
const SHELL: &str = "/usr/bin/sh";
const CACHE: &str = "/etc/ld.so.cache";
/// The jail's PATH: its directories hold only the host `sh` and `dash`.
const PATH: &str = "/usr/bin:/bin";
/// Venus alone may map the lavapipe driver's own libraries, and must.
/// Bundle wrappers exec the real program, so this is Venus's executable name.
pub(crate) const VENUS: &str = ".yazelix-venus-wrapped";

/// The host files the jail exposes, each at its own path: regular files bound
/// read-only and links recreated with their own target text.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HostSet {
    links: Vec<(PathBuf, PathBuf)>,
    files: BTreeSet<PathBuf>,
    /// Files only the lavapipe driver needs: only Venus may map them.
    lavapipe: BTreeSet<PathBuf>,
    driver: PathBuf,
}

fn plain(path: &Path) -> Result<&str> {
    path.to_str()
        .filter(|text| !text.is_empty() && !text.contains(char::is_whitespace))
        .ok_or_else(|| format!("unsupported path {}", path.display()))
}

impl HostSet {
    /// Reads the host set on the host itself. Every error starts with
    /// "--bundle needs" and names the missing prerequisite.
    pub(crate) fn discover() -> Result<Self> {
        Self::read_host().map_err(prerequisite)
    }

    fn read_host() -> Result<Self> {
        fn needs(what: &str) -> impl Fn(String) -> String + '_ {
            move |error| format!("--bundle needs {what}: {error}")
        }
        let mut set = Self::default();
        for top in MERGED {
            match fs::read_link(top) {
                Ok(target) if target.starts_with("usr") || target.starts_with("/usr") => {
                    set.links.push((top.into(), target));
                }
                _ => {
                    return Err(format!(
                        "--bundle needs a merged-/usr host: {top} is not a link into /usr"
                    ));
                }
            }
        }
        let loader = set
            .add(Path::new(LOADER))
            .map_err(needs("the host loader"))?;
        let libdir = loader.parent().expect("a host file has a directory");
        for name in GLIBC {
            let path = libdir.join(name);
            if fs::symlink_metadata(&path).is_ok() {
                set.add(&path)?;
            }
        }
        for name in BOUNDARY {
            set.add(&libdir.join(name)).map_err(needs(name))?;
        }
        set.add(Path::new(SHELL)).map_err(needs("the host sh"))?;
        set.add(Path::new(CACHE))
            .map_err(needs("the host loader cache"))?;
        let lavapipe = needs("the host's Mesa lavapipe driver");
        let icd = fs::read_to_string(ICD)
            .map_err(|error| format!("{ICD}: {error}"))
            .map_err(&lavapipe)?;
        let library = Value::parse(&icd)?
            .get("ICD")
            .and_then(|icd| icd.get("library_path"))
            .and_then(Value::as_str)
            .ok_or(format!("{ICD} names no library_path"))?
            .to_owned();
        set.add(Path::new(ICD))?;
        let base = set.files.clone();
        let driver = libdir.join(library);
        set.driver = set.add(&driver).map_err(&lavapipe)?;
        let listed = Command::new("timeout")
            .arg("10s")
            .arg(&loader)
            .arg("--list")
            .arg(&set.driver)
            .stdin(Stdio::null())
            .output()
            .map_err(failure)?;
        if !listed.status.success() {
            return Err(lavapipe(format!(
                "{} --list failed: {}",
                loader.display(),
                String::from_utf8_lossy(&listed.stderr).trim()
            )));
        }
        for library in listed_libraries(&String::from_utf8_lossy(&listed.stdout))? {
            set.add(&library).map_err(&lavapipe)?;
        }
        set.lavapipe = set.files.difference(&base).cloned().collect();
        Ok(set)
    }

    /// Adds `path` and every link on the way to its file. Returns the file.
    fn add(&mut self, path: &Path) -> Result<PathBuf> {
        let mut path = path.to_path_buf();
        for _ in 0..16 {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Err(format!("{} is not a host file", path.display()));
            };
            let here = fs::canonicalize(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?
                .join(name);
            let text = plain(&here)?;
            if !text.starts_with("/usr/") && text != CACHE {
                return Err(format!("host file {text} is outside /usr"));
            }
            let meta = fs::symlink_metadata(&here).map_err(|error| format!("{text}: {error}"))?;
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&here).map_err(failure)?;
                plain(&target)?;
                path = here.parent().expect("joined above").join(&target);
                if !self.links.iter().any(|(link, _)| *link == here) {
                    self.links.push((here, target));
                }
            } else if meta.is_file() {
                self.files.insert(here.clone());
                return Ok(here);
            } else {
                return Err(format!("host path {text} is not a file"));
            }
        }
        Err(format!(
            "host link chain from {} is too long",
            path.display()
        ))
    }

    /// Whether every jailed process may map or run `path`.
    fn base(&self, path: &Path) -> bool {
        self.files.contains(path) && !self.lavapipe.contains(path)
    }

    /// Read-only binds for the outer journey jail: each file under `HOST_ROOT`.
    pub(crate) fn outer_binds(&self) -> Vec<(PathBuf, PathBuf)> {
        self.files
            .iter()
            .map(|file| (file.clone(), Path::new(HOST_ROOT).join(rooted(file))))
            .collect()
    }

    fn mounts(&self, source: &Path) -> Result<Vec<String>> {
        let mut args = Vec::new();
        for (link, target) in &self.links {
            args.extend([
                "--symlink".into(),
                plain(target)?.into(),
                plain(link)?.into(),
            ]);
        }
        for file in &self.files {
            args.extend([
                "--ro-bind".into(),
                plain(&source.join(rooted(file)))?.into(),
                plain(file)?.into(),
            ]);
        }
        Ok(args)
    }

    /// The one representation of the host set: the inner process reads it
    /// back with `from_json`, and the receipt records it.
    pub(crate) fn to_json(&self) -> Value {
        let text = |path: &PathBuf| Value::from(path.display().to_string());
        let paths =
            |paths: &mut dyn Iterator<Item = &PathBuf>| Value::Array(paths.map(text).collect());
        object([
            (
                "links",
                Value::Array(
                    self.links
                        .iter()
                        .map(|(link, target)| Value::Array(vec![text(link), text(target)]))
                        .collect(),
                ),
            ),
            (
                "files",
                paths(&mut self.files.iter().filter(|file| self.base(file))),
            ),
            ("lavapipe", paths(&mut self.lavapipe.iter())),
            ("driver", text(&self.driver)),
        ])
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self> {
        let malformed = || format!("malformed host set: {}", value.to_pretty().trim());
        let paths = |key| -> Result<Vec<PathBuf>> {
            value
                .get(key)
                .and_then(Value::as_array)
                .ok_or_else(malformed)?
                .iter()
                .map(|path| path.as_str().map(PathBuf::from).ok_or_else(malformed))
                .collect()
        };
        let lavapipe: BTreeSet<_> = paths("lavapipe")?.into_iter().collect();
        let mut files: BTreeSet<_> = paths("files")?.into_iter().collect();
        files.extend(lavapipe.iter().cloned());
        let links = value
            .get("links")
            .and_then(Value::as_array)
            .ok_or_else(malformed)?
            .iter()
            .map(|link| match link.as_array() {
                Some([link, target]) => Ok((
                    link.as_str().ok_or_else(malformed)?.into(),
                    target.as_str().ok_or_else(malformed)?.into(),
                )),
                _ => Err(malformed()),
            })
            .collect::<Result<_>>()?;
        let driver: PathBuf = value
            .get("driver")
            .and_then(Value::as_str)
            .ok_or_else(malformed)?
            .into();
        if !lavapipe.contains(&driver) {
            return Err("host set names no lavapipe driver".into());
        }
        Ok(Self {
            links,
            files,
            lavapipe,
            driver,
        })
    }
}

fn rooted(path: &Path) -> &Path {
    path.strip_prefix("/").unwrap_or(path)
}

/// Gives a host-set error the "--bundle needs" prefix if it lacks one.
fn prerequisite(error: String) -> String {
    if error.starts_with("--bundle needs") {
        error
    } else {
        format!("--bundle needs a readable host set: {error}")
    }
}

/// The libraries a host loader's `--list` resolved.
fn listed_libraries(text: &str) -> Result<Vec<PathBuf>> {
    let mut libraries = Vec::new();
    for line in text.lines() {
        let Some((name, resolved)) = line.split_once(" => ") else {
            continue;
        };
        if resolved.starts_with("not found") {
            return Err(format!("{} is not found on the host", name.trim()));
        }
        let path = resolved.split(" (").next().unwrap_or_default();
        if path.starts_with('/') {
            libraries.push(path.into());
        }
    }
    Ok(libraries)
}

/// A private extraction of the published archive.
#[derive(Debug)]
pub(crate) struct Bundle {
    archive: PathBuf,
    sha256: String,
    bytes: u64,
    pub(crate) tree: PathBuf,
    pub(crate) entry: PathBuf,
    pub(crate) product: PathBuf,
    package: String,
    pub(crate) glibc_floor: String,
    /// The product's `share/eon-source-receipt.json`, with a `build`.
    pub(crate) receipt: Value,
}

impl Bundle {
    /// Extracts `archive` into `root`, which must not exist, hashing the very
    /// bytes it feeds to `tar` (decompressed by `zstd`). The caller owns `root`
    /// from this call on, and removes it with `remove` whether or not
    /// extraction succeeds.
    pub(crate) fn extract(
        archive: &Path,
        root: &Path,
        (tar, zstd): (&std::ffi::OsStr, &std::ffi::OsStr),
        deadline: Instant,
    ) -> Result<Self> {
        use std::io::Write;
        fs::create_dir(root).map_err(|error| format!("{}: {error}", root.display()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("bundle extraction deadline elapsed")?;
        let mut compressor = std::ffi::OsString::from("--use-compress-program=");
        compressor.push(zstd);
        let mut file =
            File::open(archive).map_err(|error| format!("{}: {error}", archive.display()))?;
        let mut tar = Command::new("timeout")
            .arg(format!("{}s", remaining.as_secs_f64()))
            .arg(tar)
            .arg("--extract")
            .arg(compressor)
            .args(["--file", "-", "--directory"])
            .arg(root)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(failure)?;
        let mut errors = tar.stderr.take().expect("piped stderr");
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = errors.read_to_string(&mut text);
            text
        });
        let mut input = tar.stdin.take().expect("piped stdin");
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        let mut bytes = 0;
        let fed = loop {
            match file.read(&mut buffer) {
                Ok(0) => break Ok(()),
                Ok(read) => {
                    hasher.update(&buffer[..read]);
                    bytes += read as u64;
                    if let Err(error) = input.write_all(&buffer[..read]) {
                        break Err(error);
                    }
                }
                Err(error) => break Err(error),
            }
        };
        drop(input);
        let status = tar.wait().map_err(failure)?;
        let errors = errors.join().unwrap_or_default();
        if !status.success() || fed.is_err() {
            return Err(format!(
                "extracting {} ({status}, fed: {fed:?}): {}",
                archive.display(),
                errors.trim()
            ));
        }
        Self::open(archive, hasher.hex(), bytes, root)
    }

    /// Validates an extraction: one top-level directory, inside `root`, whose
    /// provenance, product root, source receipt and `eon` entrypoint all
    /// resolve inside it.
    fn open(archive: &Path, sha256: String, bytes: u64, root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(failure)?;
        let entries = fs::read_dir(&root)
            .and_then(Iterator::collect::<std::io::Result<Vec<_>>>)
            .map_err(failure)?;
        let [top] = entries.as_slice() else {
            return Err("the archive must hold exactly one top-level directory".into());
        };
        let tree = fs::canonicalize(top.path()).map_err(failure)?;
        if tree.parent() != Some(&root) || !tree.is_dir() {
            return Err(format!(
                "the archive's top-level entry resolves outside the extraction, to {}",
                tree.display()
            ));
        }
        let inside = |path: &Path, what: &str| {
            let resolved = fs::canonicalize(path)
                .map_err(|error| format!("bundle {what} {}: {error}", path.display()))?;
            if !resolved.starts_with(&tree) {
                return Err(format!(
                    "bundle {what} {} resolves outside the archive, to {}",
                    path.display(),
                    resolved.display()
                ));
            }
            Ok(resolved)
        };
        let read = |path: &Path, what: &str| {
            fs::read_to_string(inside(path, what)?)
                .map_err(|error| format!("bundle {what}: {error}"))
                .and_then(|text| Value::parse(&text))
        };
        let provenance = read(&tree.join("provenance.json"), "provenance.json")?;
        let package = provenance
            .get("package")
            .and_then(Value::as_str)
            .ok_or("bundle provenance names no package")?;
        let glibc_floor = provenance
            .get("relink")
            .and_then(|relink| relink.get("glibc_floor"))
            .and_then(Value::as_str)
            .ok_or("bundle provenance names no glibc_floor")?;
        let product = inside(&tree.join(rooted(Path::new(package))), "product root")?;
        let receipt = read(
            &product.join("share/eon-source-receipt.json"),
            "source receipt",
        )?;
        if receipt.get("build").and_then(Value::as_str).is_none() {
            return Err("the bundle's source receipt has no build".into());
        }
        let entry = tree.join("eon");
        if !inside(&entry, "entrypoint")?.is_file() {
            return Err("bundle entrypoint eon is not a file".into());
        }
        Ok(Self {
            archive: archive.into(),
            sha256,
            bytes,
            tree,
            entry,
            product,
            package: package.into(),
            glibc_floor: glibc_floor.into(),
            receipt,
        })
    }

    pub(crate) fn to_json(&self) -> Value {
        let text = |path: &Path| path.display().to_string().into();
        object([
            ("archive", text(&self.archive)),
            ("sha256", self.sha256.as_str().into()),
            ("bytes", self.bytes.into()),
            ("tree", text(&self.tree)),
            ("entrypoint", text(&self.entry)),
            ("product_root", text(&self.product)),
            ("package", self.package.as_str().into()),
            ("glibc_floor", self.glibc_floor.as_str().into()),
        ])
    }
}

/// The live processes that run from or map a file under `root`: removing it
/// while any remain would leave them running from a deleted tree.
pub(crate) fn occupied(root: &Path) -> Result<Vec<u32>> {
    let mut found = Vec::new();
    for (pid, start) in launcher::namespace_processes()? {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let exe = fs::read_link(proc.join("exe")).unwrap_or_default();
        let maps = fs::read_to_string(proc.join("maps")).unwrap_or_default();
        let under = |text: &str| Path::new(text.trim_end_matches(" (deleted)")).starts_with(root);
        let mapped = maps
            .lines()
            .filter_map(|line| line.find('/').map(|start| &line[start..]))
            .any(under);
        if (under(&exe.to_string_lossy()) || mapped) && launcher::live_identity(pid, &start) {
            found.push(pid);
        }
    }
    Ok(found)
}

/// Removes an extraction root, complete or partial. Its directories may be
/// read-only, so each is made writable first, walking without following any
/// link.
pub(crate) fn remove(root: &Path) -> Result<Value> {
    fn writable(path: &Path) -> std::io::Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_dir() {
            return Ok(());
        }
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(meta.permissions().mode() | 0o700),
        )?;
        fs::read_dir(path)?.try_for_each(|entry| writable(&entry?.path()))
    }
    writable(root)
        .and_then(|()| fs::remove_dir_all(root))
        .map_err(|error| format!("removing {}: {error}", root.display()))?;
    if fs::symlink_metadata(root).is_ok() {
        return Err(format!("{} is still present", root.display()));
    }
    Ok(object([("removed", root.display().to_string().into())]))
}

/// Runs `spawn` with `fds` inherited by the processes it starts. The journey
/// spawns from one thread, so no other spawn can inherit them meanwhile.
fn inherit<R>(fds: &[BorrowedFd<'_>], spawn: impl FnOnce() -> R) -> Result<R> {
    use rustix::io::{FdFlags, fcntl_setfd};
    for fd in fds {
        fcntl_setfd(fd, FdFlags::empty()).map_err(failure)?;
    }
    let spawned = spawn();
    for fd in fds {
        fcntl_setfd(fd, FdFlags::CLOEXEC).map_err(failure)?;
    }
    Ok(spawned)
}

/// One persistent PID namespace for every candidate process. A holder keeps
/// it alive, and each candidate call joins it with a fresh procfs and the
/// jail's mounts and environment.
pub(crate) struct Jail {
    bwrap: PathBuf,
    mounts: Vec<String>,
    host: HostSet,
    xdg: PathBuf,
    user: File,
    pid: File,
    namespace: PathBuf,
    init: u32,
}

impl Jail {
    /// The jail's mounts: devices, a fresh procfs, `/tmp`, the writable `binds`
    /// the candidate shares with the bench, the account files and the host
    /// set, read from under `source`.
    pub(crate) fn mounts(host: &HostSet, source: &Path, binds: &[&Path]) -> Result<Vec<String>> {
        let mut args: Vec<String> = [
            "--die-with-parent",
            "--dev-bind",
            "/dev",
            "/dev",
            "--proc",
            "/proc",
            "--bind",
            "/tmp",
            "/tmp",
        ]
        .map(String::from)
        .into();
        for bind in binds {
            let bind = plain(bind)?;
            args.extend(["--bind".into(), bind.into(), bind.into()]);
        }
        for account in ["/etc/passwd", "/etc/group"] {
            args.extend(["--ro-bind".into(), account.into(), account.into()]);
        }
        args.extend(host.mounts(source)?);
        Ok(args)
    }

    /// Starts the holder through `spawn`, which must give it a piped stdin it
    /// keeps open: the holder lives until that pipe closes or it is killed.
    pub(crate) fn start(
        bwrap: PathBuf,
        host: HostSet,
        mounts: Vec<String>,
        xdg: PathBuf,
        env: &BTreeMap<String, String>,
        spawn: impl FnOnce(&Path, &[String]) -> Result,
    ) -> Result<Self> {
        let (mut reader, writer) = std::io::pipe().map_err(failure)?;
        let mut jail = Self {
            bwrap,
            mounts,
            host,
            xdg,
            user: File::open("/dev/null").map_err(failure)?,
            pid: File::open("/dev/null").map_err(failure)?,
            namespace: PathBuf::new(),
            init: 0,
        };
        let mut args = vec![
            "--unshare-pid".into(),
            "--info-fd".into(),
            writer.as_raw_fd().to_string(),
        ];
        args.extend(jail.mounts.iter().cloned());
        args.extend(jail.environment(env)?);
        args.extend(["--", "/bin/sh", "-c", "read -r line"].map(String::from));
        inherit(&[writer.as_fd()], || spawn(&jail.bwrap, &args))??;
        drop(writer);
        let info = read_info(&mut reader, Instant::now() + Duration::from_secs(5))?;
        jail.init = info
            .get("child-pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or(format!("jail holder reported no child-pid: {info:?}"))?;
        let namespace = |kind: &str| format!("/proc/{}/ns/{kind}", jail.init);
        jail.user = File::open(namespace("user")).map_err(failure)?;
        jail.pid = File::open(namespace("pid")).map_err(failure)?;
        jail.namespace = fs::read_link(namespace("pid")).map_err(failure)?;
        Ok(jail)
    }

    pub(crate) fn bwrap(&self) -> &Path {
        &self.bwrap
    }

    /// Runs `spawn` with the namespace descriptors a joining `bwrap` needs.
    pub(crate) fn inherit<R>(&self, spawn: impl FnOnce() -> R) -> Result<R> {
        inherit(&[self.user.as_fd(), self.pid.as_fd()], spawn)
    }

    /// `bwrap` arguments that run `program` inside the jail.
    pub(crate) fn join(
        &self,
        env: &BTreeMap<String, String>,
        program: &Path,
        args: &[&str],
    ) -> Result<Vec<String>> {
        let mut argv = vec![
            "--userns".into(),
            self.user.as_raw_fd().to_string(),
            "--pidns".into(),
            self.pid.as_raw_fd().to_string(),
        ];
        argv.extend(self.mounts.iter().cloned());
        argv.extend(self.environment(env)?);
        argv.push("--".into());
        argv.push(program.to_str().ok_or("non-UTF8 candidate program")?.into());
        argv.extend(args.iter().map(|&arg| arg.into()));
        Ok(argv)
    }

    /// `env` with the jail's PATH, Vulkan driver and private XDG search
    /// roots. No value, nor any `:`-separated element of one, may name `/nix`.
    fn environment(&self, env: &BTreeMap<String, String>) -> Result<Vec<String>> {
        let mut env = env.clone();
        let xdg = |name: &str| self.xdg.join(name).display().to_string();
        // Sway's command socket stays outside (launcher::Headless::bundle).
        env.remove("SWAYSOCK");
        env.insert("PATH".into(), PATH.into());
        env.insert("VK_DRIVER_FILES".into(), ICD.into());
        env.insert("XDG_DATA_DIRS".into(), xdg("data"));
        env.insert("XDG_CONFIG_DIRS".into(), xdg("config"));
        let mut args = vec!["--clearenv".to_owned()];
        for (key, value) in &env {
            if value.split(':').any(|item| item.starts_with("/nix/")) {
                return Err(format!("jail environment {key} names /nix: {value}"));
            }
            args.extend(["--setenv".into(), key.clone(), value.clone()]);
        }
        if let Some(home) = env.get("HOME") {
            args.extend(["--chdir".into(), home.clone()]);
        }
        Ok(args)
    }

    /// Audits the origins of every live process in the journey's PID
    /// namespace, whatever its environment: outside the jail none may run from
    /// `tree`; inside, each must pass `origin`. Each name in `required` must
    /// be live in the jail.
    pub(crate) fn audit(&self, out: &Path, tree: &Path, required: &[&str]) -> Result<Value> {
        let mut seen = Vec::new();
        let mut roles = BTreeSet::new();
        let mut outside = 0u64;
        for (pid, start) in launcher::namespace_processes()? {
            let proc = PathBuf::from(format!("/proc/{pid}"));
            // A process that exits mid-read is skipped; a live one must read.
            fn gone<T>(pid: u32, start: &str, error: std::io::Error) -> Result<Option<T>> {
                if launcher::live_identity(pid, start) {
                    Err(format!("process {pid}: {error}"))
                } else {
                    Ok(None)
                }
            }
            let read = || -> std::io::Result<_> {
                Ok((
                    fs::read_link(proc.join("ns/pid"))?,
                    fs::read_link(proc.join("exe"))?,
                ))
            };
            let Some((namespace, exe)) =
                read().map(Some).or_else(|error| gone(pid, &start, error))?
            else {
                continue;
            };
            let exe = PathBuf::from(
                exe.to_string_lossy()
                    .trim_end_matches(" (deleted)")
                    .to_owned(),
            );
            if namespace != self.namespace {
                if exe.starts_with(tree) {
                    return Err(format!(
                        "process {pid} runs {} from the bundle outside the jail",
                        exe.display()
                    ));
                }
                outside += 1;
                continue;
            }
            let evidence = if pid == self.init {
                object([("role", "jail-init".into())])
            } else {
                let read = || -> std::io::Result<_> {
                    Ok((
                        fs::read(proc.join("environ"))?,
                        fs::read_to_string(proc.join("maps"))?,
                    ))
                };
                let Some((environ, maps)) =
                    read().map(Some).or_else(|error| gone(pid, &start, error))?
                else {
                    continue;
                };
                origin(&exe, &environ, &maps, out, tree, &self.host)
                    .map_err(|error| format!("process {pid}: {error}"))?
            };
            let role = evidence.get("role").and_then(Value::as_str);
            roles.insert(role.unwrap_or_default().to_owned());
            seen.push(object([
                ("pid", u64::from(pid).into()),
                ("exe", exe.display().to_string().into()),
                ("origin", evidence),
            ]));
        }
        let missing: Vec<_> = required
            .iter()
            .filter(|&&role| !roles.contains(role))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "{missing:?} not live in the jail; jailed processes: {}",
                Value::Array(seen).to_pretty().trim()
            ));
        }
        Ok(object([
            ("jailed", Value::Array(seen)),
            ("outside", outside.into()),
        ]))
    }
}

fn read_info(reader: &mut std::io::PipeReader, deadline: Instant) -> Result<Value> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut text = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("jail holder did not report its namespace in 5s; see jail.log")?;
        let timeout = Timespec::try_from(remaining).map_err(failure)?;
        if poll(&mut [PollFd::new(reader, PollFlags::IN)], Some(&timeout)).map_err(failure)? == 0 {
            return Err("jail holder did not report its namespace in 5s; see jail.log".into());
        }
        let mut chunk = [0; 256];
        match reader.read(&mut chunk).map_err(failure)? {
            0 if text.is_empty() => {
                return Err(
                    "jail holder exited before reporting its namespace; see jail.log".into(),
                );
            }
            0 => return Value::parse(&String::from_utf8_lossy(&text)),
            read => text.extend_from_slice(&chunk[..read]),
        }
        if let Ok(info) = Value::parse(&String::from_utf8_lossy(&text)) {
            return Ok(info);
        }
    }
}

/// Checks one jailed process: it runs from `tree` or the host set, names no
/// `/nix` in its environment, and maps only files from `out` (which holds the
/// tree), devices, memfds and the host set. Lavapipe's own libraries are for
/// Venus alone, and Venus must map the bundled Vulkan loader and the host
/// lavapipe driver.
pub(crate) fn origin(
    exe: &Path,
    environ: &[u8],
    maps: &str,
    out: &Path,
    tree: &Path,
    host: &HostSet,
) -> Result<Value> {
    let role = exe
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let venus = role == VENUS;
    if !exe.starts_with(tree) && !host.base(exe) {
        return Err(format!(
            "{} runs from outside the bundle and the host set",
            exe.display()
        ));
    }
    for item in environ.split(|&byte| byte == 0) {
        let item = String::from_utf8_lossy(item);
        if let Some((key, value)) = item.split_once('=')
            && value.split(':').any(|item| item.starts_with("/nix/"))
        {
            return Err(format!("{role}: environment {key} names /nix: {value}"));
        }
    }
    let (mut loader, mut driver) = (None, false);
    for line in maps.lines() {
        let Some(start) = line.find('/') else {
            continue;
        };
        let text = line[start..].trim_end_matches(" (deleted)");
        let path = Path::new(text);
        let allowed = path.starts_with(out)
            || path.starts_with("/dev")
            || text.starts_with("/memfd:")
            || host.base(path)
            || (venus && host.lavapipe.contains(path));
        if !allowed {
            return Err(format!("{role} ({}) maps {text}", exe.display()));
        }
        if path.starts_with(tree)
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libvulkan.so"))
        {
            loader = Some(text);
        }
        driver |= path == host.driver;
    }
    if venus && (loader.is_none() || !driver) {
        return Err(format!(
            "Venus must map the bundled Vulkan loader and the host driver {}; loader {loader:?}, driver mapped: {driver}",
            host.driver.display()
        ));
    }
    let mut evidence = vec![("role", role.into())];
    if venus {
        evidence.extend([
            ("vulkan_loader", loader.into()),
            ("vulkan_driver", host.driver.display().to_string().into()),
        ]);
    }
    Ok(object(evidence))
}

/// Run in the jail as `sh -c PROBE sh LOADER ICD SWAYSOCK WAYLAND`: the host
/// loader's identity, the PATH directories' entries, every visible process,
/// any way to `/nix` or to Sway's command socket, and the Wayland socket.
pub(crate) const PROBE: &str = r#"
"$1" --version
for f in /usr/bin/* /usr/bin/.[!.]* /bin/* /bin/.[!.]*; do
    if [ -e "$f" ] || [ -L "$f" ]; then echo "path $f"; fi
done
if [ -e /nix ]; then echo "nix /nix"; fi
for p in /proc/[0-9]*; do
    if [ -e "$p/root/nix" ]; then echo "nix $p/root/nix"; fi
    if read -r comm < "$p/comm"; then echo "pid ${p#/proc/} $comm"; fi
done
if [ -f "$2" ]; then echo "icd $2"; fi
if [ -e "$3" ]; then echo "ipc $3"; fi
if [ -S "$4" ]; then echo "wayland $4"; fi
"#;

fn version(text: &str) -> Option<Vec<u32>> {
    text.split('.').map(|part| part.parse().ok()).collect()
}

/// Judges `PROBE`'s output: glibc at or above `floor`, no `/nix`, only jail
/// processes visible, exactly `sh` and `dash` on PATH, the lavapipe ICD, the
/// Wayland socket and no Sway command socket.
pub(crate) fn host_probe(text: &str, floor: &str) -> Result<Value> {
    let loader = text.lines().next().unwrap_or_default();
    let glibc = loader
        .split("release version ")
        .nth(1)
        .map(|rest| rest.trim().trim_end_matches('.'))
        .ok_or(format!("the host loader gave no identity: {loader:?}"))?;
    match (version(glibc), version(floor)) {
        (Some(host), Some(floor)) if host >= floor => {}
        _ => {
            return Err(format!(
                "host glibc {glibc} is below the bundle's floor {floor}: {loader}"
            ));
        }
    }
    let (mut tools, mut processes) = (BTreeSet::new(), Vec::new());
    let (mut icd, mut wayland) = (false, false);
    for line in text.lines().skip(1) {
        match line.split_once(' ') {
            Some(("nix", path)) => return Err(format!("the jail reaches /nix: {path}")),
            Some(("path", path)) => {
                tools.insert(path.rsplit('/').next().unwrap_or_default());
            }
            Some(("pid", process)) => {
                let comm = process.split_once(' ').map_or("", |(_, comm)| comm);
                if !matches!(comm, "bwrap" | "sh" | "dash") {
                    return Err(format!("the jail sees an outside process: {process}"));
                }
                processes.push(process.into());
            }
            Some(("icd", _)) => icd = true,
            Some(("ipc", path)) => {
                return Err(format!("the jail reaches Sway's command socket {path}"));
            }
            Some(("wayland", _)) => wayland = true,
            _ => {}
        }
    }
    if tools != BTreeSet::from(["dash", "sh"]) {
        return Err(format!(
            "the jail's PATH holds {tools:?}, not only sh and dash"
        ));
    }
    if !icd {
        return Err(format!("the jail has no lavapipe ICD {ICD}"));
    }
    if !wayland {
        return Err("the jail has no Wayland socket".into());
    }
    Ok(object([
        ("loader", loader.into()),
        ("glibc", glibc.into()),
        ("glibc_floor", floor.into()),
        (
            "path",
            Value::Array(tools.into_iter().map(Value::from).collect()),
        ),
        ("processes", Value::Array(processes)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bench-bundle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::canonicalize(root).unwrap()
    }

    fn fixture_set() -> HostSet {
        HostSet::from_json(
            &Value::parse(
                r#"{"links": [["/lib", "usr/lib"]],
                "files": ["/usr/lib/x86_64-linux-gnu/libc.so.6", "/usr/bin/dash"],
                "lavapipe": ["/usr/lib/x86_64-linux-gnu/libz.so.1",
                    "/usr/lib/x86_64-linux-gnu/libvulkan_lvp.so"],
                "driver": "/usr/lib/x86_64-linux-gnu/libvulkan_lvp.so"}"#,
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn host_set_round_trips_and_keeps_lavapipe_apart() {
        let set = fixture_set();
        assert_eq!(HostSet::from_json(&set.to_json()).unwrap(), set);
        assert!(set.base(Path::new("/usr/lib/x86_64-linux-gnu/libc.so.6")));
        assert!(!set.base(Path::new("/usr/lib/x86_64-linux-gnu/libz.so.1")));
        let no_driver = r#"{"links": [], "files": ["/usr/lib/libc.so.6"], "lavapipe": [], "driver": "/usr/lib/libc.so.6"}"#;
        assert!(HostSet::from_json(&Value::parse(no_driver).unwrap()).is_err());
        assert_eq!(
            set.mounts(Path::new(HOST_ROOT)).unwrap()[..6],
            [
                "--symlink",
                "usr/lib",
                "/lib",
                "--ro-bind",
                "/.host/usr/bin/dash",
                "/usr/bin/dash"
            ]
        );
    }

    #[test]
    fn loader_lists_resolve_and_reject_missing_libraries() {
        let listed = "\tlinux-vdso.so.1 (0x00007ffd)\n\
            \tlibz.so.1 => /lib/x86_64-linux-gnu/libz.so.1 (0x000071)\n\
            \t/lib64/ld-linux-x86-64.so.2 (0x000072)\n";
        assert_eq!(
            listed_libraries(listed).unwrap(),
            [PathBuf::from("/lib/x86_64-linux-gnu/libz.so.1")]
        );
        assert!(listed_libraries("\tlibLLVM.so.19 => not found\n").is_err());
    }

    const UBUNTU: &str = "ld.so (Ubuntu GLIBC 2.39-0ubuntu8.12) stable release version 2.39.\n\
        Copyright (C) 2024 Free Software Foundation, Inc.\n";
    const DEBIAN: &str = "ld.so (Debian GLIBC 2.36-9+deb12u4) stable release version 2.36.\n";
    const FEDORA: &str = "ld.so (GNU libc) stable release version 2.41.\n";
    const JAIL: &str = "path /usr/bin/dash\npath /usr/bin/sh\npath /bin/dash\npath /bin/sh\n\
        pid 1 bwrap\npid 2 sh\npid 7 sh\nicd /usr/share/vulkan/icd.d/lvp_icd.json\n\
        wayland /run/user/1/b001/wayland-1\n";

    #[test]
    fn host_probe_requires_the_floor_and_a_closed_jail() {
        assert!(host_probe(&format!("{UBUNTU}{JAIL}"), "2.39").is_ok());
        assert!(host_probe(&format!("{FEDORA}{JAIL}"), "2.39").is_ok());
        assert!(host_probe(&format!("{DEBIAN}{JAIL}"), "2.39").is_err());
        assert!(host_probe(&format!("{UBUNTU}{JAIL}"), "2.40").is_err());
        assert!(host_probe(JAIL, "2.39").is_err());
        for (extra, why) in [
            ("nix /proc/12/root/nix\n", "reaches /nix"),
            ("path /usr/bin/nix\n", "PATH"),
            ("pid 40 sway\n", "outside process"),
            ("ipc /run/user/1/b001-ipc/sway.sock\n", "command socket"),
        ] {
            let error = host_probe(&format!("{UBUNTU}{JAIL}{extra}"), "2.39").unwrap_err();
            assert!(error.contains(why), "{error}");
        }
        for missing in ["icd ", "wayland "] {
            let jail = JAIL.replace(missing, "other ");
            assert!(host_probe(&format!("{UBUNTU}{jail}"), "2.39").is_err());
        }
    }

    #[test]
    fn origins_admit_only_the_tree_and_the_declared_host_set() {
        let set = fixture_set();
        let out = Path::new("/evidence");
        let tree = Path::new("/evidence/bundle/eon-1-x86_64-linux");
        let store = "/evidence/bundle/eon-1-x86_64-linux/nix/store";
        let fzf = PathBuf::from(format!("{store}/a-fzf/bin/fzf"));
        let venus = PathBuf::from(format!("{store}/b-venus/bin/.yazelix-venus-wrapped"));
        let libc = "7f00-7f10 r-xp 0 08:01 1 /usr/lib/x86_64-linux-gnu/libc.so.6\n";
        let maps = format!(
            "{libc}7f10-7f20 rw-s 0 00:01 2 /memfd:wayland-shm (deleted)\n55-56 rw-p 0 0:0 0 [heap]\n"
        );
        let lavapipe = format!(
            "7f20-7f30 r-xp 0 08:01 3 {store}/c-vulkan-loader/lib/libvulkan.so.1.4\n\
             7f30-7f40 r-xp 0 08:01 4 /usr/lib/x86_64-linux-gnu/libvulkan_lvp.so\n\
             7f40-7f50 r-xp 0 08:01 5 /usr/lib/x86_64-linux-gnu/libz.so.1\n"
        );
        let env = b"PATH=/usr/bin:/bin\0EON_RUNTIME_DIR=/run/user/1/b001\0";
        let check = |exe: &Path, env: &[u8], maps: &str| origin(exe, env, maps, out, tree, &set);
        assert_eq!(
            check(&fzf, env, &maps)
                .unwrap()
                .get("role")
                .unwrap()
                .as_str(),
            Some("fzf")
        );
        let venus_ok = check(&venus, env, &format!("{maps}{lavapipe}")).unwrap();
        assert!(venus_ok.get("vulkan_loader").unwrap().as_str().is_some());
        assert!(check(Path::new("/usr/bin/dash"), env, &maps).is_ok());
        for (exe, env, maps, why) in [
            (
                &fzf,
                &env[..],
                format!("{maps}7f-80 r-xp 0 0:0 9 /nix/store/x-zlib/lib/libz.so.1\n"),
                "maps /nix",
            ),
            (
                &fzf,
                b"PATH=/usr/bin:/nix/var/nix/profiles/default/bin\0",
                maps.clone(),
                "names /nix",
            ),
            (
                &fzf,
                &env[..],
                format!("{maps}{lavapipe}"),
                "fzf (/evidence/bundle/eon-1-x86_64-linux/nix/store/a-fzf/bin/fzf) maps /usr/lib/x86_64-linux-gnu/libvulkan_lvp.so",
            ),
            (
                &venus,
                &env[..],
                format!(
                    "{maps}7f-80 r-xp 0 0:0 9 /usr/lib/x86_64-linux-gnu/libvulkan.so.1\n{lavapipe}"
                ),
                "libvulkan.so.1\n",
            ),
            (
                &venus,
                &env[..],
                format!("{maps}{}", lavapipe.lines().next().unwrap()),
                "Venus must map",
            ),
            (
                &PathBuf::from("/usr/bin/fzf"),
                &env[..],
                maps.clone(),
                "outside the bundle",
            ),
        ] {
            let error = check(exe, env, &maps).unwrap_err();
            assert!(error.contains(why.trim_end()), "{why}: {error}");
        }
    }

    fn extraction(name: &str) -> (PathBuf, PathBuf) {
        let root = scratch(name);
        let tree = root.join("eon-1-x86_64-linux");
        let bin = tree.join("nix/store/p-eon-1/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("eon"), "launcher").unwrap();
        fs::create_dir(tree.join("nix/store/p-eon-1/share")).unwrap();
        fs::write(
            tree.join("nix/store/p-eon-1/share/eon-source-receipt.json"),
            r#"{"build":"g1-fixture"}"#,
        )
        .unwrap();
        fs::write(
            tree.join("provenance.json"),
            r#"{"package":"/nix/store/p-eon-1","relink":{"glibc_floor":"2.39"}}"#,
        )
        .unwrap();
        symlink("nix/store/p-eon-1/bin/eon", tree.join("eon")).unwrap();
        (root, tree)
    }

    #[test]
    fn extraction_stays_inside_the_archive_and_removes_read_only_trees() {
        let open = |root: &Path| Bundle::open(Path::new("a.tar.zst"), "0".into(), 0, root);
        let (root, tree) = extraction("open");
        let bundle = open(&root).unwrap();
        assert_eq!(bundle.product, tree.join("nix/store/p-eon-1"));
        assert_eq!(bundle.glibc_floor, "2.39");
        assert_eq!(
            bundle.receipt.get("build").and_then(Value::as_str),
            Some("g1-fixture")
        );

        // A link out of the extraction must survive removal untouched.
        let outside = scratch("outside");
        fs::write(outside.join("kept"), "kept").unwrap();
        symlink(&outside, tree.join("nix/escape")).unwrap();
        for dir in [
            tree.join("nix/store/p-eon-1/bin"),
            tree.join("nix/store"),
            tree.join("nix"),
        ] {
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        }
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o555)).unwrap();
        remove(&root).unwrap();
        assert!(!root.exists());
        assert_eq!(
            fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
            0o555
        );
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(outside).unwrap();

        let (root, tree) = extraction("escape");
        fs::remove_file(tree.join("eon")).unwrap();
        symlink("/nix/store/p-eon-1/bin/eon", tree.join("eon")).unwrap();
        assert!(open(&root).is_err());
        remove(&root).unwrap();

        // Metadata must come from the extraction itself, never through a link
        // to a matching file elsewhere, and the receipt is mandatory.
        let (root, tree) = extraction("receipt");
        let receipt = tree.join("nix/store/p-eon-1/share/eon-source-receipt.json");
        let elsewhere = scratch("elsewhere");
        fs::rename(&receipt, elsewhere.join("receipt.json")).unwrap();
        assert!(open(&root).unwrap_err().contains("source receipt"));
        symlink(elsewhere.join("receipt.json"), &receipt).unwrap();
        assert!(open(&root).unwrap_err().contains("outside the archive"));
        fs::remove_file(&receipt).unwrap();
        fs::write(&receipt, "{}").unwrap();
        assert!(open(&root).unwrap_err().contains("no build"));
        fs::rename(
            tree.join("provenance.json"),
            elsewhere.join("provenance.json"),
        )
        .unwrap();
        symlink(
            elsewhere.join("provenance.json"),
            tree.join("provenance.json"),
        )
        .unwrap();
        assert!(open(&root).unwrap_err().contains("outside the archive"));
        remove(&root).unwrap();

        // A top-level link out of the extraction is refused.
        let root = scratch("top");
        symlink(elsewhere.parent().unwrap(), root.join("eon-1-x86_64-linux")).unwrap();
        assert!(open(&root).unwrap_err().contains("top-level entry"));
        remove(&root).unwrap();
        remove(&elsewhere).unwrap();

        let (root, tree) = extraction("package");
        fs::write(
            tree.join("provenance.json"),
            r#"{"package":"/nix/store/../../..","relink":{"glibc_floor":"2.39"}}"#,
        )
        .unwrap();
        assert!(open(&root).unwrap_err().contains("outside the archive"));
        fs::create_dir(root.join("second")).unwrap();
        assert!(open(&root).unwrap_err().contains("exactly one"));
        remove(&root).unwrap();
    }

    #[test]
    fn jail_environment_rejects_nix_values_but_not_bundle_paths() {
        let jail = Jail {
            bwrap: "bwrap".into(),
            mounts: Vec::new(),
            host: fixture_set(),
            xdg: "/evidence/xdg".into(),
            user: File::open("/dev/null").unwrap(),
            pid: File::open("/dev/null").unwrap(),
            namespace: PathBuf::new(),
            init: 0,
        };
        let env = |value: &str| BTreeMap::from([("SOME".to_owned(), value.to_owned())]);
        let args = jail
            .environment(&env("/evidence/bundle/eon/nix/store/a/lib"))
            .unwrap();
        assert!(args.windows(3).any(|w| w == ["--setenv", "PATH", PATH]));
        let mut sway = env("x");
        sway.insert("SWAYSOCK".into(), "/run/user/1/b001-ipc/sway.sock".into());
        assert!(
            !jail
                .environment(&sway)
                .unwrap()
                .contains(&"SWAYSOCK".to_owned())
        );
        assert!(jail.environment(&env("/a:/nix/store/b")).is_err());
        assert!(
            jail.environment(&env("/nix/var/nix/profiles/default/bin"))
                .is_err()
        );
        // Every candidate program goes through the jail, wherever it lives.
        let argv = jail
            .join(&env("x"), Path::new("/opt/prefix/bin/eon"), &["versions"])
            .unwrap();
        assert_eq!(
            argv[..4],
            [
                "--userns",
                &jail.user.as_raw_fd().to_string(),
                "--pidns",
                &jail.pid.as_raw_fd().to_string()
            ]
        );
        assert_eq!(
            argv[argv.len() - 3..],
            ["--", "/opt/prefix/bin/eon", "versions"]
        );
    }

    /// A holder plus joined calls on this host: no `/nix`, a fresh procfs that
    /// shows only jail processes, a detached child that outlives its call, and
    /// an outer view that still sees it.
    #[test]
    fn joined_calls_share_one_closed_pid_namespace() {
        let host = match HostSet::discover() {
            Ok(host) => host,
            Err(error) => {
                assert!(error.starts_with("--bundle needs"), "{error}");
                return;
            }
        };
        let root = scratch("jail");
        let bwrap =
            PathBuf::from(std::env::var_os("EON_BENCH_BWRAP").unwrap_or_else(|| "bwrap".into()));
        let mounts = Jail::mounts(&host, Path::new("/"), &[&root]).unwrap();
        let env = BTreeMap::from([("HOME".to_owned(), root.display().to_string())]);
        let mut holder = None;
        let jail = Jail::start(
            bwrap,
            host,
            mounts,
            root.join("xdg"),
            &env,
            |program, args| {
                holder = Some(
                    Command::new(program)
                        .args(args)
                        .stdin(Stdio::piped())
                        .spawn()
                        .map_err(failure)?,
                );
                Ok(())
            },
        )
        .unwrap();
        let mut holder = holder.unwrap();
        let call = |args: &[&str], stdin: Stdio| {
            let argv = jail.join(&env, Path::new("/bin/sh"), args).unwrap();
            // Bounded, so a descriptor leak fails the test instead of hanging it.
            let mut command = Command::new("timeout");
            command
                .arg("10s")
                .arg(jail.bwrap())
                .args(&argv)
                .stdin(stdin);
            jail.inherit(|| command.output()).unwrap().unwrap()
        };
        // The detached reader blocks on this test's pipe after its call exits.
        let (keep, writer) = std::io::pipe().unwrap();
        let first = call(
            &[
                "-c",
                "exec 3<&0; (read -r line <&3) >/dev/null 2>&1 & echo $!",
            ],
            Stdio::from(keep),
        );
        assert!(first.status.success(), "{first:?}");
        let detached = String::from_utf8(first.stdout).unwrap().trim().to_owned();
        // A socket the jail shares, and a file standing in for Sway's command
        // socket in a directory the jail does not bind.
        let wayland = root.join("wayland-1");
        let listener = std::os::unix::net::UnixListener::bind(&wayland).unwrap();
        let outside = std::env::current_dir()
            .unwrap()
            .join(format!(".bundle-ipc-{}", std::process::id()));
        fs::write(&outside, "").unwrap();
        // The jail's processes carry no candidate marker in their environment;
        // the audit selects them by namespace alone.
        let audit = jail.audit(&root, &root.join("tree"), &["dash"]);
        let probe = call(
            &[
                "-c",
                PROBE,
                "sh",
                LOADER,
                ICD,
                plain(&outside).unwrap(),
                plain(&wayland).unwrap(),
            ],
            Stdio::null(),
        );
        drop(listener);
        fs::remove_file(&outside).unwrap();
        let second = call(
            &["-c", &format!("test -e /proc/{detached} && echo alive")],
            Stdio::null(),
        );
        let text = String::from_utf8_lossy(&second.stdout).into_owned();
        drop(writer);
        drop(holder.stdin.take());
        let _ = holder.wait();
        fs::remove_dir_all(&root).unwrap();
        assert!(text.contains("alive"), "{text}");
        let audit = audit.unwrap();
        let jailed = audit.get("jailed").and_then(Value::as_array).unwrap();
        // The jail init, the holder's shell and the detached reader.
        assert!(jailed.len() >= 3, "{}", audit.to_pretty());
        assert!(probe.status.success(), "{probe:?}");
        let probe = String::from_utf8_lossy(&probe.stdout);
        let checked = host_probe(&probe, "2.0");
        assert!(checked.is_ok(), "{checked:?}\n{probe}");
    }

    fn on_path(name: &str) -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| panic!("this test needs {name} on PATH"))
    }

    #[test]
    fn extraction_hashes_the_bytes_it_extracts_and_leaves_failures_removable() {
        let (fixture, _) = extraction("stream-fixture");
        let work = scratch("stream");
        let archive = work.join("a.tar.gz");
        let packed = Command::new(on_path("tar"))
            .arg("--create")
            .arg("--gzip")
            .arg("--file")
            .arg(&archive)
            .arg("--directory")
            .arg(&fixture)
            .arg("eon-1-x86_64-linux")
            .status()
            .unwrap();
        assert!(packed.success());
        let bytes = fs::read(&archive).unwrap();
        let (tar, gzip) = (on_path("tar"), on_path("gzip"));
        let tools = (tar.as_os_str(), gzip.as_os_str());
        let deadline = || Instant::now() + Duration::from_secs(10);

        // A pipe yields its bytes once: the hash must come from the stream tar
        // extracts, never from a second read of the archive.
        let fifo = work.join("archive.fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let feed = {
            let (fifo, bytes) = (fifo.clone(), bytes.clone());
            std::thread::spawn(move || fs::write(fifo, bytes))
        };
        let root = work.join("streamed");
        let (sent, received) = std::sync::mpsc::channel();
        {
            let (fifo, root, tar, gzip) = (fifo.clone(), root.clone(), tar.clone(), gzip.clone());
            std::thread::spawn(move || {
                let tools = (tar.as_os_str(), gzip.as_os_str());
                let _ = sent.send(Bundle::extract(&fifo, &root, tools, deadline()));
            });
        }
        // Bounded: a second open of the drained pipe would block forever.
        let bundle = received
            .recv_timeout(Duration::from_secs(20))
            .expect("extraction must read the archive once")
            .unwrap();
        feed.join().unwrap().unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        assert_eq!(bundle.sha256, hasher.hex());
        assert_eq!(bundle.bytes, bytes.len() as u64);
        assert_eq!(bundle.glibc_floor, "2.39");

        // A truncated archive fails, and its partial root stays removable.
        let truncated = work.join("truncated.tar.gz");
        fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        let partial = work.join("partial");
        assert!(Bundle::extract(&truncated, &partial, tools, deadline()).is_err());
        assert!(partial.is_dir());
        remove(&partial).unwrap();
        remove(&work).unwrap();
        remove(&fixture).unwrap();
    }

    #[test]
    fn occupied_names_processes_that_run_from_the_extraction() {
        let root = scratch("occupied");
        let shell = root.join("sh");
        fs::copy("/bin/sh", &shell).unwrap();
        let mut child = Command::new(&shell)
            .args(["-c", "read -r line"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let busy = occupied(&root).unwrap();
        drop(child.stdin.take());
        child.wait().unwrap();
        assert_eq!(busy, [child.id()]);
        assert!(occupied(&root).unwrap().is_empty());
        remove(&root).unwrap();
    }

    #[test]
    fn info_reads_end_at_the_deadline_and_at_a_silent_exit() {
        let (mut reader, writer) = std::io::pipe().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(read_info(
                &mut reader,
                Instant::now() + Duration::from_millis(200),
            ));
        });
        let stalled = received
            .recv_timeout(Duration::from_secs(5))
            .expect("read_info must return at its deadline while the holder stalls");
        drop(writer);
        assert!(stalled.unwrap_err().contains("did not report"));

        let (mut reader, writer) = std::io::pipe().unwrap();
        drop(writer);
        let silent = read_info(&mut reader, Instant::now() + Duration::from_secs(5));
        assert!(silent.unwrap_err().contains("exited before"));
    }

    #[test]
    fn host_set_errors_name_the_missing_prerequisite() {
        assert_eq!(
            prerequisite("--bundle needs Mesa".into()),
            "--bundle needs Mesa"
        );
        assert!(
            prerequisite("bad.json names no library_path".into()).starts_with("--bundle needs")
        );
    }
}
