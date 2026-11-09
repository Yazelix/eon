//! A recorded startup/stop check of the shared launcher, not a timing suite.
use crate::{
    environment, isolation,
    json::Value,
    launcher::{BusPolicy, Headless, Renderer, WindowMode},
    suite,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::{fs::DirBuilderExt, net::UnixDatagram},
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, String>;
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub const HELP: &str = "\
eon-bench headless --eon PATH --out DIR [--renderer lavapipe|/dev/dri/renderD<N>]

Start and stop an out-link-built Nix Eon candidate on the shared private Sway.
Linux only. DIR must not exist. Writes result.json, trace.tsv and process logs.
Default renderer: lavapipe (EON_BENCH_LAVAPIPE names the pinned Vulkan ICD).
Hardware exposes only the named render node and rejects a software fallback.
Sway uses pixman for lavapipe, GLES for hardware, at 1280x800. The result
records Venus's actual GPU separately.
Requires bubblewrap, sway, swaymsg, dbus-daemon, sleep and timeout;
the Nix bench package supplies them. Own PID/network namespace, read-only
host root, private /tmp, /run, HOME, D-Bus and Eon root; never the live desktop.
Exit 0: presented terminal, expected renderer/output and private Stop succeeded;
1: startup/validation/stop failed; 2: invalid arguments. No timing claim.";

pub struct Options {
    pub(crate) eon: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) renderer: Renderer,
}
pub fn options(args: &[String]) -> Result<Options> {
    let mut values = BTreeMap::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if matches!(flag.as_str(), "--help" | "-h") {
            println!("{HELP}");
            return Err(String::new());
        }
        if !matches!(flag.as_str(), "--eon" | "--out" | "--renderer") {
            return Err(format!("unknown argument {flag}\n{HELP}"));
        }
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        if values.insert(flag.as_str(), value.as_str()).is_some() {
            return Err(format!("{flag} is repeated"));
        }
    }
    let result = Options {
        eon: values.get("--eon").ok_or("--eon is required")?.into(),
        out: values.get("--out").ok_or("--out is required")?.into(),
        renderer: Renderer(
            values
                .get("--renderer")
                .copied()
                .unwrap_or("lavapipe")
                .into(),
        ),
    };
    result.renderer.render_node()?;
    Ok(result)
}

pub fn run(args: &[String], options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("headless requires Linux/Wayland".into());
    }
    let eon = crate::launcher::candidate(&options.eon)?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.out)
        .map_err(error)?;
    let out = fs::canonicalize(&options.out).map_err(error)?;
    let mut arguments = args.to_vec();
    let index = arguments
        .iter()
        .position(|s| s == "--out")
        .ok_or("missing --out")?;
    arguments[index + 1] = out.to_str().ok_or("non-UTF8 output path")?.into();
    let index = arguments
        .iter()
        .position(|s| s == "--eon")
        .ok_or("missing --eon")?;
    arguments[index + 1] = eon.to_str().ok_or("non-UTF8 candidate path")?.into();
    let exe = std::env::current_exe().map_err(error)?;
    let mut command = isolation::command(&out, &[&exe]);
    isolation::display(&mut command, options.renderer.render_node()?);
    command
        .arg("--")
        .arg(exe)
        .arg("headless-inner")
        .args(arguments);
    let result = isolation::run(&mut command);
    suite::interrupt::resend();
    result
}

