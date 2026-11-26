use super::*;
#[cfg(target_os = "linux")]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};

fn monitor(outputs: &Value, size: OutputSize) -> bool {
    let Some(outputs) = outputs.as_array() else {
        return false;
    };
    let mut active = outputs
        .iter()
        .filter(|o| o.get("disabled").and_then(Value::as_bool) != Some(true));
    active.next().is_some_and(|o| {
        size.matches(o)
            && o.get("disabled").and_then(Value::as_bool) == Some(false)
            && o.get("focused").and_then(Value::as_bool) == Some(true)
            && o.get("scale").and_then(Value::as_f64) == Some(1.0)
            && o.get("hardwareDetails")
                .and_then(|h| h.get("backend"))
                .and_then(Value::as_str)
                == Some("wayland")
    }) && active.next().is_none()
}

#[cfg(any(target_os = "linux", test))]
fn gl_renderer(log: &str) -> Result<&str> {
    let renderer = log
        .lines()
        .find_map(|line| line.split_once("Renderer: ").map(|(_, value)| value.trim()))
        .filter(|value| !value.is_empty())
        .ok_or("missing GL renderer in hyprland.log")?;
    let name = renderer.to_ascii_lowercase();
    if name.contains("llvmpipe") || name.contains("softpipe") {
        return Err(format!(
            "Hyprland selected a software GL renderer: {renderer}"
        ));
    }
    Ok(renderer)
}

#[cfg(target_os = "linux")]
pub(super) struct Tools {
    pub(super) sway: PathBuf,
    hyprland: PathBuf,
    hyprctl: PathBuf,
}

#[cfg(target_os = "linux")]
fn open_render_node(node: &Path) -> Result {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(node)
        .map(drop)
        .map_err(|err| {
            format!(
                "cannot open {} read/write: {err}; {NESTED_HINT}",
                node.display()
            )
        })
}

#[cfg(target_os = "linux")]
impl Tools {
    pub(super) fn preflight(renderer: &Renderer) -> Result<Self> {
        open_render_node(renderer.render_node()?.ok_or(NESTED_HINT)?)?;
        let tool = |name: &str| {
            let path =
                terminals::tool(name).map_err(|err| format!("{name}: {err}; {NESTED_HINT}"))?;
            if fs::metadata(&path).map_err(error)?.permissions().mode() & 0o111 == 0 {
                return Err(format!("{name} is not executable; {NESTED_HINT}"));
            }
            Ok(path)
        };
        Ok(Self {
            sway: tool("sway-xdg6")?,
            hyprland: tool("hyprland")?,
            hyprctl: tool("hyprctl")?,
        })
    }
}

#[cfg(target_os = "linux")]
fn socket(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

#[cfg(target_os = "linux")]
fn display(runtime: &Path, parent: &str) -> Result<Option<String>> {
    endpoint(runtime, |e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        name != parent
            && name.starts_with("wayland-")
            && !name.ends_with(".lock")
            && socket(&e.path())
    })
}

#[cfg(target_os = "linux")]
fn instance(runtime: &Path) -> Result<Option<String>> {
    let hypr = runtime.join("hypr");
    if !fs::symlink_metadata(&hypr).is_ok_and(|m| m.is_dir()) {
        return Ok(None);
    }
    endpoint(&hypr, |e| {
        e.file_type().is_ok_and(|t| t.is_dir()) && socket(&e.path().join(".socket.sock"))
    })
}

#[cfg(target_os = "linux")]
fn endpoint(dir: &Path, accepts: impl Fn(&fs::DirEntry) -> bool) -> Result<Option<String>> {
    let mut found = None;
    for entry in fs::read_dir(dir).map_err(error)? {
        let entry = entry.map_err(error)?;
        if accepts(&entry)
            && found
                .replace(
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "non-UTF8 socket identity")?,
                )
                .is_some()
        {
            return Err(format!("ambiguous private endpoints in {}", dir.display()));
        }
    }
    Ok(found)
}

#[cfg(target_os = "linux")]
fn view(tree: &Value, size: OutputSize) -> bool {
    terminals::find_window(tree, &|node| {
        (node.get("app_id")?.as_str()? == "aquamarine"
            && node.get("focused")?.as_bool()?
            && node.get("visible")?.as_bool()?
            && size.matches(node.get("rect")?))
        .then_some(())
    })
    .is_some()
}

#[cfg(target_os = "linux")]
pub(super) struct Nested<'a> {
    tools: &'a Tools,
    parent: String,
    signature: String,
    _parent_keyboard: seat::Keyboard,
    child: usize,
}

