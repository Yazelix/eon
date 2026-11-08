//! Shared private display, environment and owned-process lifecycle for display suites.
use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

type Result<T = ()> = std::result::Result<T, String>;
fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

struct Roots(Vec<PathBuf>, bool);
impl Roots {
    fn create() -> Result<Self> {
        // SAFETY: geteuid has no side effects or arguments.
        let parent = PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }));
        let mut roots = Self(Vec::new(), false);
        for index in 0..4096 {
            let path = parent.join(format!(
                "b{:03x}",
                (SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(failure)?
                    .as_nanos() as usize
                    + index)
                    % 4096
            ));
            if path.as_os_str().len() > 19 {
                return Err("Eon test needs a runtime root <=19 bytes".into());
            }
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => roots.0.push(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(failure(error)),
            }
            if roots.0.len() == 2 {
                return Ok(roots);
            }
        }
        Err("no private runtime roots available".into())
    }
}
impl Drop for Roots {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        for root in &self.0 {
            let _ = fs::remove_dir_all(root);
        }
    }
}

pub(crate) struct Processes {
    pub(crate) children: Vec<Child>,
    pub(crate) known: BTreeMap<u32, String>,
    marker: String,
}
fn process_state(pid: u32) -> Option<(String, bool, u32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some((
        fields.get(19)?.to_string(),
        fields.first() != Some(&"Z"),
        fields.get(1)?.parse().ok()?,
    ))
}
pub(crate) fn live_identity(pid: u32, identity: &str) -> bool {
    process_state(pid).is_some_and(|(now, running, _)| now == identity && running)
}
#[cfg(target_os = "linux")]
fn signal_owned(pid: u32, identity: &str, signal: i32) {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: pidfd_open has no pointers and returns a new owned descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return;
    }
    // SAFETY: the successful syscall returned this fresh descriptor.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) };
    if live_identity(pid, identity) {
        // SAFETY: the pidfd pins this exact process even if its numeric PID is reused.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn signal_owned(_: u32, _: &str, _: i32) {}
impl Processes {
    pub(crate) fn remember(&mut self, pid: u32) -> Option<String> {
        let (identity, running, _) = process_state(pid)?;
        let owned = fs::read(format!("/proc/{pid}/environ"))
            .ok()?
            .split(|&b| b == 0)
            .any(|item| item == self.marker.as_bytes());
        if !running || !owned || !live_identity(pid, &identity) {
            return None;
        }
        self.known.insert(pid, identity.clone());
        Some(identity)
    }
    fn descendants(&mut self, pid: u32, identity: &str) {
        if !live_identity(pid, identity) {
            return;
        }
        let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) else {
            return;
        };
        if !live_identity(pid, identity) {
            return;
        }
        for child in children
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
        {
            let Some((start, true, parent)) = process_state(child) else {
                continue;
            };
            if parent != pid || !live_identity(pid, identity) {
                continue;
            }
            // Ownership and both identities must survive the environment read.
            if self.remember(child).as_deref() != Some(start.as_str()) {
                self.known.remove(&child);
                continue;
            }
            if !live_identity(pid, identity)
                || process_state(child).is_none_or(|(now, _, parent)| now != start || parent != pid)
            {
                self.known.remove(&child);
                continue;
            }
            self.descendants(child, &start);
        }
    }
    pub(crate) fn cleanup(&mut self) -> bool {
        // Saved PIDs are never roots for traversal until their start identity matches.
        for (pid, identity) in self.known.clone() {
            self.descendants(pid, &identity);
        }
        for signal in [libc::SIGTERM, libc::SIGKILL] {
            for (&pid, identity) in &self.known {
                signal_owned(pid, identity, signal);
            }
            let deadline = Instant::now() + Duration::from_secs(1);
            while self
                .known
                .iter()
                .any(|(&pid, identity)| live_identity(pid, identity))
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
        }
        for child in &mut self.children {
            let _ = child.try_wait();
        }
        !self
            .known
            .iter()
            .any(|(&pid, identity)| live_identity(pid, identity))
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub(crate) enum BusPolicy {
    Session,
    Private,
}

/// Suites supply their own config, trace consumer and workloads. The launcher owns
/// only the private display and process lifetime, including failure cleanup.
pub(crate) struct Headless {
    pub(crate) processes: Processes,
    roots: Roots,
    eon_child: Option<usize>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) eon: PathBuf,
    pub(crate) out: PathBuf,
}
impl Headless {
    pub(crate) fn new(out: &Path, eon: &Path) -> Result<Self> {
        let eon = candidate(eon)?;
        let roots = Roots::create()?;
        let out = fs::canonicalize(out).map_err(failure)?;
        let runtime = roots.0[0].clone();
        let eon_root = roots.0[1].clone();
        for directory in ["home", "config", "state"] {
            fs::create_dir(out.join(directory)).map_err(failure)?;
        }
        let sway_config = out.join("sway.conf");
        fs::write(&sway_config, "output * mode 1280x800\noutput * bg #202020 solid_color\nseat seat0 fallback true\nxwayland disable\ndefault_border none\nfocus_follows_mouse no\nfor_window [app_id=\"eon\"] fullscreen enable\n").map_err(failure)?;
        let env = BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").map_err(failure)?),
            ("HOME".into(), out.join("home").display().to_string()),
            (
                "EON_CONFIG_HOME".into(),
                out.join("config").display().to_string(),
            ),
            (
                "XDG_STATE_HOME".into(),
                out.join("state").display().to_string(),
            ),
            ("XDG_RUNTIME_DIR".into(), runtime.display().to_string()),
            ("EON_RUNTIME_DIR".into(), eon_root.display().to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".into(),
                format!("unix:path={}/bus", runtime.display()),
            ),
            ("WLR_BACKENDS".into(), "headless".into()),
            ("WLR_RENDERER".into(), "pixman".into()),
            ("WLR_LIBINPUT_NO_DEVICES".into(), "1".into()),
            ("LANG".into(), "C.UTF-8".into()),
        ]);
        Ok(Self {
            processes: Processes {
                children: Vec::new(),
                known: BTreeMap::new(),
                marker: format!("EON_RUNTIME_DIR={}", eon_root.display()),
            },
            roots,
            eon_child: None,
            env,
            eon,
            out,
        })
    }
    pub(crate) fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env_clear().envs(&self.env).stdin(Stdio::null());
        command
    }
    pub(crate) fn spawn(&mut self, program: &str, args: &[&str], log: &str) -> Result {
        let output = File::create(self.out.join(log)).map_err(failure)?;
        let child = self
            .command(program)
            .args(args)
            .stdout(output.try_clone().map_err(failure)?)
            .stderr(output)
            .spawn()
            .map_err(failure)?;
        self.processes.remember(child.id());
        self.processes.children.push(child);
        Ok(())
    }
    pub(crate) fn start_compositor(&mut self, bus: BusPolicy) -> Result {
        let runtime = PathBuf::from(&self.env["XDG_RUNTIME_DIR"]);
        let sway_config = self.out.join("sway.conf");
        let address = self.env["DBUS_SESSION_BUS_ADDRESS"].clone();
        match bus {
            BusPolicy::Session => self.spawn(
                "dbus-daemon",
                &["--session", "--nofork", &format!("--address={address}")],
                "dbus.log",
            )?,
            BusPolicy::Private => {
                let bus_config = self.out.join("dbus.conf");
                // No host service directories: the private bus must not activate desktop daemons.
                fs::write(
                    &bus_config,
                    r#"<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                )
                .map_err(failure)?;
                self.spawn(
                    "dbus-daemon",
                    &[
                        "--nofork",
                        &format!(
                            "--config-file={}",
                            bus_config.to_str().ok_or("non-UTF8 output path")?
                        ),
                        &format!("--address={address}"),
                    ],
                    "dbus.log",
                )?;
                wait_for_bus(
                    self.processes
                        .children
                        .last_mut()
                        .ok_or("missing D-Bus child")?,
                    &runtime.join("bus"),
                )?;
            }
        }
        self.spawn(
            "sway",
            &[
                "--unsupported-gpu",
                "--config",
                sway_config.to_str().ok_or("non-UTF8 output path")?,
            ],
            "sway.log",
        )?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self
                .processes
                .children
                .last_mut()
                .ok_or("missing Sway child")?
                .try_wait()
                .map_err(failure)?
                .is_some()
            {
                return Err("private Sway exited; see sway.log".into());
            }
            let files: Vec<_> = fs::read_dir(&runtime)
                .map_err(failure)?
                .filter_map(|entry| entry.ok())
                .collect();
            let wayland = files.iter().find(|entry| {
                entry.file_name().to_string_lossy().starts_with("wayland-")
                    && !entry.file_name().to_string_lossy().ends_with(".lock")
            });
            let ipc = files.iter().find(|entry| {
                entry.file_name().to_string_lossy().starts_with("sway-ipc.")
                    && entry.file_name().to_string_lossy().ends_with(".sock")
            });
            if let (Some(wayland), Some(ipc)) = (wayland, ipc) {
                self.env.insert(
                    "WAYLAND_DISPLAY".into(),
                    wayland.file_name().to_string_lossy().into(),
                );
                self.env
                    .insert("SWAYSOCK".into(), ipc.path().display().to_string());
                break;
            }
            if Instant::now() >= deadline {
                return Err("private Sway did not become ready; see sway.log".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
    pub(crate) fn start_eon(&mut self) -> Result {
        if self.eon_child.is_some() {
            return Err("private Eon already started".into());
        }
        let eon = self
            .eon
            .to_str()
            .ok_or("non-UTF8 candidate path")?
            .to_owned();
        self.spawn(&eon, &[], "eon.log")?;
        self.eon_child = Some(self.processes.children.len() - 1);
        Ok(())
    }
    pub(crate) fn stop_eon(&mut self) -> Result {
        let Some(index) = self.eon_child else {
            return Ok(());
        };
        let stopped = self
            .command("timeout")
            .arg("10s")
            .arg(&self.eon)
            .args(["window", "stop", "all", "--json"])
            .output()
            .map_err(failure)
            .and_then(|output| {
                if output.status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "private Stop failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ))
                }
            });
        if stopped.is_ok() {
            let deadline = Instant::now() + Duration::from_secs(15);
            let child = &mut self.processes.children[index];
            loop {
                if child.try_wait().map_err(failure)?.is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("private supervisor did not exit after Stop".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        stopped
    }
}
impl Drop for Headless {
    fn drop(&mut self) {
        if !self.processes.cleanup() {
            self.roots.1 = true;
            eprintln!(
                "eon-bench: private processes still live; preserving {:?}",
                self.roots.0
            );
        }
    }
}
pub(crate) fn candidate(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path).map_err(failure)?;
    if !path.starts_with("/nix/store") || !path.is_file() {
        return Err("--eon must name a Nix candidate, not a profile/checkout fallback".into());
    }
    Ok(path)
}
fn wait_for_bus(child: &mut Child, path: &Path) -> Result {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().map_err(failure)?.is_some() {
            return Err("private D-Bus exited; see dbus.log".into());
        }
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("private D-Bus did not become ready; see dbus.log".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Explicit GPU selection for measured suites; response leaves its existing
/// automatic adapter policy alone. Hardware needs a GPU compositor to advertise dmabuf.
pub(crate) struct Renderer(pub(crate) String);
impl Renderer {
    pub(crate) fn apply(&self, launch: &mut Headless) -> Result {
        if let Some(node) = self.render_node()? {
            let egl = std::env::var("EON_BENCH_EGL")
                .map_err(|_| "missing EON_BENCH_EGL (use the Nix bench package)")?;
            if !Path::new(&egl).is_file() {
                return Err("Mesa EGL vendor file is missing".into());
            }
            launch
                .env
                .insert("__EGL_VENDOR_LIBRARY_FILENAMES".into(), egl);
            for (source, target) in [
                ("EON_BENCH_GBM", "GBM_BACKENDS_PATH"),
                ("EON_BENCH_DRI", "LIBGL_DRIVERS_PATH"),
                ("EON_BENCH_VULKAN", "VK_DRIVER_FILES"),
            ] {
                let directory = std::env::var(source)
                    .map_err(|_| format!("missing {source} (use the Nix bench package)"))?;
                if !Path::new(&directory).is_dir() {
                    return Err(format!("{source} directory is missing"));
                }
                launch.env.insert(target.into(), directory);
            }
            launch.env.insert("WLR_RENDERER".into(), "gles2".into());
            launch
                .env
                .insert("WLR_RENDER_DRM_DEVICE".into(), node.display().to_string());
        } else {
            let icd = std::env::var("EON_BENCH_LAVAPIPE")
                .map_err(|_| "missing EON_BENCH_LAVAPIPE (use the Nix bench package)")?;
            if !Path::new(&icd).is_file() {
                return Err("lavapipe ICD is missing".into());
            }
            launch.env.insert("VK_DRIVER_FILES".into(), icd);
        }
        Ok(())
    }

    pub(crate) fn render_node(&self) -> Result<Option<&Path>> {
        if self.0 == "lavapipe" {
            return Ok(None);
        }
        let path = Path::new(&self.0);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if path.parent() != Some(Path::new("/dev/dri"))
            || !name
                .strip_prefix("renderD")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            || !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_char_device())
        {
            return Err(
                "--renderer must be lavapipe or an existing /dev/dri/renderD<N> device".into(),
            );
        }
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_exited_bus_is_a_startup_failure() {
        let mut child = Command::new("sh").args(["-c", "exit 1"]).spawn().unwrap();
        child.wait().unwrap();
        assert!(
            wait_for_bus(&mut child, Path::new("/missing-private-bus"))
                .unwrap_err()
                .contains("D-Bus exited")
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_refuses_reused_roots_and_unowned_descendants() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 2 & wait"])
            .env_remove("EON_RUNTIME_DIR")
            .spawn()
            .unwrap();
        let mut processes = Processes {
            children: Vec::new(),
            known: BTreeMap::from([(child.id(), "different-lifetime".into())]),
            marker: "EON_RUNTIME_DIR=private-bench-test".into(),
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        processes.descendants(child.id(), "different-lifetime");
        assert_eq!(processes.known.len(), 1);
        let identity = process_state(child.id()).unwrap().0;
        processes.descendants(child.id(), &identity);
        assert_eq!(processes.known.len(), 1);
        assert!(processes.remember(child.id()).is_none());
        assert!(processes.cleanup());
        assert!(child.try_wait().unwrap().is_none());
        // Let the short-lived unowned fixture reap its own child.
        assert!(child.wait().unwrap().success());
    }
}