fn call(launch: &Headless, program: &str, args: &[&str]) -> Result<String> {
    let result = launch
        .command("timeout")
        .arg("10s")
        .arg(program)
        .args(args)
        .output()
        .map_err(error)?;
    if !result.status.success() {
        return Err(format!(
            "{program}: {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(error)
}

pub fn run_inner(options: &Options) -> Result {
    let mut launch = Headless::new(&options.out, &options.eon, WindowMode::Fullscreen)?;
    options.renderer.apply(&mut launch)?;
    let trace_path = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]).join("trace");
    let trace = UnixDatagram::bind(&trace_path).map_err(error)?;
    trace
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(error)?;
    launch
        .env
        .insert("EON_BENCH_TRACE".into(), trace_path.display().to_string());
    let sleep = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("sleep"))
        .find(|p| p.is_file())
        .ok_or("sleep is missing")?;
    let argv = Value::Array(vec![sleep.display().to_string().into(), "600".into()]).to_pretty();
    fs::write(launch.out.join("config/config.toml"), format!("[shell]\ncommand = {}\n[anima]\nenabled = false\n[terminal]\nbackground_opacity = 1.0\n", argv.trim())).map_err(error)?;
    let versions = call(
        &launch,
        launch.eon.to_str().ok_or("non-UTF8 candidate")?,
        &["versions"],
    )?;
    fs::write(launch.out.join("versions.txt"), versions).map_err(error)?;
    let mut errors = Vec::new();
    let load_start = metadata(environment::load(), "load_start", &mut errors);
    let compositor_version = metadata(
        call(&launch, "sway", &["--version"]).map(|s| s.trim().to_owned()),
        "sway --version",
        &mut errors,
    );
    let mut gpu = None;
    let mut outputs = Value::Null;
    let observation = (|| -> Result {
        launch.start_compositor(BusPolicy::Private)?;
        outputs = Value::parse(&call(&launch, "swaymsg", &["-t", "get_outputs", "-r"])?)?;
        let active = outputs
            .as_array()
            .ok_or("Sway returned no outputs")?
            .iter()
            .filter(|o| o.get("active").and_then(Value::as_bool) == Some(true))
            .collect::<Vec<_>>();
        if active.len() != 1
            || active[0].get("current_mode").is_none_or(|m| {
                m.get("width").and_then(Value::as_u64) != Some(1280)
                    || m.get("height").and_then(Value::as_u64) != Some(800)
            })
        {
            return Err("private output is not one 1280x800 mode".into());
        }
        launch.start_eon()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut log = String::new();
        let mut dismissed = false;
        let mut bytes = [0; 4097];
        let result = loop {
            if Instant::now() >= deadline {
                break Err("no presented terminal/GPU within 30s".into());
            }
            match trace.recv(&mut bytes) {
                Ok(n) => {
                    let text = std::str::from_utf8(&bytes[..n]).map_err(error)?;
                    log.push_str(text);
                    log.push('\n');
                    let fields: Vec<_> = text.splitn(7, '\t').collect();
                    if fields.len() != 7 || fields[0] != "venus" {
                        continue;
                    }
                    match fields[5] {
                        "gpu" => {
                            gpu = Some(fields[6].to_owned());
                        }
                        "present" if !dismissed => {
                            if let Some(popup) = fields[6]
                                .split_whitespace()
                                .find_map(|s| s.strip_prefix("popup=").filter(|p| *p != "-"))
                            {
                                call(
                                    &launch,
                                    launch.eon.to_str().ok_or("non-UTF8 candidate")?,
                                    &["popup", "hide", popup, "--json"],
                                )?;
                                dismissed = true;
                            }
                        }
                        "present_revision"
                            if dismissed && !fields[6].ends_with(" None") && gpu.is_some() =>
                        {
                            break Ok(());
                        }
                        _ => {}
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => break Err(error(e)),
            }
        };
        fs::write(launch.out.join("trace.tsv"), log).map_err(error)?;
        result?;
        let software = gpu
            .as_deref()
            .is_some_and(|g| g.starts_with("software=true "));
        if software != (options.renderer.0 == "lavapipe")
            || (software
                && !gpu
                    .as_deref()
                    .is_some_and(|g| g.contains("backend: Vulkan")))
        {
            return Err(format!(
                "requested {} but observed {gpu:?}",
                options.renderer.0
            ));
        }
        Ok(())
    })();
    let stopped = launch.stop_eon();
    let cleaned = launch.processes.cleanup();
    for outcome in [observation, stopped] {
        if let Err(error) = outcome {
            errors.push(error);
        }
    }
    if !cleaned {
        errors.push("private processes still live".into());
    }
    let load_end = metadata(environment::load(), "load_end", &mut errors);
    let outcome = if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    };
    let object = Value::Object(vec![
        ("schema".into(), 1u64.into()),
        ("suite".into(), "headless".into()),
        ("eon".into(), launch.eon.display().to_string().into()),
        (
            "eon_root".into(),
            launch.env["EON_RUNTIME_DIR"].as_str().into(),
        ),
        (
            "runtime".into(),
            launch.env["XDG_RUNTIME_DIR"].as_str().into(),
        ),
        ("compositor_version".into(), compositor_version.into()),
        ("kernel".into(), environment::kernel().into()),
        ("compositor".into(), "Sway headless".into()),
        (
            "compositor_renderer".into(),
            launch.env["WLR_RENDERER"].as_str().into(),
        ),
        (
            "requested_renderer".into(),
            options.renderer.0.as_str().into(),
        ),
        ("venus_gpu".into(), gpu.into()),
        (
            "vulkan_icd".into(),
            launch.env.get("VK_DRIVER_FILES").cloned().into(),
        ),
        ("outputs".into(), outputs),
        ("load_start".into(), load_start.into()),
        ("load_end".into(), load_end.into()),
        (
            "result".into(),
            if outcome.is_ok() { "pass" } else { "fail" }.into(),
        ),
        ("error".into(), outcome.as_ref().err().cloned().into()),
    ]);
    fs::write(launch.out.join("result.json"), object.to_pretty()).map_err(error)?;
    outcome
}

fn metadata<T>(result: Result<T>, name: &str, errors: &mut Vec<String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{name}: {error}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_load_readings_are_retained_with_the_other_errors() {
        let mut errors = vec!["startup failed".into()];
        assert_eq!(
            metadata::<f64>(Err("unavailable".into()), "load_start", &mut errors),
            None
        );
        assert_eq!(
            metadata::<f64>(Err("unavailable".into()), "load_end", &mut errors),
            None
        );
        assert_eq!(
            errors,
            [
                "startup failed",
                "load_start: unavailable",
                "load_end: unavailable"
            ]
        );
    }
    #[test]
    fn options_reject_ambient_display_devices_and_ambiguous_flags() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let defaults = options(&args("--eon /candidate --out /evidence")).unwrap();
        assert_eq!(defaults.renderer.0, "lavapipe");
        for bad in [
            "--eon /candidate",
            "--out /evidence",
            "--eon /candidate --out /evidence --out /other",
            "--eon /candidate --out /evidence --renderer /dev/dri/card0",
            "--eon /candidate --out /evidence --renderer hardware",
            "--eon /candidate --out /evidence --display wayland-0",
        ] {
            assert!(options(&args(bad)).is_err(), "{bad}");
        }
    }
}