#[cfg(target_os = "linux")]
impl<'a> Nested<'a> {
    pub(super) fn start(
        tools: &'a Tools,
        state: &mut Option<Self>,
        launch: &mut Headless,
        deadline: Instant,
    ) -> Result {
        let runtime = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]);
        let parent = launch.env["WAYLAND_DISPLAY"].clone();
        if display(&runtime, "")?.as_ref() != Some(&parent) {
            return Err("missing private Sway socket".into());
        }
        let keyboard = seat::Keyboard::new(launch)?;
        let config = launch.out.join("hyprland.lua");
        fs::write(
            &config,
            format!(
                r#"
hl.monitor({{ output = "", mode = "preferred", position = "0x0", scale = 1 }})
hl.config({{
  general = {{ gaps_in = 0, gaps_out = 0, border_size = 0 }},
  input = {{ kb_layout = "us", repeat_delay = {REPEAT_DELAY}, repeat_rate = {REPEAT_RATE} }},
  animations = {{ enabled = false }},
  debug = {{ disable_logs = false, enable_stdout_logs = true }},
  xwayland = {{ enabled = false }},
  cursor = {{ no_hardware_cursors = 1 }},
  misc = {{ disable_hyprland_logo = true, force_default_wallpaper = 0 }},
  ecosystem = {{ no_update_news = true, no_donation_nag = true }},
}})
"#
            ),
        )
        .map_err(error)?;
        let program = tools.hyprland.to_str().ok_or("non-UTF8 Hyprland path")?;
        let config = config.to_str().ok_or("non-UTF8 Hyprland config")?;
        let mut env = launch.env.clone();
        for (key, value) in [("HYPRLAND_NO_SD_VARS", "1"), ("LIBSEAT_BACKEND", "seatd")] {
            env.insert(key.into(), value.into());
        }
        for (key, source) in [
            ("__EGL_VENDOR_LIBRARY_FILENAMES", "EON_BENCH_HYPRLAND_EGL"),
            ("GBM_BACKENDS_PATH", "EON_BENCH_HYPRLAND_GBM"),
            ("LIBGL_DRIVERS_PATH", "EON_BENCH_HYPRLAND_DRI"),
        ] {
            let path =
                std::env::var(source).map_err(|_| format!("missing {source}; {NESTED_HINT}"))?;
            env.insert(key.into(), path);
        }
        let parent_env = std::mem::replace(&mut launch.env, env);
        let spawned = launch.spawn(program, &["--config", config], "hyprland.log");
        launch.env = parent_env;
        spawned?;
        let child = launch.processes.children.len() - 1;
        let nested = state.insert(Self {
            tools,
            parent,
            signature: String::new(),
            _parent_keyboard: keyboard,
            child,
        });
        let deadline = deadline.min(Instant::now() + Duration::from_secs(15));
        let mut last = String::new();
        wait(deadline, "nested Hyprland", || {
            if launch.processes.children[child]
                .try_wait()
                .map_err(error)?
                .is_some()
            {
                return Err("private Hyprland exited; see hyprland.log".into());
            }
            let (Some(display), Some(signature)) =
                (display(&runtime, &nested.parent)?, instance(&runtime)?)
            else {
                return Ok(false);
            };
            let previous = std::mem::replace(&mut nested.signature, signature);
            let monitors = nested.query(launch, deadline, &["monitors", "all"]);
            if monitors.is_err() {
                nested.signature = previous;
            }
            let tree = terminals::sway(launch, probe(deadline), &["-t", "get_tree", "-r"]);
            match (monitors, tree) {
                (Ok(monitors), Ok(tree))
                    if monitor(&monitors, launch.output_size)
                        && view(&tree, launch.output_size) =>
                {
                    let log = fs::read_to_string(launch.out.join("hyprland.log")).map_err(error)?;
                    let evidence = object([
                        ("monitors", monitors),
                        ("parent", tree),
                        ("instance", nested.signature.as_str().into()),
                        ("gl_renderer", gl_renderer(&log)?.into()),
                    ]);
                    fs::write(launch.out.join("nested-ready.json"), evidence.to_pretty())
                        .map_err(error)?;
                    launch.env.insert("WAYLAND_DISPLAY".into(), display);
                    Ok(true)
                }
                (monitors, tree) => {
                    last = format!("monitors={monitors:?}; parent={tree:?}");
                    Ok(false)
                }
            }
        })
        .map_err(|err| format!("{err}; {last}"))?;
        let version = nested.call(launch, deadline, &["version"])?;
        fs::write(launch.out.join("hypr-version.txt"), version).map_err(error)
    }

    fn query(&self, launch: &Headless, deadline: Instant, args: &[&str]) -> Result<Value> {
        Value::parse(&self.call(launch, probe(deadline), &[["-j"].as_slice(), args].concat())?)
    }

    fn call(&self, launch: &Headless, deadline: Instant, args: &[&str]) -> Result<String> {
        let instance = ["-i", self.signature.as_str()];
        terminals::call(
            launch,
            deadline,
            &self.tools.hyprctl,
            &[instance.as_slice(), args].concat(),
        )
    }

    pub(super) fn fullscreen(&self, launch: &Headless, deadline: Instant) -> Option<bool> {
        let window = self.query(launch, deadline, &["activewindow"]).ok()?;
        (window.get("class")?.as_str()? == "eon")
            .then_some(window.get("fullscreen")?.as_u64()? == 2)
    }

    pub(super) fn resized(&self, launch: &Headless, deadline: Instant, size: OutputSize) -> bool {
        terminals::sway(launch, probe(deadline), &["-t", "get_outputs", "-r"])
            .is_ok_and(|outputs| size.check_outputs(&outputs).is_ok())
            && self
                .query(launch, deadline, &["monitors", "all"])
                .is_ok_and(|outputs| monitor(&outputs, size))
    }

    pub(super) fn dispatch(&self, launch: &Headless, deadline: Instant, command: &str) -> Result {
        let reply = self.call(launch, deadline, &["dispatch", command])?;
        if reply.trim() != "ok" {
            return Err(format!("Hyprland rejected {command}: {reply}"));
        }
        Ok(())
    }

    pub(super) fn capture(&self, launch: &mut Headless, deadline: Instant) -> Result {
        let path = launch.out.join("nested-1280x800.png");
        let path = path.to_str().ok_or("non-UTF8 capture path")?;
        let child = launch
            .env
            .insert("WAYLAND_DISPLAY".into(), self.parent.clone())
            .ok_or("missing child display")?;
        let result = terminals::call(launch, deadline, Path::new("grim"), &[path]);
        launch.env.insert("WAYLAND_DISPLAY".into(), child);
        result.map(|_| ())
    }

    pub(super) fn stop(&self, launch: &mut Headless) -> Result {
        if self.signature.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        let dispatched = self.dispatch(launch, deadline, "hl.dsp.exit()");
        let child = &mut launch.processes.children[self.child];
        loop {
            if let Some(exit) = child.try_wait().map_err(error)? {
                return if exit.success() {
                    dispatched
                } else {
                    Err(format!("Hyprland exited with {exit}"))
                };
            }
            if Instant::now() >= deadline {
                return Err("Hyprland did not exit in 15s".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(target_os = "linux")]
fn probe(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + Duration::from_millis(500))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_one_focused_wayland_monitor_at_exact_size() {
        let expected = r#"{"width":1280,"height":800,"scale":1,"disabled":false,"focused":true,"hardwareDetails":{"backend":"wayland"}}"#;
        let check = |rows: &str| {
            monitor(
                &Value::parse(&format!("[{rows}]")).unwrap(),
                OutputSize::DEFAULT,
            )
        };
        assert!(check(expected));
        for (from, to) in [
            ("1280", "1279"),
            ("800", "760"),
            ("scale\":1", "scale\":2"),
            ("false", "true"),
            ("focused\":true", "focused\":false"),
            ("wayland", "drm"),
        ] {
            assert!(!check(&expected.replace(from, to)), "{from}");
        }
        assert!(!check(""));
        assert!(!check(&format!("{expected},{expected}")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preflight_reports_a_failed_render_node_open() {
        let failure = open_render_node(&std::env::temp_dir()).unwrap_err();
        assert!(failure.contains("read/write") && failure.contains(NESTED_HINT));
    }

    #[test]
    fn rejects_software_or_missing_gl_renderer() {
        for renderer in ["llvmpipe (LLVM 21)", "Softpipe"] {
            let failure = gl_renderer(&format!("DEBUG ]: [EGL] Renderer: {renderer}")).unwrap_err();
            assert!(failure.contains("software GL renderer"));
        }
        assert!(gl_renderer("No renderer reported").is_err());
        let hardware = "Mesa Intel(R) Graphics (RPL-S)";
        assert_eq!(
            gl_renderer(&format!("DEBUG ]: [EGL] Renderer: {hardware}")),
            Ok(hardware)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_unique_sockets_and_rejects_noise() {
        use std::os::unix::{fs::symlink, net::UnixListener};
        let dir = std::env::temp_dir().join(format!("bench-hypr-{}", std::process::id()));
        fs::create_dir_all(dir.join("hypr/a")).unwrap();
        fs::create_dir(dir.join("hypr/b")).unwrap();
        let bind = |name| UnixListener::bind(dir.join(name)).unwrap();
        let _parent = bind("wayland-1");
        fs::write(dir.join("wayland-2.lock"), "").unwrap();
        symlink("wayland-1", dir.join("wayland-4")).unwrap();
        assert_eq!(display(&dir, "wayland-1").unwrap(), None);
        assert_eq!(instance(&dir).unwrap(), None);
        let _child = bind("wayland-2");
        let _ipc = bind("hypr/a/.socket.sock");
        assert_eq!(
            display(&dir, "wayland-1").unwrap().as_deref(),
            Some("wayland-2")
        );
        assert_eq!(instance(&dir).unwrap().as_deref(), Some("a"));
        let _extra = bind("wayland-3");
        let _extra_ipc = bind("hypr/b/.socket.sock");
        assert!(display(&dir, "wayland-1").is_err());
        assert!(instance(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
